//! Busca, na cadeia `pNext` dos create-info, as structs que o loader injeta para ligar as layers.

use crate::abi::*;
use ash::vk;
use std::ffi::c_void;

/// Percorre `pNext` atrás de uma struct com o `sType` e o `function` pedidos.
unsafe fn find<T>(
    mut next: *const c_void,
    s_type: vk::StructureType,
    function: u32,
    function_of: impl Fn(*const T) -> u32,
) -> Option<*mut T> {
    while !next.is_null() {
        // SAFETY: toda struct da cadeia começa com `sType` e `pNext` (VkBaseInStructure).
        let base = next as *const vk::BaseInStructure<'_>;
        if unsafe { (*base).s_type } == s_type && function_of(next as *const T) == function {
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
            LAYER_LINK_INFO,
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
            LAYER_LINK_INFO,
            |p: *const LayerDeviceCreateInfo| (*p).function,
        )
    }
}

/// O callback que o loader oferece para inicializar objetos despacháveis criados pela layer
/// (por exemplo `VkCommandBuffer`). Sem ele, as layers abaixo não reconhecem o objeto.
///
/// # Safety
/// `info` deve apontar para um `VkDeviceCreateInfo` válido recebido do loader.
pub unsafe fn device_loader_data(info: *const vk::DeviceCreateInfo<'_>) -> Option<PfnSetDeviceLoaderData> {
    unsafe {
        let found: *mut LayerDeviceCreateInfo = find(
            (*info).p_next,
            STRUCTURE_TYPE_LOADER_DEVICE_CREATE_INFO,
            LOADER_DATA_CALLBACK,
            |p: *const LayerDeviceCreateInfo| (*p).function,
        )?;
        Some((*found).u.pfn_set_device_loader_data)
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
    fn finds_loader_data_callback_next_to_the_link_info() {
        unsafe extern "system" fn set_data(_: vk::Device, _: *mut c_void) -> vk::Result {
            vk::Result::SUCCESS
        }
        let callback = LayerDeviceCreateInfo {
            s_type: STRUCTURE_TYPE_LOADER_DEVICE_CREATE_INFO,
            p_next: ptr::null(),
            function: LOADER_DATA_CALLBACK,
            u: LayerDeviceCreateInfoData { pfn_set_device_loader_data: set_data },
        };
        let link = LayerDeviceCreateInfo {
            s_type: STRUCTURE_TYPE_LOADER_DEVICE_CREATE_INFO,
            p_next: &callback as *const _ as *const c_void,
            function: LAYER_LINK_INFO,
            u: LayerDeviceCreateInfoData { p_layer_info: ptr::null_mut() },
        };
        let mut info = vk::DeviceCreateInfo::default();
        info.p_next = &link as *const _ as *const c_void;

        let found = unsafe { device_loader_data(&info) }.expect("deveria achar o callback");
        let expected: PfnSetDeviceLoaderData = set_data;
        assert_eq!(found as usize, expected as usize);
        // O link continua achável pela outra função, e cada busca distingue o `function`.
        assert_eq!(unsafe { device_link(&info) }.unwrap() as *const _, &link as *const _);
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
