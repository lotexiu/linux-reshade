//! Structs de `vk_layer.h` (Khronos), que o `ash` não traz. Layout conferido contra o header.

use ash::vk;
use std::ffi::{c_char, c_void};

/// `CURRENT_LOADER_LAYER_INTERFACE_VERSION`.
pub const LOADER_LAYER_INTERFACE_VERSION: u32 = 2;

/// `VK_STRUCTURE_TYPE_LOADER_INSTANCE_CREATE_INFO`.
pub const STRUCTURE_TYPE_LOADER_INSTANCE_CREATE_INFO: vk::StructureType =
    vk::StructureType::from_raw(47);
/// `VK_STRUCTURE_TYPE_LOADER_DEVICE_CREATE_INFO`.
pub const STRUCTURE_TYPE_LOADER_DEVICE_CREATE_INFO: vk::StructureType =
    vk::StructureType::from_raw(48);

/// `LAYER_NEGOTIATE_INTERFACE_STRUCT`.
pub const NEGOTIATE_INTERFACE_STRUCT: u32 = 1;
/// `VK_LAYER_LINK_INFO`.
pub const LAYER_LINK_INFO: u32 = 0;

pub type PfnGetPhysicalDeviceProcAddr =
    unsafe extern "system" fn(vk::Instance, *const c_char) -> vk::PFN_vkVoidFunction;

#[repr(C)]
pub struct NegotiateLayerInterface {
    pub s_type: u32,
    pub p_next: *mut c_void,
    pub loader_layer_interface_version: u32,
    pub pfn_get_instance_proc_addr: Option<vk::PFN_vkGetInstanceProcAddr>,
    pub pfn_get_device_proc_addr: Option<vk::PFN_vkGetDeviceProcAddr>,
    pub pfn_get_physical_device_proc_addr: Option<PfnGetPhysicalDeviceProcAddr>,
}

#[repr(C)]
pub struct LayerInstanceLink {
    pub p_next: *mut LayerInstanceLink,
    pub pfn_next_get_instance_proc_addr: vk::PFN_vkGetInstanceProcAddr,
    pub pfn_next_get_physical_device_proc_addr: Option<PfnGetPhysicalDeviceProcAddr>,
}

#[repr(C)]
pub union LayerInstanceCreateInfoData {
    pub p_layer_info: *mut LayerInstanceLink,
    /// Garante o tamanho da union do header (o maior membro tem dois ponteiros).
    _size: [usize; 2],
}

#[repr(C)]
pub struct LayerInstanceCreateInfo {
    pub s_type: vk::StructureType,
    pub p_next: *const c_void,
    pub function: u32,
    pub u: LayerInstanceCreateInfoData,
}

#[repr(C)]
pub struct LayerDeviceLink {
    pub p_next: *mut LayerDeviceLink,
    pub pfn_next_get_instance_proc_addr: vk::PFN_vkGetInstanceProcAddr,
    pub pfn_next_get_device_proc_addr: vk::PFN_vkGetDeviceProcAddr,
}

#[repr(C)]
pub union LayerDeviceCreateInfoData {
    pub p_layer_info: *mut LayerDeviceLink,
    _size: usize,
}

#[repr(C)]
pub struct LayerDeviceCreateInfo {
    pub s_type: vk::StructureType,
    pub p_next: *const c_void,
    pub function: u32,
    pub u: LayerDeviceCreateInfoData,
}

#[cfg(all(test, target_pointer_width = "64"))]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    // Valores calculados à mão a partir de `vk_layer.h` (ponteiro de 8 bytes, enums de 4).
    #[test]
    fn layouts_match_vk_layer_h() {
        assert_eq!(size_of::<NegotiateLayerInterface>(), 48);
        assert_eq!(offset_of!(NegotiateLayerInterface, p_next), 8);
        assert_eq!(offset_of!(NegotiateLayerInterface, loader_layer_interface_version), 16);
        assert_eq!(offset_of!(NegotiateLayerInterface, pfn_get_instance_proc_addr), 24);

        assert_eq!(size_of::<LayerInstanceLink>(), 24);
        assert_eq!(size_of::<LayerDeviceLink>(), 24);

        assert_eq!(size_of::<LayerInstanceCreateInfo>(), 40);
        assert_eq!(offset_of!(LayerInstanceCreateInfo, function), 16);
        assert_eq!(offset_of!(LayerInstanceCreateInfo, u), 24);

        assert_eq!(size_of::<LayerDeviceCreateInfo>(), 32);
        assert_eq!(offset_of!(LayerDeviceCreateInfo, u), 24);
    }
}
