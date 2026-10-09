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

mod params;

pub use params::{MAX_PARAM_BYTES, PublishError};

use params::Params;
use rustix::mm::{MapFlags, ProtFlags, mmap, munmap};
use std::ffi::c_void;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicU32, Ordering};

const MAGIC: u32 = 0x4C52_5348; // "LRSH"
const VERSION: u32 = 2;

pub const FLAG_EFFECTS_ENABLED: u32 = 1 << 0;

/// Layout do bloco. Só atômicos: layer e app de controle acessam sem coordenação.
#[repr(C)]
struct State {
    /// Escrito por último pelo criador (Release); o leitor só confia no resto depois de vê-lo.
    magic: AtomicU32,
    version: AtomicU32,
    flags: AtomicU32,
}

/// O arquivo inteiro: estado geral e bloco de parâmetros.
#[repr(C)]
struct Shared {
    state: State,
    params: Params,
}

const SIZE: usize = size_of::<Shared>();

/// Caminho do arquivo de estado.
pub fn state_path() -> PathBuf {
    let uid = rustix::process::getuid().as_raw();
    PathBuf::from(format!("/dev/shm/linux-reshade-{uid}")).join("state")
}

/// Cria o diretório do estado (se preciso) e garante que é do usuário atual e privado.
fn ensure_private_dir(dir: &std::path::Path) -> io::Result<()> {
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

/// Mapeamento de `SIZE` bytes. `munmap` no `Drop`.
struct Mapping {
    ptr: NonNull<Shared>,
}

// SAFETY: o conteúdo só é acessado por operações atômicas.
unsafe impl Send for Mapping {}
unsafe impl Sync for Mapping {}

impl Mapping {
    fn new(file: &File, writable: bool) -> io::Result<Self> {
        let prot = if writable { ProtFlags::READ | ProtFlags::WRITE } else { ProtFlags::READ };
        // SAFETY: mapeamento novo (ptr nulo) de um arquivo com pelo menos `SIZE` bytes.
        let raw = unsafe { mmap(ptr::null_mut(), SIZE, prot, MapFlags::SHARED, file, 0) }?;
        let ptr = NonNull::new(raw.cast::<Shared>()).ok_or_else(|| io::Error::other("mmap nulo"))?;
        Ok(Self { ptr })
    }

    fn shared(&self) -> &Shared {
        // SAFETY: o mapeamento vive enquanto `self` e é alinhado (mmap devolve página inteira).
        unsafe { self.ptr.as_ref() }
    }

    fn state(&self) -> &State {
        &self.shared().state
    }

    fn params(&self) -> &Params {
        &self.shared().params
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: mapeado em `new` com este mesmo tamanho; ninguém usa depois do Drop.
        let _ = unsafe { munmap(self.ptr.as_ptr().cast::<c_void>(), SIZE) };
    }
}

/// Lado do app de controle.
pub struct Writer {
    map: Mapping,
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
        Ok(Self { map })
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
        self.map.params().published_id()
    }
}

/// Lado da layer. Somente leitura.
pub struct Reader {
    map: Mapping,
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
        self.map.params().published_id()
    }
}

#[cfg(test)]
mod tests {
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
    fn rejects_uninitialized_file() {
        let path = temp_path("uninit");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, vec![0u8; SIZE]).unwrap(); // tamanho certo, magic zerado
        assert!(Reader::open_at(&path).unwrap().is_none());
    }
}
