//! Layer Vulkan implícita do linux-reshade. Por enquanto só repassa tudo ao próximo elo da
//! cadeia e registra instance, device, swapchain e presents (marco 1 do ROADMAP).

mod log;

use ash::vk::{self, Handle};
use log::log;
use std::ffi::{CStr, c_char};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{LazyLock, OnceLock};
use vk_layer::abi::*;
use vk_layer::chain;
use vk_layer::dispatch::{Registry, dispatch_key};

/// Um log a cada tantos presents (a primeira apresentação sempre é registrada).
const PRESENT_LOG_EVERY: u64 = 300;
/// Quantos presents com resultado diferente de SUCCESS registrar (evita inundar o log).
const ODD_PRESENT_LOG_MAX: u64 = 20;
/// De quantos em quantos presents tentar abrir o estado compartilhado enquanto o app de controle
/// ainda não o criou (uma chamada `stat`/`open`, barata).
const STATE_RETRY_EVERY: u64 = 120;

struct InstanceData {
    instance: u64,
    next_gipa: vk::PFN_vkGetInstanceProcAddr,
    destroy_instance: vk::PFN_vkDestroyInstance,
}

struct DeviceData {
    next_gdpa: vk::PFN_vkGetDeviceProcAddr,
    destroy_device: vk::PFN_vkDestroyDevice,
    create_swapchain: Option<vk::PFN_vkCreateSwapchainKHR>,
    queue_present: Option<vk::PFN_vkQueuePresentKHR>,
    presents: AtomicU64,
    /// Presents que não devolveram SUCCESS (inclui SUBOPTIMAL e OUT_OF_DATE).
    odd_presents: AtomicU64,
    /// Último valor visto da flag de efeitos, só para registrar as mudanças.
    last_effects: AtomicBool,
}

static INSTANCES: LazyLock<Registry<InstanceData>> = LazyLock::new(Registry::new);
static DEVICES: LazyLock<Registry<DeviceData>> = LazyLock::new(Registry::new);
/// Estado escrito pelo app de controle. Depois de aberto, é lido por `load` atômico, sem lock.
static CONTROL: OnceLock<ipc::Reader> = OnceLock::new();

/// Valor atual da flag de efeitos. Enquanto o app de controle não existe, os efeitos ficam desligados.
fn effects_enabled(present_number: u64) -> bool {
    if let Some(reader) = CONTROL.get() {
        return reader.effects_enabled();
    }
    if present_number % STATE_RETRY_EVERY == 1 {
        match ipc::Reader::open() {
            Ok(Some(reader)) => {
                log!("estado compartilhado aberto: {}", ipc::state_path().display());
                return CONTROL.get_or_init(|| reader).effects_enabled();
            }
            Ok(None) => {}
            Err(e) => log!("falha ao abrir o estado compartilhado: {e}"),
        }
    }
    false
}

/// Apaga o tipo de um ponteiro de função para devolvê-lo como `PFN_vkVoidFunction`.
fn erase<F: Copy>(f: F) -> vk::PFN_vkVoidFunction {
    assert_eq!(size_of::<F>(), size_of::<unsafe extern "system" fn()>());
    // SAFETY: `F` é um ponteiro de função (checado pelo tamanho e pelo uso abaixo).
    Some(unsafe { std::mem::transmute_copy::<F, unsafe extern "system" fn()>(&f) })
}

/// Reaplica o tipo de um ponteiro de função devolvido por `vkGet*ProcAddr`.
///
/// # Safety
/// `F` deve ser o tipo `PFN_*` do comando que `f` realmente aponta.
unsafe fn restore<F: Copy>(f: vk::PFN_vkVoidFunction) -> Option<F> {
    assert_eq!(size_of::<F>(), size_of::<unsafe extern "system" fn()>());
    f.map(|f| unsafe { std::mem::transmute_copy::<unsafe extern "system" fn(), F>(&f) })
}

fn name_of<'a>(p: *const c_char) -> &'a [u8] {
    // SAFETY: o loader sempre passa um nome terminado em NUL.
    unsafe { CStr::from_ptr(p) }.to_bytes()
}

/// Comandos de dispositivo que a layer intercepta.
fn intercept_device(name: &[u8]) -> vk::PFN_vkVoidFunction {
    match name {
        b"vkGetDeviceProcAddr" => erase(get_device_proc_addr as vk::PFN_vkGetDeviceProcAddr),
        b"vkDestroyDevice" => erase(destroy_device as vk::PFN_vkDestroyDevice),
        b"vkCreateSwapchainKHR" => erase(create_swapchain as vk::PFN_vkCreateSwapchainKHR),
        b"vkQueuePresentKHR" => erase(queue_present as vk::PFN_vkQueuePresentKHR),
        _ => None,
    }
}

/// Comandos de instância que a layer intercepta.
fn intercept_instance(name: &[u8]) -> vk::PFN_vkVoidFunction {
    match name {
        b"vkGetInstanceProcAddr" => erase(get_instance_proc_addr as vk::PFN_vkGetInstanceProcAddr),
        b"vkCreateInstance" => erase(create_instance as vk::PFN_vkCreateInstance),
        b"vkDestroyInstance" => erase(destroy_instance as vk::PFN_vkDestroyInstance),
        b"vkCreateDevice" => erase(create_device as vk::PFN_vkCreateDevice),
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
    if interface.s_type != NEGOTIATE_INTERFACE_STRUCT {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    }
    if interface.loader_layer_interface_version < LOADER_LAYER_INTERFACE_VERSION {
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

unsafe extern "system" fn get_instance_proc_addr(
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
    // Reconstrói a string com NUL: `name` veio de um `CStr`, mas aqui só temos os bytes.
    let cname = std::ffi::CString::new(name).ok()?;
    unsafe { (data.next_gipa)(instance, cname.as_ptr()) }
}

unsafe extern "system" fn get_device_proc_addr(
    device: vk::Device,
    name: *const c_char,
) -> vk::PFN_vkVoidFunction {
    if let Some(f) = intercept_device(name_of(name)) {
        return Some(f);
    }
    let data = DEVICES.get(unsafe { dispatch_key(device.as_raw()) })?;
    unsafe { (data.next_gdpa)(device, name) }
}

unsafe extern "system" fn create_instance(
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

        let Some(next_create) = restore::<vk::PFN_vkCreateInstance>(next_gipa(
            vk::Instance::null(),
            c"vkCreateInstance".as_ptr(),
        )) else {
            return vk::Result::ERROR_INITIALIZATION_FAILED;
        };
        let result = next_create(p_create_info, p_allocator, p_instance);
        if result != vk::Result::SUCCESS {
            return result;
        }

        let instance = *p_instance;
        let Some(destroy_instance) =
            restore::<vk::PFN_vkDestroyInstance>(next_gipa(instance, c"vkDestroyInstance".as_ptr()))
        else {
            return vk::Result::ERROR_INITIALIZATION_FAILED;
        };
        INSTANCES.insert(
            dispatch_key(instance.as_raw()),
            InstanceData { instance: instance.as_raw(), next_gipa, destroy_instance },
        );

        let app = (*p_create_info).p_application_info;
        let (app_name, api) = match app.as_ref() {
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

unsafe extern "system" fn destroy_instance(
    instance: vk::Instance,
    p_allocator: *const vk::AllocationCallbacks<'_>,
) {
    unsafe {
        if let Some(data) = INSTANCES.remove(dispatch_key(instance.as_raw())) {
            log!("vkDestroyInstance");
            (data.destroy_instance)(instance, p_allocator);
        }
    }
}

unsafe extern "system" fn create_device(
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
        let next_gipa = (*layer_info).pfn_next_get_instance_proc_addr;
        let next_gdpa = (*layer_info).pfn_next_get_device_proc_addr;
        (*link).u.p_layer_info = (*layer_info).p_next;

        let Some(next_create) = restore::<vk::PFN_vkCreateDevice>(next_gipa(
            vk::Instance::from_raw(instance.instance),
            c"vkCreateDevice".as_ptr(),
        )) else {
            return vk::Result::ERROR_INITIALIZATION_FAILED;
        };
        let result = next_create(physical_device, p_create_info, p_allocator, p_device);
        if result != vk::Result::SUCCESS {
            return result;
        }

        let device = *p_device;
        let Some(destroy_device) =
            restore::<vk::PFN_vkDestroyDevice>(next_gdpa(device, c"vkDestroyDevice".as_ptr()))
        else {
            return vk::Result::ERROR_INITIALIZATION_FAILED;
        };
        // Só existem se a extensão VK_KHR_swapchain foi habilitada para este device.
        let create_swapchain =
            restore::<vk::PFN_vkCreateSwapchainKHR>(next_gdpa(device, c"vkCreateSwapchainKHR".as_ptr()));
        let queue_present =
            restore::<vk::PFN_vkQueuePresentKHR>(next_gdpa(device, c"vkQueuePresentKHR".as_ptr()));
        DEVICES.insert(
            dispatch_key(device.as_raw()),
            DeviceData {
                next_gdpa,
                destroy_device,
                create_swapchain,
                queue_present,
                presents: AtomicU64::new(0),
                odd_presents: AtomicU64::new(0),
                last_effects: AtomicBool::new(false),
            },
        );
        log!("vkCreateDevice ok: swapchain={}", queue_present.is_some());
        vk::Result::SUCCESS
    }
}

unsafe extern "system" fn destroy_device(device: vk::Device, p_allocator: *const vk::AllocationCallbacks<'_>) {
    unsafe {
        if let Some(data) = DEVICES.remove(dispatch_key(device.as_raw())) {
            log!("vkDestroyDevice (presents={})", data.presents.load(Ordering::Relaxed));
            (data.destroy_device)(device, p_allocator);
        }
    }
}

unsafe extern "system" fn create_swapchain(
    device: vk::Device,
    p_create_info: *const vk::SwapchainCreateInfoKHR<'_>,
    p_allocator: *const vk::AllocationCallbacks<'_>,
    p_swapchain: *mut vk::SwapchainKHR,
) -> vk::Result {
    unsafe {
        let Some(data) = DEVICES.get(dispatch_key(device.as_raw())) else {
            return vk::Result::ERROR_DEVICE_LOST;
        };
        let Some(next) = data.create_swapchain else {
            return vk::Result::ERROR_EXTENSION_NOT_PRESENT;
        };
        let result = next(device, p_create_info, p_allocator, p_swapchain);
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

unsafe extern "system" fn queue_present(
    queue: vk::Queue,
    p_present_info: *const vk::PresentInfoKHR<'_>,
) -> vk::Result {
    unsafe {
        // A Queue compartilha a chave de dispatch do Device.
        let Some(data) = DEVICES.get(dispatch_key(queue.as_raw())) else {
            return vk::Result::ERROR_DEVICE_LOST;
        };
        let Some(next) = data.queue_present else {
            return vk::Result::ERROR_EXTENSION_NOT_PRESENT;
        };
        let n = data.presents.fetch_add(1, Ordering::Relaxed) + 1;
        if n == 1 || n % PRESENT_LOG_EVERY == 0 {
            log!("vkQueuePresentKHR #{n}");
        }
        let enabled = effects_enabled(n);
        if data.last_effects.swap(enabled, Ordering::Relaxed) != enabled {
            log!("efeitos {} (present #{n})", if enabled { "LIGADOS" } else { "DESLIGADOS" });
        }
        let result = next(queue, p_present_info);
        if result != vk::Result::SUCCESS && data.odd_presents.fetch_add(1, Ordering::Relaxed) < ODD_PRESENT_LOG_MAX {
            log!("vkQueuePresentKHR #{n} devolveu {}", result.as_raw());
        }
        result
    }
}

