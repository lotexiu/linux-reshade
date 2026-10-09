//! Efeito de pós-processamento de um swapchain: criação preguiçosa dos recursos do `postfx`,
//! submissão a cada present e destruição.

use crate::log::log;
use crate::state::DeviceData;
use ash::vk::{self, Handle};
use postfx::effect::{FrameInfo, Target, utc_date};
use crate::params::ParamState;
use fx_bundle::Bundle;
use postfx::Effect;
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
    effect: Effect,
    /// Identifica o bundle nos parâmetros publicados pelo app de controle.
    bundle_id: u64,
    params: crate::params::ParamState,
    /// Um semáforo por imagem do swapchain: o da imagem sinaliza que o efeito terminou nela.
    semaphores: Vec<vk::Semaphore>,
    /// Uma fence por imagem, sinalizada quando o command buffer dela termina. Garante que o buffer
    /// não é reenviado enquanto ainda executa (o que o swapchain sugere, mas não garante).
    fences: Vec<vk::Fence>,
}

/// Quanto esperar a execução anterior de uma imagem antes de desistir de aplicar o efeito neste frame.
const FENCE_TIMEOUT_NS: u64 = 1_000_000_000;

enum Fx {
    Unbuilt,
    Ready(Built),
    /// A criação falhou; não tenta de novo (evita falhar a cada frame).
    Failed,
}

pub struct SwapchainFx {
    images: Vec<vk::Image>,
    format: vk::Format,
    extent: vk::Extent2D,
    /// Falso se a superfície não aceita o uso extra (`COLOR_ATTACHMENT | TRANSFER_SRC`) ou o caso
    /// não é suportado (por exemplo, mais de uma camada).
    usable: bool,
    fx: Fx,
    /// Época do bundle com que `fx` foi construído (ver `bundle::current`).
    built_epoch: u64,
    /// Quando o present anterior deste swapchain começou (para o `frametime`).
    last_present: Option<Instant>,
}

// SAFETY: contém handles do Vulkan (inteiros/ponteiros opacos) e é sempre acessado sob o `Mutex`
// de `DeviceData::swapchains`.
unsafe impl Send for SwapchainFx {}

impl SwapchainFx {
    pub fn new(images: Vec<vk::Image>, format: vk::Format, extent: vk::Extent2D, usable: bool) -> Self {
        Self { images, format, extent, usable, fx: Fx::Unbuilt, built_epoch: 0, last_present: None }
    }

    /// Libera os recursos de GPU.
    ///
    /// # Safety
    /// A GPU não pode estar executando trabalho deste swapchain (o chamador espera o dispositivo ficar ocioso).
    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        unsafe { Self::destroy_fx(&mut self.fx, device) };
        self.fx = Fx::Failed;
    }

    unsafe fn destroy_fx(fx: &mut Fx, device: &ash::Device) {
        if let Fx::Ready(mut built) = std::mem::replace(fx, Fx::Failed) {
            unsafe {
                built.effect.destroy(device);
                for s in built.semaphores.drain(..) {
                    device.destroy_semaphore(s, None);
                }
                for f in built.fences.drain(..) {
                    device.destroy_fence(f, None);
                }
            }
        }
    }

    unsafe fn build(&mut self, data: &DeviceData, bundle: &Bundle, queue: vk::Queue, queue_family: u32) -> Result<Built, String> {
        let t = bundle.manifest.target;
        if (t.width, t.height) != (self.extent.width, self.extent.height) {
            return Err(format!(
                "o bundle foi compilado para {}x{} e o swapchain é {}x{}",
                t.width, t.height, self.extent.width, self.extent.height
            ));
        }
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
        };
        unsafe {
            let effect = Effect::new(&data.device, &data.memory, bundle, 0, &target, queue).map_err(|e| e.to_string())?;
            let mut semaphores = Vec::new();
            let mut fences = Vec::new();
            for _ in &self.images {
                let semaphore = data.device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None);
                // Nasce sinalizada: a primeira espera por ela não bloqueia.
                let fence = data
                    .device
                    .create_fence(&vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED), None);
                match (semaphore, fence) {
                    (Ok(s), Ok(f)) => {
                        semaphores.push(s);
                        fences.push(f);
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
                        let mut partial = Fx::Ready(Built { effect, bundle_id: bundle.id, params: ParamState::new(Vec::new()), semaphores, fences });
                        Self::destroy_fx(&mut partial, &data.device);
                        return Err(format!("falha ao criar semáforo/fence (erro {})", error.as_raw()));
                    }
                }
            }
            let params = ParamState::new(fx_bundle::uniforms::default_block(&bundle.manifest));
            Ok(Built { effect, bundle_id: bundle.id, params, semaphores, fences })
        }
    }
}

/// Aplica o efeito a este present, se possível. Devolve o semáforo que o present deve esperar no
/// lugar dos originais; `None` significa "apresente como o jogo pediu".
///
/// # Safety
/// `info` deve ser o `VkPresentInfoKHR` recebido do jogo, e `queue` a fila do present.
pub unsafe fn apply(data: &DeviceData, queue: vk::Queue, info: &vk::PresentInfoKHR<'_>) -> Option<vk::Semaphore> {
    // Vários swapchains num mesmo present compartilham a lista de espera: fora do caso comum.
    if info.swapchain_count != 1 {
        return None;
    }
    unsafe {
        let swapchain = *info.p_swapchains;
        let index = *info.p_image_indices as usize;
        let family = *data.queues.lock().unwrap().get(&queue.as_raw())?;

        let mut swapchains = data.swapchains.lock().unwrap();
        let fx = swapchains.get_mut(&swapchain.as_raw())?;
        if !fx.usable || index >= fx.images.len() {
            return None;
        }
        // O bundle pode ter mudado (o app de controle escolheu outro efeito): o que foi construído com o
        // antigo é descartado e refeito. A GPU precisa estar ociosa para destruir os recursos.
        let (epoch, bundle) = crate::bundle::current();
        if fx.built_epoch != epoch && !matches!(fx.fx, Fx::Unbuilt) {
            log!("efeito do swapchain {:#x} descartado: o bundle mudou", swapchain.as_raw());
            let _ = data.device.device_wait_idle();
            fx.destroy(&data.device);
            fx.fx = Fx::Unbuilt;
        }
        if matches!(fx.fx, Fx::Unbuilt) {
            fx.built_epoch = epoch;
            let Some(bundle) = bundle else {
                // Sem efeito escolhido: não há o que desenhar. O estado volta a `Unbuilt` para uma
                // escolha futura (a época muda) reconstruir; o log fala só na primeira vez.
                return None;
            };
            fx.fx = match fx.build(data, &bundle, queue, family) {
                Ok(built) => {
                    log!("efeito criado para o swapchain {:#x} ({} imagens)", swapchain.as_raw(), fx.images.len());
                    for ignored in built.effect.ignored_sources() {
                        log!("uniform sem fonte no runtime (fica no valor padrão): {ignored}");
                    }
                    Fx::Ready(built)
                }
                Err(why) => {
                    log!("efeito indisponível no swapchain {:#x}: {why}", swapchain.as_raw());
                    Fx::Failed
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
        // Primeiro os parâmetros do app de controle (a imagem inteira do buffer)...
        if let Some(reader) = crate::control::reader() {
            if let Some(bytes) = built.params.next(reader, built.bundle_id) {
                built.effect.write_uniforms(index, 0, bytes);
            }
        }
        // ...depois os valores do runtime (tempo etc.), que valem mais que qualquer parâmetro.
        built.effect.update_sources(index, &frame_info(now, previous, data.presents.load(std::sync::atomic::Ordering::Relaxed)));

        let waits = std::slice::from_raw_parts(info.p_wait_semaphores, info.wait_semaphore_count as usize);
        // ALL_COMMANDS: nada do efeito pode começar antes de o jogo terminar de desenhar o frame.
        let stages = vec![vk::PipelineStageFlags::ALL_COMMANDS; waits.len()];
        let command_buffers = [built.effect.command_buffer(index)];
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
