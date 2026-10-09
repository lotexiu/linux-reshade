//! `vkCreateInstance` / `vkDestroyInstance`.

use crate::log::log;
use crate::state::{INSTANCES, InstanceData};
use ash::vk::{self, Handle};
use std::ffi::CStr;
use vk_layer::chain;
use vk_layer::dispatch::dispatch_key;

pub unsafe extern "system" fn create_instance(
    p_create_info: *const vk::InstanceCreateInfo<'_>,
    p_allocator: *const vk::AllocationCallbacks<'_>,
    p_instance: *mut vk::Instance,
) -> vk::Result {
    unsafe {
        let Some(link) = chain::instance_link(p_create_info) else {
            return vk::Result::ERROR_INITIALIZATION_FAILED;
        };
        let layer_info = (*link).u.p_layer_info;
        let next_gipa = (*layer_info).pfn_next_get_instance_proc_addr;
        (*link).u.p_layer_info = (*layer_info).p_next; // avança a cadeia para a próxima layer

        let Some(next_create) = std::mem::transmute::<vk::PFN_vkVoidFunction, Option<vk::PFN_vkCreateInstance>>(
            next_gipa(vk::Instance::null(), c"vkCreateInstance".as_ptr()),
        ) else {
            return vk::Result::ERROR_INITIALIZATION_FAILED;
        };
        let result = next_create(p_create_info, p_allocator, p_instance);
        if result != vk::Result::SUCCESS {
            return result;
        }

        let handle = *p_instance;
        // Carrega as funções a partir do próximo elo da cadeia (não do loader).
        let instance = ash::Instance::load(&ash::StaticFn { get_instance_proc_addr: next_gipa }, handle);
        let surface = ash::khr::surface::InstanceFn::load(|name| std::mem::transmute(next_gipa(handle, name.as_ptr())));
        INSTANCES.insert(dispatch_key(handle.as_raw()), InstanceData { instance, surface, next_gipa });

        let (app_name, api) = match (*p_create_info).p_application_info.as_ref() {
            Some(a) if !a.p_application_name.is_null() => {
                (CStr::from_ptr(a.p_application_name).to_string_lossy().into_owned(), a.api_version)
            }
            Some(a) => ("?".into(), a.api_version),
            None => ("?".into(), 0),
        };
        log!("vkCreateInstance ok: app={app_name:?} api={}.{}", vk::api_version_major(api), vk::api_version_minor(api));
        vk::Result::SUCCESS
    }
}

pub unsafe extern "system" fn destroy_instance(
    instance: vk::Instance,
    p_allocator: *const vk::AllocationCallbacks<'_>,
) {
    unsafe {
        if let Some(data) = INSTANCES.remove(dispatch_key(instance.as_raw())) {
            log!("vkDestroyInstance");
            (data.instance.fp_v1_0().destroy_instance)(instance, p_allocator);
        }
    }
}
