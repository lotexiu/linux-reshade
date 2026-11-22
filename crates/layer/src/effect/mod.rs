//! Efeito de pós-processamento de um swapchain: criação preguiçosa dos recursos do `postfx`,
//! submissão a cada present e destruição.

use crate::bundle::{self, Resolved, ResolvedStep, StepParams};
use crate::log::log;
use crate::params::ParamState;
use crate::state::DeviceData;
use ash::vk::{self, Handle};
use ipc::{EffectState, SwapchainStatus, TechniqueState, TechniqueStatus};
use postfx::effect::{DepthInput, FrameInfo, Target, utc_date};
use postfx::{Chain, Effect, Step};
use std::sync::LazyLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Início da contagem do `source = "timer"` (aproximadamente o início do jogo: é tocado na criação da instância).
pub static START: LazyLock<Instant> = LazyLock::new(Instant::now);

mod build;
mod frame;
mod gpu_timer;
mod present;
mod retire;
mod swap;

pub use present::{frame_target, is_swapchain_image, largest_extent, on_depth_image_destroyed, process};
pub use retire::{reap_semaphores, wait_cleanups};
pub(crate) use retire::retire_semaphores;

use build::*;
use frame::*;
use retire::discard_job_in_background;

/// Alguma cadeia está sendo criada neste processo (atualizado a cada present por [`process`]).
static CHAIN_BUILDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Os tempos dos últimos quadros e a estatística calculada a partir deles.
struct FrameWindow {
    times: std::collections::VecDeque<f32>,
    total_ms: f32,
    stats: [f32; 4],
    since: u32,
}

/// Quantos segundos de quadros entram nas estatísticas, e de quantos em quantos quadros elas são refeitas (ordenar a janela a todo present
/// custaria à toa: o número na tela muda devagar).
const FPS_WINDOW_MS: f32 = 5000.0;

const FPS_REFRESH_FRAMES: u32 = 10;

/// Quantas limpezas em segundo plano (ver [`discard_job_in_background`]) ainda correm: o swapchain e o dispositivo esperam por elas.
static PENDING_CLEANUPS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Quantos presents depois de a cadeia sair os semáforos dela são destruídos.
const SEMAPHORE_RETIRE_PRESENTS: u32 = 8;

static RETIRED_SEMAPHORES: std::sync::Mutex<Vec<(u64, vk::Semaphore, u32)>> = std::sync::Mutex::new(Vec::new());

/// `LINUX_RESHADE_QUIET_SWAP=1`: enquanto a cadeia nova é criada a velha também não desenha (o quadro passa sem efeito). Teste da hipótese
/// de que os Xid 69 ao trocar definições vêm de a cadeia velha rodar junto com a criação da nova.
/// `LINUX_RESHADE_HARD_SWAP=1`: qualquer mudança da cadeia (não só de macros) descarta tudo e recria do zero, como o ReShade (teste).
fn hard_swap() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LINUX_RESHADE_HARD_SWAP").is_some_and(|v| v != "0"))
}

const LATE_FREE_PRESENTS: u32 = 300;

/// `LINUX_RESHADE_LATE_FREE=1`: a cadeia antiga só é destruída 300 presents depois de aposentada (teste de recurso destruído cedo demais).
fn late_free() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LINUX_RESHADE_LATE_FREE").is_some_and(|v| v != "0"))
}

pub(crate) fn quiet_swap() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LINUX_RESHADE_QUIET_SWAP").is_some_and(|v| v != "0"))
}

/// Recursos que só existem depois do primeiro present com os efeitos ligados.
struct Built {
    chain: Chain,
    /// Identifica o que foi construído (bundle e técnica de cada etapa); serve para saber se a cadeia
    /// construída ainda vale quando a escolha muda.
    signature: Vec<(u64, usize)>,
    /// De qual etapa da escolha vem cada efeito da cadeia (as puladas por falta de suporte não aparecem).
    origin: Vec<usize>,
    /// A profundidade com que foi construída (imagem e layout); mudou, reconstrói.
    depth_key: Option<(u64, i32)>,
    /// As etapas puladas por falta de suporte e o motivo.
    skipped: Vec<(usize, String)>,
    /// Um por efeito da cadeia, na mesma ordem.
    params: Vec<StepRuntime>,
    /// Um semáforo por imagem do swapchain: o da imagem sinaliza que o efeito terminou nela.
    semaphores: Vec<vk::Semaphore>,
    /// Uma fence por imagem, sinalizada quando o command buffer dela termina. Garante que o buffer
    /// não é reenviado enquanto ainda executa (o que o swapchain sugere, mas não garante).
    fences: Vec<vk::Fence>,
    /// Os carimbos de tempo da GPU (só com a medição de desempenho ligada); criados na primeira medição.
    gpu: Option<GpuTimer>,
}

/// Mede quanto a GPU gasta em cada efeito da cadeia: um carimbo de tempo entre um efeito e o seguinte (cada carimbo é um command
/// buffer mínimo, intercalado com os dos efeitos no mesmo envio).
struct GpuTimer {
    pool: vk::QueryPool,
    command_pool: vk::CommandPool,
    /// `[imagem][limite]`: o command buffer que grava o carimbo `limite` (o primeiro também zera os da imagem).
    stamps: Vec<Vec<vk::CommandBuffer>>,
    /// A imagem já tem uma medição gravada para ler?
    written: Vec<bool>,
    /// Nanossegundos por tick do carimbo.
    period: f32,
    /// O tempo suavizado de cada efeito, em microssegundos, e o nome dele.
    steps: Vec<(String, f32)>,
}

/// De onde uma etapa tira os uniforms a cada frame.
struct StepRuntime {
    /// Identifica os parâmetros do efeito nos blocos publicados pelo app de controle: igual em todas as
    /// resoluções, para um valor ajustado valer para todos os jogos.
    params_id: u64,
    source: StepSource,
}

enum StepSource {
    /// Os blocos publicados (do jogo, antes do global); só a escolha única de um efeito usa.
    Published(ParamState),
    /// A imagem do plano; `None` deixa os padrões do manifesto.
    Plan(Option<Vec<u8>>),
}

/// A assinatura de uma cadeia: o que identifica o que foi construído.
fn signature(steps: &[ResolvedStep]) -> Vec<(u64, usize)> {
    steps.iter().map(|s| (s.bundle.id, s.technique)).collect()
}

/// O que cada etapa é, sem a variante: o efeito (arquivo) e a técnica. Duas cadeias com os mesmos `kinds` e assinaturas diferentes diferem só
/// nas macros com que os efeitos foram compilados.
fn kinds(steps: &[ResolvedStep]) -> Vec<(String, usize)> {
    steps.iter().map(|s| (s.bundle.manifest.source.clone(), s.technique)).collect()
}

/// Quanto esperar a execução anterior de uma imagem antes de desistir de aplicar o efeito neste frame.
const FENCE_TIMEOUT_NS: u64 = 250_000_000;

/// Uma criação de cadeia rodando numa thread própria (a compilação dos shaders pelo driver leva segundos e não pode parar o
/// jogo). O resultado só é instalado no present seguinte ao fim dela, na thread que usa a fila.
struct Job {
    thread: Option<std::thread::JoinHandle<SendOut>>,
    /// As etapas deste trabalho (a montagem da cadeia usa estas, não as mais novas que `steps` possa ter).
    steps: Vec<ResolvedStep>,
    /// Só cria os efeitos que faltam; os outros vêm da cadeia aposentada (sem refazê-los, o histórico dos que não mudaram fica).
    incremental: bool,
    /// O que está sendo construído (bundle e técnica de cada etapa).
    signature: Vec<(u64, usize)>,
    /// A imagem de profundidade do jogo de que a cadeia tem uma view (se usa profundidade).
    depth_image: Option<u64>,
    /// A profundidade (imagem e layout) que a criação recebeu: uma falha só se repete com outra (ver `failed_depth`).
    depth_key: Option<(u64, i32)>,
    started: Instant,
}

/// O que a thread devolve: a cadeia inteira, ou só os efeitos novos (com as texturas compartilhadas que passaram para a thread).
enum Output {
    Full(Built),
    Partial(Partial),
}

/// Os efeitos novos de uma atualização incremental.
struct Partial {
    /// Um por etapa: `Some` para os criados agora.
    effects: Vec<Option<Effect>>,
    skipped: Vec<postfx::Skipped>,
    /// As texturas compartilhadas (tiradas da cadeia aposentada): os efeitos reaproveitados e os novos as apelidam.
    shared: postfx::shared::Shared,
    /// Para cada etapa, a posição do efeito que ela reaproveita na cadeia aposentada.
    reuse: Vec<Option<usize>>,
}

struct Failure {
    why: String,
    /// As texturas compartilhadas que a thread levou (voltam para ser destruídas depois da cadeia aposentada).
    shared: Option<postfx::shared::Shared>,
}

struct SendOut(Result<Output, Failure>);

// SAFETY: os handles do Vulkan são só inteiros e nada acessa o resultado entre a thread de criação e a do present.

unsafe impl Send for SendOut {}

struct SendPlan(Vec<Option<usize>>, postfx::shared::Shared);

// SAFETY: como acima; a cadeia aposentada não usa o `Shared` enquanto a thread o tem.

unsafe impl Send for SendPlan {}

enum Fx {
    Unbuilt,
    /// Sendo criada numa thread.
    Building(Job),
    Ready(Built),
    /// A criação falhou com este motivo; não tenta de novo até o bundle mudar (evita falhar a cada frame).
    Failed(String),
}

pub struct SwapchainFx {
    handle: u64,
    images: Vec<vk::Image>,
    format: vk::Format,
    extent: vk::Extent2D,
    /// `Some(motivo)` se este swapchain não aceita o efeito (a superfície não permite o uso extra, o
    /// formato é sRGB...). O motivo vai para o app de controle.
    unusable: Option<String>,
    fx: Fx,
    /// A cadeia anterior, que continua desenhando até a nova ficar pronta (sem isso o efeito some por alguns quadros a cada troca
    /// de ordem ou de efeito).
    retiring: Option<Built>,
    /// Cadeias já sem uso que esperam alguns presents antes de ser destruídas (`LINUX_RESHADE_LATE_FREE`), para testar se um Xid vem de um recurso
    /// destruído cedo demais: (cadeia, presents que faltam).
    graveyard: Vec<(Built, u32)>,
    /// Época da escolha de efeito (ver `bundle::epoch`) que este swapchain já resolveu.
    seen_epoch: u64,
    /// Profundidade escolhida quando a cadeia falhou: se mudar, vale tentar de novo.
    failed_depth: Option<(u64, i32)>,
    /// A cadeia que a escolha dá para a resolução deste swapchain (vazia = nenhuma).
    steps: Vec<ResolvedStep>,
    /// O que foi publicado para o app de controle.
    state: EffectState,
    /// Quando o present anterior deste swapchain começou (para o `frametime`).
    last_present: Option<Instant>,
}

// SAFETY: contém handles do Vulkan (inteiros/ponteiros opacos) e é sempre acessado sob o `Mutex`
// de `DeviceData::swapchains`.

unsafe impl Send for SwapchainFx {}

/// Tudo o que a criação da cadeia precisa, copiado para poder rodar noutra thread.
struct BuildArgs {
    device: ash::Device,
    memory: vk::PhysicalDeviceMemoryProperties,
    images: Vec<vk::Image>,
    format: vk::Format,
    extent: vk::Extent2D,
    steps: Vec<ResolvedStep>,
    depth: Option<crate::depth::Chosen>,
    queue_family: u32,
    set_loader_data: vk_layer::abi::PfnSetDeviceLoaderData,
}
