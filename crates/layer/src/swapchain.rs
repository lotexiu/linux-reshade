//! `vkCreateSwapchainKHR` / `vkDestroySwapchainKHR`.

use crate::effect::SwapchainFx;
use crate::log::log;
use crate::state::{DEVICES, DeviceData};
use ash::vk::{self, Handle};
use vk_layer::dispatch::dispatch_key;

/// Uso extra que o efeito precisa: escrever no swapchain (anexo de cor), copiá-lo e receber cópias (alvo sRGB, escrita sRGB).
const EFFECT_USAGE: vk::ImageUsageFlags = vk::ImageUsageFlags::from_raw(
    vk::ImageUsageFlags::COLOR_ATTACHMENT.as_raw() | vk::ImageUsageFlags::TRANSFER_SRC.as_raw() | vk::ImageUsageFlags::TRANSFER_DST.as_raw(),
);

/// A superfície aceita o uso extra?
unsafe fn surface_supports_effect(data: &DeviceData, surface: vk::SurfaceKHR) -> bool {
    let mut caps = vk::SurfaceCapabilitiesKHR::default();
    let result = unsafe {
        (data.instance.surface.get_physical_device_surface_capabilities_khr)(data.physical_device, surface, &mut caps)
    };
    result == vk::Result::SUCCESS && caps.supported_usage_flags.contains(EFFECT_USAGE)
}

unsafe fn swapchain_images(data: &DeviceData, device: vk::Device, swapchain: vk::SwapchainKHR) -> Vec<vk::Image> {
    let get = data.swapchain.get_swapchain_images_khr;
    let mut count = 0;
    unsafe {
        if get(device, swapchain, &mut count, std::ptr::null_mut()) != vk::Result::SUCCESS {
            return Vec::new();
        }
        let mut images = vec![vk::Image::null(); count as usize];
        if get(device, swapchain, &mut count, images.as_mut_ptr()) != vk::Result::SUCCESS {
            return Vec::new();
        }
        images.truncate(count as usize);
        images
    }
}

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
        // Cópia do pedido do jogo, com o uso extra se a superfície aceitar; senão, sem efeito neste swapchain.
        let mut info = *p_create_info;
        // A layer só está no processo porque o usuário a ligou para este jogo (LINUX_RESHADE=1); o efeito pode
        // ser escolhido a qualquer momento, e o uso extra só pode ser pedido na criação do swapchain.
        let supported = info.image_array_layers == 1 && surface_supports_effect(&data, info.surface);
        if supported {
            info.image_usage |= EFFECT_USAGE;
        }

        log!("vkCreateSwapchainKHR pedido: {}x{} uso={:#x} flags={:#x} imagens>={} modo={} formato={} estruturas encadeadas {:?}", info.image_extent.width, info.image_extent.height, info.image_usage.as_raw(), info.flags.as_raw(), info.min_image_count, info.present_mode.as_raw(), info.image_format.as_raw(), crate::log::chain_types(info.p_next));
        let result = (data.swapchain.create_swapchain_khr)(device, &info, p_allocator, p_swapchain);
        if result != vk::Result::SUCCESS {
            return result;
        }
        let swapchain = *p_swapchain;
        let images = swapchain_images(&data, device, swapchain);
        let unusable = if !supported {
            Some("the swapchain does not accept the extra usage the effect needs (COLOR_ATTACHMENT | TRANSFER_SRC | TRANSFER_DST)".to_string())
        } else if images.is_empty() {
            Some("could not get the swapchain images".to_string())
        } else {
            None
        };
        log!(
            "vkCreateSwapchainKHR ok: {}x{} format={} images={} present_mode={} efeito={}",
            info.image_extent.width,
            info.image_extent.height,
            info.image_format.as_raw(),
            images.len(),
            info.present_mode.as_raw(),
            if unusable.is_none() { "possível" } else { "indisponível" },
        );
        let fx = SwapchainFx::new(swapchain.as_raw(), images, info.image_format, info.image_extent, unusable);
        data.swapchains.lock().unwrap().insert(swapchain.as_raw(), fx);
        vk::Result::SUCCESS
    }
}

pub unsafe extern "system" fn destroy_swapchain(
    device: vk::Device,
    swapchain: vk::SwapchainKHR,
    p_allocator: *const vk::AllocationCallbacks<'_>,
) {
    unsafe {
        let Some(data) = DEVICES.get(dispatch_key(device.as_raw())) else { return };
        // Uma criação descartada em segundo plano guarda views das imagens do swapchain: espera ela acabar.
        crate::effect::wait_cleanups(std::time::Duration::from_secs(10));
        let removed = data.swapchains.lock().unwrap().remove(&swapchain.as_raw());
        if let Some(mut fx) = removed {
            // Um present anterior pode ainda estar executando o efeito: `destroy` espera as fences da cadeia.
            fx.destroy(&data.device);
        }
        (data.swapchain.destroy_swapchain_khr)(device, swapchain, p_allocator);
    }
}
