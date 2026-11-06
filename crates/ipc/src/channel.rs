//! O canal de **uma** layer: a escolha de efeito e os parâmetros que valem só para aquele jogo.
//!
//! Cada processo com a layer tem o arquivo de status `layer-<pid>-<instante>`; o canal dele é
//! `ctl-<pid>-<instante>`, no mesmo diretório. Quem o cria é o app de controle (a layer só mapeia para
//! leitura, como no canal global) e o contrato é o mesmo: dois blobs seqlock, a escolha e os parâmetros.
//!
//! A layer consulta o canal **antes** do global: sem escolha própria ela segue a global (`Inherit`),
//! e `NoEffect` desliga o efeito só neste jogo. O arquivo de um jogo morto é apagado por [`prune_in`].

use crate::blob::{Params, PlanBlob, PublishError, Selection, Snapshot};
use crate::plan::{MAX_PLAN_BYTES, Plan};
use crate::{Mapping, ParamSource, ParamStore, SelectionRead, SelectionStore, ensure_private_dir, read_selection, state_dir};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime};
use i18n::{fmt, tr};

const MAGIC: u32 = 0x4C52_4348; // "LRCH"
const VERSION: u32 = 2;
const SELECTION_TAG: u64 = 1;
const PLAN_TAG: u64 = 2;
const LAYER_PREFIX: &str = "layer-";
const CHANNEL_PREFIX: &str = "ctl-";
/// Um canal mais novo que isto não é apagado por órfão: o status da layer pode estar sendo recriado.
const YOUNG: Duration = Duration::from_secs(2);

/// O texto publicado na escolha para dizer "sem efeito neste jogo" (nenhum caminho começa com `!`: os
/// que o app de controle publica são absolutos).
pub const NO_EFFECT: &str = "!none";

#[repr(C)]
struct ChannelFile {
    magic: AtomicU32,
    version: AtomicU32,
    params: Params,
    selection: Selection,
    plan: PlanBlob,
}

const SIZE: usize = size_of::<ChannelFile>();

/// O que o canal de uma layer diz sobre a escolha de efeito.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelSelection {
    /// Nada próprio: vale o que o canal global escolheu.
    Inherit,
    /// Sem efeito neste jogo, mesmo que haja um escolhido globalmente.
    NoEffect,
    /// O modelo de caminho do bundle (como no canal global).
    Path(String),
    /// A leitura caiu no meio de uma escrita; tente de novo no próximo frame.
    Unstable,
}

fn decode(read: SelectionRead) -> ChannelSelection {
    match read {
        SelectionRead::Cleared => ChannelSelection::Inherit,
        SelectionRead::Path(p) if p == NO_EFFECT => ChannelSelection::NoEffect,
        SelectionRead::Path(p) => ChannelSelection::Path(p),
        SelectionRead::Unstable => ChannelSelection::Unstable,
    }
}

/// O que o canal diz sobre o plano do jogo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanRead {
    /// Nenhum plano publicado: vale a escolha única de efeito, como antes.
    None,
    /// O plano (pode ter zero etapas: o jogo roda sem efeitos).
    Plan(Plan),
    /// A leitura caiu no meio de uma escrita, ou o conteúdo não é um plano; tente no próximo frame.
    Unstable,
}

fn read_plan(blob: &PlanBlob) -> PlanRead {
    let mut buf = vec![0u8; MAX_PLAN_BYTES];
    match blob.snapshot(PLAN_TAG, &mut buf) {
        Snapshot::Torn => PlanRead::Unstable,
        Snapshot::Mismatch | Snapshot::Consistent { len: 0 } => PlanRead::None,
        Snapshot::Consistent { len } => Plan::decode(&buf[..len]).map_or(PlanRead::Unstable, PlanRead::Plan),
    }
}

/// O nome do arquivo do canal de uma layer (`layer-1-2` → `ctl-1-2`). `None` se o id não é de uma layer
/// ou tem caracteres que poderiam sair do diretório.
pub fn channel_name(layer_id: &str) -> Option<String> {
    let rest = layer_id.strip_prefix(LAYER_PREFIX)?;
    let ok = !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    ok.then(|| format!("{CHANNEL_PREFIX}{rest}"))
}

fn channel_path(dir: &Path, layer_id: &str) -> io::Result<PathBuf> {
    let name = channel_name(layer_id).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, fmt("invalid layer id: {}", &[&format!("{:?}", layer_id)])))?;
    Ok(dir.join(name))
}

/// Lado do app de controle.
pub struct ChannelWriter {
    map: Mapping<ChannelFile>,
}

impl ChannelWriter {
    /// Cria (ou reaproveita) o canal da layer `layer_id`.
    pub fn create(layer_id: &str) -> io::Result<Self> {
        Self::create_in(&state_dir(), layer_id)
    }

    pub fn create_in(dir: &Path, layer_id: &str) -> io::Result<Self> {
        let path = channel_path(dir, layer_id)?;
        ensure_private_dir(dir)?;
        let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).mode(0o600).open(&path)?;
        let fresh = file.metadata()?.len() < SIZE as u64;
        if fresh {
            file.set_len(SIZE as u64)?;
        }
        let map = Mapping::<ChannelFile>::new(&file, true)?;
        let f = map.get();
        if fresh || f.magic.load(Ordering::Acquire) != MAGIC {
            f.version.store(VERSION, Ordering::Relaxed);
            f.magic.store(MAGIC, Ordering::Release);
        } else if f.version.load(Ordering::Relaxed) != VERSION {
            return Err(io::Error::other(tr("incompatible channel version; delete the file")));
        }
        Ok(Self { map })
    }

    /// A escolha de efeito própria da layer.
    pub fn selection(&self) -> ChannelSelection {
        decode(read_selection(&self.map.get().selection))
    }

    /// Sem efeito neste jogo.
    pub fn publish_no_effect(&self) {
        let _ = self.map.get().selection.publish(SELECTION_TAG, NO_EFFECT.as_bytes());
    }

    /// Volta a seguir a escolha global.
    pub fn inherit(&self) {
        let _ = self.map.get().selection.publish(SELECTION_TAG, &[]);
    }

    /// Publica o plano do jogo (a lista de etapas que a layer executa).
    pub fn publish_plan(&self, plan: &Plan) -> Result<(), PublishError> {
        let bytes = plan.encode().map_err(|_| PublishError::TooLarge { len: usize::MAX, max: MAX_PLAN_BYTES })?;
        self.map.get().plan.publish(PLAN_TAG, &bytes)
    }

    /// Tira o plano: a layer volta à escolha única de efeito.
    pub fn clear_plan(&self) {
        let _ = self.map.get().plan.publish(PLAN_TAG, &[]);
    }

    pub fn plan(&self) -> PlanRead {
        read_plan(&self.map.get().plan)
    }

    pub fn plan_generation(&self) -> u32 {
        self.map.get().plan.generation()
    }
}

impl SelectionStore for ChannelWriter {
    fn publish_selection(&self, text: &str) -> Result<(), PublishError> {
        self.map.get().selection.publish(SELECTION_TAG, text.as_bytes())
    }
}

impl ParamSource for ChannelWriter {
    fn read_params(&self, bundle_id: u64, out: &mut [u8]) -> Option<usize> {
        self.map.get().params.read(bundle_id, out)
    }

    fn published_bundle(&self) -> u64 {
        self.map.get().params.published_tag()
    }
}

impl ParamStore for ChannelWriter {
    fn publish_params(&self, bundle_id: u64, data: &[u8]) -> Result<(), PublishError> {
        self.map.get().params.publish(bundle_id, data)
    }
}

/// Lado da layer. Somente leitura.
pub struct ChannelReader {
    map: Mapping<ChannelFile>,
}

impl ChannelReader {
    /// `Ok(None)` se o app de controle ainda não criou o canal desta layer.
    pub fn open(layer_id: &str) -> io::Result<Option<Self>> {
        Self::open_in(&state_dir(), layer_id)
    }

    pub fn open_in(dir: &Path, layer_id: &str) -> io::Result<Option<Self>> {
        let file = match File::open(channel_path(dir, layer_id)?) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let meta = file.metadata()?;
        if meta.uid() != rustix::process::getuid().as_raw() {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, tr("channel belongs to another user")));
        }
        if meta.len() < SIZE as u64 {
            return Ok(None); // o criador ainda não dimensionou
        }
        let map = Mapping::<ChannelFile>::new(&file, false)?;
        let f = map.get();
        if f.magic.load(Ordering::Acquire) != MAGIC {
            return Ok(None);
        }
        if f.version.load(Ordering::Relaxed) != VERSION {
            return Err(io::Error::other(tr("incompatible channel version")));
        }
        Ok(Some(Self { map }))
    }

    /// Muda a cada publicação da escolha: uma leitura barata para olhar a cada frame.
    pub fn selection_generation(&self) -> u32 {
        self.map.get().selection.generation()
    }

    pub fn read_selection(&self) -> ChannelSelection {
        decode(read_selection(&self.map.get().selection))
    }

    /// Muda a cada publicação do plano.
    pub fn plan_generation(&self) -> u32 {
        self.map.get().plan.generation()
    }

    pub fn read_plan(&self) -> PlanRead {
        read_plan(&self.map.get().plan)
    }
}

impl ParamSource for ChannelReader {
    fn read_params(&self, bundle_id: u64, out: &mut [u8]) -> Option<usize> {
        self.map.get().params.read(bundle_id, out)
    }

    fn published_bundle(&self) -> u64 {
        self.map.get().params.published_tag()
    }
}

/// Apaga os canais cujo status de layer não existe mais (o jogo fechou). Devolve quantos apagou.
pub fn prune() -> usize {
    prune_in(&state_dir())
}

pub fn prune_in(dir: &Path) -> usize {
    let Ok(entries) = fs::read_dir(dir) else { return 0 };
    let now = SystemTime::now();
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        // O pedido de profundidade de um jogo (`depth-pick-<id da layer>`) também some com o jogo.
        let Some(rest) = name.strip_prefix(CHANNEL_PREFIX).or_else(|| name.strip_prefix("depth-pick-layer-")) else { continue };
        if dir.join(format!("{LAYER_PREFIX}{rest}")).exists() {
            continue;
        }
        let old = entry.metadata().and_then(|m| m.modified()).ok().and_then(|t| now.duration_since(t).ok()).is_some_and(|age| age >= YOUNG);
        if old && fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lr-channel-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn names_follow_the_layer_id_and_reject_anything_else() {
        assert_eq!(channel_name("layer-123-456"), Some("ctl-123-456".into()));
        assert_eq!(channel_name("ctl-1-2"), None);
        assert_eq!(channel_name("layer-"), None);
        assert_eq!(channel_name("layer-../../etc"), None);
        assert_eq!(channel_name("layer-1/2"), None);
    }

    #[test]
    fn the_reader_waits_for_the_channel_and_then_follows_it() {
        let d = dir("follow");
        assert!(ChannelReader::open_in(&d, "layer-1-2").unwrap().is_none(), "ainda não existe");
        let writer = ChannelWriter::create_in(&d, "layer-1-2").unwrap();
        let reader = ChannelReader::open_in(&d, "layer-1-2").unwrap().expect("agora existe");
        assert_eq!(reader.read_selection(), ChannelSelection::Inherit, "nada próprio: segue o global");

        let g0 = reader.selection_generation();
        writer.publish_selection("/c/Sepia-{W}x{H}").unwrap();
        assert_eq!(reader.read_selection(), ChannelSelection::Path("/c/Sepia-{W}x{H}".into()));
        assert_eq!(writer.selection(), reader.read_selection());
        assert_ne!(reader.selection_generation(), g0);

        writer.publish_no_effect();
        assert_eq!(reader.read_selection(), ChannelSelection::NoEffect);
        writer.inherit();
        assert_eq!(reader.read_selection(), ChannelSelection::Inherit);
    }

    fn plan() -> Plan {
        Plan { steps: vec![crate::PlanStep { template: "/c/A-{W}x{H}".into(), technique: "A".into(), params_id: 5, uniforms: vec![1, 2, 3] }] }
    }

    #[test]
    fn a_plan_is_published_read_and_cleared() {
        let d = dir("plan");
        let writer = ChannelWriter::create_in(&d, "layer-1-1").unwrap();
        let reader = ChannelReader::open_in(&d, "layer-1-1").unwrap().unwrap();
        assert_eq!(reader.read_plan(), PlanRead::None, "nunca publicado");

        let g0 = reader.plan_generation();
        writer.publish_plan(&plan()).unwrap();
        assert_eq!(reader.read_plan(), PlanRead::Plan(plan()));
        assert_eq!(writer.plan(), PlanRead::Plan(plan()));
        assert_ne!(reader.plan_generation(), g0);

        // Um plano vazio é diferente de nenhum plano: o jogo roda sem efeitos.
        writer.publish_plan(&Plan::default()).unwrap();
        assert_eq!(reader.read_plan(), PlanRead::Plan(Plan::default()));
        writer.clear_plan();
        assert_eq!(reader.read_plan(), PlanRead::None);
        // E o plano não mexe na escolha única.
        assert_eq!(reader.read_selection(), ChannelSelection::Inherit);
    }

    #[test]
    fn a_plan_too_big_is_refused_and_keeps_the_old_one() {
        let d = dir("plan-big");
        let writer = ChannelWriter::create_in(&d, "layer-1-1").unwrap();
        writer.publish_plan(&plan()).unwrap();
        let big = Plan { steps: vec![crate::PlanStep { template: String::new(), technique: String::new(), params_id: 0, uniforms: vec![0; MAX_PLAN_BYTES] }] };
        assert!(writer.publish_plan(&big).is_err());
        assert_eq!(writer.plan(), PlanRead::Plan(plan()));
    }

    #[test]
    fn params_are_per_channel_and_tagged_by_bundle() {
        let d = dir("params");
        let a = ChannelWriter::create_in(&d, "layer-1-1").unwrap();
        let b = ChannelWriter::create_in(&d, "layer-2-2").unwrap();
        let reader_a = ChannelReader::open_in(&d, "layer-1-1").unwrap().unwrap();
        let reader_b = ChannelReader::open_in(&d, "layer-2-2").unwrap().unwrap();
        a.publish_params(7, &[1, 2, 3, 4]).unwrap();
        let mut out = [0u8; 4];
        assert_eq!(reader_a.read_params(7, &mut out), Some(4));
        assert_eq!(out, [1, 2, 3, 4]);
        assert_eq!(reader_b.read_params(7, &mut out), None, "o outro jogo não vê");
        assert_eq!(reader_a.read_params(8, &mut out), None, "outro efeito");
        assert_eq!(reader_a.published_bundle(), 7);
        assert_eq!(b.published_bundle(), 0);
    }

    #[test]
    fn recreating_the_writer_keeps_the_content_and_the_reader() {
        let d = dir("restart");
        let first = ChannelWriter::create_in(&d, "layer-1-1").unwrap();
        first.publish_selection("/x").unwrap();
        let reader = ChannelReader::open_in(&d, "layer-1-1").unwrap().unwrap();
        drop(first);
        let second = ChannelWriter::create_in(&d, "layer-1-1").unwrap();
        assert_eq!(second.selection(), ChannelSelection::Path("/x".into()));
        second.publish_no_effect();
        assert_eq!(reader.read_selection(), ChannelSelection::NoEffect, "o leitor antigo continua válido");
    }

    #[test]
    fn a_channel_with_another_version_is_refused() {
        let d = dir("version");
        let w = ChannelWriter::create_in(&d, "layer-1-1").unwrap();
        w.map.get().version.store(99, Ordering::Relaxed);
        assert!(ChannelReader::open_in(&d, "layer-1-1").is_err());
        assert!(ChannelWriter::create_in(&d, "layer-1-1").is_err());
    }

    #[test]
    fn prune_removes_only_channels_without_a_layer() {
        let d = dir("prune");
        for id in ["layer-1-1", "layer-2-2"] {
            ChannelWriter::create_in(&d, id).unwrap();
        }
        fs::write(d.join("layer-1-1"), "").unwrap(); // o status da primeira ainda existe
        // Recém-criados não são apagados, mesmo órfãos.
        assert_eq!(prune_in(&d), 0);
        // Envelhece os dois.
        for id in ["ctl-1-1", "ctl-2-2"] {
            let f = File::options().write(true).open(d.join(id)).unwrap();
            f.set_modified(SystemTime::now() - Duration::from_secs(60)).unwrap();
        }
        assert_eq!(prune_in(&d), 1);
        assert!(d.join("ctl-1-1").exists());
        assert!(!d.join("ctl-2-2").exists());
    }
}
