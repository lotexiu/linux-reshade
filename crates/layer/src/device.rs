//! `vkCreateDevice` / `vkDestroyDevice`.

use crate::log::log;
use crate::state::{DEVICES, DeviceData, INSTANCES};
use ash::vk::{self, Handle};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use vk_layer::chain;
use vk_layer::dispatch::dispatch_key;

pub unsafe extern "system" fn create_device(
    physical_device: vk::PhysicalDevice,
    p_create_info: *const vk::DeviceCreateInfo<'_>,
    p_allocator: *const vk::AllocationCallbacks<'_>,
    p_device: *mut vk::Device,
) -> vk::Result {
    unsafe {
        // O PhysicalDevice compartilha a chave de dispatch do Instance.
        let Some(instance) = INSTANCES.get(dispatch_key(physical_device.as_raw())) else {
            return vk::Result::ERROR_INITIALIZATION_FAILED;
        };
        let Some(link) = chain::device_link(p_create_info) else {
            return vk::Result::ERROR_INITIALIZATION_FAILED;
        };
        let layer_info = (*link).u.p_layer_info;
        let next_gdpa = (*layer_info).pfn_next_get_device_proc_addr;
        (*link).u.p_layer_info = (*layer_info).p_next;

        // `vkCreateDevice` do próximo elo, carregado junto com o Instance.
        let result = (instance.instance.fp_v1_0().create_device)(physical_device, p_create_info, p_allocator, p_device);
        if result != vk::Result::SUCCESS {
            return result;
        }

        let handle = *p_device;
        let device = ash::Device::load_with(
            |name| std::mem::transmute(next_gdpa(handle, name.as_ptr())),
            handle,
        );
        let swapchain = ash::khr::swapchain::DeviceFn::load(|name| std::mem::transmute(next_gdpa(handle, name.as_ptr())));
        DEVICES.insert(
            dispatch_key(handle.as_raw()),
            DeviceData {
                instance,
                physical_device,
                device,
                next_gdpa,
                swapchain,
                presents: AtomicU64::new(0),
                odd_presents: AtomicU64::new(0),
                last_effects: AtomicBool::new(false),
            },
        );
        log!("vkCreateDevice ok");
        vk::Result::SUCCESS
    }
}

pub unsafe extern "system" fn destroy_device(device: vk::Device, p_allocator: *const vk::AllocationCallbacks<'_>) {
    unsafe {
        if let Some(data) = DEVICES.remove(dispatch_key(device.as_raw())) {
            log!("vkDestroyDevice (presents={})", data.presents.load(Ordering::Relaxed));
            (data.device.fp_v1_0().destroy_device)(device, p_allocator);
        }
    }
}
