//! O resolve da profundidade MSAA. Uma imagem de profundidade com várias amostras por pixel não pode ser lida como as demais
//! (os efeitos esperam uma amostra por pixel), e `vkCmdResolveImage` não resolve profundidade. Então a layer desenha a amostra 0 de
//! cada pixel numa imagem própria de uma amostra (`gl_FragDepth`), dentro do command buffer do jogo, onde a profundidade está no
//! layout conhecido. O resultado é a "foto" que os efeitos já sabem usar.

use super::Snapshot;
use crate::state::DeviceData;
use ash::vk;

const VERT: &[u8] = include_bytes!("shaders/resolve.vert.spv");
const FRAG: &[u8] = include_bytes!("shaders/resolve.frag.spv");
/// O formato da imagem resolvida (profundidade pura, de uma amostra).
pub const RESOLVED_FORMAT: vk::Format = vk::Format::D32_SFLOAT;
/// Quantos conjuntos de descritores se revezam: um conjunto não pode ser reescrito enquanto um command buffer pendente o usa.
const SETS: usize = 24;

/// Os recursos do passe de resolve, criados uma vez por dispositivo.
pub(crate) struct Resolver {
    render_pass: vk::RenderPass,
    set_layout: vk::DescriptorSetLayout,
    pipeline_layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    pool: vk::DescriptorPool,
    sets: Vec<vk::DescriptorSet>,
    next: usize,
    /// view (aspecto de profundidade) de cada imagem MSAA já vista.
    views: std::collections::HashMap<u64, vk::ImageView>,
}

/// A view e o framebuffer da foto de destino (do `Snapshot`).
pub(crate) struct ResolveTarget {
    pub view: vk::ImageView,
    pub framebuffer: vk::Framebuffer,
}

fn words(bytes: &[u8]) -> Vec<u32> {
    bytes.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

impl Resolver {
    unsafe fn new(data: &DeviceData) -> Option<Self> {
        let device = &data.device;
        unsafe {
            let attachment = vk::AttachmentDescription::default()
                .format(RESOLVED_FORMAT)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::DONT_CARE)
                .store_op(vk::AttachmentStoreOp::STORE)
                .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .final_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL);
            let depth_ref = vk::AttachmentReference::default().attachment(0).layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);
            let subpass = vk::SubpassDescription::default().pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS).depth_stencil_attachment(&depth_ref);
            let dependency = vk::SubpassDependency::default()
                .src_subpass(0)
                .dst_subpass(vk::SUBPASS_EXTERNAL)
                .src_stage_mask(vk::PipelineStageFlags::LATE_FRAGMENT_TESTS)
                .dst_stage_mask(vk::PipelineStageFlags::ALL_COMMANDS)
                .src_access_mask(vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE)
                .dst_access_mask(vk::AccessFlags::MEMORY_READ);
            let render_pass = device
                .create_render_pass(&vk::RenderPassCreateInfo::default().attachments(std::slice::from_ref(&attachment)).subpasses(std::slice::from_ref(&subpass)).dependencies(std::slice::from_ref(&dependency)), None)
                .ok()?;
            let binding = vk::DescriptorSetLayoutBinding::default().binding(0).descriptor_type(vk::DescriptorType::SAMPLED_IMAGE).descriptor_count(1).stage_flags(vk::ShaderStageFlags::FRAGMENT);
            let set_layout = device.create_descriptor_set_layout(&vk::DescriptorSetLayoutCreateInfo::default().bindings(std::slice::from_ref(&binding)), None).ok()?;
            let pipeline_layout = device.create_pipeline_layout(&vk::PipelineLayoutCreateInfo::default().set_layouts(std::slice::from_ref(&set_layout)), None).ok()?;
            let vert = device.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&words(VERT)), None).ok()?;
            let frag = device.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&words(FRAG)), None).ok()?;
            let stages = [
                vk::PipelineShaderStageCreateInfo::default().stage(vk::ShaderStageFlags::VERTEX).module(vert).name(c"main"),
                vk::PipelineShaderStageCreateInfo::default().stage(vk::ShaderStageFlags::FRAGMENT).module(frag).name(c"main"),
            ];
            let dynamic = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
            let pipeline = device.create_graphics_pipelines(
                vk::PipelineCache::null(),
                &[vk::GraphicsPipelineCreateInfo::default()
                    .stages(&stages)
                    .vertex_input_state(&vk::PipelineVertexInputStateCreateInfo::default())
                    .input_assembly_state(&vk::PipelineInputAssemblyStateCreateInfo::default().topology(vk::PrimitiveTopology::TRIANGLE_LIST))
                    .viewport_state(&vk::PipelineViewportStateCreateInfo::default().viewport_count(1).scissor_count(1))
                    .rasterization_state(&vk::PipelineRasterizationStateCreateInfo::default().polygon_mode(vk::PolygonMode::FILL).cull_mode(vk::CullModeFlags::NONE).line_width(1.0))
                    .multisample_state(&vk::PipelineMultisampleStateCreateInfo::default().rasterization_samples(vk::SampleCountFlags::TYPE_1))
                    .depth_stencil_state(&vk::PipelineDepthStencilStateCreateInfo::default().depth_test_enable(true).depth_write_enable(true).depth_compare_op(vk::CompareOp::ALWAYS))
                    .color_blend_state(&vk::PipelineColorBlendStateCreateInfo::default())
                    .dynamic_state(&vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic))
                    .layout(pipeline_layout)
                    .render_pass(render_pass)
                    .subpass(0)],
                None,
            );
            device.destroy_shader_module(vert, None);
            device.destroy_shader_module(frag, None);
            let pipeline = pipeline.ok()?[0];
            let sizes = [vk::DescriptorPoolSize::default().ty(vk::DescriptorType::SAMPLED_IMAGE).descriptor_count(SETS as u32)];
            let pool = device.create_descriptor_pool(&vk::DescriptorPoolCreateInfo::default().max_sets(SETS as u32).pool_sizes(&sizes), None).ok()?;
            let layouts = vec![set_layout; SETS];
            let sets = device.allocate_descriptor_sets(&vk::DescriptorSetAllocateInfo::default().descriptor_pool(pool).set_layouts(&layouts)).ok()?;
            Some(Self { render_pass, set_layout, pipeline_layout, pipeline, pool, sets, next: 0, views: std::collections::HashMap::new() })
        }
    }

    unsafe fn destroy(&mut self, device: &ash::Device) {
        unsafe {
            for (_, v) in self.views.drain() {
                device.destroy_image_view(v, None);
            }
            device.destroy_descriptor_pool(self.pool, None);
            device.destroy_pipeline(self.pipeline, None);
            device.destroy_pipeline_layout(self.pipeline_layout, None);
            device.destroy_descriptor_set_layout(self.set_layout, None);
            device.destroy_render_pass(self.render_pass, None);
        }
    }
}

/// Esquece a view de uma imagem MSAA que o jogo vai destruir.
///
/// # Safety
/// `data.device` válido.
pub(super) unsafe fn forget_image(data: &DeviceData, image: u64) {
    if let Some(r) = data.depth.resolver.lock().unwrap().as_mut()
        && let Some(v) = r.views.remove(&image)
    {
        unsafe { data.device.destroy_image_view(v, None) };
    }
}

/// Cria (se falta) a view e o framebuffer da foto de destino.
///
/// # Safety
/// `data.device` válido; a foto é dona da imagem.
pub(super) unsafe fn ensure_target(data: &DeviceData, snapshot: &mut Snapshot) -> bool {
    if snapshot.resolve.is_some() {
        return true;
    }
    let mut guard = data.depth.resolver.lock().unwrap();
    if guard.is_none() {
        *guard = unsafe { Resolver::new(data) };
    }
    let Some(resolver) = guard.as_ref() else { return false };
    let device = &data.device;
    unsafe {
        let view_info = vk::ImageViewCreateInfo::default()
            .image(snapshot.image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(snapshot.format)
            .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::DEPTH, level_count: 1, layer_count: 1, ..Default::default() });
        let Ok(view) = device.create_image_view(&view_info, None) else { return false };
        let views = [view];
        let fb_info = vk::FramebufferCreateInfo::default().render_pass(resolver.render_pass).attachments(&views).width(snapshot.width).height(snapshot.height).layers(1);
        let Ok(framebuffer) = device.create_framebuffer(&fb_info, None) else {
            device.destroy_image_view(view, None);
            return false;
        };
        snapshot.resolve = Some(ResolveTarget { view, framebuffer });
    }
    true
}

pub(super) unsafe fn destroy_target(device: &ash::Device, target: ResolveTarget) {
    unsafe {
        device.destroy_framebuffer(target.framebuffer, None);
        device.destroy_image_view(target.view, None);
    }
}

pub(super) unsafe fn destroy_resolver(data: &DeviceData) {
    if let Some(mut r) = data.depth.resolver.lock().unwrap().take() {
        unsafe { r.destroy(&data.device) };
    }
}

/// Grava o resolve: a profundidade MSAA `src` (no layout `src_layout`) vira a foto `snapshot` (uma amostra).
///
/// # Safety
/// `cmd` em gravação, fora de um passe de renderização; `src_layout` é o layout real da imagem neste ponto.
pub(super) unsafe fn record(data: &DeviceData, cmd: vk::CommandBuffer, src: vk::Image, src_layout: vk::ImageLayout, src_format: vk::Format, width: u32, height: u32) -> bool {
    let slot = data.depth.snapshot.lock().unwrap();
    let Some(snapshot) = slot.as_ref() else { return false };
    let Some(target) = snapshot.resolve.as_ref() else { return false };
    let mut guard = data.depth.resolver.lock().unwrap();
    let Some(resolver) = guard.as_mut() else { return false };
    let device = &data.device;
    unsafe {
        // A view de profundidade da imagem MSAA (uma por imagem).
        let view = match resolver.views.get(&ash::vk::Handle::as_raw(src)) {
            Some(v) => *v,
            None => {
                let info = vk::ImageViewCreateInfo::default()
                    .image(src)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(src_format)
                    .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::DEPTH, level_count: 1, layer_count: 1, ..Default::default() });
                let Ok(v) = device.create_image_view(&info, None) else { return false };
                resolver.views.insert(ash::vk::Handle::as_raw(src), v);
                v
            }
        };
        let set = resolver.sets[resolver.next % SETS];
        resolver.next += 1;
        let image_info = [vk::DescriptorImageInfo::default().image_view(view).image_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL)];
        device.update_descriptor_sets(&[vk::WriteDescriptorSet::default().dst_set(set).dst_binding(0).descriptor_type(vk::DescriptorType::SAMPLED_IMAGE).image_info(&image_info)], &[]);

        let aspect = if postfx::effect::depth_has_stencil(src_format) { vk::ImageAspectFlags::DEPTH | vk::ImageAspectFlags::STENCIL } else { vk::ImageAspectFlags::DEPTH };
        let range = vk::ImageSubresourceRange { aspect_mask: aspect, level_count: 1, layer_count: 1, ..Default::default() };
        let barrier = |old: vk::ImageLayout, new: vk::ImageLayout| {
            vk::ImageMemoryBarrier::default()
                .image(src)
                .old_layout(old)
                .new_layout(new)
                .src_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE)
                .dst_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .subresource_range(range)
        };
        let all = vk::PipelineStageFlags::ALL_COMMANDS;
        device.cmd_pipeline_barrier(cmd, all, all, vk::DependencyFlags::empty(), &[], &[], &[barrier(src_layout, vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL)]);
        let area = vk::Rect2D { offset: vk::Offset2D::default(), extent: vk::Extent2D { width, height } };
        device.cmd_begin_render_pass(cmd, &vk::RenderPassBeginInfo::default().render_pass(resolver.render_pass).framebuffer(target.framebuffer).render_area(area), vk::SubpassContents::INLINE);
        device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, resolver.pipeline);
        device.cmd_set_viewport(cmd, 0, &[vk::Viewport { x: 0.0, y: 0.0, width: width as f32, height: height as f32, min_depth: 0.0, max_depth: 1.0 }]);
        device.cmd_set_scissor(cmd, 0, &[area]);
        device.cmd_bind_descriptor_sets(cmd, vk::PipelineBindPoint::GRAPHICS, resolver.pipeline_layout, 0, &[set], &[]);
        device.cmd_draw(cmd, 3, 1, 0, 0);
        device.cmd_end_render_pass(cmd);
        device.cmd_pipeline_barrier(cmd, all, all, vk::DependencyFlags::empty(), &[], &[], &[barrier(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL, src_layout)]);
    }
    true
}
