//! `vkQueuePresentKHR`.

use crate::control::effects_enabled;
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
        let result = (data.swapchain.queue_present_khr)(queue, p_present_info);
        if result != vk::Result::SUCCESS && data.odd_presents.fetch_add(1, Ordering::Relaxed) < ODD_PRESENT_LOG_MAX {
            log!("vkQueuePresentKHR #{n} devolveu {}", result.as_raw());
        }
        result
    }
}
