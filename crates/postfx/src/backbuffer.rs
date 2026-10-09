//! Cópias amostráveis do alvo. Um shader não pode ler e escrever a mesma imagem no mesmo passe,
//! então cada imagem do alvo ganha uma cópia própria, de onde os passes leem. Uma cópia por
//! imagem (e não uma só) evita que dois frames em voo disputem a mesma memória.

use crate::memory::find_memory_type;
use ash::vk;

#[derive(Default)]
pub struct BackbufferCopies {
    pub images: Vec<vk::Image>,
    pub views: Vec<vk::ImageView>,
    memories: Vec<vk::DeviceMemory>,
}

impl BackbufferCopies {
    /// Cria `count` cópias. Se algo falhar, o que já foi criado é liberado antes de devolver o erro.
    ///
    /// # Safety
    /// `device` deve ser válido e `memory` deve ser do mesmo dispositivo físico.
    pub unsafe fn new(
        device: &ash::Device,
        memory: &vk::PhysicalDeviceMemoryProperties,
        format: vk::Format,
        extent: vk::Extent2D,
        count: usize,
    ) -> Result<Self, vk::Result> {
        let mut copies = Self::default();
        for _ in 0..count {
            if let Err(e) = unsafe { copies.push_one(device, memory, format, extent) } {
                unsafe { copies.destroy(device) };
                return Err(e);
            }
        }
        Ok(copies)
    }

    unsafe fn push_one(
        &mut self,
        device: &ash::Device,
        memory: &vk::PhysicalDeviceMemoryProperties,
        format: vk::Format,
        extent: vk::Extent2D,
    ) -> Result<(), vk::Result> {
        unsafe {
            let info = vk::ImageCreateInfo::default()
                .image_type(vk::ImageType::TYPE_2D)
                .format(format)
                .extent(vk::Extent3D { width: extent.width, height: extent.height, depth: 1 })
                .mip_levels(1)
                .array_layers(1)
                .samples(vk::SampleCountFlags::TYPE_1)
                .tiling(vk::ImageTiling::OPTIMAL)
                .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED)
                .sharing_mode(vk::SharingMode::EXCLUSIVE)
                .initial_layout(vk::ImageLayout::UNDEFINED);
            let image = device.create_image(&info, None)?;
            self.images.push(image);

            let req = device.get_image_memory_requirements(image);
            let type_index = find_memory_type(memory, req.memory_type_bits, vk::MemoryPropertyFlags::DEVICE_LOCAL)
                .ok_or(vk::Result::ERROR_OUT_OF_DEVICE_MEMORY)?;
            let alloc = vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(type_index);
            let mem = device.allocate_memory(&alloc, None)?;
            self.memories.push(mem);
            device.bind_image_memory(image, mem, 0)?;

            let view_info = vk::ImageViewCreateInfo::default()
                .image(image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(format)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                });
            self.views.push(device.create_image_view(&view_info, None)?);
        }
        Ok(())
    }

    /// Libera tudo. Pode ser chamado mais de uma vez.
    ///
    /// # Safety
    /// A GPU não pode estar usando as imagens.
    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        unsafe {
            for v in self.views.drain(..) {
                device.destroy_image_view(v, None);
            }
            for i in self.images.drain(..) {
                device.destroy_image(i, None);
            }
            for m in self.memories.drain(..) {
                device.free_memory(m, None);
            }
        }
    }
}
