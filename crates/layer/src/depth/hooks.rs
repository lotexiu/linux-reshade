//! Os ganchos Vulkan do observador de profundidade. Cada um repassa a chamada ao próximo elo e, se o observador
//! está ligado, anota o que importa. Nada aqui muda o que o jogo envia.

use super::observe::{ImageInfo, LoadOp, PassRecord, StoreOp};
use super::{Compare, CmdState, RenderPassInfo, compare_of, is_depth_format, load_op, store_op};
use crate::log::log;
use crate::proc_addr::erase;
use crate::state::{DEVICES, DeviceData};
use ash::vk::{self, Handle};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use vk_layer::dispatch::dispatch_key;

/// Uma referência ao dado do dispositivo sem contar referências (cada gancho de desenho pagava duas operações atômicas e uma
/// busca num mapa). Vale enquanto o dispositivo existe, o que o Vulkan já exige de quem usa o command buffer.
pub(super) struct DeviceRef(*const DeviceData);

impl std::ops::Deref for DeviceRef {
    type Target = DeviceData;

    fn deref(&self) -> &DeviceData {
        // SAFETY: o registro mantém o `Arc` vivo até `vkDestroyDevice`, e usar um command buffer depois disso é erro do jogo.
        unsafe { &*self.0 }
    }
}

thread_local! {
    /// O último dispositivo visto por esta thread: `(chave de dispatch, geração, dado)`.
    static LAST_DEVICE: std::cell::RefCell<Option<(usize, u64, Arc<DeviceData>)>> = const { std::cell::RefCell::new(None) };
}

fn data_of(raw: u64) -> Option<DeviceRef> {
    // SAFETY: `raw` é um handle despachável entregue pelo loader.
    let key = unsafe { dispatch_key(raw) };
    let generation = crate::state::DEVICES_GENERATION.load(Ordering::Acquire);
    LAST_DEVICE.with(|cell| {
        let mut cache = cell.borrow_mut();
        if let Some((k, g, data)) = cache.as_ref()
            && *k == key
            && *g == generation
        {
            return Some(DeviceRef(Arc::as_ptr(data)));
        }
        let data = DEVICES.get(key)?;
        let reference = DeviceRef(Arc::as_ptr(&data));
        *cache = Some((key, generation, data));
        Some(reference)
    })
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
        b"vkCmdPipelineBarrier" => erase(cmd_pipeline_barrier as vk::PFN_vkCmdPipelineBarrier),
        b"vkCmdPipelineBarrier2" | b"vkCmdPipelineBarrier2KHR" => erase(cmd_pipeline_barrier2 as vk::PFN_vkCmdPipelineBarrier2),
        b"vkCmdExecuteCommands" => erase(cmd_execute_commands as vk::PFN_vkCmdExecuteCommands),
        b"vkCmdSetDepthCompareOp" | b"vkCmdSetDepthCompareOpEXT" => erase(cmd_set_depth_compare_op as vk::PFN_vkCmdSetDepthCompareOp),
        b"vkCmdClearAttachments" => erase(cmd_clear_attachments as vk::PFN_vkCmdClearAttachments),
        b"vkCmdClearDepthStencilImage" => erase(cmd_clear_depth_stencil_image as vk::PFN_vkCmdClearDepthStencilImage),
        b"vkCmdBlitImage" => erase(cmd_blit_image as vk::PFN_vkCmdBlitImage),
        b"vkCmdCopyImage" => erase(cmd_copy_image as vk::PFN_vkCmdCopyImage),
        b"vkCmdResolveImage" => erase(cmd_resolve_image as vk::PFN_vkCmdResolveImage),
        b"vkCmdClearColorImage" => erase(cmd_clear_color_image as vk::PFN_vkCmdClearColorImage),
        b"vkCmdBeginQuery" => erase(cmd_begin_query as vk::PFN_vkCmdBeginQuery),
        b"vkCmdEndQuery" => erase(cmd_end_query as vk::PFN_vkCmdEndQuery),
        b"vkCmdBeginQueryIndexedEXT" => erase(cmd_begin_query_indexed as vk::PFN_vkCmdBeginQueryIndexedEXT),
        b"vkCmdEndQueryIndexedEXT" => erase(cmd_end_query_indexed as vk::PFN_vkCmdEndQueryIndexedEXT),
        b"vkCmdBeginConditionalRenderingEXT" => erase(cmd_begin_conditional_rendering as vk::PFN_vkCmdBeginConditionalRenderingEXT),
        b"vkCmdEndConditionalRenderingEXT" => erase(cmd_end_conditional_rendering as vk::PFN_vkCmdEndConditionalRenderingEXT),
        b"vkCmdBeginTransformFeedbackEXT" => erase(cmd_begin_transform_feedback as vk::PFN_vkCmdBeginTransformFeedbackEXT),
        b"vkCmdEndTransformFeedbackEXT" => erase(cmd_end_transform_feedback as vk::PFN_vkCmdEndTransformFeedbackEXT),
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
        let c = &*p_create_info;
        let is_depth = c.image_type == vk::ImageType::TYPE_2D && is_depth_format(c.format) && c.usage.contains(vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT);
        // Armado: acrescenta `SAMPLED` (sem MSAA, e só se o formato aceita) para os efeitos amostrarem a imagem original.
        let mut patched = *c;
        let mut forward = p_create_info;
        if is_depth && data.depth.armed && (c.samples == vk::SampleCountFlags::TYPE_1 || msaa_sampleable(&data, c)) {
            // `SAMPLED` para os efeitos lerem a original; `TRANSFER_SRC` para a foto de um passe (cópia). Só o que o formato aceita.
            for (flag, feature) in [(vk::ImageUsageFlags::SAMPLED, vk::FormatFeatureFlags::SAMPLED_IMAGE), (vk::ImageUsageFlags::TRANSFER_SRC, vk::FormatFeatureFlags::TRANSFER_SRC)] {
                if !c.usage.contains(flag) && format_supports(&data, c.format, c.tiling, feature) {
                    patched.usage |= flag;
                    forward = &patched;
                }
            }
        }
        let result = (data.device.fp_v1_0().create_image)(device, forward, p_allocator, p_image);
        if result == vk::Result::SUCCESS && is_depth {
            let info = ImageInfo {
                width: c.extent.width,
                height: c.extent.height,
                format: c.format.as_raw(),
                samples: c.samples.as_raw(),
                sampled: (*forward).usage.contains(vk::ImageUsageFlags::SAMPLED),
            };
            data.depth.images.lock().unwrap().insert((*p_image).as_raw(), info);
        }
        result
    }
}

/// Uma imagem de profundidade MSAA que a GPU deixa amostrar (uso `SAMPLED` com essa contagem de amostras): então o resolve da
/// layer consegue ler as amostras.
unsafe fn msaa_sampleable(data: &DeviceData, c: &vk::ImageCreateInfo<'_>) -> bool {
    unsafe {
        c.tiling == vk::ImageTiling::OPTIMAL
            && data
                .instance
                .instance
                .get_physical_device_image_format_properties(data.physical_device, c.format, vk::ImageType::TYPE_2D, vk::ImageTiling::OPTIMAL, c.usage | vk::ImageUsageFlags::SAMPLED, c.flags)
                .is_ok_and(|p| p.sample_counts.contains(c.samples))
    }
}

/// O formato tem esta característica com tiling ótimo? (consulta única por formato)
unsafe fn format_supports(data: &DeviceData, format: vk::Format, tiling: vk::ImageTiling, feature: vk::FormatFeatureFlags) -> bool {
    if tiling != vk::ImageTiling::OPTIMAL {
        return false;
    }
    let mut cache = data.depth.formats.lock().unwrap();
    *cache.entry(format.as_raw()).or_insert_with(|| {
        unsafe { data.instance.instance.get_physical_device_format_properties(data.physical_device, format) }.optimal_tiling_features.as_raw() as u64
    }) & u64::from(feature.as_raw())
        != 0
}

unsafe extern "system" fn destroy_image(device: vk::Device, image: vk::Image, p_allocator: *const vk::AllocationCallbacks<'_>) {
    unsafe {
        let Some(data) = data_of(device.as_raw()) else { return };
        let tracked = data.depth.images.lock().unwrap().remove(&image.as_raw()).is_some();
        if tracked {
            super::resolve::forget_image(&data, image.as_raw());
            data.depth.image_layouts.lock().unwrap().remove(&image.as_raw());
            let mut chosen = data.depth.chosen.lock().unwrap();
            if chosen.is_some_and(|c| c.image == image.as_raw()) {
                *chosen = None;
            }
            drop(chosen);
            // Os efeitos construídos com esta imagem têm uma view dela: precisam sair antes de ela ser destruída.
            crate::effect::on_depth_image_destroyed(&data, image.as_raw());
        }
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
            if data.depth.is_on() && crate::effect::is_swapchain_image(&data, image) {
                data.depth.screen.add_view((*p_view).as_raw());
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
                data.depth.screen.add_framebuffer((*p_framebuffer).as_raw(), &views);
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
        // Teste: guardar a profundidade que o render pass descartaria (cópia dos anexos com o `storeOp` trocado).
        let mut attachments_copy: Vec<vk::AttachmentDescription> = Vec::new();
        let mut patched = *p_create_info;
        if data.depth.force_store && !(*p_create_info).p_attachments.is_null() {
            attachments_copy = std::slice::from_raw_parts((*p_create_info).p_attachments, (*p_create_info).attachment_count as usize).to_vec();
            for a in attachments_copy.iter_mut().filter(|a| is_depth_format(a.format) && a.store_op == vk::AttachmentStoreOp::DONT_CARE) {
                a.store_op = vk::AttachmentStoreOp::STORE;
            }
            patched.p_attachments = attachments_copy.as_ptr();
        }
        let forward: *const vk::RenderPassCreateInfo<'_> = if attachments_copy.is_empty() { p_create_info } else { &patched };
        let result = (data.device.fp_v1_0().create_render_pass)(device, forward, p_allocator, p_render_pass);
        if result == vk::Result::SUCCESS {
            let c = &*forward;
            if !c.p_attachments.is_null() {
                let loads = std::slice::from_raw_parts(c.p_attachments, c.attachment_count as usize).iter().map(|a| load_op(a.load_op)).collect();
                data.depth.screen.add_render_pass((*p_render_pass).as_raw(), loads);
            }
            if !c.p_subpasses.is_null() && !c.p_attachments.is_null() {
                let subpasses = std::slice::from_raw_parts(c.p_subpasses, c.subpass_count as usize);
                let attachments = std::slice::from_raw_parts(c.p_attachments, c.attachment_count as usize);
                let found = subpasses.iter().filter(|s| !s.p_depth_stencil_attachment.is_null()).map(|s| (*s.p_depth_stencil_attachment).attachment).find(|a| *a != vk::ATTACHMENT_UNUSED);
                if let Some(index) = found.filter(|i| (*i as usize) < attachments.len()) {
                    let a = &attachments[index as usize];
                    if is_depth_format(a.format) {
                        data.depth.render_passes.lock().unwrap().insert(
                            (*p_render_pass).as_raw(),
                            RenderPassInfo { depth_attachment: index, load: load_op(a.load_op), store: store_op(a.store_op), final_layout: a.final_layout.as_raw() },
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
        let mut attachments_copy: Vec<vk::AttachmentDescription2> = Vec::new();
        let mut patched = *p_create_info;
        if data.depth.force_store && !(*p_create_info).p_attachments.is_null() {
            attachments_copy = std::slice::from_raw_parts((*p_create_info).p_attachments, (*p_create_info).attachment_count as usize).to_vec();
            for a in attachments_copy.iter_mut().filter(|a| is_depth_format(a.format) && a.store_op == vk::AttachmentStoreOp::DONT_CARE) {
                a.store_op = vk::AttachmentStoreOp::STORE;
            }
            patched.p_attachments = attachments_copy.as_ptr();
        }
        let forward: *const vk::RenderPassCreateInfo2<'_> = if attachments_copy.is_empty() { p_create_info } else { &patched };
        let result = next(device, forward, p_allocator, p_render_pass);
        if result == vk::Result::SUCCESS {
            let c = &*forward;
            if !c.p_attachments.is_null() {
                let loads = std::slice::from_raw_parts(c.p_attachments, c.attachment_count as usize).iter().map(|a| load_op(a.load_op)).collect();
                data.depth.screen.add_render_pass((*p_render_pass).as_raw(), loads);
            }
            if !c.p_subpasses.is_null() && !c.p_attachments.is_null() {
                let subpasses = std::slice::from_raw_parts(c.p_subpasses, c.subpass_count as usize);
                let attachments = std::slice::from_raw_parts(c.p_attachments, c.attachment_count as usize);
                let found = subpasses.iter().filter(|s| !s.p_depth_stencil_attachment.is_null()).map(|s| (*s.p_depth_stencil_attachment).attachment).find(|a| *a != vk::ATTACHMENT_UNUSED);
                if let Some(index) = found.filter(|i| (*i as usize) < attachments.len()) {
                    let a = &attachments[index as usize];
                    if is_depth_format(a.format) {
                        data.depth.render_passes.lock().unwrap().insert(
                            (*p_render_pass).as_raw(),
                            RenderPassInfo { depth_attachment: index, load: load_op(a.load_op), store: store_op(a.store_op), final_layout: a.final_layout.as_raw() },
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
            let mut map = data.depth.pipelines.write().unwrap();
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
            data.depth.cmds.lock(cmd.as_raw()).insert(cmd.as_raw(), CmdState::default());
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
        end_layout: rp.final_layout,
        ..PassRecord::default()
    };
    if let Some(state) = data.depth.cmds.lock(cmd.as_raw()).get_mut(&cmd.as_raw()) {
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
        if let Some(pass) = data.depth.screen.begin_pass(b.render_pass.as_raw(), b.framebuffer.as_raw(), views.clone())
            && let Some(state) = data.depth.cmds.lock(cmd.as_raw()).get_mut(&cmd.as_raw())
        {
            state.screen_current = Some(pass);
        }
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
fn finish_pass(data: &DeviceData, cmd: vk::CommandBuffer) -> Option<PassRecord> {
    if !data.depth.is_on() {
        return None;
    }
    let mut cmds = data.depth.cmds.lock(cmd.as_raw());
    let state = cmds.get_mut(&cmd.as_raw())?;
    if let Some(screen) = state.screen_current.take() {
        state.screen_passes.push(screen);
    }
    state.rendering = None;
    let pass = state.current.take()?;
    if pass.end_layout != 0 {
        state.layouts.push((pass.image, pass.end_layout));
    }
    state.passes.push(pass);
    Some(pass)
}

/// Depois do fim de um passe da profundidade em observação: se é o passe pedido, grava a foto da profundidade aqui.
unsafe fn after_pass(data: &DeviceData, cmd: vk::CommandBuffer, pass: Option<PassRecord>) {
    let Some(pass) = pass else { return };
    let wanted = data.depth.snapshot_pass.load(Ordering::Relaxed);
    if wanted == 0 || pass.image != data.depth.watched_image.load(Ordering::Relaxed) || pass.end_layout == 0 {
        return;
    }
    // O passe de número `wanted` (de 1) da imagem neste quadro, na ordem de gravação; `ALL_PASSES`: depois de todos (o resultado é o
    // do último), para uma imagem MSAA, cujo resolve não pode esperar saber qual é o último passe.
    let counted = data.depth.pass_counter.fetch_add(1, Ordering::Relaxed) + 1;
    if wanted != ALL_PASSES && counted != wanted {
        return;
    }
    unsafe { snapshot_now(data, cmd, pass.image, vk::ImageLayout::from_raw(pass.end_layout)) };
}

unsafe extern "system" fn cmd_end_render_pass(cmd: vk::CommandBuffer) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        let finished = finish_pass(&data, cmd);
        (data.device.fp_v1_0().cmd_end_render_pass)(cmd);
        after_pass(&data, cmd, finished);
    }
}

unsafe extern "system" fn cmd_end_render_pass2(cmd: vk::CommandBuffer, p_end: *const vk::SubpassEndInfo<'_>) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        let Some(next) = data.depth.next.cmd_end_render_pass2 else { return };
        let finished = finish_pass(&data, cmd);
        next(cmd, p_end);
        after_pass(&data, cmd, finished);
    }
}

// ---- limpezas da profundidade ("copiar antes de limpar", como o ReShade) ----

/// Registra uma limpeza (ou recomeço) da imagem observada. Devolve se é a que a pessoa (ou a regra automática) quer copiar.
fn register_clear_event(data: &DeviceData, image: u64, splittable: bool) -> bool {
    if image == 0 || image != data.depth.watched_image.load(Ordering::Relaxed) {
        return false;
    }
    let ordinal = data.depth.clear_counter.fetch_add(1, Ordering::Relaxed) + 1;
    let draws_before = data.depth.draws_since_event.swap(0, Ordering::Relaxed);
    data.depth.frame_clears.lock().unwrap().push(super::ClearEvent { draws_before, splittable });
    ordinal == data.depth.snapshot_clear.load(Ordering::Relaxed)
}

/// Grava a cópia da profundidade de `image` (no layout `layout`) para a foto da layer, fora de um passe de renderização.
unsafe fn snapshot_now(data: &DeviceData, cmd: vk::CommandBuffer, image: u64, layout: vk::ImageLayout) {
    let Some(info) = data.depth.images.lock().unwrap().get(&image).copied() else { return };
    let Some((dst, format)) = data.depth.snapshot.lock().unwrap().as_ref().map(|s| (s.image, s.format)) else { return };
    if layout == vk::ImageLayout::UNDEFINED {
        return;
    }
    if info.samples > 1 {
        // MSAA: o resolve desenha a amostra 0 de cada pixel na foto de uma amostra.
        if format != super::resolve::RESOLVED_FORMAT || !unsafe { super::resolve::record(data, cmd, vk::Image::from_raw(image), layout, vk::Format::from_raw(info.format), info.width, info.height) } {
            return;
        }
    } else {
        if format.as_raw() != info.format {
            return;
        }
        unsafe { super::snapshot::record_copy(data, cmd, vk::Image::from_raw(image), layout, dst, format, info.width, info.height) };
    }
    data.depth.copied_frame.store(true, Ordering::Relaxed);
    if !data.depth.snapshot_written.swap(true, Ordering::Relaxed) {
        let (clear, pass) = (data.depth.snapshot_clear.load(Ordering::Relaxed), data.depth.snapshot_pass.load(Ordering::Relaxed));
        if clear > 0 {
            log!("foto da profundidade gravada antes da limpeza {clear} (primeira vez)");
        } else {
            log!("foto da profundidade gravada (primeira vez; passe {})", if pass == ALL_PASSES { "todos".to_string() } else { pass.to_string() });
        }
    }
}

/// Guarda uma cópia do `VkRenderingInfo` se o passe é sobre a imagem observada (para partir o passe numa limpeza).
unsafe fn remember_rendering(data: &DeviceData, cmd: vk::CommandBuffer, info: &vk::RenderingInfo<'_>, image: u64) {
    unsafe {
        let own = |a: &vk::RenderingAttachmentInfo<'_>| {
            let mut c = *a;
            c.p_next = std::ptr::null();
            // SAFETY: sem `p_next`, a struct não referencia nada além dela mesma.
            std::mem::transmute::<vk::RenderingAttachmentInfo<'_>, vk::RenderingAttachmentInfo<'static>>(c)
        };
        let colors = if info.p_color_attachments.is_null() { Vec::new() } else { std::slice::from_raw_parts(info.p_color_attachments, info.color_attachment_count as usize).iter().map(own).collect() };
        let depth = (!info.p_depth_attachment.is_null()).then(|| own(&*info.p_depth_attachment));
        let stencil = (!info.p_stencil_attachment.is_null()).then(|| own(&*info.p_stencil_attachment));
        // Só dá para partir um passe simples: sem estruturas encadeadas e sem suspender/retomar.
        let simple = info.p_next.is_null() && !info.flags.intersects(vk::RenderingFlags::SUSPENDING | vk::RenderingFlags::RESUMING);
        let copy = simple.then(|| super::RenderingCopy { flags: info.flags, render_area: info.render_area, layer_count: info.layer_count, view_mask: info.view_mask, colors, depth, stencil, image });
        if let Some(state) = data.depth.cmds.lock(cmd.as_raw()).get_mut(&cmd.as_raw()) {
            state.rendering = copy;
        }
    }
}

/// Parte o passe de renderização dinâmica em gravação: fecha, copia a profundidade, reabre o mesmo passe sem apagar nada.
unsafe fn split_rendering(data: &DeviceData, cmd: vk::CommandBuffer, copy: &super::RenderingCopy) -> bool {
    unsafe {
        let (Some(end), Some(begin)) = (data.depth.next.cmd_end_rendering, data.depth.next.cmd_begin_rendering) else { return false };
        let Some(depth) = copy.depth else { return false };
        end(cmd);
        snapshot_now(data, cmd, copy.image, depth.image_layout);
        // Reabre com `LOAD`: o conteúdo (cor e profundidade) segue como estava.
        let keep = |a: &vk::RenderingAttachmentInfo<'static>| {
            let mut c = *a;
            c.load_op = vk::AttachmentLoadOp::LOAD;
            c
        };
        let colors: Vec<vk::RenderingAttachmentInfo<'static>> = copy.colors.iter().map(keep).collect();
        let depth = keep(&depth);
        let stencil = copy.stencil.as_ref().map(keep);
        let mut info = vk::RenderingInfo::default().flags(copy.flags).render_area(copy.render_area).layer_count(copy.layer_count).view_mask(copy.view_mask).color_attachments(&colors).depth_attachment(&depth);
        if let Some(stencil) = &stencil {
            info = info.stencil_attachment(stencil);
        }
        begin(cmd, &info);
        true
    }
}

unsafe extern "system" fn cmd_clear_attachments(cmd: vk::CommandBuffer, count: u32, p_attachments: *const vk::ClearAttachment, rect_count: u32, p_rects: *const vk::ClearRect) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        if data.depth.is_on() && !p_attachments.is_null() && std::slice::from_raw_parts(p_attachments, count as usize).iter().any(|a| a.aspect_mask.intersects(vk::ImageAspectFlags::DEPTH | vk::ImageAspectFlags::STENCIL)) {
            data.depth.frame_clear_attachments.fetch_add(1, Ordering::Relaxed);
            // A limpeza é dentro do passe em gravação: se é sobre a imagem observada, é uma limpeza dela.
            let (image, copy, scopes) = match data.depth.cmds.lock(cmd.as_raw()).get(&cmd.as_raw()) {
                Some(state) => (state.current.map_or(0, |p| p.image), state.rendering.clone(), state.open_scopes),
                None => (0, None, 0),
            };
            // Partir o passe só se pedido: o ReShade não copia dentro de um passe ativo (é inválido, e consultas ou renderização condicional
            // abertas no passe não sobrevivem ao fim dele). O "antes da limpeza N" explícito da pessoa vale como pedido.
            let wanted = copy.is_some() && (data.depth.split_on() || data.depth.pick.lock().unwrap().clear.is_some());
            // Só se parte com o passe válido de partir: nenhuma consulta, renderização condicional ou transform feedback aberta nele.
            let allow = wanted && scopes == 0;
            if wanted && !allow && image != 0 {
                if data.depth.splits_skipped.fetch_add(1, Ordering::Relaxed) == 0 {
                    log!("passe não partido: há consulta, renderização condicional ou transform feedback aberto nele ({scopes}); a cópia da hora dos efeitos cobre");
                }
            }
            if image != 0 && register_clear_event(&data, image, allow) && allow && let Some(copy) = copy {
                data.depth.splits_done.fetch_add(1, Ordering::Relaxed);
                split_rendering(&data, cmd, &copy);
            }
        }
        (data.device.fp_v1_0().cmd_clear_attachments)(cmd, count, p_attachments, rect_count, p_rects);
    }
}

unsafe extern "system" fn cmd_clear_depth_stencil_image(
    cmd: vk::CommandBuffer,
    image: vk::Image,
    layout: vk::ImageLayout,
    p_value: *const vk::ClearDepthStencilValue,
    count: u32,
    p_ranges: *const vk::ImageSubresourceRange,
) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        if data.depth.is_on() && data.depth.images.lock().unwrap().contains_key(&image.as_raw()) {
            data.depth.frame_clear_images.fetch_add(1, Ordering::Relaxed);
            // Fora de um passe: dá para copiar logo antes.
            if register_clear_event(&data, image.as_raw(), true) {
                snapshot_now(&data, cmd, image.as_raw(), layout);
            }
        }
        (data.device.fp_v1_0().cmd_clear_depth_stencil_image)(cmd, image, layout, p_value, count, p_ranges);
    }
}

/// Conta um escopo que precisa terminar na mesma instância de passe (consulta, renderização condicional, transform feedback).
fn scope_changed(data: &DeviceData, cmd: vk::CommandBuffer, opened: bool) {
    if !data.depth.is_on() {
        return;
    }
    if let Some(state) = data.depth.cmds.lock(cmd.as_raw()).get_mut(&cmd.as_raw()) {
        state.open_scopes = if opened { state.open_scopes + 1 } else { state.open_scopes.saturating_sub(1) };
    }
}

unsafe extern "system" fn cmd_begin_query(cmd: vk::CommandBuffer, pool: vk::QueryPool, query: u32, flags: vk::QueryControlFlags) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        scope_changed(&data, cmd, true);
        (data.device.fp_v1_0().cmd_begin_query)(cmd, pool, query, flags);
    }
}

unsafe extern "system" fn cmd_end_query(cmd: vk::CommandBuffer, pool: vk::QueryPool, query: u32) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        scope_changed(&data, cmd, false);
        (data.device.fp_v1_0().cmd_end_query)(cmd, pool, query);
    }
}

unsafe extern "system" fn cmd_begin_query_indexed(cmd: vk::CommandBuffer, pool: vk::QueryPool, query: u32, flags: vk::QueryControlFlags, index: u32) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        let Some(next) = data.depth.next.cmd_begin_query_indexed else { return };
        scope_changed(&data, cmd, true);
        next(cmd, pool, query, flags, index);
    }
}

unsafe extern "system" fn cmd_end_query_indexed(cmd: vk::CommandBuffer, pool: vk::QueryPool, query: u32, index: u32) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        let Some(next) = data.depth.next.cmd_end_query_indexed else { return };
        scope_changed(&data, cmd, false);
        next(cmd, pool, query, index);
    }
}

unsafe extern "system" fn cmd_begin_conditional_rendering(cmd: vk::CommandBuffer, p_begin: *const vk::ConditionalRenderingBeginInfoEXT<'_>) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        let Some(next) = data.depth.next.cmd_begin_conditional_rendering else { return };
        scope_changed(&data, cmd, true);
        next(cmd, p_begin);
    }
}

unsafe extern "system" fn cmd_end_conditional_rendering(cmd: vk::CommandBuffer) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        let Some(next) = data.depth.next.cmd_end_conditional_rendering else { return };
        scope_changed(&data, cmd, false);
        next(cmd);
    }
}

unsafe extern "system" fn cmd_begin_transform_feedback(cmd: vk::CommandBuffer, first: u32, count: u32, p_buffers: *const vk::Buffer, p_offsets: *const vk::DeviceSize) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        let Some(next) = data.depth.next.cmd_begin_transform_feedback else { return };
        scope_changed(&data, cmd, true);
        next(cmd, first, count, p_buffers, p_offsets);
    }
}

unsafe extern "system" fn cmd_end_transform_feedback(cmd: vk::CommandBuffer, first: u32, count: u32, p_buffers: *const vk::Buffer, p_offsets: *const vk::DeviceSize) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        let Some(next) = data.depth.next.cmd_end_transform_feedback else { return };
        scope_changed(&data, cmd, false);
        next(cmd, first, count, p_buffers, p_offsets);
    }
}

unsafe extern "system" fn cmd_begin_rendering(cmd: vk::CommandBuffer, p_info: *const vk::RenderingInfo<'_>) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        let Some(next) = data.depth.next.cmd_begin_rendering else { return };
        // O que o passe novo vai passar adiante (corrigido, se for o caso: ver `fixed` abaixo).
        let mut fixed: Option<(vk::RenderingAttachmentInfo<'_>, Option<vk::RenderingAttachmentInfo<'_>>)> = None;
        let mut fixed_info;
        let mut p_info = p_info;
        if data.depth.is_on() && !p_info.is_null() {
            let info = &*p_info;
            if let Some(state) = data.depth.cmds.lock(cmd.as_raw()).get_mut(&cmd.as_raw()) {
                state.open_scopes = 0;
            }
            if !info.p_color_attachments.is_null() {
                let colors = std::slice::from_raw_parts(info.p_color_attachments, info.color_attachment_count as usize);
                if let Some(a) = colors.iter().find(|a| data.depth.screen.is_view(a.image_view.as_raw()))
                    && let Some(state) = data.depth.cmds.lock(cmd.as_raw()).get_mut(&cmd.as_raw())
                {
                    state.screen_current = Some(super::screen::ScreenPass { kind: super::screen::Kind::Pass, load: load_op(a.load_op), draws: 0 });
                }
            }
            if !info.p_depth_attachment.is_null() {
                let a = &*info.p_depth_attachment;
                let image = data.depth.views.lock().unwrap().get(&a.image_view.as_raw()).copied();
                if let Some(image) = image {
                    // Partir este passe numa limpeza (ver `split_rendering`) fecha a primeira metade com o `storeOp` do jogo: com
                    // `DONT_CARE` a profundidade (e o estêncil) ficaria indefinida e a segunda metade a leria com `LOAD`. Na imagem observada,
                    // o passe passa a guardar (o que a maioria dos drivers já faz de qualquer forma).
                    if data.depth.split_on() && image == data.depth.watched_image.load(Ordering::Relaxed) {
                        let keeps = |op: vk::AttachmentStoreOp| !matches!(op, vk::AttachmentStoreOp::DONT_CARE | vk::AttachmentStoreOp::NONE);
                        let stencil_a = (!info.p_stencil_attachment.is_null()).then(|| *info.p_stencil_attachment);
                        if !keeps(a.store_op) || stencil_a.is_some_and(|s| !keeps(s.store_op)) {
                            let mut d = *a;
                            d.store_op = vk::AttachmentStoreOp::STORE;
                            let s = stencil_a.map(|mut s| {
                                s.store_op = vk::AttachmentStoreOp::STORE;
                                s
                            });
                            fixed = Some((d, s));
                        }
                    }
                    if let Some((d, s)) = &fixed {
                        fixed_info = *info;
                        fixed_info.p_depth_attachment = d;
                        if let Some(s) = s {
                            fixed_info.p_stencil_attachment = s;
                        }
                        p_info = &fixed_info;
                    }
                    let info = &*p_info;
                    let a = &*info.p_depth_attachment;
                    let load = load_op(a.load_op);
                    let record = PassRecord {
                        image,
                        load,
                        store: store_op(a.store_op),
                        clear_depth: (load == LoadOp::Clear).then(|| a.clear_value.depth_stencil.depth),
                        area: (info.render_area.extent.width, info.render_area.extent.height),
                        end_layout: a.image_layout.as_raw(),
                        ..PassRecord::default()
                    };
                    if let Some(state) = data.depth.cmds.lock(cmd.as_raw()).get_mut(&cmd.as_raw()) {
                        state.current = Some(record);
                    }
                    // Um passe que recomeça a profundidade (`Clear` ou `DontCare`) é uma limpeza dela; copia antes, fora do passe.
                    if matches!(load, LoadOp::Clear | LoadOp::DontCare) && register_clear_event(&data, image, true) {
                        snapshot_now(&data, cmd, image, a.image_layout);
                    }
                    remember_rendering(&data, cmd, info, image);
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
        let finished = finish_pass(&data, cmd);
        next(cmd);
        after_pass(&data, cmd, finished);
    }
}

/// Soma um desenho ao passe em gravação (`vertices` zero para os indiretos).
fn on_draw(data: &DeviceData, cmd: vk::CommandBuffer, vertices: u64, indirect: bool) {
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

unsafe extern "system" fn cmd_draw(cmd: vk::CommandBuffer, vertex_count: u32, instance_count: u32, first_vertex: u32, first_instance: u32) {
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

unsafe extern "system" fn cmd_draw_indexed(cmd: vk::CommandBuffer, index_count: u32, instance_count: u32, first_index: u32, vertex_offset: i32, first_instance: u32) {
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
            let compare = data.depth.pipelines.read().unwrap().get(&pipeline.as_raw()).copied().unwrap_or(Compare::Unknown);
            if let Some(state) = data.depth.cmds.lock(cmd.as_raw()).get_mut(&cmd.as_raw()) {
                state.compare = compare;
            }
        }
        (data.device.fp_v1_0().cmd_bind_pipeline)(cmd, bind_point, pipeline);
    }
}

/// Anota as barreiras que mudam o layout de uma imagem de profundidade.
/// Anota (no command buffer) um evento sobre uma imagem do swapchain.
fn screen_event(data: &DeviceData, cmd: vk::CommandBuffer, image: vk::Image, kind: super::screen::Kind) {
    if !data.depth.is_on() || !data.depth.screen.is_image(image.as_raw(), || crate::effect::is_swapchain_image(data, image.as_raw())) {
        return;
    }
    if let Some(state) = data.depth.cmds.lock(cmd.as_raw()).get_mut(&cmd.as_raw()) {
        state.screen_passes.push(super::screen::ScreenPass::event(kind));
    }
}

fn note_layout(data: &DeviceData, cmd: vk::CommandBuffer, image: vk::Image, new_layout: vk::ImageLayout) {
    screen_event(data, cmd, image, super::screen::Kind::Barrier(new_layout.as_raw()));
    if new_layout == vk::ImageLayout::UNDEFINED || !data.depth.images.lock().unwrap().contains_key(&image.as_raw()) {
        return;
    }
    if let Some(state) = data.depth.cmds.lock(cmd.as_raw()).get_mut(&cmd.as_raw()) {
        state.layouts.push((image.as_raw(), new_layout.as_raw()));
    }
}

unsafe extern "system" fn cmd_pipeline_barrier(
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

unsafe extern "system" fn cmd_pipeline_barrier2(cmd: vk::CommandBuffer, p_info: *const vk::DependencyInfo<'_>) {
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

unsafe extern "system" fn cmd_set_depth_compare_op(cmd: vk::CommandBuffer, op: vk::CompareOp) {
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

unsafe extern "system" fn cmd_execute_commands(cmd: vk::CommandBuffer, count: u32, p_buffers: *const vk::CommandBuffer) {
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

// ---- envio à fila e fim do quadro ----

/// Junta ao quadro o que os command buffers enviados gravaram.
fn merge_submitted(data: &DeviceData, buffers: &[vk::CommandBuffer]) {
    if !data.depth.is_on() || buffers.is_empty() {
        return;
    }
    let (passes, layouts): (Vec<PassRecord>, Vec<(u64, i32)>) = {
        let mut passes = Vec::new();
        let mut layouts = Vec::new();
        for b in buffers {
            let cmds = data.depth.cmds.lock(b.as_raw());
            if let Some(s) = cmds.get(&b.as_raw()) {
                passes.extend(s.passes.iter().copied());
                layouts.extend(s.layouts.iter().copied());
                data.depth.screen.push_frame(s.screen_passes.iter().copied());
            }
        }
        (passes, layouts)
    };
    // O layout de cada imagem segue a ordem dos envios (a ordem em que a GPU os executa).
    if !layouts.is_empty() {
        let mut map = data.depth.image_layouts.lock().unwrap();
        for (image, layout) in layouts {
            map.insert(image, layout);
        }
    }
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
        crate::queue_lock::with(queue.as_raw(), || (data.device.fp_v1_0().queue_submit)(queue, count, p_submits, fence))
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
        crate::queue_lock::with(queue.as_raw(), || next(queue, count, p_submits, fence))
    }
}

/// Lê o pedido da pessoa (`lrctl depth ...`) de tempos em tempos: `/dev/shm/.../depth-pick-<id da layer>`, com linhas
/// `candidate=N` e/ou `pass=N`.
fn poll_pick(data: &DeviceData) {
    if data.depth.frames.fetch_add(1, Ordering::Relaxed) % 20 != 0 {
        return;
    }
    let Some(id) = crate::status::id() else { return };
    let text = std::fs::read_to_string(ipc::state_dir().join(format!("depth-pick-{id}"))).unwrap_or_default();
    let number = |key: &str| text.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('=')?.trim().parse::<u32>().ok()).filter(|n| *n > 0);
    let pick = super::Pick { candidate: number("candidate"), pass: number("pass"), clear: number("clear") };
    // Os ajustes de política (`lrctl depth policy`) valem na hora.
    let policy_text = std::fs::read_to_string(ipc::state_dir().join(format!("depth-policy-{id}"))).unwrap_or_default();
    let flag = |key: &str| policy_text.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('=').map(|v| v.trim() == "on").filter(|_| matches!(l.split_once('=').map(|(_, v)| v.trim()), Some("on" | "off"))));
    let policy = super::Policy { split: flag("split"), copy: flag("copy"), snapshot: flag("snapshot"), resignal: flag("resignal") };
    {
        let mut current = data.depth.policy.lock().unwrap();
        if *current != policy {
            log!("política da profundidade: {:?}", policy);
            *current = policy;
        }
    }
    let mut current = data.depth.pick.lock().unwrap();
    if *current != pick {
        log!("pedido para a profundidade: candidata {:?}, passe {:?}, limpeza {:?}", pick.candidate, pick.pass, pick.clear);
        *current = pick;
    }
}

/// `snapshot_pass` com este valor: a foto sai depois de cada passe da imagem (a do último fica).
const ALL_PASSES: u32 = u32::MAX;

/// Quantos quadros seguidos o desafiante precisa vencer para a candidata de profundidade em uso mudar.
const STICKY_SWITCH_FRAMES: u32 = 120;

/// Quantos quadros seguidos a limpeza da cena precisa mudar para a escolha automática mudar.
const AUTO_CLEAR_SWITCH_FRAMES: u32 = 8;

/// Quantos quadros seguidos sem um passe que recomeça a imagem fazem a regra automática largar a foto.
const AUTO_SCENE_FORGET_FRAMES: u32 = 600;

/// Fim de um quadro (chamado no present): escolhe a candidata e publica a lista para o app de controle.
pub fn end_frame(data: &DeviceData, frame: (u32, u32)) {
    if !data.depth.is_on() {
        return;
    }
    unsafe { super::snapshot::reap_retired(data) };
    data.depth.pass_counter.store(0, Ordering::Relaxed);
    data.depth.clear_counter.store(0, Ordering::Relaxed);
    data.depth.draws_since_event.store(0, Ordering::Relaxed);
    let clear_events = std::mem::take(&mut *data.depth.frame_clears.lock().unwrap());
    let clear_count = clear_events.len();
    poll_pick(data);
    // O present já usou a marca deste quadro (`copy_for_present`, antes de `end_frame`): começa a do próximo.
    data.depth.copied_frame.store(false, Ordering::Relaxed);
    let (frame_draws, frame_depth_draws) = (data.depth.frame_draws.swap(0, Ordering::Relaxed), data.depth.frame_depth_draws.swap(0, Ordering::Relaxed));
    let (frame_clear_attachments, frame_clear_images) = (data.depth.frame_clear_attachments.swap(0, Ordering::Relaxed), data.depth.frame_clear_images.swap(0, Ordering::Relaxed));
    let stats = std::mem::take(&mut *data.depth.frame.lock().unwrap());
    let screen_summary = data.depth.screen.take_summary();
    if stats.is_empty() {
        return;
    }
    let layouts = data.depth.image_layouts.lock().unwrap().clone();
    let mut candidates = stats.candidates(frame, &layouts);
    // O pedido da pessoa manda sobre a heurística. O número da candidata vira a IMAGEM daquele momento e ela fica: a ordem da lista
    // muda entre quadros e cenas, e o número sozinho alternava entre imagens diferentes. Se a imagem some, volta à heurística.
    let pick = *data.depth.pick.lock().unwrap();
    let mut picked_by_number = false;
    {
        let mut picked = data.depth.picked_image.lock().unwrap();
        match pick.candidate {
            Some(n) => {
                if picked.0 != n {
                    let c = candidates.get(n as usize - 1);
                    *picked = (n, c.map_or(0, |c| c.image), c.map_or((0, 0, 0), |c| (c.width, c.height, c.format)));
                }
                // O jogo recria imagens (mapas de sombra em cascata, trocas de qualidade): se a escolhida sumiu, a que tem o
                // mesmo tamanho e formato (a de mais desenhos) herda o pedido. Sem isso a escolha ia e voltava a cada recriação.
                if picked.1 == 0 || !candidates.iter().any(|c| c.image == picked.1) {
                    let sig = picked.2;
                    picked.1 = candidates.iter().filter(|c| (c.width, c.height, c.format) == sig && sig.0 != 0).max_by_key(|c| c.draws).map_or(0, |c| c.image);
                }
                // Só o que não dá para amostrar de jeito nenhum (MSAA sem `SAMPLED`) é recusado; as outras ("proporção diferente", "sem
                // desenhos") a pessoa pode escolher: a heurística não as escolheria, e o resultado pode não ser o esperado (o painel avisa
                // em amarelo). O pedido recusado fica guardado e a heurística decide.
                if picked.1 != 0 && candidates.iter().any(|c| c.image == picked.1 && c.unusable.is_some_and(super::observe::is_blocking)) {
                    if data.depth.refused_pick.swap(picked.1, Ordering::Relaxed) != picked.1 {
                        log!("pedido ignorado: a candidata escolhida não serve para os efeitos");
                    }
                } else if picked.1 != 0 {
                    let image = picked.1;
                    for c in candidates.iter_mut() {
                        c.chosen = c.image == image;
                    }
                    picked_by_number = true;
                }
            }
            None => *picked = (0, 0, (0, 0, 0)),
        }
    }
    // Histerese: a imagem em uso só perde o lugar depois de o desafiante vencer vários quadros seguidos (não vale para um pedido).
    if !picked_by_number {
        let winner = candidates.iter().find(|c| c.chosen).map_or(0, |c| c.image);
        let mut sticky = data.depth.sticky.lock().unwrap();
        let keeps = sticky.0 != 0 && sticky.0 != winner && candidates.iter().any(|c| c.image == sticky.0 && c.unusable.is_none());
        if keeps {
            if sticky.1 == winner {
                sticky.2 += 1;
            } else {
                sticky.1 = winner;
                sticky.2 = 1;
            }
            if sticky.2 >= STICKY_SWITCH_FRAMES {
                *sticky = (winner, 0, 0);
            } else {
                let current = sticky.0;
                for c in candidates.iter_mut() {
                    c.chosen = c.image == current;
                }
            }
        } else {
            *sticky = (winner, 0, 0);
        }
    }
    let active = candidates.iter().find(|c| c.chosen).cloned();
    let now_image = active.as_ref().map_or(0, |c| c.image);
    data.depth.watched_image.store(now_image, Ordering::Relaxed);
    if data.depth.last_image.swap(now_image, Ordering::Relaxed) != now_image {
        data.depth.image_changes.fetch_add(1, Ordering::Relaxed);
    }

    // O que os efeitos recebem: a original (amostrável, com layout conhecido) ou, se pediram um passe, a foto dele.
    let linear = |format: i32| {
        unsafe { data.instance.instance.get_physical_device_format_properties(data.physical_device, vk::Format::from_raw(format)) }
            .optimal_tiling_features
            .contains(vk::FormatFeatureFlags::SAMPLED_IMAGE_FILTER_LINEAR)
    };
    let mut chosen = None;
    data.depth.snapshot_pass.store(0, Ordering::Relaxed);
    data.depth.snapshot_clear.store(0, Ordering::Relaxed);
    if let Some(c) = &active {
        // A regra automática da limpeza (como o "copiar antes de limpar" do ReShade): entre as limpezas da profundidade do
        // quadro, a primeira que apaga uma parte relevante deles é a da cena (a de mais desenhos vem depois, e o índice dela varia). O número da limpeza é estável de um quadro para o outro (o de
        // passes não é: o DXVK os divide como quer), e a escolha só muda depois de vários quadros discordando.
        let auto_clear = || {
            let mut state = data.depth.auto_clear.lock().unwrap();
            if state.0 != c.image {
                *state = (c.image, 0, 0, 0);
            }
            let best = super::observe::scene_clear(&clear_events.iter().map(|e| (e.draws_before, e.splittable)).collect::<Vec<_>>()).unwrap_or(0);
            if state.1 == 0 {
                // Ainda sem escolha: só adota uma limpeza que aparece (a mesma) em vários quadros seguidos. Uma que aparece só de vez em
                // quando (a câmera, uma tela) faria a profundidade piscar entre a foto e a imagem ao vivo.
                if best != 0 && state.2 == best {
                    state.3 += 1;
                } else if best != 0 {
                    state.2 = best;
                    state.3 = 1;
                } else {
                    state.2 = 0;
                    state.3 = 0;
                }
                if state.3 >= AUTO_CLEAR_SWITCH_FRAMES {
                    crate::log::log!("profundidade: limpeza automática = {best} (de {clear_count}; desenhos antes de cada: {:?})", clear_events.iter().map(|e| e.draws_before).collect::<Vec<_>>());
                    *state = (c.image, best, best, 0);
                }
            } else if best == 0 {
                // Sem limpezas neste quadro: só esquece a escolha depois de muito tempo.
                state.3 += 1;
                if state.3 >= AUTO_SCENE_FORGET_FRAMES {
                    *state = (c.image, 0, 0, 0);
                }
            } else if best == state.1 {
                state.2 = best;
                state.3 = 0;
            } else if best == state.2 {
                state.3 += 1;
                if state.3 >= AUTO_CLEAR_SWITCH_FRAMES {
                    crate::log::log!("profundidade: limpeza automática mudou de {} para {best} (desenhos antes de cada: {:?})", state.1, clear_events.iter().map(|e| e.draws_before).collect::<Vec<_>>());
                    *state = (c.image, best, best, 0);
                }
            } else {
                state.2 = best;
                state.3 = 1;
            }
            // O quadro pode ter menos limpezas que o número escolhido (o Rocket League alterna entre 2 e 3, por exemplo): o pedido vale pelo que o
            // último quadro teve (a última limpeza que existe), e a foto continua sendo gravada todo quadro. Antes a regra não copiava nesses quadros,
            // a profundidade ficava congelada na foto antiga por 30 quadros e depois a escolha era largada e refeita (a profundidade ao vivo,
            // errada, aparecia nos 8 quadros seguintes).
            (state.1 > 0).then_some(if clear_count > 0 { state.1.min(clear_count as u32) } else { state.1 })
        };
        // Um pedido da pessoa (passe ou limpeza) manda; senão a regra da limpeza; senão a do passe (jogos sem limpezas que se
        // possa partir, como os de passes de renderização clássicos).
        let msaa = c.samples > 1;
        // Uma imagem MSAA só serve resolvida: a foto depois de TODOS os passes (o último vale), sem a regra de limpeza.
        // Diagnóstico dos Xid 69: `LINUX_RESHADE_DEPTH_NO_SNAPSHOT=1` não parte passes nem copia a profundidade (os efeitos leem a imagem do
        // jogo, que pode estar velha ou limpa). Se os erros da GPU somem, a causa é a cópia dentro dos command buffers do jogo.
        let no_snapshot = !data.depth.snapshot_on();
        let wanted_clear = if no_snapshot || pick.pass.is_some() || msaa { None } else { pick.clear.or_else(auto_clear) };
        let auto_pass = || {
            // Com histerese: depois de usar a foto de um passe, só volta à imagem original se o jogo ficar muito tempo sem
            // recomeçar a imagem; o número do passe acompanha o último visto. Sem isso, a escolha trocava entre foto e
            // original a cada poucos quadros e a cadeia era reconstruída toda vez.
            let mut state = data.depth.auto_scene.lock().unwrap();
            if state.0 != c.image {
                *state = (c.image, None, 0);
            }
            match (state.1, super::observe::scene_pass(stats.passes_of(c.image))) {
                (_, Some(p)) => *state = (c.image, Some(p), 0),
                (Some(_), None) => {
                    state.2 += 1;
                    if state.2 >= AUTO_SCENE_FORGET_FRAMES {
                        *state = (c.image, None, 0);
                    }
                }
                (None, None) => {}
            }
            state.1
        };
        let wanted_pass = if no_snapshot { None } else if msaa { Some(ALL_PASSES) } else if wanted_clear.is_some() { None } else { pick.pass.or_else(auto_pass) };
        // Modo cópia (o do ReShade): sempre há foto, e a cópia da hora dos efeitos a renova quando nenhuma regra copiou no quadro.
        let copy_live = data.depth.copy_on() && !no_snapshot && !msaa && c.sampled && c.layout != 0;
        *data.depth.live.lock().unwrap() = copy_live.then_some(super::Live { image: c.image, format: c.format, width: c.width, height: c.height });
        let snapshot = (wanted_pass.is_some() || wanted_clear.is_some() || copy_live).then_some(()).and_then(|()| {
            // A foto: precisa da imagem da foto, e a imagem original precisa ter TRANSFER_SRC (armado). MSAA: foto de uma amostra.
            let format = if msaa { super::resolve::RESOLVED_FORMAT } else { vk::Format::from_raw(c.format) };
            unsafe { super::snapshot::ensure(data, format, c.width, c.height, msaa) }
        });
        match snapshot {
            Some(image) => {
                data.depth.snapshot_pass.store(wanted_pass.unwrap_or(0), Ordering::Relaxed);
                data.depth.snapshot_clear.store(wanted_clear.unwrap_or(0), Ordering::Relaxed);
                // Antes da primeira foto a imagem não tem layout: os efeitos só a recebem depois.
                if data.depth.snapshot_written.load(Ordering::Relaxed) || copy_live {
                    chosen = Some(super::Chosen { image: image.as_raw(), format: c.format, layout: vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL.as_raw(), width: c.width, height: c.height, linear: linear(c.format) });
                }
            }
            None if c.sampled && c.layout != 0 => {
                chosen = Some(super::Chosen { image: c.image, format: c.format, layout: c.layout, width: c.width, height: c.height, linear: linear(c.format) });
            }
            None => {}
        }
    }
    *data.depth.chosen.lock().unwrap() = chosen;

    let mut last = data.depth.last_published.lock().unwrap();
    // Para entender o que o jogo desenha na profundidade (e achar o ponto certo de usá-la): os passes da escolhida.
    let mut passes_summary = String::new();
    if let Some(c) = &active {
        let briefs: Vec<String> = stats
            .passes_of(c.image)
            .iter()
            .enumerate()
            .filter(|(_, p)| p.draws > 0 || !matches!(p.load, super::observe::LoadOp::Load))
            .map(|(i, p)| format!("{}:{}d/{}i {:?}->{:?}", i + 1, p.draws, p.indirect, p.load, p.store))
            .collect();
        passes_summary = briefs.join(" ");
        if !screen_summary.is_empty() {
            passes_summary = format!("tela: {screen_summary} | {passes_summary}");
        }
        // Os contadores mudam a cada quadro: só se atualizam de 90 em 90, para não reescrever o status sem parar.
        {
            let mut diag = data.depth.diag.lock().unwrap();
            if data.depth.diag_frames.fetch_add(1, Ordering::Relaxed) % 90 == 0 || diag.is_empty() {
                *diag = format!(
                    "trocas da imagem: {}, reconstruções da cadeia: {}; um quadro: {} desenhos ({} em passes com profundidade), limpezas de profundidade fora de passe: {} de anexos e {} de imagens; limpeza escolhida: {} de {} [desenhos antes de cada uma: {}]; última mudança que reconstruiu: {}; presents sem efeito/com efeito: {}/{}; passes partidos/pulados: {}/{}",
                    data.depth.image_changes.load(Ordering::Relaxed),
                    data.depth.chain_rebuilds.load(Ordering::Relaxed),
                    frame_draws,
                    frame_depth_draws,
                    frame_clear_attachments,
                    frame_clear_images,
                    data.depth.snapshot_clear.load(Ordering::Relaxed),
                    clear_count,
                    clear_events.iter().map(|e| format!("{}{}", e.draws_before, if e.splittable { "" } else { "!" })).collect::<Vec<_>>().join(" "),
                    data.depth.last_key_change.lock().unwrap(),
                    data.depth.fx_presents[0].load(Ordering::Relaxed),
                    data.depth.fx_presents[1].load(Ordering::Relaxed),
                    data.depth.splits_done.load(Ordering::Relaxed),
                    data.depth.splits_skipped.load(Ordering::Relaxed),
                );
            }
            passes_summary = format!("{} | {passes_summary}", *diag);
        }
        let mut logged = data.depth.passes_logged.lock().unwrap();
        // O resumo muda quase a cada quadro em alguns jogos: só se registra de vez em quando.
        if *logged != passes_summary && data.depth.diag_frames.load(Ordering::Relaxed) % 90 == 1 {
            log!("passes da profundidade escolhida: {passes_summary}");
            *logged = passes_summary.clone();
        }
    }
    if *last != candidates {
        if let Some(c) = &active {
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
            let pipelines = data.depth.pipelines.read().unwrap();
            let count = |c: Compare| pipelines.values().filter(|v| **v == c).count();
            let (greater, less, other) = (count(Compare::Greater), count(Compare::Less), count(Compare::Other));
            let mut logged = data.depth.pipelines_logged.lock().unwrap();
            if *logged != (greater, less, other) {
                *logged = (greater, less, other);
                log!("pipelines com teste de profundidade fixo: {greater} comparam 'maior', {less} 'menor', {other} outros");
            }
        }
        *last = candidates.clone();
    }
    crate::status::set_depth(candidates.iter().map(to_status).collect(), passes_summary);
}

/// Libera o que a observação criou (a imagem da foto). Chamado ao destruir o dispositivo.
pub fn destroy(data: &DeviceData) {
    unsafe { super::snapshot::destroy(data) };
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
        automatic: c.automatic,
        unusable: c.unusable.map(String::from),
        sampled: c.sampled,
        layout: c.layout,
    }
}

unsafe extern "system" fn cmd_blit_image(cmd: vk::CommandBuffer, src: vk::Image, src_layout: vk::ImageLayout, dst: vk::Image, dst_layout: vk::ImageLayout, count: u32, p_regions: *const vk::ImageBlit, filter: vk::Filter) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        screen_event(&data, cmd, dst, super::screen::Kind::Blit);
        (data.device.fp_v1_0().cmd_blit_image)(cmd, src, src_layout, dst, dst_layout, count, p_regions, filter);
    }
}

unsafe extern "system" fn cmd_copy_image(cmd: vk::CommandBuffer, src: vk::Image, src_layout: vk::ImageLayout, dst: vk::Image, dst_layout: vk::ImageLayout, count: u32, p_regions: *const vk::ImageCopy) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        screen_event(&data, cmd, dst, super::screen::Kind::Copy);
        (data.device.fp_v1_0().cmd_copy_image)(cmd, src, src_layout, dst, dst_layout, count, p_regions);
    }
}

unsafe extern "system" fn cmd_resolve_image(cmd: vk::CommandBuffer, src: vk::Image, src_layout: vk::ImageLayout, dst: vk::Image, dst_layout: vk::ImageLayout, count: u32, p_regions: *const vk::ImageResolve) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        screen_event(&data, cmd, dst, super::screen::Kind::Resolve);
        (data.device.fp_v1_0().cmd_resolve_image)(cmd, src, src_layout, dst, dst_layout, count, p_regions);
    }
}

unsafe extern "system" fn cmd_clear_color_image(cmd: vk::CommandBuffer, image: vk::Image, layout: vk::ImageLayout, p_color: *const vk::ClearColorValue, count: u32, p_ranges: *const vk::ImageSubresourceRange) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        screen_event(&data, cmd, image, super::screen::Kind::Clear);
        (data.device.fp_v1_0().cmd_clear_color_image)(cmd, image, layout, p_color, count, p_ranges);
    }
}
