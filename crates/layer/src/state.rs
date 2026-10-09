//! Dados por instance e por device, achados pela chave de dispatch dos handles.

use ash::vk;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, LazyLock};
use vk_layer::dispatch::Registry;

pub struct InstanceData {
    pub instance: ash::Instance,
    /// `vkGetInstanceProcAddr` do próximo elo da cadeia.
    pub next_gipa: vk::PFN_vkGetInstanceProcAddr,
}

pub struct DeviceData {
    #[expect(dead_code, reason = "usado pelo passe de efeito (marco 2)")]
    pub instance: Arc<InstanceData>,
    #[expect(dead_code, reason = "usado pelo passe de efeito (marco 2)")]
    pub physical_device: vk::PhysicalDevice,
    pub device: ash::Device,
    /// `vkGetDeviceProcAddr` do próximo elo da cadeia.
    pub next_gdpa: vk::PFN_vkGetDeviceProcAddr,
    /// Funções de VK_KHR_swapchain, carregadas pelo `next_gdpa` (nulas se a extensão não foi habilitada).
    pub swapchain: ash::khr::swapchain::DeviceFn,
    pub presents: AtomicU64,
    /// Presents que não devolveram SUCCESS (inclui SUBOPTIMAL e OUT_OF_DATE).
    pub odd_presents: AtomicU64,
    /// Último valor visto da flag de efeitos, só para registrar as mudanças.
    pub last_effects: AtomicBool,
}

// SAFETY: os handles do Vulkan são só inteiros/ponteiros opacos e as tabelas de funções são
// imutáveis depois de carregadas; a sincronização dos objetos é responsabilidade do Vulkan
// (externally synchronized) e do chamador, como em qualquer layer.
unsafe impl Send for InstanceData {}
unsafe impl Sync for InstanceData {}
unsafe impl Send for DeviceData {}
unsafe impl Sync for DeviceData {}

pub static INSTANCES: LazyLock<Registry<InstanceData>> = LazyLock::new(Registry::new);
pub static DEVICES: LazyLock<Registry<DeviceData>> = LazyLock::new(Registry::new);
