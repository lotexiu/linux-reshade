//! Bloco de memória compartilhada entre o app de controle e a layer.
//!
//! O app de controle cria `/dev/shm/linux-reshade-<uid>/state` e escreve; a layer só mapeia para
//! leitura, uma vez por frame, sem syscall nem lock (só `load` atômico). O arquivo nunca é apagado
//! nem recriado, então um mapeamento aberto continua válido se o app de controle reiniciar.
//!
//! Por que `/dev/shm` e não `$XDG_RUNTIME_DIR`: os containers da Steam (pressure-vessel) não expõem
//! o `/run/user/<uid>` do host, mas expõem `/dev/shm`. Como esse diretório é de todos os usuários,
//! o diretório do estado precisa ser do próprio usuário (modo 0700) e o leitor só aceita um
//! arquivo que também seja do próprio usuário.

mod blob;
mod clock;
pub mod shot;
mod channel;
pub mod plan;
pub use clock::{clock, file_stamp};
mod status;

pub use blob::{MAX_PARAM_BYTES, MAX_SELECTION_BYTES, PublishError};
pub use channel::{ChannelReader, ChannelSelection, ChannelWriter, NO_EFFECT, PlanRead, channel_name, prune as prune_channels, prune_in as prune_channels_in};
pub use plan::{MAX_PLAN_BYTES, Plan, PlanError, PlanStep};
pub use status::{hold_panel_open, panel_open};
pub use status::{DepthStatus, EffectState, LayerStatus, LiveLayer, PerfStats, StatusWriter, SwapchainStatus, TechniqueState, TechniqueStatus, live_layers};

use blob::{Params, Selection, Snapshot};
use rustix::mm::{MapFlags, ProtFlags, mmap, munmap};
use std::ffi::c_void;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicU32, Ordering};
use i18n::{fmt, tr};

pub mod daemon_lock;
mod depth;
mod markers;
#[cfg(test)]
mod tests;

pub use depth::*;
pub use markers::*;

const MAGIC: u32 = 0x4C52_5348; // "LRSH"

const VERSION: u32 = 3;

/// Etiqueta do bloco de seleção (o bloco de parâmetros usa o `bundle_id` do efeito).
const SELECTION_TAG: u64 = 1;

pub const FLAG_EFFECTS_ENABLED: u32 = 1 << 0;

/// Layout do bloco. Só atômicos: layer e app de controle acessam sem coordenação.
#[repr(C)]
struct State {
    /// Escrito por último pelo criador (Release); o leitor só confia no resto depois de vê-lo.
    magic: AtomicU32,
    version: AtomicU32,
    flags: AtomicU32,
}

/// O arquivo inteiro: estado geral, parâmetros do efeito e o efeito selecionado.
#[repr(C)]
struct Shared {
    state: State,
    params: Params,
    selection: Selection,
}

const SIZE: usize = size_of::<Shared>();

/// O que a leitura da seleção encontrou.
#[derive(Debug, PartialEq, Eq)]
pub enum SelectionRead {
    /// Nenhum efeito selecionado (nunca escolhido, ou limpo com `clear_selection`).
    Cleared,
    /// O caminho do bundle selecionado.
    Path(String),
    /// A leitura caiu no meio de uma escrita; tente de novo no próximo frame.
    Unstable,
}

pub(crate) fn read_selection(blob: &Selection) -> SelectionRead {
    let mut buf = [0u8; MAX_SELECTION_BYTES];
    match blob.snapshot(SELECTION_TAG, &mut buf) {
        Snapshot::Torn => SelectionRead::Unstable,
        Snapshot::Mismatch | Snapshot::Consistent { len: 0 } => SelectionRead::Cleared,
        Snapshot::Consistent { len } => match String::from_utf8(buf[..len].to_vec()) {
            Ok(path) => SelectionRead::Path(path),
            Err(_) => SelectionRead::Cleared, // só o nosso escritor grava aqui, e só UTF-8
        },
    }
}

/// De onde se leem os parâmetros publicados (o canal global ou o de uma layer), por qualquer dos dois lados.
pub trait ParamSource {
    /// Copia para `out` o bloco publicado para o efeito `bundle_id`. `None` se não há bloco desse efeito
    /// ou se a leitura não foi consistente.
    fn read_params(&self, bundle_id: u64, out: &mut [u8]) -> Option<usize>;
    /// Para qual efeito está publicado o bloco (0 se nenhum).
    fn published_bundle(&self) -> u64;
}

/// Onde o app de controle publica parâmetros: o canal global ou o de uma layer.
pub trait ParamStore: ParamSource {
    fn publish_params(&self, bundle_id: u64, data: &[u8]) -> Result<(), PublishError>;
}

/// Onde o app de controle publica a escolha de efeito: o canal global ou o de uma layer.
pub trait SelectionStore {
    fn publish_selection(&self, text: &str) -> Result<(), PublishError>;
}

/// Caminho do arquivo de estado.
pub fn state_path() -> PathBuf {
    state_dir().join("state")
}

/// O diretório do estado: `/dev/shm/linux-reshade-<uid>`, ou o de `LINUX_RESHADE_STATE_DIR` (os testes de ponta a ponta o usam
/// para não se misturarem com o daemon do usuário).
pub fn state_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("LINUX_RESHADE_STATE_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    let uid = rustix::process::getuid().as_raw();
    PathBuf::from(format!("/dev/shm/linux-reshade-{uid}"))
}

/// Cria o diretório do estado (se preciso) e garante que é do usuário atual e privado.
pub(crate) fn ensure_private_dir(dir: &std::path::Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let meta = fs::metadata(dir)?;
    if meta.uid() != rustix::process::getuid().as_raw() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            fmt("{} belongs to another user", &[&dir.display()]),
        ));
    }
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
}

/// Mapeamento de um `T` inteiro de um arquivo. `munmap` no `Drop`.
pub(crate) struct Mapping<T> {
    ptr: NonNull<T>,
}

// SAFETY: o conteúdo (`T`) só é acessado por operações atômicas.

unsafe impl<T> Send for Mapping<T> {}

unsafe impl<T> Sync for Mapping<T> {}

impl<T> Mapping<T> {
    pub(crate) fn new(file: &File, writable: bool) -> io::Result<Self> {
        let prot = if writable { ProtFlags::READ | ProtFlags::WRITE } else { ProtFlags::READ };
        // SAFETY: mapeamento novo (ptr nulo) de um arquivo com pelo menos `size_of::<T>()` bytes.
        let raw = unsafe { mmap(ptr::null_mut(), size_of::<T>(), prot, MapFlags::SHARED, file, 0) }?;
        let ptr = NonNull::new(raw.cast::<T>()).ok_or_else(|| io::Error::other("mmap nulo"))?;
        Ok(Self { ptr })
    }

    pub(crate) fn get(&self) -> &T {
        // SAFETY: o mapeamento vive enquanto `self` e é alinhado (mmap devolve página inteira).
        unsafe { self.ptr.as_ref() }
    }
}

impl<T> Drop for Mapping<T> {
    fn drop(&mut self) {
        // SAFETY: mapeado em `new` com este mesmo tamanho; ninguém usa depois do Drop.
        let _ = unsafe { munmap(self.ptr.as_ptr().cast::<c_void>(), size_of::<T>()) };
    }
}

impl Mapping<Shared> {
    fn state(&self) -> &State {
        &self.get().state
    }

    fn params(&self) -> &Params {
        &self.get().params
    }

    fn selection(&self) -> &Selection {
        &self.get().selection
    }
}

/// Lado do app de controle.
pub struct Writer {
    map: Mapping<Shared>,
    /// O diretório do estado: é onde ficam também os canais das layers.
    dir: PathBuf,
}

impl Writer {
    /// Cria (ou reaproveita) o arquivo de estado.
    pub fn create() -> io::Result<Self> {
        Self::create_at(&state_path())
    }

    pub fn create_at(path: &std::path::Path) -> io::Result<Self> {
        if let Some(dir) = path.parent() {
            ensure_private_dir(dir)?;
        }
        let file = OpenOptions::new().read(true).write(true).create(true).mode(0o600).open(path)?;
        let fresh = file.metadata()?.len() < SIZE as u64;
        if fresh {
            file.set_len(SIZE as u64)?;
        }
        let map = Mapping::new(&file, true)?;
        let state = map.state();
        if fresh || state.magic.load(Ordering::Acquire) != MAGIC {
            state.version.store(VERSION, Ordering::Relaxed);
            state.flags.store(0, Ordering::Relaxed);
            state.magic.store(MAGIC, Ordering::Release);
        } else if state.version.load(Ordering::Relaxed) != VERSION {
            return Err(io::Error::other(tr("incompatible state version; delete the file")));
        }
        Ok(Self { map, dir: path.parent().map(std::path::Path::to_path_buf).unwrap_or_default() })
    }

    pub fn set_effects_enabled(&self, on: bool) {
        let flags = &self.map.state().flags;
        if on {
            flags.fetch_or(FLAG_EFFECTS_ENABLED, Ordering::Release);
        } else {
            flags.fetch_and(!FLAG_EFFECTS_ENABLED, Ordering::Release);
        }
    }

    /// Inverte a flag e devolve o novo valor.
    pub fn toggle_effects(&self) -> bool {
        let prev = self.map.state().flags.fetch_xor(FLAG_EFFECTS_ENABLED, Ordering::AcqRel);
        prev & FLAG_EFFECTS_ENABLED == 0
    }

    pub fn effects_enabled(&self) -> bool {
        self.map.state().flags.load(Ordering::Acquire) & FLAG_EFFECTS_ENABLED != 0
    }

    /// Publica a imagem do buffer de uniforms do efeito `bundle_id` (ver `fx_bundle::Bundle::id`).
    pub fn publish_params(&self, bundle_id: u64, data: &[u8]) -> Result<(), PublishError> {
        self.map.params().publish(bundle_id, data)
    }

    /// O bloco atualmente publicado para `bundle_id`, para o app de controle alterar só um valor.
    pub fn read_params(&self, bundle_id: u64, out: &mut [u8]) -> Option<usize> {
        self.map.params().read(bundle_id, out)
    }

    /// Para qual efeito está publicado o bloco de parâmetros (0 se nenhum).
    pub fn published_bundle(&self) -> u64 {
        self.map.params().published_tag()
    }

    /// Escolhe o efeito que a layer deve carregar: o caminho (de preferência absoluto) do bundle.
    pub fn publish_selection(&self, bundle_dir: &str) -> Result<(), PublishError> {
        self.map.selection().publish(SELECTION_TAG, bundle_dir.as_bytes())
    }

    /// Desfaz a escolha: a layer volta a usar o que vem do ambiente (`LINUX_RESHADE_BUNDLE`), se houver.
    pub fn clear_selection(&self) {
        // Um bloco vazio sempre cabe.
        let _ = self.map.selection().publish(SELECTION_TAG, &[]);
    }

    /// O efeito atualmente selecionado, se houver.
    pub fn selection(&self) -> SelectionRead {
        read_selection(self.map.selection())
    }

    /// O canal de uma layer (a escolha e os parâmetros só dela), criado se ainda não existe.
    pub fn channel(&self, layer_id: &str) -> io::Result<ChannelWriter> {
        ChannelWriter::create_in(&self.dir, layer_id)
    }

    /// A escolha própria de uma layer, sem criar o canal: `Inherit` se ela nunca teve uma.
    pub fn layer_selection(&self, layer_id: &str) -> ChannelSelection {
        match ChannelReader::open_in(&self.dir, layer_id) {
            Ok(Some(channel)) => channel.read_selection(),
            _ => ChannelSelection::Inherit,
        }
    }

    /// Apaga os canais de jogos que já fecharam.
    pub fn prune_channels(&self) -> usize {
        prune_channels_in(&self.dir)
    }
}

impl ParamSource for Writer {
    fn read_params(&self, bundle_id: u64, out: &mut [u8]) -> Option<usize> {
        Writer::read_params(self, bundle_id, out)
    }

    fn published_bundle(&self) -> u64 {
        Writer::published_bundle(self)
    }
}

impl ParamStore for Writer {
    fn publish_params(&self, bundle_id: u64, data: &[u8]) -> Result<(), PublishError> {
        Writer::publish_params(self, bundle_id, data)
    }
}

impl SelectionStore for Writer {
    fn publish_selection(&self, text: &str) -> Result<(), PublishError> {
        Writer::publish_selection(self, text)
    }
}

/// Lado da layer. Somente leitura.
pub struct Reader {
    map: Mapping<Shared>,
}

impl Reader {
    /// `Ok(None)` se o app de controle ainda não criou o arquivo.
    pub fn open() -> io::Result<Option<Self>> {
        Self::open_at(&state_path())
    }

    pub fn open_at(path: &std::path::Path) -> io::Result<Option<Self>> {
        let file = match File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let meta = file.metadata()?;
        if meta.uid() != rustix::process::getuid().as_raw() {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, tr("state belongs to another user")));
        }
        if meta.len() < SIZE as u64 {
            return Ok(None); // o criador ainda não dimensionou
        }
        let map = Mapping::new(&file, false)?;
        let state = map.state();
        if state.magic.load(Ordering::Acquire) != MAGIC {
            return Ok(None); // o criador ainda não terminou de inicializar
        }
        if state.version.load(Ordering::Relaxed) != VERSION {
            return Err(io::Error::other(tr("incompatible state version")));
        }
        Ok(Some(Self { map }))
    }

    pub fn effects_enabled(&self) -> bool {
        self.map.state().flags.load(Ordering::Acquire) & FLAG_EFFECTS_ENABLED != 0
    }

    /// Copia para `out` o bloco de parâmetros publicado para o efeito `bundle_id`. `None` se não há
    /// bloco desse efeito ou se a leitura não foi consistente (tente no próximo frame).
    pub fn read_params(&self, bundle_id: u64, out: &mut [u8]) -> Option<usize> {
        self.map.params().read(bundle_id, out)
    }

    /// Para qual efeito está publicado o bloco de parâmetros (0 se nenhum). Permite distinguir "o bloco
    /// é de outro efeito" de "a leitura caiu no meio de uma escrita".
    pub fn published_bundle(&self) -> u64 {
        self.map.params().published_tag()
    }

    /// Muda a cada vez que o efeito selecionado é publicado. Uma leitura barata, para a layer olhar a
    /// cada frame e só ler o caminho quando algo mudou.
    pub fn selection_generation(&self) -> u32 {
        self.map.selection().generation()
    }

    /// O efeito selecionado pelo app de controle.
    pub fn read_selection(&self) -> SelectionRead {
        read_selection(self.map.selection())
    }
}

impl ParamSource for Reader {
    fn read_params(&self, bundle_id: u64, out: &mut [u8]) -> Option<usize> {
        Reader::read_params(self, bundle_id, out)
    }

    fn published_bundle(&self) -> u64 {
        Reader::published_bundle(self)
    }
}
