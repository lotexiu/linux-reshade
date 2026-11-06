//! A cópia da profundidade de um passe (a "foto" depois do N-ésimo passe).
//!
//! Serve para os jogos cuja profundidade é alterada depois do passe da cena (por exemplo um objeto só de profundidade
//! desenhado no fim) e para a pessoa comparar o estado da profundidade em cada passe. A cópia é gravada no command
//! buffer do jogo, logo depois do fim do passe, onde o layout da imagem é conhecido; a imagem de destino é da layer.

use super::Snapshot;
use crate::state::DeviceData;
use ash::vk::{self, Handle};

fn aspect(format: vk::Format) -> vk::ImageAspectFlags {
    if postfx::effect::depth_has_stencil(format) { vk::ImageAspectFlags::DEPTH | vk::ImageAspectFlags::STENCIL } else { vk::ImageAspectFlags::DEPTH }
}

fn memory_type(data: &DeviceData, bits: u32) -> Option<u32> {
    (0..data.memory.memory_type_count).find(|&i| bits & (1 << i) != 0 && data.memory.memory_types[i as usize].property_flags.contains(vk::MemoryPropertyFlags::DEVICE_LOCAL))
}

/// Quantos quadros uma foto substituída continua viva antes de ser destruída.
const RETIRED_FRAMES: u32 = 300;

/// Libera a foto (imagem, memória e o que o resolve criou).
///
/// # Safety
/// A GPU não pode estar usando a foto.
pub(super) unsafe fn free(device: &ash::Device, old: Snapshot) {
    unsafe {
        if let Some(target) = old.resolve {
            super::resolve::destroy_target(device, target);
        }
        device.destroy_image(old.image, None);
        device.free_memory(old.memory, None);
    }
}

/// Conta um quadro para as fotos substituídas e destrói as que já passaram do prazo.
///
/// # Safety
/// `data.device` válido.
pub(super) unsafe fn reap_retired(data: &DeviceData) {
    let mut retired = data.depth.retired.lock().unwrap();
    if retired.is_empty() {
        return;
    }
    for (_, frames) in retired.iter_mut() {
        *frames = frames.saturating_sub(1);
    }
    let (expired, alive): (Vec<_>, Vec<_>) = std::mem::take(&mut *retired).into_iter().partition(|(_, frames)| *frames == 0);
    *retired = alive;
    drop(retired);
    for (old, _) in expired {
        unsafe { free(&data.device, old) };
    }
}

/// Garante a imagem de destino (mesmo formato e tamanho da fonte) e devolve o handle. Recria se mudou.
///
/// # Safety
/// `data.device` válido; nenhuma GPU executando a imagem antiga (quem chama espera o dispositivo ficar ocioso se for trocar).
pub(super) unsafe fn ensure(data: &DeviceData, format: vk::Format, width: u32, height: u32, msaa: bool) -> Option<vk::Image> {
    let mut slot = data.depth.snapshot.lock().unwrap();
    if let Some(s) = slot.as_mut().filter(|s| s.format == format && s.width == width && s.height == height) {
        // Para uma fonte MSAA a foto também precisa do passe de resolve.
        if msaa && !unsafe { super::resolve::ensure_target(data, s) } {
            return None;
        }
        return Some(s.image);
    }
    unsafe {
        data.depth.snapshot_written.store(false, std::sync::atomic::Ordering::Relaxed);
        if let Some(old) = slot.take() {
            // Os efeitos construídos com a foto antiga têm uma view dela: saem antes (o `device_wait_idle` é deles). Command buffers
            // do jogo já gravados com a foto antiga ainda podem ir à fila: ela só é destruída bem depois (`retire`).
            crate::effect::on_depth_image_destroyed(data, old.image.as_raw());
            data.depth.retired.lock().unwrap().push((old, RETIRED_FRAMES));
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
        let mut snapshot = Snapshot { image, memory, format, width, height, resolve: None };
        if msaa && !super::resolve::ensure_target(data, &mut snapshot) {
            free(&data.device, snapshot);
            return None;
        }
        *slot = Some(snapshot);
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

/// Command buffers próprios (um por imagem do swapchain) para a cópia da profundidade feita na hora dos efeitos.
pub(crate) struct FrameCopier {
    pool: vk::CommandPool,
    family: u32,
    buffers: Vec<vk::CommandBuffer>,
}

/// O que a cópia da hora dos efeitos fez neste quadro.
pub enum PresentCopy {
    /// Não precisa (já houve cópia no ponto de uma limpeza ou passe, ou o modo é o da imagem original).
    NotNeeded,
    /// Gravada neste command buffer: vai à frente dos efeitos no mesmo envio.
    Recorded(vk::CommandBuffer),
    /// Não deu (a imagem do jogo está sem layout conhecido, ou sumiu).
    Unavailable,
}

/// A cópia da profundidade do jogo para a foto, gravada agora com o layout de agora (o que o jogo deixou depois do último envio):
/// é a "cópia na hora dos efeitos" do ReShade. Os efeitos leem a foto, que está sempre no layout de leitura que gravamos, e não
/// dependem do layout da imagem do jogo (que variava entre quadros e estava fixo na cadeia gravada).
///
/// # Safety
/// `data.device` válido; chamado na thread do present, depois de esperar a fence do índice `index` (que protege o command buffer).
pub(crate) unsafe fn copy_for_present(data: &DeviceData, family: u32, index: usize) -> PresentCopy {
    if !data.depth.copy_mode || data.depth.copied_frame.load(std::sync::atomic::Ordering::Relaxed) {
        return PresentCopy::NotNeeded;
    }
    let Some(live) = *data.depth.live.lock().unwrap() else { return PresentCopy::Unavailable };
    let layout = data.depth.image_layouts.lock().unwrap().get(&live.image).copied().unwrap_or(0);
    let Some((dst, format)) = data.depth.snapshot.lock().unwrap().as_ref().map(|s| (s.image, s.format)) else { return PresentCopy::Unavailable };
    if layout == 0 || layout == vk::ImageLayout::UNDEFINED.as_raw() || format.as_raw() != live.format {
        return PresentCopy::Unavailable;
    }
    let device = &data.device;
    let mut slot = data.depth.frame_copier.lock().unwrap();
    unsafe {
        if slot.as_ref().is_some_and(|c| c.family != family) {
            if let Some(old) = slot.take() {
                device.destroy_command_pool(old.pool, None);
            }
        }
        if slot.is_none() {
            let Ok(pool) = device.create_command_pool(&vk::CommandPoolCreateInfo::default().queue_family_index(family).flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER), None) else {
                return PresentCopy::Unavailable;
            };
            *slot = Some(FrameCopier { pool, family, buffers: Vec::new() });
        }
        let copier = slot.as_mut().expect("criado acima");
        while copier.buffers.len() <= index {
            let Ok(cbs) = device.allocate_command_buffers(&vk::CommandBufferAllocateInfo::default().command_pool(copier.pool).command_buffer_count(1)) else {
                return PresentCopy::Unavailable;
            };
            if (data.set_loader_data)(device.handle(), cbs[0].as_raw() as *mut std::ffi::c_void) != vk::Result::SUCCESS {
                device.free_command_buffers(copier.pool, &cbs);
                return PresentCopy::Unavailable;
            }
            copier.buffers.push(cbs[0]);
        }
        let cb = copier.buffers[index];
        if device.reset_command_buffer(cb, vk::CommandBufferResetFlags::empty()).is_err() || device.begin_command_buffer(cb, &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)).is_err() {
            return PresentCopy::Unavailable;
        }
        record_copy(data, cb, vk::Image::from_raw(live.image), vk::ImageLayout::from_raw(layout), dst, format, live.width, live.height);
        if device.end_command_buffer(cb).is_err() {
            return PresentCopy::Unavailable;
        }
        data.depth.snapshot_written.store(true, std::sync::atomic::Ordering::Relaxed);
        PresentCopy::Recorded(cb)
    }
}

/// Libera a imagem da foto (ao destruir o dispositivo).
///
/// # Safety
/// A GPU não pode estar usando a imagem.
pub(super) unsafe fn destroy(data: &DeviceData) {
    for (old, _) in data.depth.retired.lock().unwrap().drain(..) {
        unsafe { free(&data.device, old) };
    }
    if let Some(s) = data.depth.snapshot.lock().unwrap().take() {
        unsafe { free(&data.device, s) };
    }
    if let Some(copier) = data.depth.frame_copier.lock().unwrap().take() {
        unsafe { data.device.destroy_command_pool(copier.pool, None) };
    }
    unsafe { super::resolve::destroy_resolver(data) };
}
