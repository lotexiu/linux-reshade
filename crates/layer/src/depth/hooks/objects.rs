//! A criação e a destruição de objetos do jogo que a profundidade precisa conhecer: imagens, views, framebuffers, passes de renderização e pipelines.

use super::*;

pub(super) unsafe extern "system" fn create_image(
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
pub(super) unsafe fn msaa_sampleable(data: &DeviceData, c: &vk::ImageCreateInfo<'_>) -> bool {
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
pub(super) unsafe fn format_supports(data: &DeviceData, format: vk::Format, tiling: vk::ImageTiling, feature: vk::FormatFeatureFlags) -> bool {
    if tiling != vk::ImageTiling::OPTIMAL {
        return false;
    }
    let mut cache = data.depth.formats.lock().unwrap();
    *cache.entry(format.as_raw()).or_insert_with(|| {
        unsafe { data.instance.instance.get_physical_device_format_properties(data.physical_device, format) }.optimal_tiling_features.as_raw() as u64
    }) & u64::from(feature.as_raw())
        != 0
}

pub(super) unsafe extern "system" fn destroy_image(device: vk::Device, image: vk::Image, p_allocator: *const vk::AllocationCallbacks<'_>) {
    unsafe {
        let Some(data) = data_of(device.as_raw()) else { return };
        let tracked = data.depth.images.lock().unwrap().remove(&image.as_raw()).is_some();
        if tracked {
            crate::depth::resolve::forget_image(&data, image.as_raw());
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

pub(super) unsafe extern "system" fn create_image_view(
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

pub(super) unsafe extern "system" fn create_framebuffer(
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

pub(super) unsafe extern "system" fn create_render_pass(
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

pub(super) unsafe extern "system" fn create_render_pass2(
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

pub(super) unsafe extern "system" fn create_graphics_pipelines(
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
