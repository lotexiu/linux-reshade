//! Render pass e pipeline de um passe do efeito.

use super::convert;
use ash::vk;
use fx_bundle::manifest::Pass;

/// Para onde um passe escreve.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Output {
    /// O frame do jogo (a imagem do swapchain).
    #[default]
    Frame,
    /// Texturas intermediárias do efeito.
    Textures,
}

/// Cria o render pass. Os anexos de textura ficam sempre em `SHADER_READ_ONLY_OPTIMAL` entre
/// passes; o frame fica em `COLOR_ATTACHMENT_OPTIMAL` (as transições de/para o layout de repouso e
/// para a cópia são barreiras explícitas na gravação).
///
/// # Safety
/// `device` válido.
pub unsafe fn create_render_pass(
    device: &ash::Device,
    pass: &Pass,
    output: Output,
    formats: &[vk::Format],
) -> Result<vk::RenderPass, vk::Result> {
    let attachment_layout = match output {
        Output::Frame => vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
        Output::Textures => vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
    };
    // Quando é seguro descartar o conteúdo: o passe limpa, ou cobre tudo (triângulo de tela cheia
    // sem blend e sem viewport menor). Fora disso, preserva o que já havia.
    let covers_all = !pass.blend.iter().any(|b| b.enable) && pass.viewport == [0, 0];
    let load_op = if pass.clear {
        vk::AttachmentLoadOp::CLEAR
    } else if output == Output::Frame && covers_all {
        vk::AttachmentLoadOp::DONT_CARE
    } else {
        vk::AttachmentLoadOp::LOAD
    };
    // Com DONT_CARE o layout inicial pode ser UNDEFINED; com LOAD precisa ser o real.
    let initial = if load_op == vk::AttachmentLoadOp::DONT_CARE { vk::ImageLayout::UNDEFINED } else { attachment_layout };

    let attachments: Vec<_> = formats
        .iter()
        .map(|&format| {
            vk::AttachmentDescription::default()
                .format(format)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(load_op)
                .store_op(vk::AttachmentStoreOp::STORE)
                .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                .initial_layout(initial)
                .final_layout(attachment_layout)
        })
        .collect();
    let refs: Vec<_> = (0..formats.len() as u32)
        .map(|attachment| vk::AttachmentReference::default().attachment(attachment).layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL))
        .collect();
    let subpass = vk::SubpassDescription::default().pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS).color_attachments(&refs);

    // Entrada: o que veio antes (leituras do passe anterior, escritas no anexo) termina antes de
    // escrevermos. Saída: nossa escrita fica visível para quem amostrar a textura depois.
    let dependencies = [
        vk::SubpassDependency::default()
            .src_subpass(vk::SUBPASS_EXTERNAL)
            .dst_subpass(0)
            .src_stage_mask(vk::PipelineStageFlags::FRAGMENT_SHADER | vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .src_access_mask(vk::AccessFlags::SHADER_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE),
        vk::SubpassDependency::default()
            .src_subpass(0)
            .dst_subpass(vk::SUBPASS_EXTERNAL)
            .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags::VERTEX_SHADER | vk::PipelineStageFlags::FRAGMENT_SHADER | vk::PipelineStageFlags::TRANSFER)
            .dst_access_mask(vk::AccessFlags::SHADER_READ | vk::AccessFlags::TRANSFER_READ),
    ];
    let subpasses = [subpass];
    unsafe {
        device.create_render_pass(
            &vk::RenderPassCreateInfo::default().attachments(&attachments).subpasses(&subpasses).dependencies(&dependencies),
            None,
        )
    }
}

/// Cria o pipeline gráfico do passe com os módulos SPIR-V já carregados.
///
/// # Safety
/// `device` válido; `vertex`/`pixel` são SPIR-V válido com os entry points `vertex_name`/`pixel_name`.
pub unsafe fn create_pipeline(
    device: &ash::Device,
    pass: &Pass,
    render_pass: vk::RenderPass,
    layout: vk::PipelineLayout,
    vertex: &[u32],
    pixel: &[u32],
    attachment_count: usize,
) -> Result<vk::Pipeline, vk::Result> {
    unsafe {
        let vert = device.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(vertex), None)?;
        let frag = match device.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(pixel), None) {
            Ok(m) => m,
            Err(e) => {
                device.destroy_shader_module(vert, None);
                return Err(e);
            }
        };
        // Os nomes vêm do manifesto; o `CString` precisa viver até a criação do pipeline.
        let vert_name = std::ffi::CString::new(pass.vertex_entry.as_str()).map_err(|_| vk::Result::ERROR_INITIALIZATION_FAILED)?;
        let frag_name = std::ffi::CString::new(pass.pixel_entry.as_str()).map_err(|_| vk::Result::ERROR_INITIALIZATION_FAILED)?;
        let stages = [
            vk::PipelineShaderStageCreateInfo::default().stage(vk::ShaderStageFlags::VERTEX).module(vert).name(&vert_name),
            vk::PipelineShaderStageCreateInfo::default().stage(vk::ShaderStageFlags::FRAGMENT).module(frag).name(&frag_name),
        ];

        let blend_attachments: Vec<_> = (0..attachment_count)
            .map(|k| {
                let b = pass.blend.get(k).or(pass.blend.first());
                match b {
                    Some(b) => vk::PipelineColorBlendAttachmentState::default()
                        .blend_enable(b.enable)
                        .src_color_blend_factor(convert::blend_factor(b.src_color))
                        .dst_color_blend_factor(convert::blend_factor(b.dst_color))
                        .color_blend_op(convert::blend_op(b.color_op))
                        .src_alpha_blend_factor(convert::blend_factor(b.src_alpha))
                        .dst_alpha_blend_factor(convert::blend_factor(b.dst_alpha))
                        .alpha_blend_op(convert::blend_op(b.alpha_op))
                        .color_write_mask(vk::ColorComponentFlags::from_raw(b.write_mask & 0xF)),
                    None => vk::PipelineColorBlendAttachmentState::default().color_write_mask(vk::ColorComponentFlags::RGBA),
                }
            })
            .collect();

        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default().topology(convert::topology(pass.topology));
        let viewport = vk::PipelineViewportStateCreateInfo::default().viewport_count(1).scissor_count(1);
        let raster = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .line_width(1.0);
        let multisample = vk::PipelineMultisampleStateCreateInfo::default().rasterization_samples(vk::SampleCountFlags::TYPE_1);
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
        device.destroy_shader_module(vert, None);
        device.destroy_shader_module(frag, None);
        result.map(|p| p[0]).map_err(|(_, e)| e)
    }
}
