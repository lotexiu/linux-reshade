//! Dados por instance e por device, achados pela chave de dispatch dos handles.

use crate::effect::SwapchainFx;
use ash::vk;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, LazyLock, Mutex};
use vk_layer::dispatch::Registry;

pub struct InstanceData {
    pub instance: ash::Instance,
    /// Funções de VK_KHR_surface (só válidas se a aplicação habilitou a extensão).
    pub surface: ash::khr::surface::InstanceFn,
    /// `vkGetInstanceProcAddr` do próximo elo da cadeia.
    pub next_gipa: vk::PFN_vkGetInstanceProcAddr,
}

pub struct DeviceData {
    pub instance: Arc<InstanceData>,
    pub physical_device: vk::PhysicalDevice,
    pub memory: vk::PhysicalDeviceMemoryProperties,
    pub device: ash::Device,
    /// `vkGetDeviceProcAddr` do próximo elo da cadeia.
    pub next_gdpa: vk::PFN_vkGetDeviceProcAddr,
    /// Funções de VK_KHR_swapchain, carregadas pelo `next_gdpa` (nulas se a extensão não foi habilitada).
    pub swapchain: ash::khr::swapchain::DeviceFn,
    /// Callback do loader para inicializar os objetos despacháveis que a layer cria (command buffers).
    pub set_loader_data: vk_layer::abi::PfnSetDeviceLoaderData,
    pub presents: AtomicU64,
    /// Presents que não devolveram SUCCESS (inclui SUBOPTIMAL e OUT_OF_DATE).
    pub odd_presents: AtomicU64,
    /// Último valor visto da flag de efeitos, só para registrar as mudanças.
    pub last_effects: AtomicBool,
    /// Família de fila de cada `VkQueue` (chave: handle bruto), aprendida em `vkGetDeviceQueue*`.
    pub queues: Mutex<HashMap<u64, u32>>,
    /// Estado de efeito de cada swapchain (chave: handle bruto).
    pub swapchains: Mutex<HashMap<u64, SwapchainFx>>,
    /// Observação da profundidade do jogo (ligada só quando pedida).
    pub depth: crate::depth::Depth,
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

/// Sobe a cada device criado ou destruído: invalida os caches de dispositivo das threads (ver `depth::hooks::data_of`).
pub static DEVICES_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
