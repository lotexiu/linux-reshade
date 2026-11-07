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

        // Os efeitos pedem alguns recursos opcionais (gather com offset, imagens de armazenamento de formatos estendidos e
        // sem formato); a layer os liga se a GPU os tem e o jogo não ligou.
        let supported = instance.instance.get_physical_device_features(physical_device);
        let mut patched = *p_create_info;
        let mut features = vk::PhysicalDeviceFeatures::default();
        if let Some(f2) = features2_in_chain(p_create_info) {
            // O jogo usa `VkPhysicalDeviceFeatures2`: acrescenta na própria struct dele (memória dele, válida e gravável).
            enable_extra_features(&mut (*f2).features, &supported);
        } else {
            if !(*p_create_info).p_enabled_features.is_null() {
                features = *(*p_create_info).p_enabled_features;
            }
            enable_extra_features(&mut features, &supported);
            patched.p_enabled_features = &features;
        }

        // `vkCreateDevice` do próximo elo, carregado junto com o Instance.
        let result = (instance.instance.fp_v1_0().create_device)(physical_device, &patched, p_allocator, p_device);
        if result != vk::Result::SUCCESS {
            return result;
        }

        let handle = *p_device;
        // Os pipelines dos efeitos ficam num cache em arquivo (a segunda cadeia igual monta muito mais rápido).
        // Em disco (não em /dev/shm, que some a cada reinicialização): o cache de pipelines dos efeitos sobrevive e o primeiro minuto do jogo
        // não compila tudo de novo. A gravação é atômica (arquivo temporário e `rename`), então vários jogos abertos não o corrompem.
        let cache_dir = std::env::var_os("XDG_CACHE_HOME").map(std::path::PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache"))).map(|d| d.join("linux-reshade"));
        postfx::pipeline_cache::set_path(cache_dir.map_or_else(|| ipc::state_dir().join("pipeline-cache.bin"), |d| d.join("pipeline-cache.bin")));
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
                depth: crate::depth::Depth::new(next_gdpa, handle),
            },
        );
        crate::state::DEVICES_GENERATION.fetch_add(1, Ordering::AcqRel);
        {
            let info = &*p_create_info;
            let names: Vec<String> = (0..info.enabled_extension_count as usize).map(|i| std::ffi::CStr::from_ptr(*info.pp_enabled_extension_names.add(i)).to_string_lossy().into_owned()).collect();
            let queues: Vec<String> = (0..info.queue_create_info_count as usize).map(|i| { let q = &*info.p_queue_create_infos.add(i); format!("família {} x{}", q.queue_family_index, q.queue_count) }).collect();
            log!("vkCreateDevice ok: filas [{}]; extensões [{}]; estruturas encadeadas {:?}", queues.join(", "), names.join(" "), crate::log::chain_types(info.p_next));
        }
        vk::Result::SUCCESS
    }
}

pub unsafe extern "system" fn destroy_device(device: vk::Device, p_allocator: *const vk::AllocationCallbacks<'_>) {
    unsafe {
        if let Some(data) = DEVICES.remove(dispatch_key(device.as_raw())) {
            crate::state::DEVICES_GENERATION.fetch_add(1, Ordering::AcqRel);
            log!("vkDestroyDevice (presents={})", data.presents.load(Ordering::Relaxed));
            // Os efeitos dos swapchains que o jogo não destruiu ainda usam o dispositivo.
            crate::effect::wait_cleanups(std::time::Duration::from_secs(10));
            let _ = data.device.device_wait_idle();
            for (_, mut fx) in data.swapchains.lock().unwrap().drain() {
                fx.destroy(&data.device);
            }
            crate::effect::reap_semaphores(&data.device, true);
            crate::depth::destroy(&data);
            postfx::pipeline_cache::release(&data.device);
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

/// A `VkPhysicalDeviceFeatures2` da cadeia `pNext` do jogo, se houver.
unsafe fn features2_in_chain(info: *const vk::DeviceCreateInfo<'_>) -> Option<*mut vk::PhysicalDeviceFeatures2<'static>> {
    unsafe {
        let mut next = (*info).p_next as *const vk::BaseInStructure<'_>;
        while !next.is_null() {
            if (*next).s_type == vk::StructureType::PHYSICAL_DEVICE_FEATURES_2 {
                return Some(next as *mut vk::PhysicalDeviceFeatures2<'static>);
            }
            next = (*next).p_next;
        }
        None
    }
}

/// Liga o que os efeitos usam e o jogo não pediu, só se a GPU suporta.
fn enable_extra_features(wanted: &mut vk::PhysicalDeviceFeatures, supported: &vk::PhysicalDeviceFeatures) {
    let on = |have: &mut vk::Bool32, can: vk::Bool32| {
        if can == vk::TRUE {
            *have = vk::TRUE;
        }
    };
    on(&mut wanted.shader_image_gather_extended, supported.shader_image_gather_extended);
    on(&mut wanted.shader_storage_image_extended_formats, supported.shader_storage_image_extended_formats);
    on(&mut wanted.shader_storage_image_read_without_format, supported.shader_storage_image_read_without_format);
    on(&mut wanted.shader_storage_image_write_without_format, supported.shader_storage_image_write_without_format);
}
