//! O que cada layer conta ao app de controle: qual jogo é, e o tamanho/formato de cada swapchain e
//! o estado do efeito nele. É o que permite ao app compilar o efeito para a resolução certa.
//!
//! Cada processo com a layer cria seu próprio arquivo `layer-<pid>-<instante>` no diretório do estado.
//! Dentro do contêiner da Steam o `pid` que a layer enxerga não é o do host, então "o processo ainda
//! existe?" não pode ser decidido por `/proc`: a layer mantém um `flock` exclusivo no arquivo enquanto
//! vive, e quem lê tenta um `flock` compartilhado. Se conseguir, ninguém o segura mais (o processo
//! morreu) e o arquivo é descartado. O sistema solta o `flock` sozinho quando o processo termina.

use crate::blob::{Blob, PublishError, Snapshot};
use crate::{Mapping, ensure_private_dir, state_dir};
use rustix::fs::{FlockOperation, flock};
use rustix::io::Errno;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAGIC: u32 = 0x4C52_5354; // "LRST"
const VERSION: u32 = 3;
/// Etiqueta do bloco de texto (só há um por arquivo).
const TAG: u64 = 1;
/// Tamanho máximo do texto de status.
pub const MAX_STATUS_BYTES: usize = 8192;
/// Quantos swapchains um status descreve (os mais recentes, se houver mais).
const MAX_SWAPCHAINS: usize = 8;
/// Quantas técnicas da cadeia o status carrega.
const MAX_TECHNIQUES: usize = 32;
/// Quantas candidatas de profundidade o status carrega.
const MAX_DEPTH: usize = 8;
/// Um arquivo mais novo que isto não é descartado mesmo sem `flock`: o dono pode ainda estar o criando.
const YOUNG: Duration = Duration::from_secs(2);

/// O estado do efeito num swapchain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectState {
    /// Nenhum efeito escolhido.
    Idle,
    /// O efeito foi escolhido, mas ainda não existe um bundle para esta resolução.
    Waiting,
    /// O efeito está construído e pronto (aparece quando a flag está ligada).
    Ready,
    /// Este swapchain não aceita o efeito (por exemplo, formato sRGB). O texto diz por quê.
    Unavailable(String),
    /// A construção do efeito falhou. O texto diz por quê.
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwapchainStatus {
    pub width: u32,
    pub height: u32,
    /// `VkFormat` como número.
    pub format: i32,
    pub images: u32,
    pub effect: EffectState,
}

/// Uma candidata a "profundidade da cena" que a layer observou no jogo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepthStatus {
    pub width: u32,
    pub height: u32,
    /// `VkFormat` como número.
    pub format: i32,
    pub samples: u32,
    pub draws: u32,
    pub vertices: u64,
    pub passes: u32,
    /// O que o último passe faz com a profundidade ao começar e ao terminar (`load`, `clear`, `dont_care`, `none`).
    pub load: String,
    pub store: String,
    /// Profundidade invertida (reversed-Z)? `None` se não deu para saber.
    pub reversed: Option<bool>,
    /// Posição do último passe dentro do quadro.
    pub last_pass: u32,
    /// A candidata em uso pelos efeitos (a da pessoa, a que a histerese mantém ou a automática).
    pub chosen: bool,
    /// A que a regra automática escolheria sozinha (pode não ser a em uso: um pedido da pessoa manda).
    pub automatic: bool,
    /// Por que não serve (MSAA, proporção...); `None` se serve.
    pub unusable: Option<String>,
    /// A imagem original pode ser amostrada (uso `SAMPLED`).
    pub sampled: bool,
    /// Layout em que ela ficou depois do último envio à fila (`VkImageLayout` como número; 0 = não sei).
    pub layout: i32,
}

/// O que aconteceu com uma técnica da cadeia no jogo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TechniqueState {
    /// Está na cadeia que a layer executa.
    Applied,
    /// A layer não a executa (o runtime ainda não suporta algo que ela pede): o motivo.
    Skipped(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TechniqueStatus {
    pub name: String,
    pub state: TechniqueState,
}

/// O desempenho medido pela layer (médias do último segundo), quando a medição está ligada (`ipc::stats_on`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PerfStats {
    /// Quadros por segundo, em centésimos (6000 = 60,00).
    pub fps_centi: u32,
    /// Tempo médio entre dois quadros, em microssegundos.
    pub frame_us: u32,
    /// Tempo médio por quadro que a layer gasta no processador (sem contar o present do driver), em microssegundos.
    pub layer_us: u32,
    /// Do total, o que é aplicar os efeitos (preparar e enviar) e observar a profundidade no fim do quadro.
    pub effects_us: u32,
    pub depth_us: u32,
    /// Estimativa do que os ganchos de desenho (observação da profundidade) custam por quadro, nas threads do jogo.
    pub hooks_us: u32,
    /// Tempo da GPU com os efeitos, por quadro, e de cada etapa da cadeia (nome, microssegundos).
    pub gpu_us: u32,
    pub gpu_steps: Vec<(String, u32)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LayerStatus {
    /// Medições de desempenho (só com a medição ligada).
    pub perf: Option<PerfStats>,
    /// As candidatas a profundidade da cena do último quadro (só com a observação ligada).
    pub depth: Vec<DepthStatus>,
    /// Resumo (uma linha) dos passes que usaram a profundidade escolhida no último quadro, para entender o que o jogo
    /// desenha nela.
    pub depth_passes: String,
    /// A cadeia que a layer construiu, em ordem (vazia: nada construído). A do swapchain mais recente.
    pub techniques: Vec<TechniqueStatus>,
    /// Nome do executável sem a extensão (`RocketLeague` para `RocketLeague.exe`); vazio se a layer não soube.
    pub exe: String,
    /// O nome que o jogo deu ao Vulkan (`VkApplicationInfo`), ou o do processo.
    pub app: String,
    pub swapchains: Vec<SwapchainStatus>,
}

fn one_line(text: &str, max: usize) -> String {
    text.chars().map(|c| if c.is_control() { ' ' } else { c }).take(max).collect()
}

impl LayerStatus {
    /// O id do jogo para achar o preset dele: o executável (estável e fácil de saber) e, sem ele, o nome da aplicação.
    pub fn game_id(&self) -> &str {
        if self.exe.is_empty() { &self.app } else { &self.exe }
    }

    /// Texto do status, com no máximo `MAX_STATUS_BYTES`. Se não couber, descarta os swapchains mais antigos.
    pub fn encode(&self) -> String {
        let mut swapchains: Vec<&SwapchainStatus> =
            self.swapchains.iter().rev().take(MAX_SWAPCHAINS).collect::<Vec<_>>().into_iter().rev().collect();
        loop {
            let mut text = format!("app={}\nexe={}\n", one_line(&self.app, 120), one_line(&self.exe, 120));
            if !self.depth_passes.is_empty() {
                text.push_str(&format!("depthpasses={}\n", one_line(&self.depth_passes, 700)));
            }
            for d in self.depth.iter().take(MAX_DEPTH) {
                let reversed = match d.reversed {
                    Some(true) => "yes",
                    Some(false) => "no",
                    None => "?",
                };
                text.push_str(&format!(
                    "depth={}\t{}x{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                    if d.chosen { "*" } else { "-" },
                    d.width,
                    d.height,
                    d.format,
                    d.samples,
                    d.draws,
                    d.vertices,
                    d.passes,
                    one_line(&d.load, 12),
                    one_line(&d.store, 12),
                    reversed,
                    d.last_pass,
                    one_line(d.unusable.as_deref().unwrap_or(""), 60),
                    if d.sampled { "sampled" } else { "-" },
                    d.layout,
                    if d.automatic { "auto" } else { "-" },
                ));
            }
            if let Some(p) = &self.perf {
                text.push_str(&format!("perf={}\t{}\t{}\t{}\t{}\t{}\t{}\n", p.fps_centi, p.frame_us, p.layer_us, p.effects_us, p.depth_us, p.hooks_us, p.gpu_us));
                for (name, us) in p.gpu_steps.iter().take(MAX_TECHNIQUES) {
                    text.push_str(&format!("gpustep={}\t{us}\n", one_line(name, 60)));
                }
            }
            for t in self.techniques.iter().take(MAX_TECHNIQUES) {
                match &t.state {
                    TechniqueState::Applied => text.push_str(&format!("technique=applied\t{}\n", one_line(&t.name, 60))),
                    TechniqueState::Skipped(why) => text.push_str(&format!("technique=skipped\t{}\t{}\n", one_line(&t.name, 60), one_line(why, 120))),
                }
            }
            for s in &swapchains {
                let (state, detail) = match &s.effect {
                    EffectState::Idle => ("idle", String::new()),
                    EffectState::Waiting => ("waiting", String::new()),
                    EffectState::Ready => ("ready", String::new()),
                    EffectState::Unavailable(why) => ("unavailable", one_line(why, 100)),
                    EffectState::Failed(why) => ("failed", one_line(why, 100)),
                };
                text.push_str(&format!("swapchain={}x{} {} {} {state} {detail}\n", s.width, s.height, s.format, s.images));
            }
            if text.len() <= MAX_STATUS_BYTES || swapchains.is_empty() {
                return text;
            }
            swapchains.remove(0);
        }
    }

    /// Lê o texto de [`encode`](Self::encode). Linhas desconhecidas ou mal formadas são ignoradas, para
    /// que versões diferentes dos dois lados continuem se entendendo.
    pub fn parse(text: &str) -> Self {
        let mut status = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            match key {
                "app" => status.app = value.to_string(),
                "exe" => status.exe = value.to_string(),
                "depthpasses" => status.depth_passes = value.to_string(),
                "perf" => {
                    let f: Vec<u32> = value.split('\t').filter_map(|v| v.parse().ok()).collect();
                    if f.len() == 7 {
                        status.perf = Some(PerfStats { fps_centi: f[0], frame_us: f[1], layer_us: f[2], effects_us: f[3], depth_us: f[4], hooks_us: f[5], gpu_us: f[6], gpu_steps: Vec::new() });
                    }
                }
                "gpustep" => {
                    if let (Some(perf), Some((name, us))) = (status.perf.as_mut(), value.split_once('\t')) {
                        if let Ok(us) = us.parse() {
                            perf.gpu_steps.push((name.to_string(), us));
                        }
                    }
                }
                "depth" => {
                    if let Some(d) = parse_depth(value) {
                        status.depth.push(d);
                    }
                }
                "technique" => {
                    let mut parts = value.splitn(3, '\t');
                    let state = parts.next();
                    let name = parts.next().unwrap_or("").to_string();
                    let detail = parts.next().unwrap_or("").to_string();
                    match state {
                        Some("applied") if !name.is_empty() => status.techniques.push(TechniqueStatus { name, state: TechniqueState::Applied }),
                        Some("skipped") if !name.is_empty() => status.techniques.push(TechniqueStatus { name, state: TechniqueState::Skipped(detail) }),
                        _ => {}
                    }
                }
                "swapchain" => {
                    if let Some(s) = parse_swapchain(value) {
                        status.swapchains.push(s);
                    }
                }
                _ => {}
            }
        }
        status
    }
}

fn parse_depth(value: &str) -> Option<DepthStatus> {
    let f: Vec<&str> = value.split('\t').collect();
    if f.len() < 12 {
        return None;
    }
    let (width, height) = f[1].split_once('x')?;
    Some(DepthStatus {
        chosen: f[0] == "*",
        width: width.parse().ok()?,
        height: height.parse().ok()?,
        format: f[2].parse().ok()?,
        samples: f[3].parse().ok()?,
        draws: f[4].parse().ok()?,
        vertices: f[5].parse().ok()?,
        passes: f[6].parse().ok()?,
        load: f[7].to_string(),
        store: f[8].to_string(),
        reversed: match f[9] {
            "yes" => Some(true),
            "no" => Some(false),
            _ => None,
        },
        last_pass: f[10].parse().ok()?,
        unusable: Some(f[11].to_string()).filter(|u| !u.is_empty()),
        sampled: f.get(12).is_some_and(|v| *v == "sampled"),
        layout: f.get(13).and_then(|v| v.parse().ok()).unwrap_or(0),
        automatic: f.get(14).is_some_and(|v| *v == "auto"),
    })
}

fn parse_swapchain(value: &str) -> Option<SwapchainStatus> {
    let mut parts = value.splitn(5, ' ');
    let (width, height) = parts.next()?.split_once('x')?;
    let format = parts.next()?.parse().ok()?;
    let images = parts.next()?.parse().ok()?;
    let state = parts.next()?;
    let detail = parts.next().unwrap_or("").trim().to_string();
    let effect = match state {
        "idle" => EffectState::Idle,
        "waiting" => EffectState::Waiting,
        "ready" => EffectState::Ready,
        "unavailable" => EffectState::Unavailable(detail),
        "failed" => EffectState::Failed(detail),
        _ => return None,
    };
    Some(SwapchainStatus { width: width.parse().ok()?, height: height.parse().ok()?, format, images, effect })
}

type StatusText = Blob<{ MAX_STATUS_BYTES / 4 }>;

#[repr(C)]
struct StatusFile {
    magic: AtomicU32,
    version: AtomicU32,
    text: StatusText,
}

/// Lado da layer: dono do arquivo de status do próprio processo. Apaga o arquivo ao ser descartado.
pub struct StatusWriter {
    map: Mapping<StatusFile>,
    path: PathBuf,
    /// Mantém o `flock` exclusivo enquanto o processo vive.
    _lock: File,
    /// Apagar o arquivo ao descartar (sempre, exceto nos testes que simulam um processo morto).
    cleanup: bool,
}

impl StatusWriter {
    pub fn create() -> io::Result<Self> {
        Self::create_in(&state_dir())
    }

    pub fn create_in(dir: &Path) -> io::Result<Self> {
        ensure_private_dir(dir)?;
        let unique = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let path = dir.join(format!("layer-{}-{unique}", std::process::id()));
        let file = OpenOptions::new().read(true).write(true).create_new(true).mode(0o600).open(&path)?;
        // O `flock` vem antes de qualquer conteúdo: quem lê só descarta arquivos sem dono E já antigos.
        flock(&file, FlockOperation::NonBlockingLockExclusive)?;
        file.set_len(size_of::<StatusFile>() as u64)?;
        let map = Mapping::<StatusFile>::new(&file, true)?;
        map.get().version.store(VERSION, Ordering::Relaxed);
        map.get().magic.store(MAGIC, Ordering::Release);
        Ok(Self { map, path, _lock: file, cleanup: true })
    }

    /// O identificador desta layer (o nome do arquivo de status): o que o app de controle usa para falar com ela.
    pub fn id(&self) -> String {
        self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }

    /// Publica o status atual (substitui o anterior).
    pub fn publish(&self, status: &LayerStatus) -> Result<(), PublishError> {
        self.map.get().text.publish(TAG, status.encode().as_bytes())
    }

    /// Larga o `flock` sem apagar o arquivo, como se o processo tivesse morrido (só para testes).
    #[cfg(test)]
    fn abandon(mut self) {
        self.cleanup = false;
    }
}

impl Drop for StatusWriter {
    fn drop(&mut self) {
        if self.cleanup {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Uma layer viva, como o app de controle a vê.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveLayer {
    /// Nome do arquivo (identifica o processo).
    pub id: String,
    pub status: LayerStatus,
}

/// As layers vivas. Descarta (apaga) os arquivos de processos que morreram.
/// O painel avisa que está aberto segurando um `flock` exclusivo em `panel-open` (o sistema o solta se ele morrer). O daemon liga os
/// atalhos globais também enquanto o painel está aberto: é quando a pessoa os configura e o KDE pede a confirmação (sem jogo aberto).
pub fn hold_panel_open() -> Option<File> {
    let dir = state_dir();
    fs::create_dir_all(&dir).ok()?;
    let file = fs::OpenOptions::new().create(true).write(true).truncate(false).open(dir.join("panel-open")).ok()?;
    flock(&file, FlockOperation::NonBlockingLockExclusive).ok()?;
    Some(file)
}

pub fn panel_open() -> bool {
    let Ok(file) = File::open(state_dir().join("panel-open")) else { return false };
    flock(&file, FlockOperation::NonBlockingLockShared).is_err()
}

pub fn live_layers() -> io::Result<Vec<LiveLayer>> {
    live_layers_in(&state_dir(), YOUNG)
}

pub fn live_layers_in(dir: &Path, young: Duration) -> io::Result<Vec<LiveLayer>> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let me = rustix::process::getuid().as_raw();
    let mut layers = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("layer-") {
            continue;
        }
        let path = entry.path();
        let Ok(file) = File::open(&path) else { continue };
        let Ok(meta) = file.metadata() else { continue };
        if meta.uid() != me || meta.len() < size_of::<StatusFile>() as u64 {
            continue;
        }
        // Se o `flock` compartilhado funciona, ninguém segura o arquivo: o dono morreu.
        match flock(&file, FlockOperation::NonBlockingLockShared) {
            Ok(()) => {
                let age = SystemTime::now().duration_since(UNIX_EPOCH).map(|now| now.as_secs() as i64 - meta.ctime()).unwrap_or(0);
                if age >= young.as_secs() as i64 {
                    let _ = fs::remove_file(&path);
                }
                continue;
            }
            Err(Errno::WOULDBLOCK) => {} // alguém o segura: está vivo
            Err(_) => continue,
        }
        let Ok(map) = Mapping::<StatusFile>::new(&file, false) else { continue };
        let f = map.get();
        if f.magic.load(Ordering::Acquire) != MAGIC || f.version.load(Ordering::Relaxed) != VERSION {
            continue;
        }
        let mut buf = vec![0u8; MAX_STATUS_BYTES];
        if let Snapshot::Consistent { len } = f.text.snapshot(TAG, &mut buf) {
            let status = LayerStatus::parse(&String::from_utf8_lossy(&buf[..len]));
            layers.push(LiveLayer { id: name, status });
        }
    }
    layers.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(layers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lr-status-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    fn swapchain(w: u32, h: u32, effect: EffectState) -> SwapchainStatus {
        SwapchainStatus { width: w, height: h, format: 44, images: 3, effect }
    }

    #[test]
    fn text_roundtrips_every_state() {
        let status = LayerStatus { perf: None,
            depth: Vec::new(), depth_passes: String::new(),
            techniques: Vec::new(),
            exe: String::new(),
            app: "Idols of Ash".into(),
            swapchains: vec![
                swapchain(1920, 1080, EffectState::Ready),
                swapchain(1914, 1048, EffectState::Waiting),
                swapchain(800, 600, EffectState::Idle),
                swapchain(640, 480, EffectState::Unavailable("formato sRGB".into())),
                swapchain(320, 200, EffectState::Failed("sem memória".into())),
            ],
        };
        assert_eq!(LayerStatus::parse(&status.encode()), status);
    }

    #[test]
    fn the_chain_state_roundtrips_and_tolerates_odd_text() {
        let status = LayerStatus { perf: None,
            depth: Vec::new(), depth_passes: String::new(),
            techniques: vec![
                TechniqueStatus { name: "Sepia".into(), state: TechniqueState::Applied },
                TechniqueStatus { name: "CAS".into(), state: TechniqueState::Skipped("passe 0: escrita sRGB\tcom tab\ne linha".into()) },
            ],
            ..LayerStatus::default()
        };
        let back = LayerStatus::parse(&status.encode());
        assert_eq!(back.techniques[0], status.techniques[0]);
        assert_eq!(back.techniques[1].name, "CAS");
        assert!(matches!(&back.techniques[1].state, TechniqueState::Skipped(w) if w.contains("escrita sRGB")), "{back:?}");
        // Linhas mal formadas são ignoradas.
        assert!(LayerStatus::parse("technique=applied\ntechnique=quebrada\ttexto\n").techniques.is_empty());
    }

    #[test]
    fn depth_candidates_roundtrip() {
        let status = LayerStatus { perf: None,
            depth: vec![
                DepthStatus { width: 1920, height: 1080, format: 126, samples: 1, draws: 300, vertices: 500_000, passes: 2, load: "clear".into(), store: "store".into(), reversed: Some(true), last_pass: 3, chosen: true, automatic: true, unusable: None, sampled: true, layout: 1000 },
                DepthStatus { width: 2048, height: 2048, format: 126, samples: 4, draws: 10, vertices: 900, passes: 1, load: "load".into(), store: "dont_care".into(), reversed: None, last_pass: 0, chosen: false, automatic: false, unusable: Some("MSAA (precisa de resolve)".into()), sampled: false, layout: 0 },
            ],
            ..LayerStatus::default()
        };
        assert_eq!(LayerStatus::parse(&status.encode()).depth, status.depth);
        assert!(LayerStatus::parse("depth=*\t1x1\n").depth.is_empty(), "linha curta é ignorada");
    }

    #[test]
    fn parsing_ignores_what_it_does_not_understand() {
        let text = "app=Jogo\ncampo-novo=1\nlinha sem igual\nswapchain=lixo\nswapchain=10x20 44 3 ready\nswapchain=1x2 44 3 estado-novo\n";
        let s = LayerStatus::parse(text);
        assert_eq!(s.app, "Jogo");
        assert_eq!(s.swapchains, vec![swapchain(10, 20, EffectState::Ready)]);
    }

    #[test]
    fn encoding_keeps_the_most_recent_swapchains_within_the_size_limit() {
        let many: Vec<_> = (0..40).map(|i| swapchain(1000 + i, 1000, EffectState::Failed("x".repeat(500)))).collect();
        let text = LayerStatus { perf: None, depth: Vec::new(), depth_passes: String::new(), techniques: Vec::new(), exe: String::new(), app: "A".repeat(500), swapchains: many }.encode();
        assert!(text.len() <= MAX_STATUS_BYTES, "{} bytes", text.len());
        let back = LayerStatus::parse(&text);
        assert!(back.app.len() <= 120, "o nome do jogo é cortado");
        assert!(back.swapchains.len() <= MAX_SWAPCHAINS);
        assert_eq!(back.swapchains.last().unwrap().width, 1039, "o mais recente é o que fica");
    }

    #[test]
    fn control_characters_cannot_break_the_format() {
        let s = LayerStatus { perf: None,
            depth: Vec::new(), depth_passes: String::new(),
            techniques: Vec::new(),
            exe: String::new(),
            app: "Jogo\nswapchain=1x1 44 3 ready".into(),
            swapchains: vec![swapchain(10, 10, EffectState::Failed("a\nb\u{0}c".into()))],
        };
        let back = LayerStatus::parse(&s.encode());
        assert_eq!(back.swapchains.len(), 1, "o nome do jogo não injetou um swapchain");
        assert!(matches!(&back.swapchains[0].effect, EffectState::Failed(m) if !m.contains('\n')));
    }

    #[test]
    fn a_live_layer_is_listed_and_removes_its_file_when_dropped() {
        let d = dir("live");
        let writer = StatusWriter::create_in(&d).unwrap();
        assert!(live_layers_in(&d, Duration::ZERO).unwrap().is_empty(), "sem nada publicado ainda");

        let status = LayerStatus { perf: None, depth: Vec::new(), depth_passes: String::new(), techniques: Vec::new(), exe: String::new(), app: "RocketLeague.exe".into(), swapchains: vec![swapchain(1920, 1080, EffectState::Ready)] };
        writer.publish(&status).unwrap();
        let live = live_layers_in(&d, Duration::ZERO).unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].status, status);

        let newer = LayerStatus { perf: None, depth: Vec::new(), depth_passes: String::new(), techniques: Vec::new(), exe: String::new(), app: "RocketLeague.exe".into(), swapchains: vec![swapchain(1914, 1048, EffectState::Waiting)] };
        writer.publish(&newer).unwrap();
        assert_eq!(live_layers_in(&d, Duration::ZERO).unwrap()[0].status, newer, "a publicação nova substitui a antiga");

        drop(writer);
        assert!(live_layers_in(&d, Duration::ZERO).unwrap().is_empty());
        assert_eq!(fs::read_dir(&d).unwrap().count(), 0, "o arquivo foi apagado");
    }

    #[test]
    fn two_layers_are_listed_separately() {
        let d = dir("two");
        let a = StatusWriter::create_in(&d).unwrap();
        let b = StatusWriter::create_in(&d).unwrap();
        a.publish(&LayerStatus { perf: None, depth: Vec::new(), depth_passes: String::new(), techniques: Vec::new(), exe: String::new(), app: "A".into(), swapchains: vec![] }).unwrap();
        b.publish(&LayerStatus { perf: None, depth: Vec::new(), depth_passes: String::new(), techniques: Vec::new(), exe: String::new(), app: "B".into(), swapchains: vec![] }).unwrap();
        let mut apps: Vec<_> = live_layers_in(&d, Duration::ZERO).unwrap().into_iter().map(|l| l.status.app).collect();
        apps.sort();
        assert_eq!(apps, ["A", "B"]);
    }

    #[test]
    fn a_dead_layers_file_is_pruned_by_the_reader() {
        let d = dir("dead");
        let writer = StatusWriter::create_in(&d).unwrap();
        writer.publish(&LayerStatus { perf: None, depth: Vec::new(), depth_passes: String::new(), techniques: Vec::new(), exe: String::new(), app: "Morto".into(), swapchains: vec![] }).unwrap();
        writer.abandon(); // solta o flock sem apagar: é o que acontece quando o processo morre
        assert_eq!(fs::read_dir(&d).unwrap().count(), 1, "o arquivo ficou para trás");

        // Recém-criado: o leitor não o apaga (o dono pode estar no meio da criação)...
        assert!(live_layers_in(&d, Duration::from_secs(60)).unwrap().is_empty());
        assert_eq!(fs::read_dir(&d).unwrap().count(), 1, "arquivo jovem é poupado");
        // ...mas depois de velho sem dono, é descartado.
        assert!(live_layers_in(&d, Duration::ZERO).unwrap().is_empty());
        assert_eq!(fs::read_dir(&d).unwrap().count(), 0, "arquivo órfão foi apagado");
    }

    #[test]
    fn a_missing_directory_means_no_layers() {
        assert!(live_layers_in(&dir("missing"), Duration::ZERO).unwrap().is_empty());
    }
}
