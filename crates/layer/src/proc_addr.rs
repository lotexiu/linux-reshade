//! Negociação com o loader e as tabelas de `vkGet*ProcAddr`: o que a layer intercepta e o que
//! repassa ao próximo elo.

use crate::state::{DEVICES, INSTANCES};
use crate::{device, instance, present, swapchain};
use ash::vk::{self, Handle};
use std::ffi::{CStr, CString, c_char};
use vk_layer::abi::*;
use vk_layer::dispatch::dispatch_key;

/// Apaga o tipo de um ponteiro de função para devolvê-lo como `PFN_vkVoidFunction`.
pub fn erase<F: Copy>(f: F) -> vk::PFN_vkVoidFunction {
    assert_eq!(size_of::<F>(), size_of::<unsafe extern "system" fn()>());
    // SAFETY: `F` é um ponteiro de função (checado pelo tamanho e pelo uso nos chamadores).
    Some(unsafe { std::mem::transmute_copy::<F, unsafe extern "system" fn()>(&f) })
}

fn name_of<'a>(p: *const c_char) -> &'a [u8] {
    // SAFETY: o loader sempre passa um nome terminado em NUL.
    unsafe { CStr::from_ptr(p) }.to_bytes()
}

/// Comandos de dispositivo que a layer intercepta.
fn intercept_device(name: &[u8]) -> vk::PFN_vkVoidFunction {
    match name {
        b"vkGetDeviceProcAddr" => erase(get_device_proc_addr as vk::PFN_vkGetDeviceProcAddr),
        b"vkDestroyDevice" => erase(device::destroy_device as vk::PFN_vkDestroyDevice),
        b"vkGetDeviceQueue" => erase(device::get_device_queue as vk::PFN_vkGetDeviceQueue),
        b"vkGetDeviceQueue2" => erase(device::get_device_queue2 as vk::PFN_vkGetDeviceQueue2),
        b"vkCreateSwapchainKHR" => erase(swapchain::create_swapchain as vk::PFN_vkCreateSwapchainKHR),
        b"vkDestroySwapchainKHR" => erase(swapchain::destroy_swapchain as vk::PFN_vkDestroySwapchainKHR),
        b"vkQueuePresentKHR" => erase(present::queue_present as vk::PFN_vkQueuePresentKHR),
        _ => None,
    }
}

/// Comandos de instância que a layer intercepta.
fn intercept_instance(name: &[u8]) -> vk::PFN_vkVoidFunction {
    match name {
        b"vkGetInstanceProcAddr" => erase(get_instance_proc_addr as vk::PFN_vkGetInstanceProcAddr),
        b"vkCreateInstance" => erase(instance::create_instance as vk::PFN_vkCreateInstance),
        b"vkDestroyInstance" => erase(instance::destroy_instance as vk::PFN_vkDestroyInstance),
        b"vkCreateDevice" => erase(device::create_device as vk::PFN_vkCreateDevice),
        _ => None,
    }
}

/// Ponto de entrada que o loader procura (ver `functions` no manifesto).
///
/// # Safety
/// Chamada pelo loader com um `VkNegotiateLayerInterface` válido.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn vkNegotiateLoaderLayerInterfaceVersion(
    interface: *mut NegotiateLayerInterface,
) -> vk::Result {
    let interface = unsafe { &mut *interface };
    if interface.s_type != NEGOTIATE_INTERFACE_STRUCT
        || interface.loader_layer_interface_version < LOADER_LAYER_INTERFACE_VERSION
    {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    }
    interface.loader_layer_interface_version = LOADER_LAYER_INTERFACE_VERSION;
    interface.pfn_get_instance_proc_addr = Some(get_instance_proc_addr);
    interface.pfn_get_device_proc_addr = Some(get_device_proc_addr);
    interface.pfn_get_physical_device_proc_addr = None;
    vk::Result::SUCCESS
}

/// Também exportada pelo nome, porque alguns loaders a procuram na biblioteca.
///
/// # Safety
/// Contrato de `vkGetInstanceProcAddr`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn vkGetInstanceProcAddr(
    instance: vk::Instance,
    name: *const c_char,
) -> vk::PFN_vkVoidFunction {
    unsafe { get_instance_proc_addr(instance, name) }
}

/// # Safety
/// Contrato de `vkGetDeviceProcAddr`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn vkGetDeviceProcAddr(
    device: vk::Device,
    name: *const c_char,
) -> vk::PFN_vkVoidFunction {
    unsafe { get_device_proc_addr(device, name) }
}

pub unsafe extern "system" fn get_instance_proc_addr(
    instance: vk::Instance,
    name: *const c_char,
) -> vk::PFN_vkVoidFunction {
    let name = name_of(name);
    if let Some(f) = intercept_instance(name) {
        return Some(f);
    }
    if instance == vk::Instance::null() {
        return None; // comandos globais não interceptados não passam por aqui
    }
    if let Some(f) = intercept_device(name) {
        return Some(f);
    }
    let data = INSTANCES.get(unsafe { dispatch_key(instance.as_raw()) })?;
    // Reconstrói a string com NUL: aqui só temos os bytes.
    let cname = CString::new(name).ok()?;
    unsafe { (data.next_gipa)(instance, cname.as_ptr()) }
}

pub unsafe extern "system" fn get_device_proc_addr(
    device: vk::Device,
    name: *const c_char,
) -> vk::PFN_vkVoidFunction {
    if let Some(f) = intercept_device(name_of(name)) {
        return Some(f);
    }
    let data = DEVICES.get(unsafe { dispatch_key(device.as_raw()) })?;
    let next = unsafe { (data.next_gdpa)(device, name) };
    // Os ganchos de profundidade só valem para o que o próximo elo também oferece (extensão não habilitada = nulo).
    if next.is_some()
        && let Some(hook) = crate::depth::intercept_name(name_of(name))
    {
        return Some(hook);
    }
    next
}
