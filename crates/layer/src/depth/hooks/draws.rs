//! Os desenhos: contagem de desenhos e vértices por imagem de profundidade, e o pipeline em uso.

use super::*;

/// Soma um desenho ao passe em gravação (`vertices` zero para os indiretos).
pub(super) fn on_draw(data: &DeviceData, cmd: vk::CommandBuffer, vertices: u64, indirect: bool) {
    if !data.depth.is_on() {
        return;
    }
    data.depth.frame_draws.fetch_add(1, Ordering::Relaxed);
    if let Some(state) = data.depth.cmds.lock(cmd.as_raw()).get_mut(&cmd.as_raw()) {
        if let Some(p) = state.current {
            data.depth.frame_depth_draws.fetch_add(1, Ordering::Relaxed);
            if p.image == data.depth.watched_image.load(Ordering::Relaxed) {
                data.depth.draws_since_event.fetch_add(1, Ordering::Relaxed);
            }
        }
        let compare = state.compare;
        if let Some(s) = state.screen_current.as_mut() {
            s.draws += 1;
        }
        if let Some(p) = state.current.as_mut() {
            p.draws += 1;
            if indirect {
                p.indirect += 1;
            } else {
                p.vertices += vertices;
            }
            match compare {
                Compare::Greater => p.greater += 1,
                Compare::Less => p.less += 1,
                _ => {}
            }
        }
    }
}

pub(super) unsafe extern "system" fn cmd_draw(cmd: vk::CommandBuffer, vertex_count: u32, instance_count: u32, first_vertex: u32, first_instance: u32) {
    unsafe {
        let sample = crate::stats::hook_should_sample().then(std::time::Instant::now);
        let Some(data) = data_of(cmd.as_raw()) else { return };
        on_draw(&data, cmd, u64::from(vertex_count) * u64::from(instance_count.max(1)), false);
        if let Some(t) = sample {
            crate::stats::hook_sampled(t.elapsed());
        }
        (data.device.fp_v1_0().cmd_draw)(cmd, vertex_count, instance_count, first_vertex, first_instance);
    }
}

pub(super) unsafe extern "system" fn cmd_draw_indexed(cmd: vk::CommandBuffer, index_count: u32, instance_count: u32, first_index: u32, vertex_offset: i32, first_instance: u32) {
    unsafe {
        let sample = crate::stats::hook_should_sample().then(std::time::Instant::now);
        let Some(data) = data_of(cmd.as_raw()) else { return };
        on_draw(&data, cmd, u64::from(index_count) * u64::from(instance_count.max(1)), false);
        if let Some(t) = sample {
            crate::stats::hook_sampled(t.elapsed());
        }
        (data.device.fp_v1_0().cmd_draw_indexed)(cmd, index_count, instance_count, first_index, vertex_offset, first_instance);
    }
}

pub(super) unsafe extern "system" fn cmd_draw_indirect(cmd: vk::CommandBuffer, buffer: vk::Buffer, offset: vk::DeviceSize, draw_count: u32, stride: u32) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        on_draw(&data, cmd, 0, true);
        (data.device.fp_v1_0().cmd_draw_indirect)(cmd, buffer, offset, draw_count, stride);
    }
}

pub(super) unsafe extern "system" fn cmd_draw_indexed_indirect(cmd: vk::CommandBuffer, buffer: vk::Buffer, offset: vk::DeviceSize, draw_count: u32, stride: u32) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        on_draw(&data, cmd, 0, true);
        (data.device.fp_v1_0().cmd_draw_indexed_indirect)(cmd, buffer, offset, draw_count, stride);
    }
}

pub(super) unsafe extern "system" fn cmd_draw_indirect_count(
    cmd: vk::CommandBuffer,
    buffer: vk::Buffer,
    offset: vk::DeviceSize,
    count_buffer: vk::Buffer,
    count_offset: vk::DeviceSize,
    max_draw_count: u32,
    stride: u32,
) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        let Some(next) = data.depth.next.cmd_draw_indirect_count else { return };
        on_draw(&data, cmd, 0, true);
        next(cmd, buffer, offset, count_buffer, count_offset, max_draw_count, stride);
    }
}

pub(super) unsafe extern "system" fn cmd_draw_indexed_indirect_count(
    cmd: vk::CommandBuffer,
    buffer: vk::Buffer,
    offset: vk::DeviceSize,
    count_buffer: vk::Buffer,
    count_offset: vk::DeviceSize,
    max_draw_count: u32,
    stride: u32,
) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        let Some(next) = data.depth.next.cmd_draw_indexed_indirect_count else { return };
        on_draw(&data, cmd, 0, true);
        next(cmd, buffer, offset, count_buffer, count_offset, max_draw_count, stride);
    }
}

pub(super) unsafe extern "system" fn cmd_bind_pipeline(cmd: vk::CommandBuffer, bind_point: vk::PipelineBindPoint, pipeline: vk::Pipeline) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        if data.depth.is_on() && bind_point == vk::PipelineBindPoint::GRAPHICS {
            let compare = data.depth.pipelines.read().unwrap().get(&pipeline.as_raw()).copied().unwrap_or(Compare::Unknown);
            if let Some(state) = data.depth.cmds.lock(cmd.as_raw()).get_mut(&cmd.as_raw()) {
                state.compare = compare;
            }
        }
        (data.device.fp_v1_0().cmd_bind_pipeline)(cmd, bind_point, pipeline);
    }
}
