//! `vkCreateDevice` / `vkDestroyDevice`.

use crate::log::log;
use crate::state::{DEVICES, DeviceData, INSTANCES};
use std::collections::HashMap;
use std::sync::Mutex;
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
        // Precisa ser lido antes da criação: a struct só é válida durante esta chamada.
        let Some(set_loader_data) = chain::device_loader_data(p_create_info) else {
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
        let memory = instance.instance.get_physical_device_memory_properties(physical_device);
        DEVICES.insert(
            dispatch_key(handle.as_raw()),
            DeviceData {
                instance,
                physical_device,
                memory,
                device,
                next_gdpa,
                swapchain,
                set_loader_data,
                presents: AtomicU64::new(0),
                odd_presents: AtomicU64::new(0),
                last_effects: AtomicBool::new(false),
                queues: Mutex::new(HashMap::new()),
                swapchains: Mutex::new(HashMap::new()),
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
            // Os efeitos dos swapchains que o jogo não destruiu ainda usam o dispositivo.
            let _ = data.device.device_wait_idle();
            for (_, mut fx) in data.swapchains.lock().unwrap().drain() {
                fx.destroy(&data.device);
            }
            (data.device.fp_v1_0().destroy_device)(device, p_allocator);
        }
    }
}

/// Aprende a família de cada fila para saber onde gravar os command buffers do efeito.
pub unsafe extern "system" fn get_device_queue(
    device: vk::Device,
    queue_family_index: u32,
    queue_index: u32,
    p_queue: *mut vk::Queue,
) {
    unsafe {
        let Some(data) = DEVICES.get(dispatch_key(device.as_raw())) else { return };
        (data.device.fp_v1_0().get_device_queue)(device, queue_family_index, queue_index, p_queue);
        data.queues.lock().unwrap().insert((*p_queue).as_raw(), queue_family_index);
    }
}

pub unsafe extern "system" fn get_device_queue2(
    device: vk::Device,
    p_queue_info: *const vk::DeviceQueueInfo2<'_>,
    p_queue: *mut vk::Queue,
) {
    unsafe {
        let Some(data) = DEVICES.get(dispatch_key(device.as_raw())) else { return };
        (data.device.fp_v1_1().get_device_queue2)(device, p_queue_info, p_queue);
        if *p_queue != vk::Queue::null() {
            data.queues.lock().unwrap().insert((*p_queue).as_raw(), (*p_queue_info).queue_family_index);
        }
    }
}
