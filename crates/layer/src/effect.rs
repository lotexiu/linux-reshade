//! Efeito de pós-processamento de um swapchain: criação preguiçosa dos recursos do `postfx`,
//! submissão a cada present e destruição.

use crate::log::log;
use crate::state::DeviceData;
use ash::vk::{self, Handle};
use postfx::{InvertPass, Target};

/// Recursos que só existem depois do primeiro present com os efeitos ligados.
struct Built {
    pass: InvertPass,
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
}

// SAFETY: contém handles do Vulkan (inteiros/ponteiros opacos) e é sempre acessado sob o `Mutex`
// de `DeviceData::swapchains`.
unsafe impl Send for SwapchainFx {}

impl SwapchainFx {
    pub fn new(images: Vec<vk::Image>, format: vk::Format, extent: vk::Extent2D, usable: bool) -> Self {
        Self { images, format, extent, usable, fx: Fx::Unbuilt }
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
                built.pass.destroy(device);
                for s in built.semaphores.drain(..) {
                    device.destroy_semaphore(s, None);
                }
                for f in built.fences.drain(..) {
                    device.destroy_fence(f, None);
                }
            }
        }
    }

    unsafe fn build(&mut self, data: &DeviceData, queue_family: u32) -> Result<Built, vk::Result> {
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
            let pass = InvertPass::new(&data.device, &data.memory, &target)?;
            let mut built = Built { pass, semaphores: Vec::new(), fences: Vec::new() };
            for _ in &self.images {
                let semaphore = data.device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None);
                // Nasce sinalizada: a primeira espera por ela não bloqueia.
                let fence = data
                    .device
                    .create_fence(&vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED), None);
                match (semaphore, fence) {
                    (Ok(s), Ok(f)) => {
                        built.semaphores.push(s);
                        built.fences.push(f);
                    }
                    (s, f) => {
                        // Libera o par que deu certo e tudo o que já foi criado.
                        if let Ok(s) = s {
                            data.device.destroy_semaphore(s, None);
                        }
                        if let Ok(f) = f {
                            data.device.destroy_fence(f, None);
                        }
                        let error = s.err().or(f.err()).unwrap_or(vk::Result::ERROR_UNKNOWN);
                        let mut fx = Fx::Ready(built);
                        Self::destroy_fx(&mut fx, &data.device);
                        return Err(error);
                    }
                }
            }
            Ok(built)
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
        if matches!(fx.fx, Fx::Unbuilt) {
            fx.fx = match fx.build(data, family) {
                Ok(built) => {
                    log!("efeito criado para o swapchain {:#x} ({} imagens)", swapchain.as_raw(), fx.images.len());
                    Fx::Ready(built)
                }
                Err(e) => {
                    log!("falha ao criar o efeito do swapchain {:#x}: erro {}", swapchain.as_raw(), e.as_raw());
                    Fx::Failed
                }
            };
        }
        let Fx::Ready(built) = &fx.fx else { return None };

        // A execução anterior desta imagem precisa ter terminado antes de reenviar o mesmo buffer.
        let fence = built.fences[index];
        if data.device.wait_for_fences(&[fence], true, FENCE_TIMEOUT_NS).is_err() || data.device.reset_fences(&[fence]).is_err() {
            log!("efeito pulado: a execução anterior da imagem {index} não terminou");
            return None;
        }

        let waits = std::slice::from_raw_parts(info.p_wait_semaphores, info.wait_semaphore_count as usize);
        let stages = vec![vk::PipelineStageFlags::TRANSFER; waits.len()];
        let command_buffers = [built.pass.command_buffer(index)];
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
