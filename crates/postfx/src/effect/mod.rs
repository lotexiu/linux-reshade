//! Executa uma técnica de um bundle (passes de pós-processamento) sobre as imagens de um alvo.
//!
//! Visão geral de um frame, por imagem do alvo (um command buffer pré-gravado por imagem):
//! barreira global -> (se algum passe amostra `COLOR`) copia o frame para uma imagem amostrável ->
//! passes em ordem, cada um num render pass próprio -> frame de volta ao layout de repouso.
//!
//! Limites da v1, recusados com `Error::Unsupported` em vez de falhar em silêncio: alvo sRGB,
//! mipmaps, sRGB por sampler/passe, texturas vindas de arquivo, profundidade, vários anexos mistos
//! entre frame e texturas, e compute.

mod convert;
mod pipeline;
mod record;
mod resources;
mod sources;

pub use convert::is_srgb;
pub use sources::{FrameInfo, utc_date};

use crate::backbuffer::BackbufferCopies;
use crate::error::{Error, unsupported};
use crate::image::{ALL_LEVELS, COLOR_RANGE, GpuImage};
use ash::vk;
use fx_bundle::Bundle;
use fx_bundle::manifest::{Texture, TextureFormat};
use pipeline::Output;
use resources::UniformBuffer;
use sources::Sources;
use std::collections::HashMap;

/// O que o efeito processa.
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

/// Recursos de um passe.
#[derive(Default)]
pub(crate) struct PassRes {
    pub output: Output,
    pub render_pass: vk::RenderPass,
    pub pipeline: vk::Pipeline,
    pub pipeline_layout: vk::PipelineLayout,
    pub set1_layout: vk::DescriptorSetLayout,
    pub set1: vk::DescriptorSet,
    /// Um por imagem do alvo (`Output::Frame`) ou um só (`Output::Textures`).
    pub framebuffers: Vec<vk::Framebuffer>,
    pub extent: vk::Extent2D,
    pub viewport: [u32; 2],
    pub attachment_count: usize,
    pub num_vertices: u32,
    pub samples_color: bool,
    /// Escreve no frame com `SRGBWriteEnable`: desenha numa imagem sRGB e copia os bytes para o frame.
    pub srgb_shadow: bool,
    /// O passe precisa do conteúdo atual do frame (blend, viewport menor): o espelho começa como cópia dele.
    pub preserves_frame: bool,
    /// Texturas com mipmaps que este passe escreve: os níveis seguintes são gerados depois dele.
    pub mips: Vec<MipChain>,
}

/// O que gerar os níveis de mipmap de uma textura precisa saber.
#[derive(Clone, Copy)]
pub(crate) struct MipChain {
    pub image: vk::Image,
    pub extent: vk::Extent2D,
    pub levels: u32,
    /// Formatos de 32 bits float não filtram linearmente em toda GPU: o blit usa o vizinho mais próximo.
    pub linear: bool,
}

#[derive(Default)]
pub struct Effect {
    copy: BackbufferCopies,
    /// Cópia do frame em formato sRGB, para samplers com `SRGBTexture` (a leitura decodifica sozinha).
    copy_srgb: GpuImage,
    /// Espelho sRGB do frame, onde desenham os passes com `SRGBWriteEnable` (a escrita codifica sozinha).
    frame_srgb: GpuImage,
    /// Paralelo a `manifest.textures`; só as texturas intermediárias (render targets) têm imagem.
    textures: Vec<GpuImage>,
    samplers: Vec<vk::Sampler>,
    /// Um por imagem do alvo; vazio se o efeito não tem uniforms.
    uniform_buffers: Vec<UniformBuffer>,
    set0_layout: vk::DescriptorSetLayout,
    descriptor_pool: vk::DescriptorPool,
    sets0: Vec<vk::DescriptorSet>,
    passes: Vec<PassRes>,
    target_views: Vec<vk::ImageView>,
    command_pool: vk::CommandPool,
    command_buffers: Vec<vk::CommandBuffer>,
    /// Uniforms alimentados pelo runtime (`source = "timer"` etc.).
    sources: Sources,
}

/// Posição de uma textura no manifesto.
fn tex_position(m: &fx_bundle::Manifest, tex: &Texture) -> usize {
    m.textures.iter().position(|t| t.unique_name == tex.unique_name).unwrap_or(0)
}

/// Uma textura de arquivo só roda se o bundle trouxe a imagem e o formato é de 8 bits.
fn file_texture_supported(bundle: &Bundle, tex: &Texture, index: usize) -> Result<(), Error> {
    let source = fx_bundle::images::source_of(tex).unwrap_or("?");
    if bundle.image(index).is_none() {
        return Err(unsupported(format!("textura {:?}: o bundle não tem a imagem de {source:?}", tex.name)));
    }
    if convert::pack_pixels(tex.format, &[0; 4]).is_none() {
        return Err(unsupported(format!("textura {:?}: formato {:?} a partir de um arquivo", tex.name, tex.format)));
    }
    Ok(())
}

/// Para cada textura de arquivo, se os samplers que a leem pedem sRGB. Os samplers de uma mesma textura têm de
/// concordar: a imagem tem um formato só.
fn file_texture_srgb(m: &fx_bundle::Manifest) -> Result<HashMap<usize, bool>, Error> {
    let mut out: HashMap<usize, bool> = HashMap::new();
    for s in &m.samplers {
        let Some((index, tex)) = m.textures.iter().enumerate().find(|(_, t)| t.unique_name == s.texture) else { continue };
        if tex.semantic.is_some() || tex.render_target {
            continue;
        }
        if let Some(previous) = out.insert(index, s.srgb)
            && previous != s.srgb
        {
            return Err(unsupported(format!("textura {:?}: lida com e sem sRGB", tex.name)));
        }
    }
    Ok(out)
}

/// Textura que um sampler amostra: o frame (cópia) ou uma textura intermediária.
fn sampled_texture<'a>(bundle: &'a Bundle, sampler: u32) -> Result<(&'a Texture, usize), Error> {
    let m = &bundle.manifest;
    let s = m.samplers.get(sampler as usize).ok_or_else(|| Error::Invalid(format!("sampler {sampler}")))?;
    let (index, tex) = m
        .textures
        .iter()
        .enumerate()
        .find(|(_, t)| t.unique_name == s.texture)
        .ok_or_else(|| Error::Invalid(format!("textura {:?}", s.texture)))?;
    Ok((tex, index))
}

impl Effect {
    /// Cria o efeito para a técnica `technique` do bundle. Em caso de erro, libera o que já criou.
    ///
    /// `queue` só é usada aqui, uma vez, para inicializar as texturas intermediárias.
    ///
    /// # Safety
    /// `device` válido; `memory` do mesmo dispositivo físico; `target.images` válidas e livres de uso;
    /// `queue` da família `target.queue_family`, sem outro uso concorrente durante a chamada.
    pub unsafe fn new(
        device: &ash::Device,
        memory: &vk::PhysicalDeviceMemoryProperties,
        bundle: &Bundle,
        technique: usize,
        target: &Target<'_>,
        queue: vk::Queue,
    ) -> Result<Self, Error> {
        let mut effect = Self::default();
        if let Err(e) = unsafe { effect.build(device, memory, bundle, technique, target, queue) } {
            unsafe { effect.destroy(device) };
            return Err(e);
        }
        Ok(effect)
    }

    /// Command buffer que processa a imagem `index` do alvo.
    ///
    /// Submissão (feita por quem usa): espere o semáforo do renderizador em `ALL_COMMANDS`.
    pub fn command_buffer(&self, index: usize) -> vk::CommandBuffer {
        self.command_buffers[index]
    }

    /// Escreve `bytes` no buffer de uniforms da imagem `index`, a partir de `offset`. Sem efeito se o
    /// efeito não tem uniforms.
    ///
    /// # Safety
    /// A GPU não deve estar executando o command buffer dessa imagem (espere a fence dela).
    pub unsafe fn write_uniforms(&self, index: usize, offset: usize, bytes: &[u8]) {
        if let Some(ub) = self.uniform_buffers.get(index) {
            unsafe { ub.write(offset, bytes) };
        }
    }

    /// Escreve no buffer de uniforms da imagem `index` os valores que o runtime fornece neste frame
    /// (tempo, número do frame, aleatório...). Chame depois de esperar a fence da imagem e antes de
    /// submeter o command buffer.
    ///
    /// # Safety
    /// A GPU não deve estar executando o command buffer dessa imagem.
    pub unsafe fn update_sources(&mut self, index: usize, frame: &FrameInfo) {
        if self.sources.is_empty() {
            return;
        }
        for (offset, bytes) in self.sources.evaluate(frame) {
            unsafe { self.write_uniforms(index, offset, &bytes) };
        }
    }

    /// Uniforms com `source` que o runtime não alimenta (mouse, teclado, overlay...): ficam com o
    /// valor padrão do manifesto. Útil para a layer registrar no log.
    pub fn ignored_sources(&self) -> &[String] {
        self.sources.ignored()
    }

    unsafe fn build(
        &mut self,
        device: &ash::Device,
        memory: &vk::PhysicalDeviceMemoryProperties,
        bundle: &Bundle,
        technique: usize,
        target: &Target<'_>,
        queue: vk::Queue,
    ) -> Result<(), Error> {
        let m = &bundle.manifest;
        let tech = m.techniques.get(technique).ok_or_else(|| Error::Invalid(format!("técnica {technique} não existe")))?;
        let count = target.images.len();
        if count == 0 {
            return Err(Error::Invalid("alvo sem imagens".into()));
        }
        if !m.y_flipped {
            return Err(unsupported("bundle compilado sem inverter o Y (recompile com o fx2spv atual)"));
        }
        if convert::is_srgb(target.format) {
            return Err(unsupported("alvo em formato sRGB (os efeitos esperam valores já codificados)"));
        }

        // ---- Verificações da v1 (antes de criar qualquer coisa) ----
        let mut samples_color = vec![false; tech.passes.len()];
        let file_srgb = file_texture_srgb(m)?;
        for (k, pass) in tech.passes.iter().enumerate() {
            let writes_frame = pass.render_targets.iter().flatten().next().is_none();
            // Escrever sRGB num alvo de textura (e não no frame) ainda não é suportado.
            if pass.srgb_write && !writes_frame {
                return Err(unsupported(format!("passe {k}: escrita sRGB numa textura")));
            }
            for sb in &pass.samplers {
                let sampler = &m.samplers[sb.sampler as usize];
                let (tex, _) = sampled_texture(bundle, sb.sampler)?;
                if sampler.srgb && tex.render_target {
                    return Err(unsupported(format!("sampler {:?}: leitura sRGB de uma textura de render target", sampler.name)));
                }
                match tex.semantic.as_deref() {
                    Some("COLOR") => samples_color[k] = true,
                    None if tex.render_target => {}
                    None => file_texture_supported(bundle, tex, tex_position(m, tex))?,
                    Some(other) => return Err(unsupported(format!("textura {:?} com semântica {other}", tex.name))),
                }
                // Mipmaps só em texturas de render target (geradas depois de cada passe que as escreve).
                if tex.levels > 1 && tex.semantic.is_some() {
                    return Err(unsupported(format!("textura {:?} com mipmaps", tex.name)));
                }
            }
        }

        // ---- Imagens ----
        self.copy = unsafe { BackbufferCopies::new(device, memory, target.format, target.extent, 1)? };
        if let Some(srgb) = convert::srgb_counterpart(target.format) {
            let mut needs_copy = false;
            let mut needs_shadow = false;
            for pass in &tech.passes {
                needs_shadow |= pass.srgb_write && pass.render_targets.iter().flatten().next().is_none();
                needs_copy |= pass.samplers.iter().any(|sb| m.samplers[sb.sampler as usize].srgb);
            }
            if needs_copy {
                let usage = vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED;
                self.copy_srgb = unsafe { GpuImage::new(device, memory, srgb, target.extent, usage)? };
            }
            if needs_shadow {
                let usage = vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC | vk::ImageUsageFlags::TRANSFER_DST;
                self.frame_srgb = unsafe { GpuImage::new(device, memory, srgb, target.extent, usage)? };
            }
        }

        // Texturas intermediárias: todas as que são render target, para qualquer passe poder usá-las.
        let tex_index: HashMap<&str, usize> = m.textures.iter().enumerate().map(|(i, t)| (t.unique_name.as_str(), i)).collect();
        self.textures = m.textures.iter().map(|_| GpuImage::default()).collect();
        for (i, t) in m.textures.iter().enumerate() {
            if t.semantic.is_none() && !t.render_target {
                // Textura de arquivo: os pixels são enviados em `initialize_textures`.
                let Some(image) = bundle.image(i) else { continue };
                let base = match t.format {
                    TextureFormat::R8 => vk::Format::R8_UNORM,
                    TextureFormat::Rg8 => vk::Format::R8G8_UNORM,
                    _ => vk::Format::R8G8B8A8_UNORM,
                };
                let format = if file_srgb.get(&i).copied().unwrap_or(false) { convert::srgb_counterpart(base).unwrap_or(base) } else { base };
                let extent = vk::Extent2D { width: image.width, height: image.height };
                let mut usage = vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST;
                if t.levels > 1 {
                    usage |= vk::ImageUsageFlags::TRANSFER_SRC;
                }
                self.textures[i] = unsafe { GpuImage::with_levels(device, memory, format, extent, usage, t.levels)? };
            } else if t.semantic.is_none() && t.render_target {
                let extent = vk::Extent2D { width: t.width, height: t.height };
                let mut usage = vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST;
                if t.levels > 1 {
                    usage |= vk::ImageUsageFlags::TRANSFER_SRC; // os níveis seguintes saem de blits do anterior
                }
                self.textures[i] = unsafe { GpuImage::with_levels(device, memory, convert::texture_format(t.format), extent, usage, t.levels)? };
            }
        }

        for &image in target.images {
            let info = vk::ImageViewCreateInfo::default()
                .image(image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(target.format)
                .subresource_range(COLOR_RANGE);
            self.target_views.push(unsafe { device.create_image_view(&info, None)? });
        }

        // ---- Samplers e uniforms ----
        for s in &m.samplers {
            self.samplers.push(unsafe { resources::create_sampler(device, s)? });
        }
        self.sources = Sources::collect(m);
        let has_uniforms = m.total_uniform_size > 0;
        if has_uniforms {
            let defaults = fx_bundle::uniforms::default_block(m);
            for _ in 0..count {
                self.uniform_buffers.push(unsafe { UniformBuffer::new(device, memory, &defaults)? });
            }
        }

        // ---- Descritores ----
        let set0_bindings = [vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)];
        let set0_used: &[vk::DescriptorSetLayoutBinding<'_>] = if has_uniforms { &set0_bindings } else { &[] };
        self.set0_layout = unsafe {
            device.create_descriptor_set_layout(&vk::DescriptorSetLayoutCreateInfo::default().bindings(set0_used), None)?
        };

        let sampler_descriptors: usize = tech.passes.iter().map(|p| p.samplers.len()).sum();
        let sizes = [
            vk::DescriptorPoolSize::default().ty(vk::DescriptorType::UNIFORM_BUFFER).descriptor_count(count.max(1) as u32),
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(sampler_descriptors.max(1) as u32),
        ];
        let max_sets = (if has_uniforms { count } else { 0 } + tech.passes.len()).max(1) as u32;
        self.descriptor_pool = unsafe {
            device.create_descriptor_pool(&vk::DescriptorPoolCreateInfo::default().max_sets(max_sets).pool_sizes(&sizes), None)?
        };
        if has_uniforms {
            let layouts = vec![self.set0_layout; count];
            self.sets0 = unsafe {
                device.allocate_descriptor_sets(
                    &vk::DescriptorSetAllocateInfo::default().descriptor_pool(self.descriptor_pool).set_layouts(&layouts),
                )?
            };
            for (set, ub) in self.sets0.iter().zip(&self.uniform_buffers) {
                let buffer_info = [vk::DescriptorBufferInfo::default().buffer(ub.buffer()).offset(0).range(ub.size() as u64)];
                let write = vk::WriteDescriptorSet::default()
                    .dst_set(*set)
                    .dst_binding(0)
                    .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                    .buffer_info(&buffer_info);
                unsafe { device.update_descriptor_sets(&[write], &[]) };
            }
        }

        // ---- Passes ----
        for (k, pass) in tech.passes.iter().enumerate() {
            let mut res = PassRes { samples_color: samples_color[k], num_vertices: pass.num_vertices, ..PassRes::default() };
            // Só vale com um espelho (alvo de 8 bits); sem versão sRGB do formato, a flag é ignorada como no ReShade.
            res.srgb_shadow = pass.srgb_write && self.frame_srgb.image != vk::Image::null();

            // Para onde escreve.
            let some: Vec<&str> = pass.render_targets.iter().flatten().map(String::as_str).collect();
            let (output, formats, extent) = if some.is_empty() {
                if pass.render_targets.len() > 1 {
                    return Err(unsupported(format!("passe {k}: vários anexos para o frame")));
                }
                let format = if res.srgb_shadow { convert::srgb_counterpart(target.format).unwrap_or(target.format) } else { target.format };
                res.preserves_frame = !pass.clear && (pass.blend.iter().any(|b| b.enable) || pass.viewport != [0, 0]);
                (Output::Frame, vec![format], target.extent)
            } else if some.len() == pass.render_targets.len() {
                let first = &m.textures[tex_index[some[0]]];
                let mut formats = Vec::new();
                for name in &some {
                    let t = &m.textures[tex_index[*name]];
                    if (t.width, t.height) != (first.width, first.height) {
                        return Err(unsupported(format!("passe {k}: anexos de tamanhos diferentes")));
                    }
                    formats.push(convert::texture_format(t.format));
                }
                (Output::Textures, formats, vk::Extent2D { width: first.width, height: first.height })
            } else {
                return Err(unsupported(format!("passe {k}: mistura o frame com texturas nos anexos")));
            };
            res.output = output;
            if output == Output::Textures {
                for name in &some {
                    let (t, image) = (&m.textures[tex_index[*name]], &self.textures[tex_index[*name]]);
                    if image.levels > 1 {
                        let linear = !matches!(t.format, TextureFormat::R32f | TextureFormat::Rg32f | TextureFormat::Rgba32f);
                        res.mips.push(MipChain { image: image.image, extent: vk::Extent2D { width: t.width, height: t.height }, levels: image.levels, linear });
                    }
                }
            }
            res.extent = extent;
            res.attachment_count = formats.len();
            res.viewport = if pass.viewport == [0, 0] { [extent.width, extent.height] } else { pass.viewport };

            res.render_pass = unsafe { pipeline::create_render_pass(device, pass, output, &formats)? };
            // Guarda já no vetor: se algo adiante falhar, o `destroy` libera o que existe.
            self.passes.push(res);
            let res = self.passes.last_mut().expect("acabou de entrar");

            // Descritores do passe (set 1): um COMBINED_IMAGE_SAMPLER por sampler usado.
            let bindings: Vec<_> = pass
                .samplers
                .iter()
                .map(|sb| {
                    vk::DescriptorSetLayoutBinding::default()
                        .binding(sb.binding)
                        .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                        .descriptor_count(1)
                        .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)
                })
                .collect();
            res.set1_layout =
                unsafe { device.create_descriptor_set_layout(&vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings), None)? };
            let layouts = [self.set0_layout, res.set1_layout];
            res.pipeline_layout =
                unsafe { device.create_pipeline_layout(&vk::PipelineLayoutCreateInfo::default().set_layouts(&layouts), None)? };
            let set_layouts = [res.set1_layout];
            res.set1 = unsafe {
                device.allocate_descriptor_sets(
                    &vk::DescriptorSetAllocateInfo::default().descriptor_pool(self.descriptor_pool).set_layouts(&set_layouts),
                )?[0]
            };
            for sb in &pass.samplers {
                let (tex, index) = sampled_texture(bundle, sb.sampler)?;
                let view = match (tex.semantic.as_deref(), m.samplers[sb.sampler as usize].srgb) {
                    (Some("COLOR"), true) if self.copy_srgb.image != vk::Image::null() => self.copy_srgb.view,
                    (Some("COLOR"), _) => self.copy.images[0].view,
                    _ => self.textures[index].view,
                };
                let image_info = [vk::DescriptorImageInfo::default()
                    .sampler(self.samplers[sb.sampler as usize])
                    .image_view(view)
                    .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
                let write = vk::WriteDescriptorSet::default()
                    .dst_set(res.set1)
                    .dst_binding(sb.binding)
                    .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                    .image_info(&image_info);
                unsafe { device.update_descriptor_sets(&[write], &[]) };
            }

            let vertex = bundle.shader(&pass.vertex_entry).ok_or_else(|| Error::Invalid(format!("sem SPIR-V de {:?}", pass.vertex_entry)))?;
            let pixel = bundle.shader(&pass.pixel_entry).ok_or_else(|| Error::Invalid(format!("sem SPIR-V de {:?}", pass.pixel_entry)))?;
            res.pipeline = unsafe {
                pipeline::create_pipeline(device, pass, res.render_pass, res.pipeline_layout, vertex, pixel, formats.len())?
            };

            // Framebuffers.
            match output {
                Output::Frame if res.srgb_shadow => {
                    let attachments = [self.frame_srgb.view];
                    let info = vk::FramebufferCreateInfo::default()
                        .render_pass(res.render_pass)
                        .attachments(&attachments)
                        .width(extent.width)
                        .height(extent.height)
                        .layers(1);
                    res.framebuffers.push(unsafe { device.create_framebuffer(&info, None)? });
                }
                Output::Frame => {
                    for view in &self.target_views {
                        let attachments = [*view];
                        let info = vk::FramebufferCreateInfo::default()
                            .render_pass(res.render_pass)
                            .attachments(&attachments)
                            .width(extent.width)
                            .height(extent.height)
                            .layers(1);
                        res.framebuffers.push(unsafe { device.create_framebuffer(&info, None)? });
                    }
                }
                Output::Textures => {
                    let views: Vec<vk::ImageView> = some.iter().map(|name| self.textures[tex_index[*name]].attachment_view()).collect();
                    let info = vk::FramebufferCreateInfo::default()
                        .render_pass(res.render_pass)
                        .attachments(&views)
                        .width(extent.width)
                        .height(extent.height)
                        .layers(1);
                    res.framebuffers.push(unsafe { device.create_framebuffer(&info, None)? });
                }
            }
        }

        // ---- Command buffers ----
        self.command_pool = unsafe {
            device.create_command_pool(&vk::CommandPoolCreateInfo::default().queue_family_index(target.queue_family), None)?
        };
        self.command_buffers = unsafe {
            device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(self.command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(count as u32),
            )?
        };
        if let Some(init) = target.init_command_buffer {
            for &cb in &self.command_buffers {
                init(cb)?;
            }
        }

        unsafe { self.initialize_textures(device, memory, bundle, target, queue)? };

        for (i, &cb) in self.command_buffers.iter().enumerate() {
            let rec = record::Recording {
                device,
                passes: &self.passes,
                copy: self.copy.images[0].image,
                copy_srgb: (self.copy_srgb.image != vk::Image::null()).then_some(self.copy_srgb.image),
                shadow: (self.frame_srgb.image != vk::Image::null()).then_some(self.frame_srgb.image),
                frame: target.images[i],
                frame_extent: target.extent,
                rest_layout: target.rest_layout,
                set0: self.sets0.get(i).copied(),
            };
            unsafe { record::record(&rec, cb, i)? };
        }
        Ok(())
    }

    /// Leva as texturas ao layout de repouso (`SHADER_READ_ONLY_OPTIMAL`): as intermediárias zeradas, as de arquivo
    /// com os pixels do bundle. Uma submissão única, no momento da criação.
    unsafe fn initialize_textures(
        &self,
        device: &ash::Device,
        memory: &vk::PhysicalDeviceMemoryProperties,
        bundle: &Bundle,
        target: &Target<'_>,
        queue: vk::Queue,
    ) -> Result<(), Error> {
        let m = &bundle.manifest;
        let mut clear: Vec<vk::Image> = Vec::new();
        let mut upload: Vec<usize> = Vec::new();
        for (i, (image, t)) in self.textures.iter().zip(&m.textures).enumerate() {
            if image.image == vk::Image::null() {
                continue;
            }
            if t.render_target {
                clear.push(image.image);
            } else {
                upload.push(i);
            }
        }
        if clear.is_empty() && upload.is_empty() {
            return Ok(());
        }
        let mut staging: Vec<resources::UniformBuffer> = Vec::new();
        let result = unsafe { self.record_texture_init(device, memory, bundle, target, queue, &clear, &upload, &mut staging) };
        for mut buffer in staging {
            unsafe { buffer.destroy(device) };
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn record_texture_init(
        &self,
        device: &ash::Device,
        memory: &vk::PhysicalDeviceMemoryProperties,
        bundle: &Bundle,
        target: &Target<'_>,
        queue: vk::Queue,
        clear: &[vk::Image],
        upload: &[usize],
        staging: &mut Vec<resources::UniformBuffer>,
    ) -> Result<(), Error> {
        let barrier = |image: vk::Image, range, old, new, src: vk::AccessFlags, dst: vk::AccessFlags| {
            vk::ImageMemoryBarrier::default()
                .image(image)
                .old_layout(old)
                .new_layout(new)
                .src_access_mask(src)
                .dst_access_mask(dst)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .subresource_range(range)
        };
        use vk::ImageLayout as L;
        unsafe {
            let cb = device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(self.command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )?[0];
            if let Some(init) = target.init_command_buffer {
                init(cb)?;
            }
            device.begin_command_buffer(cb, &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT))?;

            // Tudo vai a TRANSFER_DST de uma vez.
            let all: Vec<vk::Image> = clear.iter().copied().chain(upload.iter().map(|&i| self.textures[i].image)).collect();
            let to_dst: Vec<_> = all.iter().map(|&im| barrier(im, ALL_LEVELS, L::UNDEFINED, L::TRANSFER_DST_OPTIMAL, vk::AccessFlags::empty(), vk::AccessFlags::TRANSFER_WRITE)).collect();
            device.cmd_pipeline_barrier(cb, vk::PipelineStageFlags::TOP_OF_PIPE, vk::PipelineStageFlags::TRANSFER, vk::DependencyFlags::empty(), &[], &[], &to_dst);

            for &image in clear {
                device.cmd_clear_color_image(cb, image, L::TRANSFER_DST_OPTIMAL, &vk::ClearColorValue { float32: [0.0; 4] }, &[ALL_LEVELS]);
            }
            let mut mipped: Vec<MipChain> = Vec::new();
            for &i in upload {
                let (t, gpu) = (&bundle.manifest.textures[i], &self.textures[i]);
                let image = bundle.image(i).expect("só há imagem GPU para textura com imagem");
                let pixels = convert::pack_pixels(t.format, &image.rgba).ok_or_else(|| unsupported(format!("textura {:?}", t.name)))?;
                staging.push(resources::UniformBuffer::staging(device, memory, &pixels)?);
                let region = vk::BufferImageCopy::default()
                    .image_subresource(vk::ImageSubresourceLayers { aspect_mask: vk::ImageAspectFlags::COLOR, mip_level: 0, base_array_layer: 0, layer_count: 1 })
                    .image_extent(vk::Extent3D { width: image.width, height: image.height, depth: 1 });
                device.cmd_copy_buffer_to_image(cb, staging.last().expect("acabou de entrar").buffer(), gpu.image, L::TRANSFER_DST_OPTIMAL, &[region]);
                if gpu.levels > 1 {
                    mipped.push(MipChain { image: gpu.image, extent: vk::Extent2D { width: image.width, height: image.height }, levels: gpu.levels, linear: true });
                }
            }

            // Para a leitura: o que não tem mipmaps vai direto; o que tem passa pelo gerador (que espera o nível 0 lido).
            let mut to_read: Vec<_> = Vec::new();
            for &image in clear {
                to_read.push(barrier(image, ALL_LEVELS, L::TRANSFER_DST_OPTIMAL, L::SHADER_READ_ONLY_OPTIMAL, vk::AccessFlags::TRANSFER_WRITE, vk::AccessFlags::SHADER_READ));
            }
            for &i in upload {
                let gpu = &self.textures[i];
                let range = if gpu.levels > 1 { COLOR_RANGE } else { ALL_LEVELS };
                to_read.push(barrier(gpu.image, range, L::TRANSFER_DST_OPTIMAL, L::SHADER_READ_ONLY_OPTIMAL, vk::AccessFlags::TRANSFER_WRITE, vk::AccessFlags::SHADER_READ));
            }
            device.cmd_pipeline_barrier(
                cb,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::FRAGMENT_SHADER | vk::PipelineStageFlags::VERTEX_SHADER | vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &to_read,
            );
            for chain in &mipped {
                record::generate_mips(device, cb, chain);
            }
            device.end_command_buffer(cb)?;
            let cbs = [cb];
            device.queue_submit(queue, &[vk::SubmitInfo::default().command_buffers(&cbs)], vk::Fence::null())?;
            device.queue_wait_idle(queue)?;
            device.free_command_buffers(self.command_pool, &cbs);
        }
        Ok(())
    }

    /// Libera tudo. Pode ser chamado mais de uma vez.
    ///
    /// # Safety
    /// A GPU não pode estar executando os command buffers deste efeito.
    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        unsafe {
            if self.command_pool != vk::CommandPool::null() {
                device.destroy_command_pool(std::mem::take(&mut self.command_pool), None); // libera os command buffers
            }
            self.command_buffers.clear();
            for mut pass in self.passes.drain(..) {
                for f in pass.framebuffers.drain(..) {
                    device.destroy_framebuffer(f, None);
                }
                if pass.pipeline != vk::Pipeline::null() {
                    device.destroy_pipeline(pass.pipeline, None);
                }
                if pass.pipeline_layout != vk::PipelineLayout::null() {
                    device.destroy_pipeline_layout(pass.pipeline_layout, None);
                }
                if pass.set1_layout != vk::DescriptorSetLayout::null() {
                    device.destroy_descriptor_set_layout(pass.set1_layout, None);
                }
                if pass.render_pass != vk::RenderPass::null() {
                    device.destroy_render_pass(pass.render_pass, None);
                }
            }
            if self.descriptor_pool != vk::DescriptorPool::null() {
                device.destroy_descriptor_pool(std::mem::take(&mut self.descriptor_pool), None); // libera os sets
            }
            self.sets0.clear();
            if self.set0_layout != vk::DescriptorSetLayout::null() {
                device.destroy_descriptor_set_layout(std::mem::take(&mut self.set0_layout), None);
            }
            for s in self.samplers.drain(..) {
                device.destroy_sampler(s, None);
            }
            for mut ub in self.uniform_buffers.drain(..) {
                ub.destroy(device);
            }
            for mut t in self.textures.drain(..) {
                t.destroy(device);
            }
            for v in self.target_views.drain(..) {
                device.destroy_image_view(v, None);
            }
            self.copy.destroy(device);
            self.copy_srgb.destroy(device);
            self.frame_srgb.destroy(device);
        }
    }
}
