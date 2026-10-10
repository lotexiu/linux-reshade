//! Os ganchos Vulkan do observador de profundidade. Cada um repassa a chamada ao próximo elo e, se o observador
//! está ligado, anota o que importa. Nada aqui muda o que o jogo envia.

use super::observe::{ImageInfo, LoadOp, PassRecord, StoreOp};
use super::{Compare, CmdState, RenderPassInfo, compare_of, is_depth_format, load_op, store_op};
use crate::log::log;
use crate::proc_addr::erase;
use crate::state::{DEVICES, DeviceData};
use ash::vk::{self, Handle};
use std::sync::Arc;
use vk_layer::dispatch::dispatch_key;

fn data_of(raw: u64) -> Option<Arc<DeviceData>> {
    // SAFETY: `raw` é um handle despachável entregue pelo loader.
    DEVICES.get(unsafe { dispatch_key(raw) })
}

/// O gancho de um comando de profundidade, pelo nome (e pelos apelidos de extensão).
pub fn intercept_name(name: &[u8]) -> vk::PFN_vkVoidFunction {
    match name {
        b"vkCreateImage" => erase(create_image as vk::PFN_vkCreateImage),
        b"vkDestroyImage" => erase(destroy_image as vk::PFN_vkDestroyImage),
        b"vkCreateImageView" => erase(create_image_view as vk::PFN_vkCreateImageView),
        b"vkCreateFramebuffer" => erase(create_framebuffer as vk::PFN_vkCreateFramebuffer),
        b"vkCreateRenderPass" => erase(create_render_pass as vk::PFN_vkCreateRenderPass),
        b"vkCreateRenderPass2" | b"vkCreateRenderPass2KHR" => erase(create_render_pass2 as vk::PFN_vkCreateRenderPass2),
        b"vkCreateGraphicsPipelines" => erase(create_graphics_pipelines as vk::PFN_vkCreateGraphicsPipelines),
        b"vkBeginCommandBuffer" => erase(begin_command_buffer as vk::PFN_vkBeginCommandBuffer),
        b"vkCmdBeginRenderPass" => erase(cmd_begin_render_pass as vk::PFN_vkCmdBeginRenderPass),
        b"vkCmdBeginRenderPass2" | b"vkCmdBeginRenderPass2KHR" => erase(cmd_begin_render_pass2 as vk::PFN_vkCmdBeginRenderPass2),
        b"vkCmdEndRenderPass" => erase(cmd_end_render_pass as vk::PFN_vkCmdEndRenderPass),
        b"vkCmdEndRenderPass2" | b"vkCmdEndRenderPass2KHR" => erase(cmd_end_render_pass2 as vk::PFN_vkCmdEndRenderPass2),
        b"vkCmdBeginRendering" | b"vkCmdBeginRenderingKHR" => erase(cmd_begin_rendering as vk::PFN_vkCmdBeginRendering),
        b"vkCmdEndRendering" | b"vkCmdEndRenderingKHR" => erase(cmd_end_rendering as vk::PFN_vkCmdEndRendering),
        b"vkCmdDraw" => erase(cmd_draw as vk::PFN_vkCmdDraw),
        b"vkCmdDrawIndexed" => erase(cmd_draw_indexed as vk::PFN_vkCmdDrawIndexed),
        b"vkCmdDrawIndirect" => erase(cmd_draw_indirect as vk::PFN_vkCmdDrawIndirect),
        b"vkCmdDrawIndexedIndirect" => erase(cmd_draw_indexed_indirect as vk::PFN_vkCmdDrawIndexedIndirect),
        b"vkCmdDrawIndirectCount" | b"vkCmdDrawIndirectCountKHR" => erase(cmd_draw_indirect_count as vk::PFN_vkCmdDrawIndirectCount),
        b"vkCmdDrawIndexedIndirectCount" | b"vkCmdDrawIndexedIndirectCountKHR" => {
            erase(cmd_draw_indexed_indirect_count as vk::PFN_vkCmdDrawIndexedIndirectCount)
        }
        b"vkCmdBindPipeline" => erase(cmd_bind_pipeline as vk::PFN_vkCmdBindPipeline),
        b"vkCmdExecuteCommands" => erase(cmd_execute_commands as vk::PFN_vkCmdExecuteCommands),
        b"vkCmdSetDepthCompareOp" | b"vkCmdSetDepthCompareOpEXT" => erase(cmd_set_depth_compare_op as vk::PFN_vkCmdSetDepthCompareOp),
        b"vkQueueSubmit" => erase(queue_submit as vk::PFN_vkQueueSubmit),
        b"vkQueueSubmit2" | b"vkQueueSubmit2KHR" => erase(queue_submit2 as vk::PFN_vkQueueSubmit2),
        _ => None,
    }
}

// ---- criação de objetos (sempre anotada) ----

unsafe extern "system" fn create_image(
    device: vk::Device,
    p_create_info: *const vk::ImageCreateInfo<'_>,
    p_allocator: *const vk::AllocationCallbacks<'_>,
    p_image: *mut vk::Image,
) -> vk::Result {
    unsafe {
        let Some(data) = data_of(device.as_raw()) else { return vk::Result::ERROR_DEVICE_LOST };
        let result = (data.device.fp_v1_0().create_image)(device, p_create_info, p_allocator, p_image);
        if result == vk::Result::SUCCESS {
            let c = &*p_create_info;
            if c.image_type == vk::ImageType::TYPE_2D && is_depth_format(c.format) && c.usage.contains(vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT) {
                let info = ImageInfo { width: c.extent.width, height: c.extent.height, format: c.format.as_raw(), samples: c.samples.as_raw() };
                data.depth.images.lock().unwrap().insert((*p_image).as_raw(), info);
            }
        }
        result
    }
}

unsafe extern "system" fn destroy_image(device: vk::Device, image: vk::Image, p_allocator: *const vk::AllocationCallbacks<'_>) {
    unsafe {
        let Some(data) = data_of(device.as_raw()) else { return };
        data.depth.images.lock().unwrap().remove(&image.as_raw());
        (data.device.fp_v1_0().destroy_image)(device, image, p_allocator);
    }
}

unsafe extern "system" fn create_image_view(
    device: vk::Device,
    p_create_info: *const vk::ImageViewCreateInfo<'_>,
    p_allocator: *const vk::AllocationCallbacks<'_>,
    p_view: *mut vk::ImageView,
) -> vk::Result {
    unsafe {
        let Some(data) = data_of(device.as_raw()) else { return vk::Result::ERROR_DEVICE_LOST };
        let result = (data.device.fp_v1_0().create_image_view)(device, p_create_info, p_allocator, p_view);
        if result == vk::Result::SUCCESS {
            let image = (*p_create_info).image.as_raw();
            if data.depth.images.lock().unwrap().contains_key(&image) {
                data.depth.views.lock().unwrap().insert((*p_view).as_raw(), image);
            }
        }
        result
    }
}

unsafe extern "system" fn create_framebuffer(
    device: vk::Device,
    p_create_info: *const vk::FramebufferCreateInfo<'_>,
    p_allocator: *const vk::AllocationCallbacks<'_>,
    p_framebuffer: *mut vk::Framebuffer,
) -> vk::Result {
    unsafe {
        let Some(data) = data_of(device.as_raw()) else { return vk::Result::ERROR_DEVICE_LOST };
        let result = (data.device.fp_v1_0().create_framebuffer)(device, p_create_info, p_allocator, p_framebuffer);
        if result == vk::Result::SUCCESS {
            let c = &*p_create_info;
            // Framebuffer sem imagens (imageless) não tem views aqui: as views vêm em cada início de passe.
            if !c.p_attachments.is_null() && c.attachment_count > 0 {
                let views: Vec<u64> = std::slice::from_raw_parts(c.p_attachments, c.attachment_count as usize).iter().map(|v| v.as_raw()).collect();
                let depth_views = data.depth.views.lock().unwrap();
                if views.iter().any(|v| depth_views.contains_key(v)) {
                    drop(depth_views);
                    data.depth.framebuffers.lock().unwrap().insert((*p_framebuffer).as_raw(), views);
                }
            }
        }
        result
    }
}

unsafe extern "system" fn create_render_pass(
    device: vk::Device,
    p_create_info: *const vk::RenderPassCreateInfo<'_>,
    p_allocator: *const vk::AllocationCallbacks<'_>,
    p_render_pass: *mut vk::RenderPass,
) -> vk::Result {
    unsafe {
        let Some(data) = data_of(device.as_raw()) else { return vk::Result::ERROR_DEVICE_LOST };
        let result = (data.device.fp_v1_0().create_render_pass)(device, p_create_info, p_allocator, p_render_pass);
        if result == vk::Result::SUCCESS {
            let c = &*p_create_info;
            if !c.p_subpasses.is_null() && !c.p_attachments.is_null() {
                let subpasses = std::slice::from_raw_parts(c.p_subpasses, c.subpass_count as usize);
                let attachments = std::slice::from_raw_parts(c.p_attachments, c.attachment_count as usize);
                let found = subpasses.iter().filter(|s| !s.p_depth_stencil_attachment.is_null()).map(|s| (*s.p_depth_stencil_attachment).attachment).find(|a| *a != vk::ATTACHMENT_UNUSED);
                if let Some(index) = found.filter(|i| (*i as usize) < attachments.len()) {
                    let a = &attachments[index as usize];
                    if is_depth_format(a.format) {
                        data.depth.render_passes.lock().unwrap().insert(
                            (*p_render_pass).as_raw(),
                            RenderPassInfo { depth_attachment: index, load: load_op(a.load_op), store: store_op(a.store_op) },
                        );
                    }
                }
            }
        }
        result
    }
}

unsafe extern "system" fn create_render_pass2(
    device: vk::Device,
    p_create_info: *const vk::RenderPassCreateInfo2<'_>,
    p_allocator: *const vk::AllocationCallbacks<'_>,
    p_render_pass: *mut vk::RenderPass,
) -> vk::Result {
    unsafe {
        let Some(data) = data_of(device.as_raw()) else { return vk::Result::ERROR_DEVICE_LOST };
        let Some(next) = data.depth.next.create_render_pass2 else { return vk::Result::ERROR_EXTENSION_NOT_PRESENT };
        let result = next(device, p_create_info, p_allocator, p_render_pass);
        if result == vk::Result::SUCCESS {
            let c = &*p_create_info;
            if !c.p_subpasses.is_null() && !c.p_attachments.is_null() {
                let subpasses = std::slice::from_raw_parts(c.p_subpasses, c.subpass_count as usize);
                let attachments = std::slice::from_raw_parts(c.p_attachments, c.attachment_count as usize);
                let found = subpasses.iter().filter(|s| !s.p_depth_stencil_attachment.is_null()).map(|s| (*s.p_depth_stencil_attachment).attachment).find(|a| *a != vk::ATTACHMENT_UNUSED);
                if let Some(index) = found.filter(|i| (*i as usize) < attachments.len()) {
                    let a = &attachments[index as usize];
                    if is_depth_format(a.format) {
                        data.depth.render_passes.lock().unwrap().insert(
                            (*p_render_pass).as_raw(),
                            RenderPassInfo { depth_attachment: index, load: load_op(a.load_op), store: store_op(a.store_op) },
                        );
                    }
                }
            }
        }
        result
    }
}

unsafe extern "system" fn create_graphics_pipelines(
    device: vk::Device,
    cache: vk::PipelineCache,
    count: u32,
    p_create_infos: *const vk::GraphicsPipelineCreateInfo<'_>,
    p_allocator: *const vk::AllocationCallbacks<'_>,
    p_pipelines: *mut vk::Pipeline,
) -> vk::Result {
    unsafe {
        let Some(data) = data_of(device.as_raw()) else { return vk::Result::ERROR_DEVICE_LOST };
        let result = (data.device.fp_v1_0().create_graphics_pipelines)(device, cache, count, p_create_infos, p_allocator, p_pipelines);
        // Com sucesso parcial os handles inválidos são nulos; só anota os que existem.
        if result == vk::Result::SUCCESS && !p_create_infos.is_null() {
            let infos = std::slice::from_raw_parts(p_create_infos, count as usize);
            let pipelines = std::slice::from_raw_parts(p_pipelines, count as usize);
            let mut map = data.depth.pipelines.lock().unwrap();
            for (info, pipeline) in infos.iter().zip(pipelines) {
                let compare = if info.p_depth_stencil_state.is_null() {
                    Compare::Unknown
                } else {
                    let ds = &*info.p_depth_stencil_state;
                    let dynamic = !info.p_dynamic_state.is_null() && {
                        let d = &*info.p_dynamic_state;
                        !d.p_dynamic_states.is_null() && std::slice::from_raw_parts(d.p_dynamic_states, d.dynamic_state_count as usize).contains(&vk::DynamicState::DEPTH_COMPARE_OP)
                    };
                    if ds.depth_test_enable == vk::FALSE || dynamic { Compare::Unknown } else { compare_of(ds.depth_compare_op) }
                };
                if compare != Compare::Unknown {
                    map.insert(pipeline.as_raw(), compare);
                }
            }
        }
        result
    }
}

// ---- gravação ----

unsafe extern "system" fn begin_command_buffer(cmd: vk::CommandBuffer, p_begin_info: *const vk::CommandBufferBeginInfo<'_>) -> vk::Result {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return vk::Result::ERROR_DEVICE_LOST };
        if data.depth.is_on() {
            data.depth.cmds.lock().unwrap().insert(cmd.as_raw(), CmdState::default());
        }
        (data.device.fp_v1_0().begin_command_buffer)(cmd, p_begin_info)
    }
}

/// Começa o registro de um passe com profundidade, se o render pass tem uma.
fn start_pass(data: &DeviceData, cmd: vk::CommandBuffer, render_pass: vk::RenderPass, views: Option<Vec<u64>>, framebuffer: vk::Framebuffer, area: vk::Rect2D, clear_depth: impl Fn(u32) -> Option<f32>) {
    let rp = data.depth.render_passes.lock().unwrap().get(&render_pass.as_raw()).copied();
    let Some(rp) = rp else { return };
    let views = views.or_else(|| data.depth.framebuffers.lock().unwrap().get(&framebuffer.as_raw()).cloned());
    let Some(view) = views.and_then(|v| v.get(rp.depth_attachment as usize).copied()) else { return };
    let Some(image) = data.depth.views.lock().unwrap().get(&view).copied() else { return };
    let record = PassRecord {
        image,
        load: rp.load,
        store: rp.store,
        clear_depth: if rp.load == LoadOp::Clear { clear_depth(rp.depth_attachment) } else { None },
        area: (area.extent.width, area.extent.height),
        ..PassRecord::default()
    };
    if let Some(state) = data.depth.cmds.lock().unwrap().get_mut(&cmd.as_raw()) {
        state.current = Some(record);
    }
}

/// As views dos anexos dadas no início do passe (framebuffer sem imagens), se houver.
unsafe fn imageless_views(mut next: *const std::ffi::c_void) -> Option<Vec<u64>> {
    unsafe {
        while !next.is_null() {
            let base = &*(next as *const vk::BaseInStructure<'_>);
            if base.s_type == vk::StructureType::RENDER_PASS_ATTACHMENT_BEGIN_INFO {
                let info = &*(next as *const vk::RenderPassAttachmentBeginInfo<'_>);
                if info.p_attachments.is_null() {
                    return None;
                }
                return Some(std::slice::from_raw_parts(info.p_attachments, info.attachment_count as usize).iter().map(|v| v.as_raw()).collect());
            }
            next = base.p_next as *const std::ffi::c_void;
        }
        None
    }
}

unsafe fn on_begin_render_pass(data: &DeviceData, cmd: vk::CommandBuffer, begin: *const vk::RenderPassBeginInfo<'_>) {
    unsafe {
        if !data.depth.is_on() || begin.is_null() {
            return;
        }
        let b = &*begin;
        let clears: &[vk::ClearValue] = if b.p_clear_values.is_null() { &[] } else { std::slice::from_raw_parts(b.p_clear_values, b.clear_value_count as usize) };
        let views = imageless_views(b.p_next);
        start_pass(data, cmd, b.render_pass, views, b.framebuffer, b.render_area, |i| clears.get(i as usize).map(|c| c.depth_stencil.depth));
    }
}

unsafe extern "system" fn cmd_begin_render_pass(cmd: vk::CommandBuffer, p_begin: *const vk::RenderPassBeginInfo<'_>, contents: vk::SubpassContents) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        on_begin_render_pass(&data, cmd, p_begin);
        (data.device.fp_v1_0().cmd_begin_render_pass)(cmd, p_begin, contents);
    }
}

unsafe extern "system" fn cmd_begin_render_pass2(cmd: vk::CommandBuffer, p_begin: *const vk::RenderPassBeginInfo<'_>, p_subpass: *const vk::SubpassBeginInfo<'_>) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        let Some(next) = data.depth.next.cmd_begin_render_pass2 else { return };
        on_begin_render_pass(&data, cmd, p_begin);
        next(cmd, p_begin, p_subpass);
    }
}

/// Fecha o passe que estava sendo gravado e o guarda no command buffer.
fn finish_pass(data: &DeviceData, cmd: vk::CommandBuffer) {
    if !data.depth.is_on() {
        return;
    }
    if let Some(state) = data.depth.cmds.lock().unwrap().get_mut(&cmd.as_raw())
        && let Some(pass) = state.current.take()
    {
        state.passes.push(pass);
    }
}

unsafe extern "system" fn cmd_end_render_pass(cmd: vk::CommandBuffer) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        finish_pass(&data, cmd);
        (data.device.fp_v1_0().cmd_end_render_pass)(cmd);
    }
}

unsafe extern "system" fn cmd_end_render_pass2(cmd: vk::CommandBuffer, p_end: *const vk::SubpassEndInfo<'_>) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        let Some(next) = data.depth.next.cmd_end_render_pass2 else { return };
        finish_pass(&data, cmd);
        next(cmd, p_end);
    }
}

unsafe extern "system" fn cmd_begin_rendering(cmd: vk::CommandBuffer, p_info: *const vk::RenderingInfo<'_>) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        let Some(next) = data.depth.next.cmd_begin_rendering else { return };
        if data.depth.is_on() && !p_info.is_null() {
            let info = &*p_info;
            if !info.p_depth_attachment.is_null() {
                let a = &*info.p_depth_attachment;
                let image = data.depth.views.lock().unwrap().get(&a.image_view.as_raw()).copied();
                if let Some(image) = image {
                    let load = load_op(a.load_op);
                    let record = PassRecord {
                        image,
                        load,
                        store: store_op(a.store_op),
                        clear_depth: (load == LoadOp::Clear).then(|| a.clear_value.depth_stencil.depth),
                        area: (info.render_area.extent.width, info.render_area.extent.height),
                        ..PassRecord::default()
                    };
                    if let Some(state) = data.depth.cmds.lock().unwrap().get_mut(&cmd.as_raw()) {
                        state.current = Some(record);
                    }
                }
            }
        }
        next(cmd, p_info);
    }
}

unsafe extern "system" fn cmd_end_rendering(cmd: vk::CommandBuffer) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        let Some(next) = data.depth.next.cmd_end_rendering else { return };
        finish_pass(&data, cmd);
        next(cmd);
    }
}

/// Soma um desenho ao passe em gravação (`vertices` zero para os indiretos).
fn on_draw(data: &DeviceData, cmd: vk::CommandBuffer, vertices: u64, indirect: bool) {
    if !data.depth.is_on() {
        return;
    }
    if let Some(state) = data.depth.cmds.lock().unwrap().get_mut(&cmd.as_raw()) {
        let compare = state.compare;
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

unsafe extern "system" fn cmd_draw(cmd: vk::CommandBuffer, vertex_count: u32, instance_count: u32, first_vertex: u32, first_instance: u32) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        on_draw(&data, cmd, u64::from(vertex_count) * u64::from(instance_count.max(1)), false);
        (data.device.fp_v1_0().cmd_draw)(cmd, vertex_count, instance_count, first_vertex, first_instance);
    }
}

unsafe extern "system" fn cmd_draw_indexed(cmd: vk::CommandBuffer, index_count: u32, instance_count: u32, first_index: u32, vertex_offset: i32, first_instance: u32) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        on_draw(&data, cmd, u64::from(index_count) * u64::from(instance_count.max(1)), false);
        (data.device.fp_v1_0().cmd_draw_indexed)(cmd, index_count, instance_count, first_index, vertex_offset, first_instance);
    }
}

unsafe extern "system" fn cmd_draw_indirect(cmd: vk::CommandBuffer, buffer: vk::Buffer, offset: vk::DeviceSize, draw_count: u32, stride: u32) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        on_draw(&data, cmd, 0, true);
        (data.device.fp_v1_0().cmd_draw_indirect)(cmd, buffer, offset, draw_count, stride);
    }
}

unsafe extern "system" fn cmd_draw_indexed_indirect(cmd: vk::CommandBuffer, buffer: vk::Buffer, offset: vk::DeviceSize, draw_count: u32, stride: u32) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        on_draw(&data, cmd, 0, true);
        (data.device.fp_v1_0().cmd_draw_indexed_indirect)(cmd, buffer, offset, draw_count, stride);
    }
}

unsafe extern "system" fn cmd_draw_indirect_count(
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

unsafe extern "system" fn cmd_draw_indexed_indirect_count(
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

unsafe extern "system" fn cmd_bind_pipeline(cmd: vk::CommandBuffer, bind_point: vk::PipelineBindPoint, pipeline: vk::Pipeline) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        if data.depth.is_on() && bind_point == vk::PipelineBindPoint::GRAPHICS {
            let compare = data.depth.pipelines.lock().unwrap().get(&pipeline.as_raw()).copied().unwrap_or(Compare::Unknown);
            if let Some(state) = data.depth.cmds.lock().unwrap().get_mut(&cmd.as_raw()) {
                state.compare = compare;
            }
        }
        (data.device.fp_v1_0().cmd_bind_pipeline)(cmd, bind_point, pipeline);
    }
}

unsafe extern "system" fn cmd_set_depth_compare_op(cmd: vk::CommandBuffer, op: vk::CompareOp) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        let Some(next) = data.depth.next.cmd_set_depth_compare_op else { return };
        if data.depth.is_on()
            && let Some(state) = data.depth.cmds.lock().unwrap().get_mut(&cmd.as_raw())
        {
            state.compare = compare_of(op);
        }
        next(cmd, op);
    }
}

unsafe extern "system" fn cmd_execute_commands(cmd: vk::CommandBuffer, count: u32, p_buffers: *const vk::CommandBuffer) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        // Os command buffers secundários não são seguidos: contam como desenhos indiretos do passe em gravação
        // (o ReShade faz o mesmo), para a imagem aparecer nas candidatas.
        if data.depth.is_on()
            && let Some(state) = data.depth.cmds.lock().unwrap().get_mut(&cmd.as_raw())
            && let Some(p) = state.current.as_mut()
        {
            p.draws += count;
            p.indirect += count;
        }
        (data.device.fp_v1_0().cmd_execute_commands)(cmd, count, p_buffers);
    }
}

// ---- envio à fila e fim do quadro ----

/// Junta ao quadro o que os command buffers enviados gravaram.
fn merge_submitted(data: &DeviceData, buffers: &[vk::CommandBuffer]) {
    if !data.depth.is_on() || buffers.is_empty() {
        return;
    }
    let passes: Vec<PassRecord> = {
        let cmds = data.depth.cmds.lock().unwrap();
        buffers.iter().filter_map(|b| cmds.get(&b.as_raw())).flat_map(|s| s.passes.iter().copied()).collect()
    };
    if passes.is_empty() {
        return;
    }
    let images = data.depth.images.lock().unwrap().clone();
    data.depth.frame.lock().unwrap().merge(&passes, &images);
}

unsafe extern "system" fn queue_submit(queue: vk::Queue, count: u32, p_submits: *const vk::SubmitInfo<'_>, fence: vk::Fence) -> vk::Result {
    unsafe {
        let Some(data) = data_of(queue.as_raw()) else { return vk::Result::ERROR_DEVICE_LOST };
        if data.depth.is_on() && !p_submits.is_null() {
            for submit in std::slice::from_raw_parts(p_submits, count as usize) {
                if !submit.p_command_buffers.is_null() {
                    merge_submitted(&data, std::slice::from_raw_parts(submit.p_command_buffers, submit.command_buffer_count as usize));
                }
            }
        }
        (data.device.fp_v1_0().queue_submit)(queue, count, p_submits, fence)
    }
}

unsafe extern "system" fn queue_submit2(queue: vk::Queue, count: u32, p_submits: *const vk::SubmitInfo2<'_>, fence: vk::Fence) -> vk::Result {
    unsafe {
        let Some(data) = data_of(queue.as_raw()) else { return vk::Result::ERROR_DEVICE_LOST };
        let Some(next) = data.depth.next.queue_submit2 else { return vk::Result::ERROR_EXTENSION_NOT_PRESENT };
        if data.depth.is_on() && !p_submits.is_null() {
            for submit in std::slice::from_raw_parts(p_submits, count as usize) {
                if !submit.p_command_buffer_infos.is_null() {
                    let buffers: Vec<vk::CommandBuffer> = std::slice::from_raw_parts(submit.p_command_buffer_infos, submit.command_buffer_info_count as usize).iter().map(|i| i.command_buffer).collect();
                    merge_submitted(&data, &buffers);
                }
            }
        }
        next(queue, count, p_submits, fence)
    }
}

/// Fim de um quadro (chamado no present): escolhe a candidata e publica a lista para o app de controle.
pub fn end_frame(data: &DeviceData, frame: (u32, u32)) {
    if !data.depth.is_on() {
        return;
    }
    let stats = std::mem::take(&mut *data.depth.frame.lock().unwrap());
    if stats.is_empty() {
        return;
    }
    let candidates = stats.candidates(frame);
    let mut last = data.depth.last_published.lock().unwrap();
    if *last == candidates {
        return;
    }
    if let Some(c) = candidates.iter().find(|c| c.chosen) {
        let key = |c: &super::observe::Candidate| (c.width, c.height, c.format, c.load, c.store, c.reversed);
        if last.iter().find(|p| p.chosen).map(key) != Some(key(c)) {
            log!(
                "profundidade escolhida: {}x{} formato {} · {} desenhos, {} vértices, {} passe(s) · começa com {:?}, termina com {:?} · invertida: {:?}",
                c.width,
                c.height,
                c.format,
                c.draws,
                c.vertices,
                c.passes,
                c.load,
                c.store,
                c.reversed
            );
        }
    }
    // Para entender jogos em que a comparação de profundidade não aparece: o que os pipelines criados declaram.
    {
        let pipelines = data.depth.pipelines.lock().unwrap();
        let count = |c: Compare| pipelines.values().filter(|v| **v == c).count();
        let (greater, less, other) = (count(Compare::Greater), count(Compare::Less), count(Compare::Other));
        let mut logged = data.depth.pipelines_logged.lock().unwrap();
        if *logged != (greater, less, other) {
            *logged = (greater, less, other);
            log!("pipelines com teste de profundidade fixo: {greater} comparam 'maior', {less} 'menor', {other} outros");
        }
    }
    crate::status::set_depth(candidates.iter().map(to_status).collect());
    *last = candidates;
}

fn to_status(c: &super::observe::Candidate) -> ipc::DepthStatus {
    let load = match c.load {
        LoadOp::Load => "load",
        LoadOp::Clear => "clear",
        LoadOp::DontCare => "dont_care",
        LoadOp::None => "none",
    };
    let store = match c.store {
        StoreOp::Store => "store",
        StoreOp::DontCare => "dont_care",
        StoreOp::None => "none",
    };
    ipc::DepthStatus {
        width: c.width,
        height: c.height,
        format: c.format,
        samples: c.samples,
        draws: c.draws,
        vertices: c.vertices,
        passes: c.passes,
        load: load.into(),
        store: store.into(),
        reversed: c.reversed,
        last_pass: c.last_pass,
        chosen: c.chosen,
        unusable: c.unusable.map(String::from),
    }
}
