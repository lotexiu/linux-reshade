//! Barreiras, layouts, a comparação de profundidade dinâmica e os command buffers secundários.

use super::*;

/// Anota as barreiras que mudam o layout de uma imagem de profundidade.
/// Anota (no command buffer) um evento sobre uma imagem do swapchain.
pub(super) fn screen_event(data: &DeviceData, cmd: vk::CommandBuffer, image: vk::Image, kind: crate::depth::screen::Kind) {
    if !data.depth.is_on() || !data.depth.screen.is_image(image.as_raw(), || crate::effect::is_swapchain_image(data, image.as_raw())) {
        return;
    }
    if let Some(state) = data.depth.cmds.lock(cmd.as_raw()).get_mut(&cmd.as_raw()) {
        state.screen_passes.push(crate::depth::screen::ScreenPass::event(kind));
    }
}

pub(super) fn note_layout(data: &DeviceData, cmd: vk::CommandBuffer, image: vk::Image, new_layout: vk::ImageLayout) {
    screen_event(data, cmd, image, crate::depth::screen::Kind::Barrier(new_layout.as_raw()));
    if new_layout == vk::ImageLayout::UNDEFINED || !data.depth.images.lock().unwrap().contains_key(&image.as_raw()) {
        return;
    }
    if let Some(state) = data.depth.cmds.lock(cmd.as_raw()).get_mut(&cmd.as_raw()) {
        state.layouts.push((image.as_raw(), new_layout.as_raw()));
    }
}

pub(super) unsafe extern "system" fn cmd_pipeline_barrier(
    cmd: vk::CommandBuffer,
    src_stage: vk::PipelineStageFlags,
    dst_stage: vk::PipelineStageFlags,
    dependency: vk::DependencyFlags,
    memory_count: u32,
    p_memory: *const vk::MemoryBarrier<'_>,
    buffer_count: u32,
    p_buffer: *const vk::BufferMemoryBarrier<'_>,
    image_count: u32,
    p_image: *const vk::ImageMemoryBarrier<'_>,
) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        if data.depth.is_on() && !p_image.is_null() {
            for b in std::slice::from_raw_parts(p_image, image_count as usize) {
                note_layout(&data, cmd, b.image, b.new_layout);
            }
        }
        (data.device.fp_v1_0().cmd_pipeline_barrier)(cmd, src_stage, dst_stage, dependency, memory_count, p_memory, buffer_count, p_buffer, image_count, p_image);
    }
}

pub(super) unsafe extern "system" fn cmd_pipeline_barrier2(cmd: vk::CommandBuffer, p_info: *const vk::DependencyInfo<'_>) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        let Some(next) = data.depth.next.cmd_pipeline_barrier2 else { return };
        if data.depth.is_on() && !p_info.is_null() {
            let info = &*p_info;
            if !info.p_image_memory_barriers.is_null() {
                for b in std::slice::from_raw_parts(info.p_image_memory_barriers, info.image_memory_barrier_count as usize) {
                    note_layout(&data, cmd, b.image, b.new_layout);
                }
            }
        }
        next(cmd, p_info);
    }
}

pub(super) unsafe extern "system" fn cmd_set_depth_compare_op(cmd: vk::CommandBuffer, op: vk::CompareOp) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        let Some(next) = data.depth.next.cmd_set_depth_compare_op else { return };
        if data.depth.is_on()
            && let Some(state) = data.depth.cmds.lock(cmd.as_raw()).get_mut(&cmd.as_raw())
        {
            state.compare = compare_of(op);
        }
        next(cmd, op);
    }
}

pub(super) unsafe extern "system" fn cmd_execute_commands(cmd: vk::CommandBuffer, count: u32, p_buffers: *const vk::CommandBuffer) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        // Os command buffers secundários não são seguidos: contam como desenhos indiretos do passe em gravação
        // (o ReShade faz o mesmo), para a imagem aparecer nas candidatas.
        if data.depth.is_on()
            && let Some(state) = data.depth.cmds.lock(cmd.as_raw()).get_mut(&cmd.as_raw())
        {
            if let Some(s) = state.screen_current.as_mut() {
                s.draws += count;
            }
            if let Some(p) = state.current.as_mut() {
                p.draws += count;
                p.indirect += count;
            }
        }
        (data.device.fp_v1_0().cmd_execute_commands)(cmd, count, p_buffers);
    }
}
