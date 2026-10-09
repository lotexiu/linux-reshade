use ash::vk;

/// Primeiro tipo de memória permitido por `type_bits` que tenha todas as propriedades pedidas.
pub fn find_memory_type(
    props: &vk::PhysicalDeviceMemoryProperties,
    type_bits: u32,
    wanted: vk::MemoryPropertyFlags,
) -> Option<u32> {
    (0..props.memory_type_count).find(|&i| {
        type_bits & (1 << i) != 0 && props.memory_types[i as usize].property_flags.contains(wanted)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(types: &[vk::MemoryPropertyFlags]) -> vk::PhysicalDeviceMemoryProperties {
        let mut p = vk::PhysicalDeviceMemoryProperties::default();
        p.memory_type_count = types.len() as u32;
        for (i, f) in types.iter().enumerate() {
            p.memory_types[i].property_flags = *f;
        }
        p
    }

    #[test]
    fn respects_type_bits_and_flags() {
        let p = props(&[
            vk::MemoryPropertyFlags::HOST_VISIBLE,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
            vk::MemoryPropertyFlags::DEVICE_LOCAL | vk::MemoryPropertyFlags::HOST_VISIBLE,
        ]);
        let local = vk::MemoryPropertyFlags::DEVICE_LOCAL;
        assert_eq!(find_memory_type(&p, 0b111, local), Some(1));
        assert_eq!(find_memory_type(&p, 0b100, local), Some(2)); // o tipo 1 não é permitido
        assert_eq!(find_memory_type(&p, 0b001, local), None);
    }
}
