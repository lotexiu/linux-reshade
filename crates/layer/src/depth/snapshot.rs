//! A cópia da profundidade de um passe (a "foto" depois do N-ésimo passe).
//!
//! Serve para os jogos cuja profundidade é alterada depois do passe da cena (por exemplo um objeto só de profundidade
//! desenhado no fim) e para a pessoa comparar o estado da profundidade em cada passe. A cópia é gravada no command
//! buffer do jogo, logo depois do fim do passe, onde o layout da imagem é conhecido; a imagem de destino é da layer.

use super::Snapshot;
use crate::state::DeviceData;
use ash::vk;

fn aspect(format: vk::Format) -> vk::ImageAspectFlags {
    if postfx::effect::depth_has_stencil(format) { vk::ImageAspectFlags::DEPTH | vk::ImageAspectFlags::STENCIL } else { vk::ImageAspectFlags::DEPTH }
}

fn memory_type(data: &DeviceData, bits: u32) -> Option<u32> {
    (0..data.memory.memory_type_count).find(|&i| bits & (1 << i) != 0 && data.memory.memory_types[i as usize].property_flags.contains(vk::MemoryPropertyFlags::DEVICE_LOCAL))
}

/// Garante a imagem de destino (mesmo formato e tamanho da fonte) e devolve o handle. Recria se mudou.
///
/// # Safety
/// `data.device` válido; nenhuma GPU executando a imagem antiga (quem chama espera o dispositivo ficar ocioso se for trocar).
pub(super) unsafe fn ensure(data: &DeviceData, format: vk::Format, width: u32, height: u32) -> Option<vk::Image> {
    let mut slot = data.depth.snapshot.lock().unwrap();
    if let Some(s) = slot.as_ref().filter(|s| s.format == format && s.width == width && s.height == height) {
        return Some(s.image);
    }
    unsafe {
        data.depth.snapshot_written.store(false, std::sync::atomic::Ordering::Relaxed);
        if let Some(old) = slot.take() {
            let _ = data.device.device_wait_idle();
            data.device.destroy_image(old.image, None);
            data.device.free_memory(old.memory, None);
        }
        let info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(vk::Extent3D { width, height, depth: 1 })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let image = data.device.create_image(&info, None).ok()?;
        let req = data.device.get_image_memory_requirements(image);
        let Some(type_index) = memory_type(data, req.memory_type_bits) else {
            data.device.destroy_image(image, None);
            return None;
        };
        let Ok(memory) = data.device.allocate_memory(&vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(type_index), None) else {
            data.device.destroy_image(image, None);
            return None;
        };
        if data.device.bind_image_memory(image, memory, 0).is_err() {
            data.device.destroy_image(image, None);
            data.device.free_memory(memory, None);
            return None;
        }
        *slot = Some(Snapshot { image, memory, format, width, height });
        Some(image)
    }
}

/// Grava no command buffer a cópia da profundidade (aspecto de profundidade) para a imagem da foto, deixando a fonte no
/// layout em que estava e a foto em `DEPTH_STENCIL_READ_ONLY_OPTIMAL`.
///
/// # Safety
/// `cmd` em gravação, fora de um passe de renderização; `src_layout` é o layout real da fonte neste ponto.
pub(super) unsafe fn record_copy(data: &DeviceData, cmd: vk::CommandBuffer, src: vk::Image, src_layout: vk::ImageLayout, dst: vk::Image, format: vk::Format, width: u32, height: u32) {
    let all = aspect(format);
    let depth_only = vk::ImageSubresourceLayers { aspect_mask: vk::ImageAspectFlags::DEPTH, mip_level: 0, base_array_layer: 0, layer_count: 1 };
    let range = vk::ImageSubresourceRange { aspect_mask: all, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 };
    let barrier = |image: vk::Image, old: vk::ImageLayout, new: vk::ImageLayout| {
        vk::ImageMemoryBarrier::default()
            .image(image)
            .old_layout(old)
            .new_layout(new)
            .src_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE)
            .dst_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .subresource_range(range)
    };
    let f = data.device.fp_v1_0();
    unsafe {
        let all_stages = vk::PipelineStageFlags::ALL_COMMANDS;
        (f.cmd_pipeline_barrier)(
            cmd,
            all_stages,
            all_stages,
            vk::DependencyFlags::empty(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            2,
            [barrier(src, src_layout, vk::ImageLayout::TRANSFER_SRC_OPTIMAL), barrier(dst, vk::ImageLayout::UNDEFINED, vk::ImageLayout::TRANSFER_DST_OPTIMAL)].as_ptr(),
        );
        let region = vk::ImageCopy { src_subresource: depth_only, src_offset: vk::Offset3D::default(), dst_subresource: depth_only, dst_offset: vk::Offset3D::default(), extent: vk::Extent3D { width, height, depth: 1 } };
        (f.cmd_copy_image)(cmd, src, vk::ImageLayout::TRANSFER_SRC_OPTIMAL, dst, vk::ImageLayout::TRANSFER_DST_OPTIMAL, 1, &region);
        (f.cmd_pipeline_barrier)(
            cmd,
            all_stages,
            all_stages,
            vk::DependencyFlags::empty(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            2,
            [barrier(src, vk::ImageLayout::TRANSFER_SRC_OPTIMAL, src_layout), barrier(dst, vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL)].as_ptr(),
        );
    }
}

/// Libera a imagem da foto (ao destruir o dispositivo).
///
/// # Safety
/// A GPU não pode estar usando a imagem.
pub(super) unsafe fn destroy(data: &DeviceData) {
    if let Some(s) = data.depth.snapshot.lock().unwrap().take() {
        unsafe {
            data.device.destroy_image(s.image, None);
            data.device.free_memory(s.memory, None);
        }
    }
}
