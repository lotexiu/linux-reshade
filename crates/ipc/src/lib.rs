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
mod channel;
pub mod plan;
pub use clock::clock;
mod status;

pub use blob::{MAX_PARAM_BYTES, MAX_SELECTION_BYTES, PublishError};
pub use channel::{ChannelReader, ChannelSelection, ChannelWriter, NO_EFFECT, PlanRead, channel_name, prune as prune_channels, prune_in as prune_channels_in};
pub use plan::{MAX_PLAN_BYTES, Plan, PlanError, PlanStep};
pub use status::{DepthStatus, EffectState, LayerStatus, LiveLayer, StatusWriter, SwapchainStatus, TechniqueState, TechniqueStatus, live_layers};

use blob::{Params, Selection, Snapshot};
use rustix::mm::{MapFlags, ProtFlags, mmap, munmap};
use std::ffi::c_void;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicU32, Ordering};

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

/// O marcador "este jogo precisa da profundidade" (`depth-arm-<jogo>`): o app de controle o cria quando o preset do
/// jogo tem efeitos que a usam, e a layer o lê ao criar o dispositivo (antes de o jogo criar as imagens), para armar a
/// observação sem a pessoa precisar de `LINUX_RESHADE_DEPTH=1`.
fn depth_arm_file(dir: &std::path::Path, game: &str) -> Option<PathBuf> {
    (!game.is_empty() && !game.starts_with('.') && !game.contains(['/', '\\', '\0'])).then(|| dir.join(format!("depth-arm-{game}")))
}

pub fn depth_arm_wanted_in(dir: &std::path::Path, game: &str) -> bool {
    depth_arm_file(dir, game).is_some_and(|f| f.exists())
}

pub fn depth_arm_wanted(game: &str) -> bool {
    depth_arm_wanted_in(&state_dir(), game)
}

/// Cria ou remove o marcador; não mexe no disco se já está como se pede.
pub fn set_depth_arm_in(dir: &std::path::Path, game: &str, on: bool) -> io::Result<()> {
    let Some(file) = depth_arm_file(dir, game) else { return Ok(()) };
    match (on, file.exists()) {
        (true, false) => {
            ensure_private_dir(dir)?;
            fs::write(file, b"")
        }
        (false, true) => fs::remove_file(file),
        _ => Ok(()),
    }
}

/// O controle que a pessoa está editando no painel, para os efeitos que mostram um auxílio visual enquanto se edita
/// (`source = "overlay_active"`, como os pontos do autofoco do Depth of Field). O arquivo guarda o efeito e o uniform; vale
/// por pouco tempo depois da última edição.
fn overlay_file(dir: &std::path::Path, game: &str) -> Option<PathBuf> {
    (!game.is_empty() && !game.starts_with('.') && !game.contains(['/', '\\', '\0'])).then(|| dir.join(format!("overlay-{game}")))
}

pub fn set_overlay_in(dir: &std::path::Path, game: &str, effect: &str, uniform: &str) -> io::Result<()> {
    let Some(file) = overlay_file(dir, game) else { return Ok(()) };
    ensure_private_dir(dir)?;
    fs::write(file, format!("{effect}\n{uniform}\n"))
}

pub fn set_overlay(game: &str, effect: &str, uniform: &str) -> io::Result<()> {
    set_overlay_in(&state_dir(), game, effect, uniform)
}

/// `(efeito, uniform)` editado nos últimos `max_age`; `None` se faz mais tempo (ou nunca).
pub fn overlay_in(dir: &std::path::Path, game: &str, max_age: std::time::Duration) -> Option<(String, String)> {
    let file = overlay_file(dir, game)?;
    let age = fs::metadata(&file).and_then(|m| m.modified()).ok()?.elapsed().unwrap_or_default();
    if age > max_age {
        return None;
    }
    let text = fs::read_to_string(file).ok()?;
    let mut lines = text.lines();
    Some((lines.next()?.to_string(), lines.next()?.to_string()))
}

pub fn overlay(game: &str, max_age: std::time::Duration) -> Option<(String, String)> {
    overlay_in(&state_dir(), game, max_age)
}

pub fn set_depth_arm(game: &str, on: bool) -> io::Result<()> {
    set_depth_arm_in(&state_dir(), game, on)
}

/// O pedido da pessoa para a profundidade de um jogo: a layer lê o arquivo `depth-pick-<id da layer>` (ver `docs/depth.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DepthPick {
    /// A heurística da layer.
    #[default]
    Auto,
    /// A N-ésima candidata (de 1) da lista que a layer publica.
    Candidate(u32),
    /// A profundidade logo depois do N-ésimo passe (de 1) que a desenha: a layer faz uma cópia nesse ponto.
    Pass(u32),
    /// A profundidade logo antes da N-ésima limpeza (de 1) dela no quadro, como o "copiar antes de limpar" do ReShade.
    Clear(u32),
}

impl DepthPick {
    /// Como se grava no `game.ini` (`pass:2`, `candidate:1`); `None` para a automática.
    pub fn text(self) -> Option<String> {
        match self {
            DepthPick::Auto => None,
            DepthPick::Candidate(n) => Some(format!("candidate:{n}")),
            DepthPick::Pass(n) => Some(format!("pass:{n}")),
            DepthPick::Clear(n) => Some(format!("clear:{n}")),
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        let (kind, n) = text.trim().split_once(':')?;
        let n: u32 = n.trim().parse().ok().filter(|n| *n > 0)?;
        match kind.trim() {
            "candidate" => Some(DepthPick::Candidate(n)),
            "pass" => Some(DepthPick::Pass(n)),
            "clear" => Some(DepthPick::Clear(n)),
            _ => None,
        }
    }
}

fn depth_pick_file(dir: &std::path::Path, layer_id: &str) -> Option<PathBuf> {
    (!layer_id.is_empty() && !layer_id.contains(['/', '\\', '\0'])).then(|| dir.join(format!("depth-pick-{layer_id}")))
}

pub fn set_depth_pick_in(dir: &std::path::Path, layer_id: &str, pick: DepthPick) -> io::Result<()> {
    let Some(path) = depth_pick_file(dir, layer_id) else { return Ok(()) };
    match pick {
        DepthPick::Auto => match fs::remove_file(&path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
        DepthPick::Candidate(n) => {
            ensure_private_dir(dir)?;
            fs::write(path, format!("candidate={n}\n"))
        }
        DepthPick::Pass(n) => {
            ensure_private_dir(dir)?;
            fs::write(path, format!("pass={n}\n"))
        }
        DepthPick::Clear(n) => {
            ensure_private_dir(dir)?;
            fs::write(path, format!("clear={n}\n"))
        }
    }
}

pub fn set_depth_pick(layer_id: &str, pick: DepthPick) -> io::Result<()> {
    set_depth_pick_in(&state_dir(), layer_id, pick)
}

/// O que a pessoa pediu (o que o arquivo diz; `Auto` se não há arquivo).
pub fn depth_pick_in(dir: &std::path::Path, layer_id: &str) -> DepthPick {
    let Some(path) = depth_pick_file(dir, layer_id) else { return DepthPick::Auto };
    let text = fs::read_to_string(path).unwrap_or_default();
    for line in text.lines() {
        match line.split_once('=') {
            Some(("candidate", n)) => return n.trim().parse().map_or(DepthPick::Auto, DepthPick::Candidate),
            Some(("pass", n)) => return n.trim().parse().map_or(DepthPick::Auto, DepthPick::Pass),
            Some(("clear", n)) => return n.trim().parse().map_or(DepthPick::Auto, DepthPick::Clear),
            _ => {}
        }
    }
    DepthPick::Auto
}

pub fn depth_pick(layer_id: &str) -> DepthPick {
    depth_pick_in(&state_dir(), layer_id)
}

/// Cria o diretório do estado (se preciso) e garante que é do usuário atual e privado.
pub(crate) fn ensure_private_dir(dir: &std::path::Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let meta = fs::metadata(dir)?;
    if meta.uid() != rustix::process::getuid().as_raw() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} pertence a outro usuário", dir.display()),
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
            return Err(io::Error::other("versão do estado incompatível; apague o arquivo"));
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
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "estado pertence a outro usuário"));
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
            return Err(io::Error::other("versão do estado incompatível"));
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

#[cfg(test)]
mod tests {
    #[test]
    fn the_depth_pick_text_round_trips_and_rejects_garbage() {
        use super::*;
        for pick in [DepthPick::Candidate(3), DepthPick::Pass(2), DepthPick::Clear(2)] {
            assert_eq!(DepthPick::parse(&pick.text().unwrap()), Some(pick));
        }
        assert_eq!(DepthPick::Auto.text(), None);
        for bad in ["", "pass", "pass:0", "pass:x", "outra:1", "candidate:-1"] {
            assert_eq!(DepthPick::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_depth_pick_round_trips_through_its_file() {
        use super::*;
        let dir = std::env::temp_dir().join(format!("lr-pick-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(depth_pick_in(&dir, "layer-1"), DepthPick::Auto);
        set_depth_pick_in(&dir, "layer-1", DepthPick::Candidate(2)).unwrap();
        assert_eq!(depth_pick_in(&dir, "layer-1"), DepthPick::Candidate(2));
        set_depth_pick_in(&dir, "layer-1", DepthPick::Pass(5)).unwrap();
        assert_eq!(depth_pick_in(&dir, "layer-1"), DepthPick::Pass(5));
        set_depth_pick_in(&dir, "layer-1", DepthPick::Auto).unwrap();
        assert_eq!(depth_pick_in(&dir, "layer-1"), DepthPick::Auto);
        assert_eq!(depth_pick_in(&dir, "../x"), DepthPick::Auto);
        let _ = fs::remove_dir_all(&dir);
    }

    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("linux-reshade-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir.join("state")
    }

    #[test]
    fn reader_sees_nothing_before_writer_exists() {
        let path = temp_path("absent");
        assert!(Reader::open_at(&path).unwrap().is_none());
    }

    #[test]
    fn reader_follows_writer() {
        let path = temp_path("follow");
        let writer = Writer::create_at(&path).unwrap();
        let reader = Reader::open_at(&path).unwrap().expect("arquivo existe");
        assert!(!reader.effects_enabled());
        writer.set_effects_enabled(true);
        assert!(reader.effects_enabled());
        assert!(!writer.toggle_effects());
        assert!(!reader.effects_enabled());
        assert!(writer.toggle_effects());
        assert!(reader.effects_enabled());
    }

    #[test]
    fn restarting_writer_keeps_state_and_existing_reader() {
        let path = temp_path("restart");
        let first = Writer::create_at(&path).unwrap();
        first.set_effects_enabled(true);
        let reader = Reader::open_at(&path).unwrap().unwrap();
        drop(first);
        let second = Writer::create_at(&path).unwrap();
        assert!(second.effects_enabled(), "estado preservado");
        second.set_effects_enabled(false);
        assert!(!reader.effects_enabled(), "o leitor antigo continua válido");
    }

    #[test]
    fn params_travel_from_writer_to_reader_through_the_file() {
        let path = temp_path("params");
        let writer = Writer::create_at(&path).unwrap();
        let reader = Reader::open_at(&path).unwrap().unwrap();
        let mut out = [0u8; 8];
        assert_eq!(reader.read_params(9, &mut out), None, "nada publicado ainda");

        writer.publish_params(9, &[1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
        assert_eq!(reader.read_params(9, &mut out), Some(8));
        assert_eq!(out, [1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(reader.read_params(10, &mut out), None, "outro efeito");
        assert_eq!(writer.published_bundle(), 9);

        // O escritor também lê o que publicou (é como o `lrctl set` altera só um valor).
        let mut again = [0u8; 8];
        assert_eq!(writer.read_params(9, &mut again), Some(8));
        assert_eq!(again, out);
    }

    #[test]
    fn params_survive_a_writer_restart() {
        let path = temp_path("params-restart");
        let first = Writer::create_at(&path).unwrap();
        first.publish_params(3, &[7; 4]).unwrap();
        drop(first);
        let second = Writer::create_at(&path).unwrap();
        let mut out = [0u8; 4];
        assert_eq!(second.read_params(3, &mut out), Some(4));
        assert_eq!(out, [7; 4]);
    }

    #[test]
    fn an_old_smaller_state_file_is_upgraded() {
        // Um arquivo de uma versão anterior (menor, só com o estado) deixado em /dev/shm.
        let path = temp_path("old");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut old = Vec::new();
        for word in [MAGIC, 1u32, FLAG_EFFECTS_ENABLED] {
            old.extend_from_slice(&word.to_ne_bytes());
        }
        fs::write(&path, &old).unwrap();
        assert!(Reader::open_at(&path).unwrap().is_none(), "ainda pequeno demais para o layout novo");

        let writer = Writer::create_at(&path).unwrap();
        writer.publish_params(1, &[5; 4]).unwrap();
        let reader = Reader::open_at(&path).unwrap().expect("agora é do tamanho certo");
        assert!(!reader.effects_enabled(), "o arquivo antigo é reinicializado");
        assert_eq!(reader.read_params(1, &mut [0u8; 4]), Some(4));
    }

    #[test]
    fn the_selected_effect_travels_to_the_reader_and_can_be_cleared() {
        let path = temp_path("selection");
        let writer = Writer::create_at(&path).unwrap();
        let reader = Reader::open_at(&path).unwrap().unwrap();
        assert_eq!(reader.read_selection(), SelectionRead::Cleared, "nada escolhido ainda");

        let g0 = reader.selection_generation();
        writer.publish_selection("/jogos/efeitos/Sepia-1920x1080").unwrap();
        assert_eq!(reader.read_selection(), SelectionRead::Path("/jogos/efeitos/Sepia-1920x1080".into()));
        assert_eq!(writer.selection(), reader.read_selection());
        let g1 = reader.selection_generation();
        assert_ne!(g0, g1, "a geração muda ao publicar");
        assert_eq!(reader.selection_generation(), g1, "e não muda sozinha");

        writer.clear_selection();
        assert_eq!(reader.read_selection(), SelectionRead::Cleared);
        assert_ne!(reader.selection_generation(), g1, "limpar também é uma mudança");
    }

    #[test]
    fn a_path_with_non_ascii_characters_and_the_size_limit() {
        let path = temp_path("selection-utf8");
        let writer = Writer::create_at(&path).unwrap();
        let reader = Reader::open_at(&path).unwrap().unwrap();
        writer.publish_selection("/home/são josé/efeitos/Sépia-1920x1080").unwrap();
        assert_eq!(reader.read_selection(), SelectionRead::Path("/home/são josé/efeitos/Sépia-1920x1080".into()));
        let huge = "a".repeat(MAX_SELECTION_BYTES + 1);
        assert!(writer.publish_selection(&huge).is_err());
        assert_eq!(
            reader.read_selection(),
            SelectionRead::Path("/home/são josé/efeitos/Sépia-1920x1080".into()),
            "a recusa não estraga a seleção anterior"
        );
    }

    #[test]
    fn selection_and_params_are_independent() {
        let path = temp_path("independent");
        let writer = Writer::create_at(&path).unwrap();
        let reader = Reader::open_at(&path).unwrap().unwrap();
        writer.publish_params(7, &[1, 2, 3, 4]).unwrap();
        writer.publish_selection("/a").unwrap();
        writer.publish_params(7, &[5, 6, 7, 8]).unwrap();
        assert_eq!(reader.read_selection(), SelectionRead::Path("/a".into()));
        let mut out = [0u8; 4];
        assert_eq!(reader.read_params(7, &mut out), Some(4));
        assert_eq!(out, [5, 6, 7, 8]);
    }

    #[test]
    fn rejects_uninitialized_file() {
        let path = temp_path("uninit");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, vec![0u8; SIZE]).unwrap(); // tamanho certo, magic zerado
        assert!(Reader::open_at(&path).unwrap().is_none());
    }
}
