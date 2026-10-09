//! Gravação do command buffer de uma imagem do alvo: a coreografia de layouts do frame, a cópia
//! amostrável e a execução dos passes em ordem.

use super::{PassRes, pipeline::Output};
use crate::image::COLOR_RANGE;
use ash::vk;

/// O que a gravação precisa saber, sem emprestar o `Effect` inteiro.
pub struct Recording<'a> {
    pub device: &'a ash::Device,
    pub passes: &'a [PassRes],
    /// Imagem de cópia do frame (alvo das amostragens de `COLOR`).
    pub copy: vk::Image,
    pub frame: vk::Image,
    pub frame_extent: vk::Extent2D,
    pub rest_layout: vk::ImageLayout,
    /// `sets0[index]` quando o efeito tem uniforms; vazio quando não tem.
    pub set0: Option<vk::DescriptorSet>,
}

/// Estágios e acessos que cada layout implica, para as barreiras de transição.
fn scope(layout: vk::ImageLayout, destination: bool) -> (vk::PipelineStageFlags, vk::AccessFlags) {
    use vk::{AccessFlags as A, ImageLayout as L, PipelineStageFlags as S};
    match layout {
        L::UNDEFINED => (S::TOP_OF_PIPE, A::empty()),
        L::TRANSFER_SRC_OPTIMAL => (S::TRANSFER, A::TRANSFER_READ),
        L::TRANSFER_DST_OPTIMAL => (S::TRANSFER, A::TRANSFER_WRITE),
        L::COLOR_ATTACHMENT_OPTIMAL => (S::COLOR_ATTACHMENT_OUTPUT, A::COLOR_ATTACHMENT_READ | A::COLOR_ATTACHMENT_WRITE),
        L::SHADER_READ_ONLY_OPTIMAL => (S::VERTEX_SHADER | S::FRAGMENT_SHADER, A::SHADER_READ),
        // Imagem apresentável: chega já visível (o semáforo de espera cobre) e sai para a apresentação.
        L::PRESENT_SRC_KHR if destination => (S::BOTTOM_OF_PIPE, A::empty()),
        L::PRESENT_SRC_KHR => (S::TOP_OF_PIPE, A::empty()),
        _ => (S::ALL_COMMANDS, A::MEMORY_READ | A::MEMORY_WRITE),
    }
}

unsafe fn transition(device: &ash::Device, cb: vk::CommandBuffer, image: vk::Image, old: vk::ImageLayout, new: vk::ImageLayout) {
    let (src_stage, src_access) = scope(old, false);
    let (dst_stage, dst_access) = scope(new, true);
    let barrier = vk::ImageMemoryBarrier::default()
        .image(image)
        .old_layout(old)
        .new_layout(new)
        .src_access_mask(src_access)
        .dst_access_mask(dst_access)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .subresource_range(COLOR_RANGE);
    unsafe {
        device.cmd_pipeline_barrier(cb, src_stage, dst_stage, vk::DependencyFlags::empty(), &[], &[], &[barrier]);
    }
}

/// Grava os comandos de `rec` em `cb` para a imagem `index`.
///
/// # Safety
/// `cb` em estado inicial; os handles de `rec` válidos.
pub unsafe fn record(rec: &Recording<'_>, cb: vk::CommandBuffer, index: usize) -> Result<(), vk::Result> {
    let device = rec.device;
    unsafe {
        device.begin_command_buffer(cb, &vk::CommandBufferBeginInfo::default())?;

        // Ordena este trabalho contra TUDO que veio antes na fila (o frame anterior do efeito
        // inclusive): é o que permite compartilhar entre frames a cópia e as texturas intermediárias.
        let memory = vk::MemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE)
            .dst_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE);
        device.cmd_pipeline_barrier(
            cb,
            vk::PipelineStageFlags::ALL_COMMANDS,
            vk::PipelineStageFlags::ALL_COMMANDS,
            vk::DependencyFlags::empty(),
            &[memory],
            &[],
            &[],
        );

        let mut frame_layout = rec.rest_layout;
        let mut copy_valid = false;
        let layers = vk::ImageSubresourceLayers { aspect_mask: vk::ImageAspectFlags::COLOR, mip_level: 0, base_array_layer: 0, layer_count: 1 };

        for pass in rec.passes {
            // A cópia do frame precisa refletir o que os passes anteriores escreveram nele.
            if pass.samples_color && !copy_valid {
                transition(device, cb, rec.frame, frame_layout, vk::ImageLayout::TRANSFER_SRC_OPTIMAL);
                frame_layout = vk::ImageLayout::TRANSFER_SRC_OPTIMAL;
                transition(device, cb, rec.copy, vk::ImageLayout::UNDEFINED, vk::ImageLayout::TRANSFER_DST_OPTIMAL);
                let region = vk::ImageCopy::default()
                    .src_subresource(layers)
                    .dst_subresource(layers)
                    .extent(vk::Extent3D { width: rec.frame_extent.width, height: rec.frame_extent.height, depth: 1 });
                device.cmd_copy_image(
                    cb,
                    rec.frame,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    rec.copy,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[region],
                );
                transition(device, cb, rec.copy, vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
                copy_valid = true;
            }

            if pass.output == Output::Frame && frame_layout != vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL {
                transition(device, cb, rec.frame, frame_layout, vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
                frame_layout = vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL;
            }

            let framebuffer = match pass.output {
                Output::Frame => pass.framebuffers[index],
                Output::Textures => pass.framebuffers[0],
            };
            let area = vk::Rect2D { offset: vk::Offset2D::default(), extent: pass.extent };
            let clears = vec![vk::ClearValue { color: vk::ClearColorValue { float32: [0.0; 4] } }; pass.attachment_count];
            device.cmd_begin_render_pass(
                cb,
                &vk::RenderPassBeginInfo::default()
                    .render_pass(pass.render_pass)
                    .framebuffer(framebuffer)
                    .render_area(area)
                    .clear_values(&clears),
                vk::SubpassContents::INLINE,
            );
            device.cmd_bind_pipeline(cb, vk::PipelineBindPoint::GRAPHICS, pass.pipeline);
            device.cmd_set_viewport(
                cb,
                0,
                &[vk::Viewport {
                    x: 0.0,
                    y: 0.0,
                    width: pass.viewport[0] as f32,
                    height: pass.viewport[1] as f32,
                    min_depth: 0.0,
                    max_depth: 1.0,
                }],
            );
            device.cmd_set_scissor(cb, 0, &[area]);
            match rec.set0 {
                Some(set0) => device.cmd_bind_descriptor_sets(cb, vk::PipelineBindPoint::GRAPHICS, pass.pipeline_layout, 0, &[set0, pass.set1], &[]),
                None => device.cmd_bind_descriptor_sets(cb, vk::PipelineBindPoint::GRAPHICS, pass.pipeline_layout, 1, &[pass.set1], &[]),
            }
            device.cmd_draw(cb, pass.num_vertices, 1, 0, 0);
            device.cmd_end_render_pass(cb);

            if pass.output == Output::Frame {
                copy_valid = false; // o frame mudou: uma amostragem de COLOR depois precisa de cópia nova
            }
        }

        if frame_layout != rec.rest_layout {
            transition(device, cb, rec.frame, frame_layout, rec.rest_layout);
        }
        device.end_command_buffer(cb)
    }
}
