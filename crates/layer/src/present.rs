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
        let n = data.presents.fetch_add(1, Ordering::Relaxed) + 1;
        if n == 1 || n % PRESENT_LOG_EVERY == 0 {
            log!("vkQueuePresentKHR #{n}");
        }
        let enabled = effects_enabled(n);
        if data.last_effects.swap(enabled, Ordering::Relaxed) != enabled {
            log!("efeitos {} (present #{n})", if enabled { "LIGADOS" } else { "DESLIGADOS" });
        }

        // Com o efeito aplicado, o present passa a esperar o semáforo do efeito, não os do jogo
        // (que o submit do efeito já consumiu).
        let effect_semaphore = effect::process(&data, queue, &*p_present_info, enabled);
        let wait = [effect_semaphore.unwrap_or_default()];
        let patched;
        let p_info = if effect_semaphore.is_some() {
            let mut copy = *p_present_info;
            copy.wait_semaphore_count = 1;
            copy.p_wait_semaphores = wait.as_ptr();
            patched = copy;
            &patched as *const vk::PresentInfoKHR<'_>
        } else {
            p_present_info
        };

        let result = (data.swapchain.queue_present_khr)(queue, p_info);
        if result != vk::Result::SUCCESS && data.odd_presents.fetch_add(1, Ordering::Relaxed) < ODD_PRESENT_LOG_MAX {
            log!("vkQueuePresentKHR #{n} devolveu {}", result.as_raw());
        }
        result
    }
}
