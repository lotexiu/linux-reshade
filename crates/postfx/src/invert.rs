//! Passe que inverte as cores do alvo, no lugar.
//!
//! Por imagem do alvo, um command buffer pré-gravado faz: alvo -> cópia amostrável -> desenha um
//! triângulo de tela cheia que lê a cópia e escreve de volta no alvo. Como as imagens do swapchain
//! e as dimensões não mudam entre frames, tudo é gravado uma vez, na criação.
//!
//! Nota: se o formato do alvo for sRGB, a leitura converte para linear e a escrita volta a
//! codificar, então `1 - x` acontece no espaço linear. Serve ao marco 2; os efeitos de verdade
//! (marco 3) decidirão como tratar o espaço de cor.

use crate::{backbuffer::BackbufferCopies, spirv};
use ash::vk;

const VERT: &[u8] = include_bytes!("../shaders/fullscreen.vert.spv");
const FRAG: &[u8] = include_bytes!("../shaders/invert.frag.spv");

/// O que o passe processa.
pub struct Target<'a> {
    /// Imagens do alvo; precisam ter `COLOR_ATTACHMENT | TRANSFER_SRC` no uso.
    pub images: &'a [vk::Image],
    pub format: vk::Format,
    pub extent: vk::Extent2D,
    /// Layout em que cada imagem chega e para o qual volta (`PRESENT_SRC_KHR` num swapchain).
    pub rest_layout: vk::ImageLayout,
    /// Família da fila em que os command buffers serão submetidos.
    pub queue_family: u32,
    /// Chamado em cada command buffer logo após a alocação. Uma layer Vulkan usa isto para
    /// `vkSetDeviceLoaderData`; sem isso, as layers abaixo não reconhecem o objeto. `None` fora de layers.
    pub init_command_buffer: Option<&'a dyn Fn(vk::CommandBuffer) -> Result<(), vk::Result>>,
}

#[derive(Default)]
pub struct InvertPass {
    copies: BackbufferCopies,
    target_views: Vec<vk::ImageView>,
    render_pass: vk::RenderPass,
    set_layout: vk::DescriptorSetLayout,
    pipeline_layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    descriptor_pool: vk::DescriptorPool,
    framebuffers: Vec<vk::Framebuffer>,
    command_pool: vk::CommandPool,
    command_buffers: Vec<vk::CommandBuffer>,
}

const COLOR: vk::ImageSubresourceRange = vk::ImageSubresourceRange {
    aspect_mask: vk::ImageAspectFlags::COLOR,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};

fn barrier<'a>(
    image: vk::Image,
    old: vk::ImageLayout,
    new: vk::ImageLayout,
    src: vk::AccessFlags,
    dst: vk::AccessFlags,
) -> vk::ImageMemoryBarrier<'a> {
    vk::ImageMemoryBarrier::default()
        .image(image)
        .old_layout(old)
        .new_layout(new)
        .src_access_mask(src)
        .dst_access_mask(dst)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .subresource_range(COLOR)
}

impl InvertPass {
    /// Cria tudo. Em caso de erro, libera o que já criou antes de devolvê-lo.
    ///
    /// # Safety
    /// `device` válido; `memory` do mesmo dispositivo físico; `target.images` válidas e livres de uso.
    pub unsafe fn new(
        device: &ash::Device,
        memory: &vk::PhysicalDeviceMemoryProperties,
        target: &Target<'_>,
    ) -> Result<Self, vk::Result> {
        let mut pass = Self::default();
        if let Err(e) = unsafe { pass.build(device, memory, target) } {
            unsafe { pass.destroy(device) };
            return Err(e);
        }
        Ok(pass)
    }

    /// Command buffer que processa a imagem `index` do alvo.
    ///
    /// Submissão (feita por quem usa): espere o semáforo do renderizador no estágio `TRANSFER`.
    pub fn command_buffer(&self, index: usize) -> vk::CommandBuffer {
        self.command_buffers[index]
    }

    unsafe fn build(
        &mut self,
        device: &ash::Device,
        memory: &vk::PhysicalDeviceMemoryProperties,
        target: &Target<'_>,
    ) -> Result<(), vk::Result> {
        let count = target.images.len();
        unsafe {
            self.copies = BackbufferCopies::new(device, memory, target.format, target.extent, count)?;

            for &image in target.images {
                let info = vk::ImageViewCreateInfo::default()
                    .image(image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(target.format)
                    .subresource_range(COLOR);
                self.target_views.push(device.create_image_view(&info, None)?);
            }

            self.render_pass = Self::create_render_pass(device, target)?;

            let binding = vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT);
            let bindings = [binding];
            self.set_layout = device
                .create_descriptor_set_layout(&vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings), None)?;
            let layouts = [self.set_layout];
            self.pipeline_layout =
                device.create_pipeline_layout(&vk::PipelineLayoutCreateInfo::default().set_layouts(&layouts), None)?;

            self.pipeline = Self::create_pipeline(device, self.render_pass, self.pipeline_layout)?;

            let sizes = [vk::DescriptorPoolSize::default().ty(vk::DescriptorType::SAMPLED_IMAGE).descriptor_count(count as u32)];
            self.descriptor_pool = device.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default().max_sets(count as u32).pool_sizes(&sizes),
                None,
            )?;
            let set_layouts = vec![self.set_layout; count];
            let sets = device.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default().descriptor_pool(self.descriptor_pool).set_layouts(&set_layouts),
            )?;
            for (set, copy) in sets.iter().zip(&self.copies.images) {
                let image_info = [vk::DescriptorImageInfo::default()
                    .image_view(copy.view)
                    .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
                let write = vk::WriteDescriptorSet::default()
                    .dst_set(*set)
                    .dst_binding(0)
                    .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                    .image_info(&image_info);
                device.update_descriptor_sets(&[write], &[]);
            }

            for view in &self.target_views {
                let attachments = [*view];
                let info = vk::FramebufferCreateInfo::default()
                    .render_pass(self.render_pass)
                    .attachments(&attachments)
                    .width(target.extent.width)
                    .height(target.extent.height)
                    .layers(1);
                self.framebuffers.push(device.create_framebuffer(&info, None)?);
            }

            self.command_pool = device.create_command_pool(
                &vk::CommandPoolCreateInfo::default().queue_family_index(target.queue_family),
                None,
            )?;
            self.command_buffers = device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(self.command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(count as u32),
            )?;
            if let Some(init) = target.init_command_buffer {
                for &cb in &self.command_buffers {
                    init(cb)?;
                }
            }
            for (i, &cb) in self.command_buffers.iter().enumerate() {
                self.record(device, cb, i, &sets[i], target)?;
            }
        }
        Ok(())
    }

    unsafe fn create_render_pass(device: &ash::Device, target: &Target<'_>) -> Result<vk::RenderPass, vk::Result> {
        let attachment = vk::AttachmentDescription::default()
            .format(target.format)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::DONT_CARE) // o triângulo cobre tudo
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .final_layout(target.rest_layout);
        let color_refs = [vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)];
        let subpass = vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&color_refs);
        // Torna a escrita visível para o que vier depois (a apresentação), junto da troca de layout final.
        let dependency = vk::SubpassDependency::default()
            .src_subpass(0)
            .dst_subpass(vk::SUBPASS_EXTERNAL)
            .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags::BOTTOM_OF_PIPE)
            .dst_access_mask(vk::AccessFlags::empty());
        let attachments = [attachment];
        let subpasses = [subpass];
        let dependencies = [dependency];
        unsafe {
            device.create_render_pass(
                &vk::RenderPassCreateInfo::default()
                    .attachments(&attachments)
                    .subpasses(&subpasses)
                    .dependencies(&dependencies),
                None,
            )
        }
    }

    unsafe fn create_pipeline(
        device: &ash::Device,
        render_pass: vk::RenderPass,
        layout: vk::PipelineLayout,
    ) -> Result<vk::Pipeline, vk::Result> {
        let vert_words = spirv::words(VERT)?;
        let frag_words = spirv::words(FRAG)?;
        unsafe {
            let vert = device.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&vert_words), None)?;
            let frag = match device.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&frag_words), None) {
                Ok(m) => m,
                Err(e) => {
                    device.destroy_shader_module(vert, None);
                    return Err(e);
                }
            };
            let stages = [
                vk::PipelineShaderStageCreateInfo::default().stage(vk::ShaderStageFlags::VERTEX).module(vert).name(c"main"),
                vk::PipelineShaderStageCreateInfo::default().stage(vk::ShaderStageFlags::FRAGMENT).module(frag).name(c"main"),
            ];
            let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
            let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default().topology(vk::PrimitiveTopology::TRIANGLE_LIST);
            let viewport = vk::PipelineViewportStateCreateInfo::default().viewport_count(1).scissor_count(1);
            let raster = vk::PipelineRasterizationStateCreateInfo::default()
                .polygon_mode(vk::PolygonMode::FILL)
                .cull_mode(vk::CullModeFlags::NONE)
                .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
                .line_width(1.0);
            let multisample = vk::PipelineMultisampleStateCreateInfo::default().rasterization_samples(vk::SampleCountFlags::TYPE_1);
            let blend_attachments = [vk::PipelineColorBlendAttachmentState::default().color_write_mask(vk::ColorComponentFlags::RGBA)];
            let blend = vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_attachments);
            let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
            let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
            let info = vk::GraphicsPipelineCreateInfo::default()
                .stages(&stages)
                .vertex_input_state(&vertex_input)
                .input_assembly_state(&input_assembly)
                .viewport_state(&viewport)
                .rasterization_state(&raster)
                .multisample_state(&multisample)
                .color_blend_state(&blend)
                .dynamic_state(&dynamic)
                .layout(layout)
                .render_pass(render_pass)
                .subpass(0);
            let result = device.create_graphics_pipelines(vk::PipelineCache::null(), &[info], None);
            // Os módulos só são necessários durante a criação do pipeline.
            device.destroy_shader_module(vert, None);
            device.destroy_shader_module(frag, None);
            result.map(|p| p[0]).map_err(|(_, e)| e)
        }
    }

    unsafe fn record(
        &self,
        device: &ash::Device,
        cb: vk::CommandBuffer,
        index: usize,
        set: &vk::DescriptorSet,
        target: &Target<'_>,
    ) -> Result<(), vk::Result> {
        let image = target.images[index];
        let copy = self.copies.images[index].image;
        let layers = vk::ImageSubresourceLayers {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            mip_level: 0,
            base_array_layer: 0,
            layer_count: 1,
        };
        let none = vk::AccessFlags::empty();
        unsafe {
            device.begin_command_buffer(cb, &vk::CommandBufferBeginInfo::default())?;

            // 1. Alvo vira origem de cópia; a cópia amostrável vira destino (conteúdo anterior descartado).
            device.cmd_pipeline_barrier(
                cb,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[
                    barrier(image, target.rest_layout, vk::ImageLayout::TRANSFER_SRC_OPTIMAL, none, vk::AccessFlags::TRANSFER_READ),
                    barrier(copy, vk::ImageLayout::UNDEFINED, vk::ImageLayout::TRANSFER_DST_OPTIMAL, none, vk::AccessFlags::TRANSFER_WRITE),
                ],
            );

            // 2. Copia o frame do jogo.
            let region = vk::ImageCopy::default()
                .src_subresource(layers)
                .dst_subresource(layers)
                .extent(vk::Extent3D { width: target.extent.width, height: target.extent.height, depth: 1 });
            device.cmd_copy_image(
                cb,
                image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                copy,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[region],
            );

            // 3. A cópia vira textura; o alvo vira anexo de cor.
            device.cmd_pipeline_barrier(
                cb,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::FRAGMENT_SHADER | vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[
                    barrier(copy, vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL, vk::AccessFlags::TRANSFER_WRITE, vk::AccessFlags::SHADER_READ),
                    barrier(image, vk::ImageLayout::TRANSFER_SRC_OPTIMAL, vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL, none, vk::AccessFlags::COLOR_ATTACHMENT_WRITE),
                ],
            );

            // 4. Desenha o triângulo de tela cheia; o render pass devolve o alvo ao layout de repouso.
            let area = vk::Rect2D { offset: vk::Offset2D::default(), extent: target.extent };
            device.cmd_begin_render_pass(
                cb,
                &vk::RenderPassBeginInfo::default()
                    .render_pass(self.render_pass)
                    .framebuffer(self.framebuffers[index])
                    .render_area(area),
                vk::SubpassContents::INLINE,
            );
            device.cmd_bind_pipeline(cb, vk::PipelineBindPoint::GRAPHICS, self.pipeline);
            device.cmd_set_viewport(
                cb,
                0,
                &[vk::Viewport {
                    x: 0.0,
                    y: 0.0,
                    width: target.extent.width as f32,
                    height: target.extent.height as f32,
                    min_depth: 0.0,
                    max_depth: 1.0,
                }],
            );
            device.cmd_set_scissor(cb, 0, &[area]);
            device.cmd_bind_descriptor_sets(cb, vk::PipelineBindPoint::GRAPHICS, self.pipeline_layout, 0, &[*set], &[]);
            device.cmd_draw(cb, 3, 1, 0, 0);
            device.cmd_end_render_pass(cb);

            device.end_command_buffer(cb)
        }
    }

    /// Libera tudo. Pode ser chamado mais de uma vez.
    ///
    /// # Safety
    /// A GPU não pode estar executando os command buffers deste passe.
    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        unsafe {
            if self.command_pool != vk::CommandPool::null() {
                device.destroy_command_pool(std::mem::take(&mut self.command_pool), None); // libera os command buffers
            }
            self.command_buffers.clear();
            for f in self.framebuffers.drain(..) {
                device.destroy_framebuffer(f, None);
            }
            if self.pipeline != vk::Pipeline::null() {
                device.destroy_pipeline(std::mem::take(&mut self.pipeline), None);
            }
            if self.pipeline_layout != vk::PipelineLayout::null() {
                device.destroy_pipeline_layout(std::mem::take(&mut self.pipeline_layout), None);
            }
            if self.descriptor_pool != vk::DescriptorPool::null() {
                device.destroy_descriptor_pool(std::mem::take(&mut self.descriptor_pool), None); // libera os sets
            }
            if self.set_layout != vk::DescriptorSetLayout::null() {
                device.destroy_descriptor_set_layout(std::mem::take(&mut self.set_layout), None);
            }
            if self.render_pass != vk::RenderPass::null() {
                device.destroy_render_pass(std::mem::take(&mut self.render_pass), None);
            }
            for v in self.target_views.drain(..) {
                device.destroy_image_view(v, None);
            }
            self.copies.destroy(device);
        }
    }
}
