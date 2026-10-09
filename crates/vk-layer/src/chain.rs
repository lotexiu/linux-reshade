//! Busca, na cadeia `pNext` dos create-info, as structs que o loader injeta para ligar as layers.

use crate::abi::*;
use ash::vk;
use std::ffi::c_void;

/// Percorre `pNext` atrás de uma struct com o `sType` e o `function` pedidos.
unsafe fn find<T>(
    mut next: *const c_void,
    s_type: vk::StructureType,
    function_of: impl Fn(*const T) -> u32,
) -> Option<*mut T> {
    while !next.is_null() {
        // SAFETY: toda struct da cadeia começa com `sType` e `pNext` (VkBaseInStructure).
        let base = next as *const vk::BaseInStructure<'_>;
        if unsafe { (*base).s_type } == s_type && function_of(next as *const T) == LAYER_LINK_INFO {
            return Some(next as *mut T);
        }
        next = unsafe { (*base).p_next } as *const c_void;
    }
    None
}

/// # Safety
/// `info` deve apontar para um `VkInstanceCreateInfo` válido recebido do loader.
pub unsafe fn instance_link(info: *const vk::InstanceCreateInfo<'_>) -> Option<*mut LayerInstanceCreateInfo> {
    unsafe {
        find(
            (*info).p_next,
            STRUCTURE_TYPE_LOADER_INSTANCE_CREATE_INFO,
            |p: *const LayerInstanceCreateInfo| (*p).function,
        )
    }
}

/// # Safety
/// `info` deve apontar para um `VkDeviceCreateInfo` válido recebido do loader.
pub unsafe fn device_link(info: *const vk::DeviceCreateInfo<'_>) -> Option<*mut LayerDeviceCreateInfo> {
    unsafe {
        find(
            (*info).p_next,
            STRUCTURE_TYPE_LOADER_DEVICE_CREATE_INFO,
            |p: *const LayerDeviceCreateInfo| (*p).function,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr;

    #[test]
    fn finds_instance_link_behind_other_structs() {
        let mut link = LayerInstanceLink {
            p_next: ptr::null_mut(),
            pfn_next_get_instance_proc_addr: dummy_gipa,
            pfn_next_get_physical_device_proc_addr: None,
        };
        let loader = LayerInstanceCreateInfo {
            s_type: STRUCTURE_TYPE_LOADER_INSTANCE_CREATE_INFO,
            p_next: ptr::null(),
            function: LAYER_LINK_INFO,
            u: LayerInstanceCreateInfoData { p_layer_info: &mut link },
        };
        // Uma struct qualquer (sType diferente) na frente da do loader.
        let other = vk::BaseInStructure {
            s_type: vk::StructureType::VALIDATION_FEATURES_EXT,
            p_next: &loader as *const _ as *const vk::BaseInStructure<'_>,
            _marker: Default::default(),
        };
        let mut info = vk::InstanceCreateInfo::default();
        info.p_next = &other as *const _ as *const c_void;

        let found = unsafe { instance_link(&info) }.expect("deveria achar");
        assert_eq!(found as *const _, &loader as *const _);
    }

    #[test]
    fn returns_none_without_loader_struct() {
        let info = vk::InstanceCreateInfo::default();
        assert!(unsafe { instance_link(&info) }.is_none());
    }

    unsafe extern "system" fn dummy_gipa(_: vk::Instance, _: *const std::ffi::c_char) -> vk::PFN_vkVoidFunction {
        None
    }
}
