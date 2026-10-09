//! Recursos de GPU que um efeito possui além das imagens: buffers de uniforms e samplers.

use super::convert;
use crate::memory::find_memory_type;
use ash::vk;
use fx_bundle::manifest::Sampler;

/// Buffer de uniforms visível ao host, mapeado de forma permanente.
pub struct UniformBuffer {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    ptr: *mut u8,
    size: usize,
}

impl Default for UniformBuffer {
    fn default() -> Self {
        Self { buffer: vk::Buffer::null(), memory: vk::DeviceMemory::null(), ptr: std::ptr::null_mut(), size: 0 }
    }
}

impl UniformBuffer {
    /// Cria um buffer de `initial.len()` bytes (no mínimo 16) já preenchido com `initial`.
    ///
    /// # Safety
    /// `device` válido e `memory` do mesmo dispositivo físico.
    pub unsafe fn new(
        device: &ash::Device,
        memory: &vk::PhysicalDeviceMemoryProperties,
        initial: &[u8],
    ) -> Result<Self, vk::Result> {
        let mut ub = Self::default();
        if let Err(e) = unsafe { ub.build(device, memory, initial) } {
            unsafe { ub.destroy(device) };
            return Err(e);
        }
        Ok(ub)
    }

    unsafe fn build(
        &mut self,
        device: &ash::Device,
        memory: &vk::PhysicalDeviceMemoryProperties,
        initial: &[u8],
    ) -> Result<(), vk::Result> {
        self.size = initial.len().max(16);
        unsafe {
            let info = vk::BufferCreateInfo::default()
                .size(self.size as u64)
                .usage(vk::BufferUsageFlags::UNIFORM_BUFFER)
                .sharing_mode(vk::SharingMode::EXCLUSIVE);
            self.buffer = device.create_buffer(&info, None)?;
            let req = device.get_buffer_memory_requirements(self.buffer);
            let wanted = vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
            let type_index = find_memory_type(memory, req.memory_type_bits, wanted).ok_or(vk::Result::ERROR_OUT_OF_HOST_MEMORY)?;
            self.memory = device
                .allocate_memory(&vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(type_index), None)?;
            device.bind_buffer_memory(self.buffer, self.memory, 0)?;
            self.ptr = device.map_memory(self.memory, 0, self.size as u64, vk::MemoryMapFlags::empty())? as *mut u8;
            std::ptr::write_bytes(self.ptr, 0, self.size);
            std::ptr::copy_nonoverlapping(initial.as_ptr(), self.ptr, initial.len());
        }
        Ok(())
    }

    pub fn buffer(&self) -> vk::Buffer {
        self.buffer
    }

    pub fn size(&self) -> usize {
        self.size
    }

    /// Escreve `bytes` em `offset`. Os bytes que passariam do fim do buffer são ignorados.
    ///
    /// # Safety
    /// A GPU não deve estar lendo o buffer neste momento (o chamador espera a fence da imagem).
    pub unsafe fn write(&self, offset: usize, bytes: &[u8]) {
        if self.ptr.is_null() || offset >= self.size {
            return;
        }
        let n = bytes.len().min(self.size - offset);
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), self.ptr.add(offset), n) };
    }

    /// Libera tudo. Pode ser chamado mais de uma vez.
    ///
    /// # Safety
    /// A GPU não pode estar usando o buffer.
    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        unsafe {
            if self.memory != vk::DeviceMemory::null() {
                if !self.ptr.is_null() {
                    device.unmap_memory(self.memory);
                    self.ptr = std::ptr::null_mut();
                }
                device.free_memory(std::mem::take(&mut self.memory), None);
            }
            if self.buffer != vk::Buffer::null() {
                device.destroy_buffer(std::mem::take(&mut self.buffer), None);
            }
        }
    }
}

/// Cria o `VkSampler` correspondente ao sampler do manifesto.
///
/// # Safety
/// `device` válido.
pub unsafe fn create_sampler(device: &ash::Device, s: &Sampler) -> Result<vk::Sampler, vk::Result> {
    // O ReShade usa ±FLT_MAX como "sem limite"; o Vulkan quer 0..LOD_CLAMP_NONE.
    let min_lod = s.min_lod.max(0.0);
    let max_lod = if s.max_lod >= 1.0e30 { vk::LOD_CLAMP_NONE } else { s.max_lod.max(min_lod) };
    let info = vk::SamplerCreateInfo::default()
        .mag_filter(convert::filter(s.mag_filter))
        .min_filter(convert::filter(s.min_filter))
        .mipmap_mode(convert::mipmap_mode(s.mip_filter))
        .address_mode_u(convert::address(s.address_u))
        .address_mode_v(convert::address(s.address_v))
        .address_mode_w(convert::address(s.address_w))
        .mip_lod_bias(s.lod_bias)
        .min_lod(min_lod)
        .max_lod(max_lod)
        .border_color(vk::BorderColor::FLOAT_TRANSPARENT_BLACK);
    unsafe { device.create_sampler(&info, None) }
}
