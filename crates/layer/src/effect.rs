//! Efeito de pós-processamento de um swapchain: criação preguiçosa dos recursos do `postfx`,
//! submissão a cada present e destruição.

use crate::bundle::{self, Resolved, ResolvedStep, StepParams};
use crate::log::log;
use crate::params::ParamState;
use crate::state::DeviceData;
use ash::vk::{self, Handle};
use ipc::{EffectState, SwapchainStatus, TechniqueState, TechniqueStatus};
use postfx::effect::{DepthInput, FrameInfo, Target, utc_date};
use postfx::{Chain, Step};
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
const FENCE_TIMEOUT_NS: u64 = 1_000_000_000;

enum Fx {
    Unbuilt,
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
        let fx = Self { handle, images, format, extent, unusable, fx: Fx::Unbuilt, seen_epoch: u64::MAX, failed_depth: None, steps: Vec::new(), state, last_present: None };
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
                // A GPU precisa estar ociosa para destruir os recursos.
                unsafe {
                    let _ = data.device.device_wait_idle();
                    Self::destroy_fx(&mut self.fx, &data.device);
                }
                self.fx = Fx::Unbuilt;
            } else if let Some(built) = match &mut self.fx { Fx::Ready(b) => Some(b), _ => None } {
                // A mesma cadeia: só os uniforms do plano podem ter mudado.
                for (runtime, &from) in built.params.iter_mut().zip(&built.origin) {
                    if let (StepSource::Plan(bytes), Some(StepParams::Plan(new))) = (&mut runtime.source, steps.get(from).map(|s| &s.params)) {
                        *bytes = new.clone();
                    }
                }
            }
        }
        // Uma falha anterior era da cadeia antiga: com outra vale tentar de novo.
        if matches!(self.fx, Fx::Failed(_)) && signature(&self.steps) != new_signature {
            self.fx = Fx::Unbuilt;
        }
        self.steps = steps;
        let state = match &self.fx {
            Fx::Failed(why) => EffectState::Failed(why.clone()),
            _ => state,
        };
        self.set_state(state);
    }

    /// Libera os recursos de GPU.
    ///
    /// # Safety
    /// A GPU não pode estar executando trabalho deste swapchain (o chamador espera o dispositivo ficar ocioso).
    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        unsafe { Self::destroy_fx(&mut self.fx, device) };
        self.fx = Fx::Failed("destruído".into());
        crate::status::remove(self.handle);
    }

    unsafe fn destroy_fx(fx: &mut Fx, device: &ash::Device) {
        if let Fx::Ready(mut built) = std::mem::replace(fx, Fx::Unbuilt) {
            unsafe {
                built.chain.destroy(device);
                for s in built.semaphores.drain(..) {
                    device.destroy_semaphore(s, None);
                }
                for f in built.fences.drain(..) {
                    device.destroy_fence(f, None);
                }
            }
        }
    }

    unsafe fn build(&mut self, data: &DeviceData, steps: &[ResolvedStep], queue: vk::Queue, queue_family: u32) -> Result<Built, String> {
        for (n, step) in steps.iter().enumerate() {
            let t = step.bundle.manifest.target;
            if (t.width, t.height) != (self.extent.width, self.extent.height) {
                return Err(format!(
                    "etapa {n}: o bundle foi compilado para {}x{} e o swapchain é {}x{}",
                    t.width, t.height, self.extent.width, self.extent.height
                ));
            }
        }
        // A profundidade do jogo, só se alguma etapa a usa (e já foi achada).
        let depth = if steps_need_depth(steps) { data.depth.chosen() } else { None };
        let target = Target {
            images: &self.images,
            format: self.format,
            extent: self.extent,
            rest_layout: vk::ImageLayout::PRESENT_SRC_KHR,
            queue_family,
            // O command buffer nasce sem o ponteiro de dispatch do loader; as layers abaixo precisam dele.
            init_command_buffer: Some(&|cb| {
                let result = unsafe { (data.set_loader_data)(data.device.handle(), cb.as_raw() as *mut std::ffi::c_void) };
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
        let chain_steps: Vec<Step<'_>> = steps.iter().map(|s| Step { bundle: &s.bundle, technique: s.technique }).collect();
        unsafe {
            let (chain, skipped) = Chain::new_lenient(&data.device, &data.memory, &chain_steps, &target, queue).map_err(|e| e.to_string())?;
            for s in &skipped {
                log!("etapa {} pulada: {}", s.step, s.why);
            }
            if chain.is_empty() && !steps.is_empty() {
                return Err(format!("nenhuma etapa é suportada ({})", skipped.iter().map(|s| format!("etapa {}: {}", s.step, s.why)).collect::<Vec<_>>().join("; ")));
            }
            // Os parâmetros seguem as etapas que ficaram na cadeia (as puladas não têm efeito a alimentar).
            let origin: Vec<usize> = (0..chain.len()).filter_map(|i| chain.origin(i)).collect();
            let params = origin
                .iter()
                .map(|&i| &steps[i])
                .map(|s| StepRuntime {
                    params_id: s.bundle.params_id(),
                    source: match &s.params {
                        StepParams::Legacy => StepSource::Published(ParamState::new(fx_bundle::uniforms::default_block(&s.bundle.manifest))),
                        StepParams::Plan(bytes) => StepSource::Plan(bytes.clone()),
                    },
                })
                .collect();
            let skipped = skipped.iter().map(|s| (s.step, s.why.clone())).collect();
            let mut built = Built { chain, signature: signature(steps), origin, skipped, depth_key: depth.map(|d| (d.image, d.layout)), params, semaphores: Vec::new(), fences: Vec::new() };
            for _ in &self.images {
                let semaphore = data.device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None);
                // Nasce sinalizada: a primeira espera por ela não bloqueia.
                let fence = data.device.create_fence(&vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED), None);
                match (semaphore, fence) {
                    (Ok(s), Ok(f)) => {
                        built.semaphores.push(s);
                        built.fences.push(f);
                    }
                    (s, f) => {
                        // Libera o par que deu certo e tudo o que já foi criado.
                        let error = s.as_ref().err().or(f.as_ref().err()).copied().unwrap_or(vk::Result::ERROR_UNKNOWN);
                        if let Ok(s) = s {
                            data.device.destroy_semaphore(s, None);
                        }
                        if let Ok(f) = f {
                            data.device.destroy_fence(f, None);
                        }
                        let mut partial = Fx::Ready(built);
                        Self::destroy_fx(&mut partial, &data.device);
                        return Err(format!("falha ao criar semáforo/fence (erro {})", error.as_raw()));
                    }
                }
            }
            Ok(built)
        }
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
                let _ = data.device.device_wait_idle();
                SwapchainFx::destroy_fx(&mut fx.fx, &data.device);
                fx.fx = Fx::Unbuilt;
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
            let steps = fx.steps.clone();
            let family = *data.queues.lock().unwrap().get(&queue.as_raw())?;
            fx.fx = match fx.build(data, &steps, queue, family) {
                Ok(built) => {
                    log!("efeito criado para o swapchain {:#x} ({} imagens)", swapchain.as_raw(), fx.images.len());
                    log!("cadeia de {} etapa(s) criada", built.chain.len());
                    crate::status::set_techniques(
                        fx.steps
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
                    Fx::Ready(built)
                }
                Err(why) => {
                    log!("efeito indisponível no swapchain {:#x}: {why}", swapchain.as_raw());
                    fx.failed_depth = data.depth.chosen().map(|d| (d.image, d.layout));
                    fx.set_state(EffectState::Failed(why.clone()));
                    Fx::Failed(why)
                }
            };
        }
        let now = Instant::now();
        let previous = fx.last_present.replace(now);
        let Fx::Ready(built) = &mut fx.fx else { return None };

        // A execução anterior desta imagem precisa ter terminado antes de reenviar o mesmo buffer.
        let fence = built.fences[index];
        if data.device.wait_for_fences(&[fence], true, FENCE_TIMEOUT_NS).is_err() || data.device.reset_fences(&[fence]).is_err() {
            log!("efeito pulado: a execução anterior da imagem {index} não terminou");
            return None;
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
        built.chain.update_sources(index, &frame_info(now, previous, data.presents.load(std::sync::atomic::Ordering::Relaxed)));

        let waits = std::slice::from_raw_parts(info.p_wait_semaphores, info.wait_semaphore_count as usize);
        // ALL_COMMANDS: nada do efeito pode começar antes de o jogo terminar de desenhar o frame.
        let stages = vec![vk::PipelineStageFlags::ALL_COMMANDS; waits.len()];
        let command_buffers = built.chain.command_buffers(index);
        let signal = [built.semaphores[index]];
        let submit = vk::SubmitInfo::default()
            .wait_semaphores(waits)
            .wait_dst_stage_mask(&stages)
            .command_buffers(&command_buffers)
            .signal_semaphores(&signal);
        match data.device.queue_submit(queue, &[submit], fence) {
            Ok(()) => Some(signal[0]),
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
        if matches!(&fx.fx, Fx::Ready(b) if b.depth_key.is_some_and(|k| k.0 == image)) {
            unsafe {
                let _ = data.device.device_wait_idle();
                SwapchainFx::destroy_fx(&mut fx.fx, &data.device);
            }
            fx.fx = Fx::Unbuilt;
            log!("efeito descartado: o jogo destruiu a imagem de profundidade que ele usava");
        }
    }
}

/// O tamanho do maior swapchain do dispositivo (a "tela" a que a profundidade deve se parecer); 0x0 se não há.
pub fn largest_extent(data: &DeviceData) -> (u32, u32) {
    data.swapchains.lock().unwrap().values().map(|fx| (fx.extent.width, fx.extent.height)).max_by_key(|(w, h)| u64::from(*w) * u64::from(*h)).unwrap_or((0, 0))
}
