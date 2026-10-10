//! Imagem 2D de cor com view e memória própria: o bloco de construção das cópias do backbuffer e
//! das texturas intermediárias dos efeitos.

use crate::memory::find_memory_type;
use ash::vk;

#[derive(Default)]
pub struct GpuImage {
    pub image: vk::Image,
    /// View de todos os níveis (para amostrar).
    pub view: vk::ImageView,
    /// View só do nível 0, para usar como anexo de render pass; nula se a imagem tem um nível só (`view` serve).
    view0: vk::ImageView,
    pub levels: u32,
    memory: vk::DeviceMemory,
}

/// Quantos níveis de mipmap uma imagem desse tamanho comporta.
pub fn max_levels(extent: vk::Extent2D) -> u32 {
    32 - extent.width.max(extent.height).max(1).leading_zeros()
}

impl GpuImage {
    /// Cria a imagem (memória local do dispositivo) e uma view 2D. Se algo falhar, libera o que
    /// já criou antes de devolver o erro.
    ///
    /// # Safety
    /// `device` válido e `memory` do mesmo dispositivo físico.
    pub unsafe fn new(
        device: &ash::Device,
        memory: &vk::PhysicalDeviceMemoryProperties,
        format: vk::Format,
        extent: vk::Extent2D,
        usage: vk::ImageUsageFlags,
    ) -> Result<Self, vk::Result> {
        unsafe { Self::with_levels(device, memory, format, extent, usage, 1) }
    }

    /// Como [`new`](Self::new), com `levels` níveis de mipmap (limitado ao que o tamanho comporta).
    ///
    /// # Safety
    /// As mesmas de [`new`](Self::new).
    pub unsafe fn with_levels(
        device: &ash::Device,
        memory: &vk::PhysicalDeviceMemoryProperties,
        format: vk::Format,
        extent: vk::Extent2D,
        usage: vk::ImageUsageFlags,
        levels: u32,
    ) -> Result<Self, vk::Result> {
        let mut gpu = Self { levels: levels.clamp(1, max_levels(extent)), ..Self::default() };
        if let Err(e) = unsafe { gpu.build(device, memory, format, extent, usage) } {
            unsafe { gpu.destroy(device) };
            return Err(e);
        }
        Ok(gpu)
    }

    unsafe fn build(
        &mut self,
        device: &ash::Device,
        memory: &vk::PhysicalDeviceMemoryProperties,
        format: vk::Format,
        extent: vk::Extent2D,
        usage: vk::ImageUsageFlags,
    ) -> Result<(), vk::Result> {
        unsafe {
            let info = vk::ImageCreateInfo::default()
                .image_type(vk::ImageType::TYPE_2D)
                .format(format)
                .extent(vk::Extent3D { width: extent.width, height: extent.height, depth: 1 })
                .mip_levels(self.levels)
                .array_layers(1)
                .samples(vk::SampleCountFlags::TYPE_1)
                .tiling(vk::ImageTiling::OPTIMAL)
                .usage(usage)
                .sharing_mode(vk::SharingMode::EXCLUSIVE)
                .initial_layout(vk::ImageLayout::UNDEFINED);
            self.image = device.create_image(&info, None)?;

            let req = device.get_image_memory_requirements(self.image);
            let type_index = find_memory_type(memory, req.memory_type_bits, vk::MemoryPropertyFlags::DEVICE_LOCAL)
                .ok_or(vk::Result::ERROR_OUT_OF_DEVICE_MEMORY)?;
            self.memory = device
                .allocate_memory(&vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(type_index), None)?;
            device.bind_image_memory(self.image, self.memory, 0)?;

            let view_info = vk::ImageViewCreateInfo::default()
                .image(self.image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(format)
                .subresource_range(vk::ImageSubresourceRange { level_count: self.levels, ..COLOR_RANGE });
            self.view = device.create_image_view(&view_info, None)?;
            if self.levels > 1 {
                self.view0 = device.create_image_view(&view_info.subresource_range(COLOR_RANGE), None)?;
            }
        }
        Ok(())
    }

    /// A view para usar como anexo de render pass (só o nível 0).
    pub fn attachment_view(&self) -> vk::ImageView {
        if self.view0 == vk::ImageView::null() { self.view } else { self.view0 }
    }

    /// Libera tudo. Pode ser chamado mais de uma vez.
    ///
    /// # Safety
    /// A GPU não pode estar usando a imagem.
    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        unsafe {
            if self.view0 != vk::ImageView::null() {
                device.destroy_image_view(std::mem::take(&mut self.view0), None);
            }
            if self.view != vk::ImageView::null() {
                device.destroy_image_view(std::mem::take(&mut self.view), None);
            }
            if self.image != vk::Image::null() {
                device.destroy_image(std::mem::take(&mut self.image), None);
            }
            if self.memory != vk::DeviceMemory::null() {
                device.free_memory(std::mem::take(&mut self.memory), None);
            }
        }
    }
}

/// Todos os níveis de mipmap, para barreiras e limpezas que valem para a imagem inteira.
pub const ALL_LEVELS: vk::ImageSubresourceRange = vk::ImageSubresourceRange { level_count: vk::REMAINING_MIP_LEVELS, ..COLOR_RANGE };

/// Subrecurso padrão: nível 0, camada 0, aspecto de cor.
pub const COLOR_RANGE: vk::ImageSubresourceRange = vk::ImageSubresourceRange {
    aspect_mask: vk::ImageAspectFlags::COLOR,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};
