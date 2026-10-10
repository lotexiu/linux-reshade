//! Gravação do command buffer de uma imagem do alvo: a coreografia de layouts do frame, a cópia
//! amostrável e a execução dos passes em ordem.

use super::{MipChain, PassRes, pipeline::Output};
use crate::image::COLOR_RANGE;
use ash::vk;

/// A imagem de profundidade do jogo que os passes amostram: vai ao layout de leitura antes e volta depois.
#[derive(Clone, Copy)]
pub struct DepthBarrier {
    pub image: vk::Image,
    /// O formato tem estêncil (a barreira nomeia os dois aspectos).
    pub stencil: bool,
    /// O layout em que a imagem está (e para o qual deve voltar).
    pub layout: vk::ImageLayout,
}

/// O que a gravação precisa saber, sem emprestar o `Effect` inteiro.
pub struct Recording<'a> {
    pub device: &'a ash::Device,
    pub passes: &'a [PassRes],
    /// Imagem de cópia do frame (alvo das amostragens de `COLOR`).
    pub copy: vk::Image,
    /// Cópia sRGB do frame, se algum sampler lê o frame com `SRGBTexture`.
    pub copy_srgb: Option<vk::Image>,
    /// Espelho sRGB do frame, se algum passe escreve com `SRGBWriteEnable`.
    pub shadow: Option<vk::Image>,
    /// Imagem de trabalho quando o alvo é sRGB: os passes a usam no lugar do frame, que só recebe a cópia dos bytes.
    pub work: Option<vk::Image>,
    pub depth: Option<DepthBarrier>,
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

unsafe fn depth_transition(device: &ash::Device, cb: vk::CommandBuffer, d: &DepthBarrier, old: vk::ImageLayout, new: vk::ImageLayout) {
    let aspect = if d.stencil { vk::ImageAspectFlags::DEPTH | vk::ImageAspectFlags::STENCIL } else { vk::ImageAspectFlags::DEPTH };
    let barrier = vk::ImageMemoryBarrier::default()
        .image(d.image)
        .old_layout(old)
        .new_layout(new)
        .src_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE)
        .dst_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .subresource_range(vk::ImageSubresourceRange { aspect_mask: aspect, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
    unsafe {
        device.cmd_pipeline_barrier(cb, vk::PipelineStageFlags::ALL_COMMANDS, vk::PipelineStageFlags::ALL_COMMANDS, vk::DependencyFlags::empty(), &[], &[], &[barrier]);
    }
}

/// Gera os níveis de mipmap de uma textura que um passe acabou de escrever (o nível 0 chega em
/// `SHADER_READ_ONLY_OPTIMAL`, como o render pass o deixa) e a devolve a esse layout, em todos os níveis.
pub(super) unsafe fn generate_mips(device: &ash::Device, cb: vk::CommandBuffer, chain: &MipChain) {
    let range = |base: u32, count: u32| vk::ImageSubresourceRange { base_mip_level: base, level_count: count, ..COLOR_RANGE };
    let barrier = |range: vk::ImageSubresourceRange, old: vk::ImageLayout, new: vk::ImageLayout| {
        let b = vk::ImageMemoryBarrier::default()
            .image(chain.image)
            .old_layout(old)
            .new_layout(new)
            .src_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE)
            .dst_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .subresource_range(range);
        unsafe {
            device.cmd_pipeline_barrier(cb, vk::PipelineStageFlags::ALL_COMMANDS, vk::PipelineStageFlags::ALL_COMMANDS, vk::DependencyFlags::empty(), &[], &[], &[b]);
        }
    };
    use vk::ImageLayout as L;
    let last = chain.levels - 1;
    barrier(range(0, 1), L::SHADER_READ_ONLY_OPTIMAL, L::TRANSFER_SRC_OPTIMAL);
    barrier(range(1, last), L::UNDEFINED, L::TRANSFER_DST_OPTIMAL);
    let size = |level: u32| vk::Offset3D { x: (chain.extent.width >> level).max(1) as i32, y: (chain.extent.height >> level).max(1) as i32, z: 1 };
    let layers = |level: u32| vk::ImageSubresourceLayers { aspect_mask: vk::ImageAspectFlags::COLOR, mip_level: level, base_array_layer: 0, layer_count: 1 };
    for level in 1..=last {
        if level > 1 {
            barrier(range(level - 1, 1), L::TRANSFER_DST_OPTIMAL, L::TRANSFER_SRC_OPTIMAL);
        }
        let blit = vk::ImageBlit::default()
            .src_subresource(layers(level - 1))
            .src_offsets([vk::Offset3D::default(), size(level - 1)])
            .dst_subresource(layers(level))
            .dst_offsets([vk::Offset3D::default(), size(level)]);
        unsafe {
            device.cmd_blit_image(
                cb,
                chain.image,
                L::TRANSFER_SRC_OPTIMAL,
                chain.image,
                L::TRANSFER_DST_OPTIMAL,
                &[blit],
                if chain.linear { vk::Filter::LINEAR } else { vk::Filter::NEAREST },
            );
        }
    }
    barrier(range(0, last), L::TRANSFER_SRC_OPTIMAL, L::SHADER_READ_ONLY_OPTIMAL);
    barrier(range(last, 1), L::TRANSFER_DST_OPTIMAL, L::SHADER_READ_ONLY_OPTIMAL);
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

        // Com imagem de trabalho (alvo sRGB), o "frame" dos passes é ela: entra uma cópia dos bytes do alvo e, no fim,
        // eles voltam. Copiar entre formatos UNORM e sRGB de mesmo tamanho não converte nada.
        let layers0 = vk::ImageSubresourceLayers { aspect_mask: vk::ImageAspectFlags::COLOR, mip_level: 0, base_array_layer: 0, layer_count: 1 };
        let extent3 = vk::Extent3D { width: rec.frame_extent.width, height: rec.frame_extent.height, depth: 1 };
        let whole0 = vk::ImageCopy::default().src_subresource(layers0).dst_subresource(layers0).extent(extent3);
        let (frame, rest_layout) = match rec.work {
            Some(work) => {
                transition(device, cb, rec.frame, rec.rest_layout, vk::ImageLayout::TRANSFER_SRC_OPTIMAL);
                transition(device, cb, work, vk::ImageLayout::UNDEFINED, vk::ImageLayout::TRANSFER_DST_OPTIMAL);
                device.cmd_copy_image(cb, rec.frame, vk::ImageLayout::TRANSFER_SRC_OPTIMAL, work, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &[whole0]);
                transition(device, cb, work, vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::ImageLayout::TRANSFER_SRC_OPTIMAL);
                (work, vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            }
            None => (rec.frame, rec.rest_layout),
        };
        // A profundidade do jogo fica em layout de leitura enquanto os passes a amostram e volta ao que era depois.
        if let Some(d) = &rec.depth
            && rec.passes.iter().any(|p| p.samples_depth)
        {
            depth_transition(device, cb, d, d.layout, vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL);
        }
        let mut frame_layout = rest_layout;
        let mut copy_valid = false;
        let layers = vk::ImageSubresourceLayers { aspect_mask: vk::ImageAspectFlags::COLOR, mip_level: 0, base_array_layer: 0, layer_count: 1 };

        let whole = vk::ImageCopy::default()
            .src_subresource(layers)
            .dst_subresource(layers)
            .extent(vk::Extent3D { width: rec.frame_extent.width, height: rec.frame_extent.height, depth: 1 });

        for pass in rec.passes {
            // A cópia do frame precisa refletir o que os passes anteriores escreveram nele.
            if pass.samples_color && !copy_valid {
                transition(device, cb, frame, frame_layout, vk::ImageLayout::TRANSFER_SRC_OPTIMAL);
                frame_layout = vk::ImageLayout::TRANSFER_SRC_OPTIMAL;
                // Os mesmos bytes do frame, em formato UNORM e (se preciso) sRGB: a leitura de um sampler com
                // `SRGBTexture` decodifica sozinha.
                for copy in std::iter::once(rec.copy).chain(rec.copy_srgb) {
                    transition(device, cb, copy, vk::ImageLayout::UNDEFINED, vk::ImageLayout::TRANSFER_DST_OPTIMAL);
                    device.cmd_copy_image(cb, frame, vk::ImageLayout::TRANSFER_SRC_OPTIMAL, copy, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &[whole]);
                    transition(device, cb, copy, vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
                }
                copy_valid = true;
            }

            let shadow = if pass.srgb_shadow { rec.shadow } else { None };
            if let Some(shadow) = shadow {
                // O passe desenha no espelho sRGB (a GPU codifica ao escrever); o conteúdo atual do frame, se o
                // passe precisa dele, entra como cópia de bytes.
                if pass.preserves_frame {
                    if frame_layout != vk::ImageLayout::TRANSFER_SRC_OPTIMAL {
                        transition(device, cb, frame, frame_layout, vk::ImageLayout::TRANSFER_SRC_OPTIMAL);
                        frame_layout = vk::ImageLayout::TRANSFER_SRC_OPTIMAL;
                    }
                    transition(device, cb, shadow, vk::ImageLayout::UNDEFINED, vk::ImageLayout::TRANSFER_DST_OPTIMAL);
                    device.cmd_copy_image(cb, frame, vk::ImageLayout::TRANSFER_SRC_OPTIMAL, shadow, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &[whole]);
                    transition(device, cb, shadow, vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
                }
            } else if pass.output == Output::Frame && frame_layout != vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL {
                transition(device, cb, frame, frame_layout, vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
                frame_layout = vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL;
            }

            let framebuffer = match pass.output {
                Output::Frame if shadow.is_some() => pass.framebuffers[0],
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
            for chain in &pass.mips {
                generate_mips(device, cb, chain);
            }

            if let Some(shadow) = shadow {
                // Do espelho para o frame: cópia de bytes, sem conversão (já saíram codificados).
                transition(device, cb, shadow, vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL, vk::ImageLayout::TRANSFER_SRC_OPTIMAL);
                if frame_layout != vk::ImageLayout::TRANSFER_DST_OPTIMAL {
                    transition(device, cb, frame, frame_layout, vk::ImageLayout::TRANSFER_DST_OPTIMAL);
                    frame_layout = vk::ImageLayout::TRANSFER_DST_OPTIMAL;
                }
                device.cmd_copy_image(cb, shadow, vk::ImageLayout::TRANSFER_SRC_OPTIMAL, frame, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &[whole]);
            }
            if pass.output == Output::Frame {
                copy_valid = false; // o frame mudou: uma amostragem de COLOR depois precisa de cópia nova
            }
        }

        if frame_layout != rest_layout {
            transition(device, cb, frame, frame_layout, rest_layout);
        }
        if let Some(d) = &rec.depth
            && rec.passes.iter().any(|p| p.samples_depth)
        {
            depth_transition(device, cb, d, vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL, d.layout);
        }
        if rec.work.is_some() {
            // O resultado volta para a imagem do swapchain e ela segue para a apresentação.
            transition(device, cb, rec.frame, vk::ImageLayout::TRANSFER_SRC_OPTIMAL, vk::ImageLayout::TRANSFER_DST_OPTIMAL);
            device.cmd_copy_image(cb, frame, vk::ImageLayout::TRANSFER_SRC_OPTIMAL, rec.frame, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &[whole0]);
            transition(device, cb, rec.frame, vk::ImageLayout::TRANSFER_DST_OPTIMAL, rec.rest_layout);
        }
        device.end_command_buffer(cb)
    }
}
