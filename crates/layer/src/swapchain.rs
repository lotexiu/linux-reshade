//! `vkCreateSwapchainKHR`.

use crate::log::log;
use crate::state::DEVICES;
use ash::vk::{self, Handle};
use vk_layer::dispatch::dispatch_key;

pub unsafe extern "system" fn create_swapchain(
    device: vk::Device,
    p_create_info: *const vk::SwapchainCreateInfoKHR<'_>,
    p_allocator: *const vk::AllocationCallbacks<'_>,
    p_swapchain: *mut vk::SwapchainKHR,
) -> vk::Result {
    unsafe {
        let Some(data) = DEVICES.get(dispatch_key(device.as_raw())) else {
            return vk::Result::ERROR_DEVICE_LOST;
        };
        let result = (data.swapchain.create_swapchain_khr)(device, p_create_info, p_allocator, p_swapchain);
        if result == vk::Result::SUCCESS {
            let info = &*p_create_info;
            log!(
                "vkCreateSwapchainKHR ok: {}x{} format={} images>={} present_mode={}",
                info.image_extent.width,
                info.image_extent.height,
                info.image_format.as_raw(),
                info.min_image_count,
                info.present_mode.as_raw(),
            );
        }
        result
    }
}
