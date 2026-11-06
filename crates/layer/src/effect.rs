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

/// Valores do frame que o runtime fornece aos uniforms com `source`.
fn frame_info(now: Instant, previous: Option<Instant>, frame_count: u64) -> FrameInfo {
    let unix = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    FrameInfo {
        time_ms: now.duration_since(*START).as_secs_f32() * 1000.0,
        frame_time_ms: previous.map(|p| now.duration_since(p).as_secs_f32() * 1000.0).unwrap_or(0.0),
        frame_count: frame_count as u32,
        date: utc_date(unix),
    }
}

/// O controle que a pessoa edita no painel (ver `ipc::overlay`), relido a cada 80 ms: o arquivo é pequeno, mas o present roda
/// centenas de vezes por segundo.
fn active_control() -> Option<String> {
    static CACHE: std::sync::Mutex<Option<(Instant, Option<String>)>> = std::sync::Mutex::new(None);
    let mut cache = CACHE.lock().unwrap();
    if let Some((at, value)) = cache.as_ref()
        && at.elapsed() < std::time::Duration::from_millis(80)
    {
        return value.clone();
    }
    let value = ipc::overlay(&crate::status::game_id(), std::time::Duration::from_millis(2500)).map(|(_, uniform)| uniform);
    *cache = Some((Instant::now(), value.clone()));
    value
}

/// Quantas limpezas em segundo plano (ver [`discard_job_in_background`]) ainda correm: o swapchain e o dispositivo esperam por elas.
static PENDING_CLEANUPS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Espera as limpezas em segundo plano acabarem (até `timeout`). Chamada antes de destruir um swapchain ou o dispositivo: a cadeia
/// em criação guarda views das imagens dele.
pub fn wait_cleanups(timeout: std::time::Duration) {
    let started = Instant::now();
    while PENDING_CLEANUPS.load(std::sync::atomic::Ordering::Acquire) > 0 && started.elapsed() < timeout {
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

/// Descarta uma criação em andamento sem parar o jogo: outra thread espera o fim da criação e destrói o resultado (que a GPU
/// nunca usou).
fn discard_job_in_background(fx: &mut Fx, device: &ash::Device) {
    let Fx::Building(mut job) = std::mem::replace(fx, Fx::Unbuilt) else { return };
    let Some(thread) = job.thread.take() else { return };
    let device = device.clone();
    PENDING_CLEANUPS.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    let spawned = std::thread::Builder::new().name("lr-cleanup".into()).spawn({
        let device = device.clone();
        move || {
            if let Ok(SendOut(Ok(Output::Full(built)))) = thread.join() {
                let mut dead = Fx::Ready(built);
                // SAFETY: a cadeia acabou de ser criada e nunca foi enviada à GPU.
                unsafe { SwapchainFx::destroy_fx(&mut dead, &device) };
            }
            PENDING_CLEANUPS.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
        }
    });
    if spawned.is_err() {
        PENDING_CLEANUPS.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

/// Quantos presents depois de a cadeia sair os semáforos dela são destruídos.
const SEMAPHORE_RETIRE_PRESENTS: u32 = 8;

static RETIRED_SEMAPHORES: std::sync::Mutex<Vec<(u64, vk::Semaphore, u32)>> = std::sync::Mutex::new(Vec::new());

pub(crate) fn retire_semaphores(device: &ash::Device, semaphores: impl Iterator<Item = vk::Semaphore>) {
    let key = device.handle().as_raw();
    let mut retired = RETIRED_SEMAPHORES.lock().unwrap();
    retired.extend(semaphores.map(|s| (key, s, SEMAPHORE_RETIRE_PRESENTS)));
}

/// Conta um present para os semáforos aposentados deste dispositivo e destrói os que chegaram ao fim (todos, com `all`).
pub unsafe fn reap_semaphores(device: &ash::Device, all: bool) {
    let key = device.handle().as_raw();
    let mut retired = RETIRED_SEMAPHORES.lock().unwrap();
    retired.retain_mut(|(owner, semaphore, left)| {
        if *owner != key {
            return true;
        }
        *left = left.saturating_sub(1);
        if all || *left == 0 {
            unsafe { device.destroy_semaphore(*semaphore, None) };
            false
        } else {
            true
        }
    });
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

impl GpuTimer {
    /// `None` se a fila não tem carimbos ou algo falhou (a medição simplesmente não existe).
    unsafe fn new(data: &DeviceData, family: u32, images: usize, names: Vec<String>) -> Option<Self> {
        let instance = &data.instance.instance;
        let queue_props = unsafe { instance.get_physical_device_queue_family_properties(data.physical_device) };
        if queue_props.get(family as usize).is_none_or(|p| p.timestamp_valid_bits == 0) {
            return None;
        }
        let period = unsafe { instance.get_physical_device_properties(data.physical_device) }.limits.timestamp_period;
        let boundaries = names.len() + 1;
        let device = &data.device;
        unsafe {
            let pool = device.create_query_pool(&vk::QueryPoolCreateInfo::default().query_type(vk::QueryType::TIMESTAMP).query_count((images * boundaries) as u32), None).ok()?;
            let command_pool = match device.create_command_pool(&vk::CommandPoolCreateInfo::default().queue_family_index(family), None) {
                Ok(p) => p,
                Err(_) => {
                    device.destroy_query_pool(pool, None);
                    return None;
                }
            };
            let mut timer = Self { pool, command_pool, stamps: Vec::new(), written: vec![false; images], period, steps: names.into_iter().map(|n| (n, 0.0)).collect() };
            for image in 0..images {
                let cbs = device.allocate_command_buffers(&vk::CommandBufferAllocateInfo::default().command_pool(command_pool).command_buffer_count(boundaries as u32)).ok();
                let Some(cbs) = cbs else {
                    timer.destroy(device);
                    return None;
                };
                for (boundary, cb) in cbs.iter().enumerate() {
                    let init = (data.set_loader_data)(device.handle(), cb.as_raw() as *mut std::ffi::c_void);
                    if init != vk::Result::SUCCESS || device.begin_command_buffer(*cb, &vk::CommandBufferBeginInfo::default()).is_err() {
                        timer.stamps.push(cbs.clone());
                        timer.destroy(device);
                        return None;
                    }
                    let query = (image * boundaries + boundary) as u32;
                    if boundary == 0 {
                        device.cmd_reset_query_pool(*cb, pool, (image * boundaries) as u32, boundaries as u32);
                    }
                    device.cmd_write_timestamp(*cb, vk::PipelineStageFlags::BOTTOM_OF_PIPE, pool, query);
                    if device.end_command_buffer(*cb).is_err() {
                        timer.stamps.push(cbs.clone());
                        timer.destroy(device);
                        return None;
                    }
                }
                timer.stamps.push(cbs);
            }
            Some(timer)
        }
    }

    /// Lê a medição que a imagem gravou da última vez que foi usada (a fence dela já foi esperada) e a soma às médias.
    unsafe fn collect(&mut self, device: &ash::Device, image: usize) {
        if !self.written.get(image).copied().unwrap_or(false) {
            return;
        }
        let boundaries = self.steps.len() + 1;
        let mut ticks = vec![0u64; boundaries];
        let read = unsafe { device.get_query_pool_results(self.pool, (image * boundaries) as u32, &mut ticks, vk::QueryResultFlags::TYPE_64) };
        if read.is_ok() {
            for (i, (_, smoothed)) in self.steps.iter_mut().enumerate() {
                let us = ticks[i + 1].saturating_sub(ticks[i]) as f32 * self.period / 1000.0;
                *smoothed = if *smoothed == 0.0 { us } else { *smoothed * 0.9 + us * 0.1 };
            }
        }
    }

    unsafe fn destroy(&mut self, device: &ash::Device) {
        unsafe {
            device.destroy_command_pool(self.command_pool, None);
            device.destroy_query_pool(self.pool, None);
        }
        self.stamps.clear();
    }
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

impl SwapchainFx {
    pub fn new(handle: u64, images: Vec<vk::Image>, format: vk::Format, extent: vk::Extent2D, unusable: Option<String>) -> Self {
        let state = unusable.clone().map_or(EffectState::Idle, EffectState::Unavailable);
        let fx = Self { handle, images, format, extent, unusable, fx: Fx::Unbuilt, retiring: None, seen_epoch: u64::MAX, failed_depth: None, steps: Vec::new(), state, last_present: None };
        crate::status::update(handle, fx.status());
        fx
    }

    fn status(&self) -> SwapchainStatus {
        SwapchainStatus {
            width: self.extent.width,
            height: self.extent.height,
            format: self.format.as_raw(),
            images: self.images.len() as u32,
            effect: self.state.clone(),
        }
    }

    /// Muda o estado e, se mudou, conta ao app de controle.
    fn set_state(&mut self, state: EffectState) {
        if self.state != state {
            self.state = state;
            crate::status::update(self.handle, self.status());
        }
    }

    /// A escolha de efeito mudou (ou é a primeira vez): resolve o bundle desta resolução e descarta o
    /// que foi construído com outro. Não toca na GPU além disso; a construção é preguiçosa.
    unsafe fn on_epoch(&mut self, data: &DeviceData, epoch: u64) {
        self.seen_epoch = epoch;
        if self.unusable.is_some() {
            return;
        }
        let (steps, state) = match bundle::resolve(self.extent.width, self.extent.height) {
            Resolved::None => (Vec::new(), EffectState::Idle),
            Resolved::Waiting(dirs) => {
                let dirs: Vec<_> = dirs.iter().map(|d| d.display().to_string()).collect();
                log!("swapchain {}x{}: ainda não há bundle, esperando {}", self.extent.width, self.extent.height, dirs.join(", "));
                (Vec::new(), EffectState::Waiting)
            }
            Resolved::Invalid(why) => (Vec::new(), EffectState::Failed(why)),
            Resolved::Ready(steps) => (steps, EffectState::Ready),
        };
        let new_signature = signature(&steps);
        if let Fx::Ready(built) = &self.fx {
            if built.signature != new_signature {
                log!("efeito do swapchain {:#x} descartado: o bundle mudou (a cadeia é outra)", self.handle);
                crate::status::set_techniques(Vec::new());
                // A cadeia velha segue desenhando até a nova ficar pronta.
                self.retire(&data.device);
            } else if let Some(built) = match &mut self.fx { Fx::Ready(b) => Some(b), _ => None } {
                // A mesma cadeia: só os uniforms do plano podem ter mudado.
                for (runtime, &from) in built.params.iter_mut().zip(&built.origin) {
                    if let (StepSource::Plan(bytes), Some(StepParams::Plan(new))) = (&mut runtime.source, steps.get(from).map(|s| &s.params)) {
                        *bytes = new.clone();
                    }
                }
            }
        }
        if let Fx::Building(job) = &self.fx
            && job.signature != new_signature
        {
            if job.incremental {
                // Termina e instala (as texturas compartilhadas estão na thread); depois da instalação a escolha é olhada de novo e a
                // diferença vira o próximo trabalho.
            } else {
                log!("criação da cadeia descartada: a escolha mudou");
                // Sem esperar a thread terminar (a primeira compilação leva segundos e parava o jogo): ela se desfaz sozinha.
                discard_job_in_background(&mut self.fx, &data.device);
            }
        }
        // Uma falha anterior era da cadeia antiga: com outra vale tentar de novo.
        if matches!(self.fx, Fx::Failed(_)) && signature(&self.steps) != new_signature {
            self.fx = Fx::Unbuilt;
        }
        if steps.is_empty() {
            // Nenhum efeito para esta resolução: nada mais a desenhar.
            unsafe { self.drop_retiring(&data.device) };
        }
        self.steps = steps;
        let state = match &self.fx {
            Fx::Failed(why) => EffectState::Failed(why.clone()),
            _ => state,
        };
        self.set_state(state);
    }

    /// `LINUX_RESHADE_SAFE_SWAP=1`: espera a GPU ficar ociosa antes de destruir uma cadeia de efeitos.
fn safe_swap() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LINUX_RESHADE_SAFE_SWAP").is_some_and(|v| v != "0"))
}

/// Libera os recursos de GPU.
    ///
    /// # Safety
    /// A GPU não pode estar executando trabalho deste swapchain (o chamador espera o dispositivo ficar ocioso).
    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        unsafe {
            self.drop_retiring(device);
            Self::destroy_fx(&mut self.fx, device);
        }
        self.fx = Fx::Failed("destruído".into());
        crate::status::remove(self.handle);
    }

    /// A cadeia em uso sai de cena, mas continua desenhando até a próxima ficar pronta.
    fn retire(&mut self, device: &ash::Device) {
        if let Fx::Ready(_) = &self.fx {
            let old = std::mem::replace(&mut self.fx, Fx::Unbuilt);
            // Uma cadeia já aposentada que ainda não foi trocada sai de vez (só uma espera).
            unsafe { self.drop_retiring(device) };
            if let Fx::Ready(built) = old {
                self.retiring = Some(built);
            }
        }
    }

    unsafe fn drop_retiring(&mut self, device: &ash::Device) {
        if let Some(built) = self.retiring.take() {
            // Diagnóstico dos Xid 69 (erro de classe da GPU) que aparecem depois de trocar a cadeia de efeitos: com
            // `LINUX_RESHADE_SAFE_SWAP=1` a GPU fica ociosa antes de a cadeia antiga ser destruída. Se os erros somem, a causa é um
            // recurso destruído enquanto a GPU ainda o usava.
            if Self::safe_swap() {
                let _ = unsafe { device.device_wait_idle() };
            }
            let mut old = Fx::Ready(built);
            unsafe { Self::destroy_fx(&mut old, device) };
        }
    }

    unsafe fn destroy_fx(fx: &mut Fx, device: &ash::Device) {
        let built = match std::mem::replace(fx, Fx::Unbuilt) {
            Fx::Ready(built) => Some(built),
            // Uma criação em andamento: espera a thread terminar e descarta o resultado (nada o usou).
            Fx::Building(mut job) => match job.thread.take().and_then(|t| t.join().ok()) {
                Some(SendOut(Ok(Output::Full(built)))) => Some(built),
                // Os efeitos novos saem antes das texturas compartilhadas que apelidam; a cadeia aposentada (que também as apelida)
                // já saiu antes (ver `drop_retiring` nos chamadores).
                Some(SendOut(Ok(Output::Partial(mut partial)))) => {
                    unsafe {
                        for effect in partial.effects.iter_mut().flatten() {
                            effect.destroy(device);
                        }
                        partial.shared.destroy(device);
                    }
                    None
                }
                Some(SendOut(Err(mut failure))) => {
                    if let Some(shared) = failure.shared.as_mut() {
                        unsafe { shared.destroy(device) };
                    }
                    None
                }
                None => None,
            },
            _ => None,
        };
        if let Some(mut built) = built {
            unsafe {
                // Só o trabalho da própria cadeia precisa terminar (suas fences). `vkDeviceWaitIdle` exige que nenhuma outra thread
                // use NENHUMA fila durante a espera, e o jogo submete de outras threads: era a suspeita dos erros de GPU (Xid 69)
                // ao trocar de efeitos.
                if !built.fences.is_empty() {
                    let _ = device.wait_for_fences(&built.fences, true, 5_000_000_000);
                }
                if let Some(timer) = built.gpu.as_mut() {
                    timer.destroy(device);
                }
                built.chain.destroy(device);
                // Um present já enfileirado ainda pode esperar o semáforo do efeito: ele sai alguns presents depois.
                retire_semaphores(device, built.semaphores.drain(..));
                for f in built.fences.drain(..) {
                    device.destroy_fence(f, None);
                }
            }
        }
    }

}

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

/// De onde vêm os uniforms de uma etapa nova.
fn runtime_for(step: &ResolvedStep) -> StepRuntime {
    StepRuntime {
        params_id: step.bundle.params_id(),
        source: match &step.params {
            StepParams::Legacy => StepSource::Published(ParamState::new(fx_bundle::uniforms::default_block(&step.bundle.manifest))),
            StepParams::Plan(bytes) => StepSource::Plan(bytes.clone()),
        },
    }
}

/// Cria só os efeitos que faltam (ver [`Partial`]) numa thread: os de `reuse` já existem na cadeia aposentada.
fn build_partial(a: BuildArgs, reuse: Vec<Option<usize>>, mut shared: postfx::shared::Shared) -> Result<Output, Failure> {
    for (n, step) in a.steps.iter().enumerate() {
        let t = step.bundle.manifest.target;
        if (t.width, t.height) != (a.extent.width, a.extent.height) {
            return Err(Failure { why: format!("etapa {n}: o bundle foi compilado para {}x{} e o swapchain é {}x{}", t.width, t.height, a.extent.width, a.extent.height), shared: Some(shared) });
        }
    }
    let device = &a.device;
    let device_handle = device.handle();
    let set_loader_data = a.set_loader_data;
    let depth = a.depth;
    let target = Target {
        images: &a.images,
        format: a.format,
        extent: a.extent,
        rest_layout: vk::ImageLayout::PRESENT_SRC_KHR,
        queue_family: a.queue_family,
        init_command_buffer: Some(&move |cb| {
            let result = unsafe { set_loader_data(device_handle, cb.as_raw() as *mut std::ffi::c_void) };
            if result == vk::Result::SUCCESS { Ok(()) } else { Err(result) }
        }),
        depth: depth.map(|d| DepthInput {
            image: vk::Image::from_raw(d.image),
            format: vk::Format::from_raw(d.format),
            layout: vk::ImageLayout::from_raw(d.layout),
            extent: vk::Extent2D { width: d.width, height: d.height },
            linear_filter: d.linear,
        }),
    };
    let chain_steps: Vec<Step<'_>> = a.steps.iter().map(|s| Step { bundle: &s.bundle, technique: s.technique }).collect();
    let flags: Vec<bool> = reuse.iter().map(Option::is_some).collect();
    match unsafe { postfx::build_missing(device, &a.memory, &chain_steps, &flags, &target, &mut shared) } {
        Ok((effects, skipped)) => {
            for s in &skipped {
                log!("etapa {} pulada: {}", s.step, s.why);
            }
            Ok(Output::Partial(Partial { effects, skipped, shared, reuse }))
        }
        Err(e) => Err(Failure { why: e.to_string(), shared: Some(shared) }),
    }
}

/// Junta os efeitos reaproveitados da cadeia aposentada com os novos, na ordem das etapas. A cadeia aposentada deixa de existir
/// (o que sobrou dela é destruído); as fences e os semáforos dela passam para a nova.
fn assemble_partial(old: Option<Built>, partial: Partial, steps: &[ResolvedStep], depth_key: Option<(u64, i32)>, device: &ash::Device) -> Result<Built, (String, Option<postfx::shared::Shared>)> {
    let Partial { effects: mut new_effects, skipped, shared, reuse } = partial;
    let Some(old) = old else {
        for effect in new_effects.iter_mut().flatten() {
            unsafe { effect.destroy(device) };
        }
        return Err(("a cadeia anterior sumiu antes da atualização terminar".into(), Some(shared)));
    };
    unsafe {
        if !old.fences.is_empty() {
            let _ = device.wait_for_fences(&old.fences, true, 5_000_000_000);
        }
    }
    let Built { chain, params: old_params, semaphores, fences, gpu, .. } = old;
    if let Some(mut timer) = gpu {
        unsafe { timer.destroy(device) };
    }
    let mut parts = chain.into_parts();
    let mut old_params: Vec<Option<StepRuntime>> = old_params.into_iter().map(Some).collect();
    let (mut effects, mut origin, mut params) = (Vec::new(), Vec::new(), Vec::new());
    for (n, step) in steps.iter().enumerate() {
        if let Some(i) = reuse.get(n).copied().flatten() {
            let (Some(effect), Some(mut runtime)) = (parts.effects.get_mut(i).and_then(Option::take), old_params.get_mut(i).and_then(Option::take)) else { continue };
            // Os uniforms do plano podem ter mudado.
            if let (StepSource::Plan(bytes), StepParams::Plan(new)) = (&mut runtime.source, &step.params) {
                *bytes = new.clone();
            }
            effects.push(effect);
            origin.push(n);
            params.push(runtime);
        } else if let Some(effect) = new_effects.get_mut(n).and_then(Option::take) {
            effects.push(effect);
            origin.push(n);
            params.push(runtime_for(step));
        }
    }
    // O que a cadeia aposentada tinha e a nova não usa.
    for effect in parts.effects.iter_mut().flatten() {
        unsafe { effect.destroy(device) };
    }
    let chain = Chain::from_parts(effects, shared, origin.clone());
    Ok(Built { chain, signature: signature(steps), origin, skipped: skipped.iter().map(|s| (s.step, s.why.clone())).collect(), depth_key, params, semaphores, fences, gpu: None })
}

/// Cria a cadeia (sem usar nenhuma fila: a inicialização das texturas termina depois, na thread do present).
fn build_chain(a: BuildArgs) -> Result<Built, String> {
    for (n, step) in a.steps.iter().enumerate() {
        let t = step.bundle.manifest.target;
        if (t.width, t.height) != (a.extent.width, a.extent.height) {
            return Err(format!("etapa {n}: o bundle foi compilado para {}x{} e o swapchain é {}x{}", t.width, t.height, a.extent.width, a.extent.height));
        }
    }
    let device = &a.device;
    let device_handle = device.handle();
    let set_loader_data = a.set_loader_data;
    let depth = a.depth;
    let target = Target {
        images: &a.images,
        format: a.format,
        extent: a.extent,
        rest_layout: vk::ImageLayout::PRESENT_SRC_KHR,
        queue_family: a.queue_family,
        // O command buffer nasce sem o ponteiro de dispatch do loader; as layers abaixo precisam dele.
        init_command_buffer: Some(&move |cb| {
            let result = unsafe { set_loader_data(device_handle, cb.as_raw() as *mut std::ffi::c_void) };
            if result == vk::Result::SUCCESS { Ok(()) } else { Err(result) }
        }),
        depth: depth.map(|d| DepthInput {
            image: vk::Image::from_raw(d.image),
            format: vk::Format::from_raw(d.format),
            layout: vk::ImageLayout::from_raw(d.layout),
            extent: vk::Extent2D { width: d.width, height: d.height },
            linear_filter: d.linear,
        }),
    };
    let steps = &a.steps;
    let chain_steps: Vec<Step<'_>> = steps.iter().map(|s| Step { bundle: &s.bundle, technique: s.technique }).collect();
    unsafe {
        let (chain, skipped) = Chain::new_lenient_with(device, &a.memory, &chain_steps, &target, None).map_err(|e| e.to_string())?;
        for s in &skipped {
            log!("etapa {} pulada: {}", s.step, s.why);
        }
        if chain.is_empty() && !steps.is_empty() {
            return Err(format!("nenhuma etapa é suportada ({})", skipped.iter().map(|s| format!("etapa {}: {}", s.step, s.why)).collect::<Vec<_>>().join("; ")));
        }
        // Os parâmetros seguem as etapas que ficaram na cadeia (as puladas não têm efeito a alimentar).
        let origin: Vec<usize> = (0..chain.len()).filter_map(|i| chain.origin(i)).collect();
        let params = origin.iter().map(|&i| runtime_for(&steps[i])).collect();
        let skipped = skipped.iter().map(|s| (s.step, s.why.clone())).collect();
        let mut built = Built { chain, signature: signature(steps), origin, skipped, depth_key: depth.map(|d| (d.image, d.layout)), params, semaphores: Vec::new(), fences: Vec::new(), gpu: None };
        for _ in &a.images {
            let semaphore = device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None);
            // Nasce sinalizada: a primeira espera por ela não bloqueia.
            let fence = device.create_fence(&vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED), None);
            match (semaphore, fence) {
                (Ok(s), Ok(f)) => {
                    built.semaphores.push(s);
                    built.fences.push(f);
                }
                (s, f) => {
                    // Libera o par que deu certo e tudo o que já foi criado.
                    let error = s.as_ref().err().or(f.as_ref().err()).copied().unwrap_or(vk::Result::ERROR_UNKNOWN);
                    if let Ok(s) = s {
                        device.destroy_semaphore(s, None);
                    }
                    if let Ok(f) = f {
                        device.destroy_fence(f, None);
                    }
                    let mut partial = Fx::Ready(built);
                    SwapchainFx::destroy_fx(&mut partial, device);
                    return Err(format!("falha ao criar semáforo/fence (erro {})", error.as_raw()));
                }
            }
        }
        Ok(built)
    }
}

/// Trata um present: atualiza o estado do efeito do swapchain (sempre) e, com os efeitos ligados,
/// aplica o efeito. Devolve o semáforo que o present deve esperar no lugar dos originais; `None`
/// significa "apresente como o jogo pediu".
///
/// # Safety
/// `info` deve ser o `VkPresentInfoKHR` recebido do jogo, e `queue` a fila do present.
pub unsafe fn process(data: &DeviceData, queue: vk::Queue, info: &vk::PresentInfoKHR<'_>, enabled: bool) -> Option<vk::Semaphore> {
    // Vários swapchains num mesmo present compartilham a lista de espera: fora do caso comum.
    if info.swapchain_count != 1 {
        return None;
    }
    unsafe {
        reap_semaphores(&data.device, false);
        let swapchain = *info.p_swapchains;
        let index = *info.p_image_indices as usize;

        let mut swapchains = data.swapchains.lock().unwrap();
        let fx = swapchains.get_mut(&swapchain.as_raw())?;

        // Mesmo com os efeitos desligados: é aqui que se descobre que falta um bundle para esta
        // resolução, e o app de controle só sabe a tempo de compilá-lo se a layer avisar já.
        let epoch = bundle::epoch();
        if fx.seen_epoch != epoch {
            fx.on_epoch(data, epoch);
        }
        if !enabled || fx.unusable.is_some() || index >= fx.images.len() {
            if !enabled && crate::stats::enabled() {
                // Sem efeitos rodando, a medição da GPU de antes não vale mais.
                crate::stats::set_gpu(Vec::new());
            }
            return None;
        }

        // Uma etapa pede profundidade: liga a observação (custo zero enquanto nenhuma pede).
        let needs_depth = steps_need_depth(&fx.steps);
        if needs_depth {
            data.depth.need();
        }
        // A profundidade escolhida mudou (outra imagem, outro layout, apareceu ou sumiu): reconstrói a cadeia.
        if needs_depth && let Fx::Ready(built) = &fx.fx {
            let now = data.depth.chosen().map(|d| (d.image, d.layout));
            if built.depth_key != now {
                let old = built.depth_key;
                fx.retire(&data.device);
                data.depth.chain_rebuilds.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                *data.depth.last_key_change.lock().unwrap() = format!("{old:x?} -> {now:x?}");
                log!("profundidade para os efeitos mudou: cadeia será reconstruída");
            }
        }

        if needs_depth && matches!(fx.fx, Fx::Failed(_)) && fx.failed_depth != data.depth.chosen().map(|d| (d.image, d.layout)) {
            fx.fx = Fx::Unbuilt;
        }

        if matches!(fx.fx, Fx::Unbuilt) {
            if fx.steps.is_empty() {
                return None; // sem bundle para esta resolução: nada a desenhar
            }
            let family = *data.queues.lock().unwrap().get(&queue.as_raw())?;
            // A criação (a compilação dos shaders pelo driver leva segundos) roda noutra thread: o jogo continua sem o efeito
            // até ela terminar, em vez de parar.
            let depth = if steps_need_depth(&fx.steps) { data.depth.chosen() } else { None };
            let args = BuildArgs {
                device: data.device.clone(),
                memory: data.memory,
                images: fx.images.clone(),
                format: fx.format,
                extent: fx.extent,
                steps: fx.steps.clone(),
                depth,
                queue_family: family,
                set_loader_data: data.set_loader_data,
            };
            let signature = signature(&fx.steps);
            // Uma cadeia aposentada que tem efeitos iguais aos novos (mesmo bundle, técnica e profundidade) os cede: só o que mudou é
            // criado, e o que não mudou segue com o histórico (os efeitos temporais não "piscam" porque outro foi ligado).
            let depth_key_now = depth.map(|d| (d.image, d.layout));
            let mut plan: Option<SendPlan> = None;
            if let Some(old) = fx.retiring.as_mut()
                && old.depth_key == depth_key_now
                && old.fences.len() == fx.images.len()
            {
                let chain_steps: Vec<Step<'_>> = fx.steps.iter().map(|s| Step { bundle: &s.bundle, technique: s.technique }).collect();
                if old.chain.shared_mut().merge_scan(&chain_steps) {
                    let mut available: Vec<Option<(u64, usize)>> = old.chain.keys().into_iter().map(Some).collect();
                    let reuse: Vec<Option<usize>> = fx
                        .steps
                        .iter()
                        .map(|s| {
                            let position = available.iter().position(|k| *k == Some((s.bundle.id, s.technique)));
                            if let Some(p) = position {
                                available[p] = None;
                            }
                            position
                        })
                        .collect();
                    if reuse.iter().any(Option::is_some) {
                        plan = Some(SendPlan(reuse, old.chain.take_shared()));
                    }
                }
            }
            let incremental = plan.is_some();
            let steps_for_job = fx.steps.clone();
            let work = move || {
                SendOut(match plan {
                    Some(SendPlan(reuse, shared)) => build_partial(args, reuse, shared),
                    None => build_chain(args).map(Output::Full).map_err(|why| Failure { why, shared: None }),
                })
            };
            match std::thread::Builder::new().name("lr-build".into()).spawn(work) {
                Ok(thread) => {
                    log!("criação da cadeia iniciada noutra thread ({} etapa(s){})", fx.steps.len(), if incremental { ", só as que mudaram" } else { "" });
                    fx.fx = Fx::Building(Job { thread: Some(thread), steps: steps_for_job, incremental, signature, depth_image: depth.map(|d| d.image), depth_key: depth_key_now, started: Instant::now() });
                }
                Err(e) => {
                    log!("não consegui criar a thread da cadeia: {e}");
                    fx.set_state(EffectState::Failed(format!("thread: {e}")));
                    fx.fx = Fx::Failed(format!("thread: {e}"));
                }
            }
            // Com uma cadeia aposentada o quadro segue com ela; sem nenhuma, passa sem efeito.
            if fx.retiring.is_none() {
                return None;
            }
        }
        let build_finished = matches!(&fx.fx, Fx::Building(job) if job.thread.as_ref().is_some_and(|t| t.is_finished()));
        if matches!(&fx.fx, Fx::Building(_)) && !build_finished && fx.retiring.is_none() {
            return None; // ainda criando: o quadro passa sem o efeito
        }
        if build_finished && let Fx::Building(job) = &mut fx.fx {
            let started = job.started;
            let built_with = job.depth_key;
            let job_steps = std::mem::take(&mut job.steps);
            let incremental = job.incremental;
            let result = job.thread.take().map(|t| t.join()).unwrap_or(Err(Box::new("sem thread")));
            let outcome: Result<Built, (String, Option<postfx::shared::Shared>)> = match result {
                Ok(SendOut(Ok(Output::Full(built)))) => Ok(built),
                Ok(SendOut(Ok(Output::Partial(partial)))) => assemble_partial(fx.retiring.take(), partial, &job_steps, built_with, &data.device),
                Ok(SendOut(Err(failure))) => Err((failure.why, failure.shared)),
                Err(_) => Err(("a thread de criação da cadeia falhou".to_string(), None)),
            };
            fx.fx = match outcome {
                Ok(mut built) => {
                    // A inicialização das texturas (só as dos efeitos novos) precisa da fila: aqui, na thread que a usa.
                    match built.chain.finish_init(&data.device, queue) {
                        Ok(()) => {
                            log!("efeito criado para o swapchain {:#x} ({} imagens) em {} ms", swapchain.as_raw(), fx.images.len(), started.elapsed().as_millis());
                            log!("cadeia de {} etapa(s) criada", built.chain.len());
                            crate::status::set_techniques(
                                job_steps
                                    .iter()
                                    .enumerate()
                                    .map(|(i, step)| TechniqueStatus {
                                        name: step.bundle.manifest.techniques.get(step.technique).map(|t| t.name.clone()).unwrap_or_default(),
                                        state: match built.skipped.iter().find(|(s, _)| *s == i) {
                                            Some((_, why)) => TechniqueState::Skipped(why.clone()),
                                            None => TechniqueState::Applied,
                                        },
                                    })
                                    .collect(),
                            );
                            for (step, ignored) in built.chain.ignored_sources() {
                                log!("etapa {step}: uniform sem fonte no runtime (fica no valor padrão): {ignored}");
                            }
                            fx.set_state(EffectState::Ready);
                            if incremental {
                                // A escolha pode ter mudado de novo durante o trabalho: olha-a outra vez no próximo present.
                                fx.seen_epoch = u64::MAX;
                            }
                            Fx::Ready(built)
                        }
                        Err(e) => {
                            let why = e.to_string();
                            log!("efeito indisponível no swapchain {:#x}: {why}", swapchain.as_raw());
                            let mut dead = Fx::Ready(built);
                            SwapchainFx::destroy_fx(&mut dead, &data.device);
                            fx.failed_depth = built_with;
                            fx.set_state(EffectState::Failed(why.clone()));
                            Fx::Failed(why)
                        }
                    }
                }
                Err((why, shared)) => {
                    log!("efeito indisponível no swapchain {:#x}: {why}", swapchain.as_raw());
                    // A cadeia aposentada também apelida as texturas compartilhadas: sai antes delas.
                    fx.drop_retiring(&data.device);
                    if let Some(mut shared) = shared {
                        shared.destroy(&data.device);
                    }
                    fx.failed_depth = built_with;
                    fx.set_state(EffectState::Failed(why.clone()));
                    Fx::Failed(why)
                }
            };
        }
        // A cadeia nova pronta (ou que falhou) substitui a aposentada.
        if matches!(fx.fx, Fx::Ready(_) | Fx::Failed(_)) && fx.retiring.is_some() {
            fx.drop_retiring(&data.device);
        }
        let now = Instant::now();
        let previous = fx.last_present.replace(now);
        let built = match &mut fx.fx {
            Fx::Ready(built) => built,
            _ => match fx.retiring.as_mut() {
                Some(built) => built,
                None => return None,
            },
        };

        // A execução anterior desta imagem precisa ter terminado antes de reenviar o mesmo buffer.
        let fence = built.fences[index];
        if data.device.wait_for_fences(&[fence], true, FENCE_TIMEOUT_NS).is_err() || data.device.reset_fences(&[fence]).is_err() {
            log!("efeito pulado: a execução anterior da imagem {index} não terminou");
            return None;
        }

        // Medição de desempenho (só quando pedida): lê o que a GPU gravou da última vez que usou esta imagem e garante o cronômetro.
        let measure = crate::stats::enabled();
        if measure {
            if built.gpu.is_none() {
                let family = data.queues.lock().unwrap().get(&queue.as_raw()).copied();
                let names: Vec<String> = built
                    .origin
                    .iter()
                    .map(|&i| fx.steps.get(i).and_then(|s| s.bundle.manifest.techniques.get(s.technique)).map_or_else(|| format!("etapa {i}"), |t| t.name.clone()))
                    .collect();
                built.gpu = family.and_then(|f| GpuTimer::new(data, f, fx.images.len(), names));
            }
            if let Some(timer) = built.gpu.as_mut() {
                timer.collect(&data.device, index);
                if data.presents.load(std::sync::atomic::Ordering::Relaxed) % 30 == 0 {
                    crate::stats::set_gpu(timer.steps.clone());
                }
            }
        }

        // A fence garante que a GPU terminou com o buffer de uniforms desta imagem: pode reescrevê-lo.
        // Primeiro os parâmetros de cada etapa (a imagem inteira do buffer)...
        let own = crate::control::channel().map(|c| c as &dyn ipc::ParamSource);
        let global = crate::control::reader().map(|r| r as &dyn ipc::ParamSource);
        for (step, runtime) in built.params.iter_mut().enumerate() {
            match &mut runtime.source {
                StepSource::Published(state) => {
                    if (own.is_some() || global.is_some()) && let Some(bytes) = state.next(own, global, runtime.params_id) {
                        built.chain.write_uniforms(step, index, 0, bytes);
                    }
                }
                StepSource::Plan(Some(bytes)) => built.chain.write_uniforms(step, index, 0, bytes),
                StepSource::Plan(None) => {}
            }
        }
        // ...depois os valores do runtime (tempo etc.), que valem mais que qualquer parâmetro.
        built.chain.set_active_control(active_control().as_deref());
        built.chain.update_sources(index, &frame_info(now, previous, data.presents.load(std::sync::atomic::Ordering::Relaxed)));

        let waits = std::slice::from_raw_parts(info.p_wait_semaphores, info.wait_semaphore_count as usize);
        // ALL_COMMANDS: nada do efeito pode começar antes de o jogo terminar de desenhar o frame.
        let stages = vec![vk::PipelineStageFlags::ALL_COMMANDS; waits.len()];
        let mut command_buffers = built.chain.command_buffers(index);
        let mut timed = false;
        if measure && let Some(timer) = &built.gpu && timer.stamps.get(index).is_some_and(|c| c.len() == command_buffers.len() + 1) {
            // Um carimbo antes do primeiro efeito e um depois de cada um.
            let stamps = &timer.stamps[index];
            let mut all = Vec::with_capacity(command_buffers.len() * 2 + 1);
            all.push(stamps[0]);
            for (i, cb) in command_buffers.iter().enumerate() {
                all.push(*cb);
                all.push(stamps[i + 1]);
            }
            command_buffers = all;
            timed = true;
        }
        let signal = [built.semaphores[index]];
        let submit = vk::SubmitInfo::default()
            .wait_semaphores(waits)
            .wait_dst_stage_mask(&stages)
            .command_buffers(&command_buffers)
            .signal_semaphores(&signal);
        match data.device.queue_submit(queue, &[submit], fence) {
            Ok(()) => {
                if timed && let Some(timer) = built.gpu.as_mut() {
                    timer.written[index] = true;
                }
                Some(signal[0])
            }
            Err(e) => {
                log!("vkQueueSubmit do efeito falhou: erro {}", e.as_raw());
                // A fence já foi resetada; um lote vazio a sinaliza de novo para não travar o frame seguinte.
                let _ = data.device.queue_submit(queue, &[vk::SubmitInfo::default()], fence);
                None
            }
        }
    }
}

/// Alguma etapa amostra a profundidade do jogo (semântica `DEPTH` num sampler de um passe da técnica)?
fn steps_need_depth(steps: &[ResolvedStep]) -> bool {
    steps.iter().any(|s| {
        let m = &s.bundle.manifest;
        let Some(t) = m.techniques.get(s.technique) else { return false };
        t.passes.iter().flat_map(|p| &p.samplers).any(|b| {
            m.samplers.get(b.sampler as usize).is_some_and(|sampler| m.textures.iter().any(|tex| tex.unique_name == sampler.texture && tex.semantic.as_deref() == Some("DEPTH")))
        })
    })
}

/// O jogo vai destruir uma imagem de profundidade: os efeitos que a usam saem antes (têm uma view dela).
pub fn on_depth_image_destroyed(data: &DeviceData, image: u64) {
    let mut swapchains = data.swapchains.lock().unwrap();
    for fx in swapchains.values_mut() {
        if fx.retiring.as_ref().is_some_and(|b| b.depth_key.is_some_and(|k| k.0 == image)) {
            unsafe { fx.drop_retiring(&data.device) };
        }
        if matches!(&fx.fx, Fx::Ready(b) if b.depth_key.is_some_and(|k| k.0 == image)) || matches!(&fx.fx, Fx::Building(j) if j.depth_image == Some(image)) {
            unsafe { SwapchainFx::destroy_fx(&mut fx.fx, &data.device) };
            fx.fx = Fx::Unbuilt;
            log!("efeito descartado: o jogo destruiu a imagem de profundidade que ele usava");
        }
    }
}

/// A imagem que o swapchain apresenta agora (para a captura de tela); `None` se o swapchain não aceita o uso extra.
pub fn frame_target(data: &DeviceData, swapchain: u64, index: usize) -> Option<crate::screenshot::Target> {
    let swapchains = data.swapchains.lock().unwrap();
    let fx = swapchains.get(&swapchain)?;
    if fx.unusable.is_some() {
        return None;
    }
    Some(crate::screenshot::Target { image: *fx.images.get(index)?, extent: fx.extent, format: fx.format })
}

/// O tamanho do maior swapchain do dispositivo (a "tela" a que a profundidade deve se parecer); 0x0 se não há.
pub fn largest_extent(data: &DeviceData) -> (u32, u32) {
    data.swapchains.lock().unwrap().values().map(|fx| (fx.extent.width, fx.extent.height)).max_by_key(|(w, h)| u64::from(*w) * u64::from(*h)).unwrap_or((0, 0))
}

/// A imagem é de um swapchain deste dispositivo?
pub fn is_swapchain_image(data: &DeviceData, image: u64) -> bool {
    data.swapchains.lock().unwrap().values().any(|fx| fx.images.iter().any(|i| i.as_raw() == image))
}
