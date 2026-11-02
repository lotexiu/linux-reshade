//! `vkQueuePresentKHR`: ponto onde o efeito é aplicado, no mesmo frame que o jogo acabou de desenhar.

use crate::control::effects_enabled;
use crate::effect;
use crate::log::log;
use crate::state::DEVICES;
use ash::vk::{self, Handle};
use std::sync::atomic::Ordering;
use vk_layer::dispatch::dispatch_key;

/// Um log a cada tantos presents (a primeira apresentação sempre é registrada).
const PRESENT_LOG_EVERY: u64 = 300;
/// Quantos presents com resultado diferente de SUCCESS registrar (evita inundar o log).
const ODD_PRESENT_LOG_MAX: u64 = 20;

pub unsafe extern "system" fn queue_present(
    queue: vk::Queue,
    p_present_info: *const vk::PresentInfoKHR<'_>,
) -> vk::Result {
    unsafe {
        // A Queue compartilha a chave de dispatch do Device.
        let Some(data) = DEVICES.get(dispatch_key(queue.as_raw())) else {
            return vk::Result::ERROR_DEVICE_LOST;
        };
        let measure = crate::stats::enabled();
        let started = measure.then(std::time::Instant::now);
        let n = data.presents.fetch_add(1, Ordering::Relaxed) + 1;
        if n == 1 || n % PRESENT_LOG_EVERY == 0 {
            log!("vkQueuePresentKHR #{n}");
        }
        let enabled = effects_enabled(n);
        if data.last_effects.swap(enabled, Ordering::Relaxed) != enabled {
            log!("efeitos {} (present #{n})", if enabled { "LIGADOS" } else { "DESLIGADOS" });
        }

        // Captura de tela pedida: a cópia do quadro do jogo (sem efeitos) entra na fila antes do efeito, que passa a esperar por ela.
        let info = &*p_present_info;
        let mut effect_info = *info;
        let mut before = None;
        let mut also_without = false;
        let mut number = 0;
        let pre_wait;
        if info.swapchain_count == 1
            && let Some(request) = crate::screenshot::poll()
        {
            let (swapchain, index) = (*info.p_swapchains, *info.p_image_indices as usize);
            let family = data.queues.lock().unwrap().get(&queue.as_raw()).copied();
            if let (Some(target), Some(family)) = (effect::frame_target(&data, swapchain.as_raw(), index), family) {
                let waits = std::slice::from_raw_parts(info.p_wait_semaphores, info.wait_semaphore_count as usize);
                before = crate::screenshot::capture(&data, queue, family, target, waits, request.number);
                also_without = request.also_without_effects;
                number = request.number;
            }
        }
        if let Some(capture) = &before {
            pre_wait = [capture.semaphore];
            effect_info.wait_semaphore_count = 1;
            effect_info.p_wait_semaphores = pre_wait.as_ptr();
        }

        // Com o efeito aplicado, o present passa a esperar o semáforo do efeito, não os do jogo
        // (que o submit do efeito já consumiu).
        let process_started = measure.then(std::time::Instant::now);
        let effect_semaphore = effect::process(&data, queue, &effect_info, enabled);
        let process_time = process_started.map(|t| t.elapsed());
        if enabled {
            data.depth.fx_presents[usize::from(effect_semaphore.is_some())].fetch_add(1, Ordering::Relaxed);
        }
        // O que o present espera: o efeito; ou, sem efeito mas com captura, a cópia (que consumiu os semáforos do jogo).
        let mut final_wait = effect_semaphore.or(before.as_ref().map(|c| c.semaphore));
        if let Some(pre) = before {
            let family = data.queues.lock().unwrap().get(&queue.as_raw()).copied().unwrap_or(0);
            match (effect_semaphore, effect::frame_target(&data, (*info.p_swapchains).as_raw(), *info.p_image_indices as usize)) {
                (Some(effect_done), Some(target)) => {
                    // Com efeitos: a cópia do resultado espera o efeito e o present espera a cópia.
                    let after = crate::screenshot::capture(&data, queue, family, target, &[effect_done], number);
                    if let Some(after) = &after {
                        final_wait = Some(after.semaphore);
                    }
                    if also_without {
                        crate::screenshot::finish(&data, pre, ipc::shot::Kind::WithoutEffects);
                    } else {
                        crate::screenshot::finish_discard(&data, pre);
                    }
                    if let Some(after) = after {
                        crate::screenshot::finish(&data, after, ipc::shot::Kind::WithEffects);
                    }
                }
                _ => crate::screenshot::finish(&data, pre, ipc::shot::Kind::WithEffects),
            }
        }
        let wait = [final_wait.unwrap_or_default()];
        let patched;
        let p_info = if final_wait.is_some() {
            let mut copy = *p_present_info;
            copy.wait_semaphore_count = 1;
            copy.p_wait_semaphores = wait.as_ptr();
            patched = copy;
            &patched as *const vk::PresentInfoKHR<'_>
        } else {
            p_present_info
        };

        let depth_started = measure.then(std::time::Instant::now);
        crate::depth::end_frame(&data, effect::largest_extent(&data));
        let depth_time = depth_started.map(|t| t.elapsed());
        let before_driver = started.map(|t| t.elapsed());
        let result = (data.swapchain.queue_present_khr)(queue, p_info);
        if let (Some(layer), Some(process), Some(depth)) = (before_driver, process_time, depth_time) {
            crate::stats::frame_done(layer, process, depth);
        }
        if result != vk::Result::SUCCESS && data.odd_presents.fetch_add(1, Ordering::Relaxed) < ODD_PRESENT_LOG_MAX {
            log!("vkQueuePresentKHR #{n} devolveu {}", result.as_raw());
        }
        result
    }
}
