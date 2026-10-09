//! Cópias amostráveis do alvo. Um shader não pode ler e escrever a mesma imagem no mesmo passe,
//! então o frame do jogo é copiado para uma imagem própria, de onde os passes leem.

use crate::image::GpuImage;
use ash::vk;

#[derive(Default)]
pub struct BackbufferCopies {
    pub images: Vec<GpuImage>,
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
            match unsafe {
                GpuImage::new(device, memory, format, extent, vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED)
            } {
                Ok(image) => copies.images.push(image),
                Err(e) => {
                    unsafe { copies.destroy(device) };
                    return Err(e);
                }
            }
        }
        Ok(copies)
    }

    /// Libera tudo. Pode ser chamado mais de uma vez.
    ///
    /// # Safety
    /// A GPU não pode estar usando as imagens.
    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        for mut image in self.images.drain(..) {
            unsafe { image.destroy(device) };
        }
    }
}
