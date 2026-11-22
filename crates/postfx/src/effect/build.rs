//! A construção de um efeito: texturas, passes de desenho e de compute, pipelines e a inicialização dos recursos na GPU.

use super::*;

impl Effect {
    pub(super) unsafe fn build(
        &mut self,
        device: &ash::Device,
        memory: &vk::PhysicalDeviceMemoryProperties,
        bundle: &Bundle,
        technique: usize,
        target: &Target<'_>,
        queue: Option<vk::Queue>,
        mut shared: Option<&mut Shared>,
    ) -> Result<(), Error> {
        let m = &bundle.manifest;
        let tech = m.techniques.get(technique).ok_or_else(|| Error::Invalid(format!("technique {technique} does not exist")))?;
        let count = target.images.len();
        if count == 0 {
            return Err(Error::Invalid("alvo sem imagens".into()));
        }
        if !m.y_flipped {
            return Err(unsupported("bundle compiled without flipping Y (recompile with the current fx2spv)"));
        }
        // Um alvo sRGB (o swapchain de muitos jogos): os efeitos esperam os valores já codificados, como num UNORM.
        // Trabalha-se numa imagem UNORM própria e os bytes entram e saem do alvo por cópia, que não converte.
        let working_format = match convert::unorm_counterpart(target.format) {
            Some(unorm) => unorm,
            None if convert::is_srgb(target.format) => return Err(unsupported("sRGB target format without a UNORM equivalent")),
            None => target.format,
        };
        let through_work = working_format != target.format;

        // ---- Verificações da v1 (antes de criar qualquer coisa) ----
        let mut samples_color = vec![false; tech.passes.len()];
        let mut samples_depth = vec![false; tech.passes.len()];
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
                if sampler.srgb && tex.semantic.is_none() && !is_file(tex) {
                    return Err(unsupported(format!("sampler {:?}: sRGB read of a render target texture", sampler.name)));
                }
                match tex.semantic.as_deref() {
                    Some("COLOR") => samples_color[k] = true,
                    Some("DEPTH") => {
                        let Some(depth) = &target.depth else {
                            return Err(unsupported(format!("pass {k}: samples the depth and this game does not offer it (or it has not been found yet)")));
                        };
                        if depth.layout == vk::ImageLayout::UNDEFINED {
                            return Err(unsupported(format!("pass {k}: samples the depth, but the image layout is unknown")));
                        }
                        samples_depth[k] = true;
                    }
                    None if !is_file(tex) => {}
                    None => file_texture_supported(bundle, tex, tex_position(m, tex))?,
                    Some(other) => return Err(unsupported(format!("texture {:?} with semantic {other}", tex.name))),
                }
                // Mipmaps só em texturas de render target (geradas depois de cada passe que as escreve).
                if tex.levels > 1 && tex.semantic.is_some() {
                    return Err(unsupported(format!("texture {:?} with mipmaps", tex.name)));
                }
            }
        }

        // ---- Imagens ----
        self.copy = unsafe { BackbufferCopies::new(device, memory, working_format, target.extent, 1)? };
        if let Some(srgb) = convert::srgb_counterpart(working_format) {
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
        // Texturas que um passe de compute escreve (`storage2D`) e as que algum passe desta técnica amostra.
        let stored: Vec<bool> = m.textures.iter().map(|t| m.storages.iter().any(|st| st.texture == t.unique_name)).collect();
        let mut referenced = vec![false; m.textures.len()];
        for pass in &tech.passes {
            for sb in &pass.samplers {
                if let Some(s) = m.samplers.get(sb.sampler as usize)
                    && let Some(i) = tex_index.get(m.samplers[sb.sampler as usize].texture.as_str())
                {
                    let _ = s;
                    referenced[*i] = true;
                }
            }
        }
        self.init_here = vec![false; m.textures.len()];
        for (i, t) in m.textures.iter().enumerate() {
            if t.semantic.is_some() {
                continue;
            }
            if is_file(t) {
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
                continue;
            }
            // Intermediária: alvo de renderização, de armazenamento ou só lida (às vezes escrita por outra etapa da cadeia).
            if !(t.render_target || stored[i] || referenced[i]) {
                continue;
            }
            let desc = Desc::of(t);
            if let Some(shared) = shared.as_deref()
                && let Some(image) = shared.find(&t.unique_name, &desc)
            {
                self.textures[i] = image;
                continue;
            }
            let uses = shared.as_deref().map(|s| s.uses(&t.unique_name)).unwrap_or_default();
            let extent = vk::Extent2D { width: t.width, height: t.height };
            let mut usage = vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST;
            if t.render_target || uses.render_target {
                usage |= vk::ImageUsageFlags::COLOR_ATTACHMENT;
            }
            if stored[i] || uses.stored {
                usage |= vk::ImageUsageFlags::STORAGE;
            }
            if t.levels > 1 {
                usage |= vk::ImageUsageFlags::TRANSFER_SRC; // os níveis seguintes saem de blits do anterior
            }
            let dims = match t.kind {
                fx_bundle::manifest::TextureType::D1 => Dims::D1,
                fx_bundle::manifest::TextureType::D2 => Dims::D2,
                fx_bundle::manifest::TextureType::D3 => Dims::D3(t.depth),
            };
            let image = unsafe { GpuImage::with_dims(device, memory, convert::texture_format(t.format), dims, extent, usage, t.levels)? };
            self.textures[i] = match shared.as_deref_mut() {
                Some(shared) => shared.insert(&t.unique_name, desc, image),
                None => image,
            };
            self.init_here[i] = true;
        }

        if through_work {
            let usage = vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC | vk::ImageUsageFlags::TRANSFER_DST;
            self.work = unsafe { GpuImage::new(device, memory, working_format, target.extent, usage)? };
        } else {
            for &image in target.images {
                let info = vk::ImageViewCreateInfo::default()
                    .image(image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(target.format)
                    .subresource_range(COLOR_RANGE);
                self.target_views.push(unsafe { device.create_image_view(&info, None)? });
            }
        }
        // Onde os passes que escrevem no frame desenham: a view de cada imagem do alvo, ou a imagem de trabalho.
        let frame_views: Vec<vk::ImageView> = if through_work { vec![self.work.view; count] } else { self.target_views.clone() };

        // A profundidade do jogo: uma view só do aspecto de profundidade.
        if let Some(depth) = target.depth.filter(|_| samples_depth.iter().any(|d| *d)) {
            let info = vk::ImageViewCreateInfo::default()
                .image(depth.image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(depth.format)
                .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::DEPTH, ..COLOR_RANGE });
            self.depth_view = unsafe { device.create_image_view(&info, None)? };
            self.depth_barrier = Some(record::DepthBarrier { image: depth.image, stencil: depth_has_stencil(depth.format), layout: depth.layout });
        }

        // ---- Samplers e uniforms ----
        for s in &m.samplers {
            // Amostrar profundidade com filtro linear nem sempre é permitido: vizinho mais próximo (como os efeitos pedem).
            let is_depth = m.textures.iter().any(|t| t.unique_name == s.texture && t.semantic.as_deref() == Some("DEPTH"));
            // Profundidade menor que a tela (FSR, resolução dinâmica): com vizinho mais próximo, cada texel aparece em
            // vários pixels e as diferenças entre pixels vizinhas (normais, bordas) viram degraus e linhas; filtro
            // linear suaviza (se o formato permite). Do mesmo tamanho da tela, vizinho mais próximo, que é exato.
            let upscaled = target.depth.is_some_and(|d| d.linear_filter && d.extent != target.extent);
            let mode = match (is_depth, upscaled) {
                (false, _) => resources::DepthFilter::NotDepth,
                (true, false) => resources::DepthFilter::Nearest,
                (true, true) => resources::DepthFilter::Linear,
            };
            self.samplers.push(unsafe { resources::create_sampler(device, s, mode)? });
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
            .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT | vk::ShaderStageFlags::COMPUTE)];
        let set0_used: &[vk::DescriptorSetLayoutBinding<'_>] = if has_uniforms { &set0_bindings } else { &[] };
        self.set0_layout = unsafe {
            device.create_descriptor_set_layout(&vk::DescriptorSetLayoutCreateInfo::default().bindings(set0_used), None)?
        };

        let sampler_descriptors: usize = tech.passes.iter().map(|p| p.samplers.len()).sum();
        let storage_descriptors: usize = tech.passes.iter().map(|p| p.storages.len()).sum();
        let compute_passes = tech.passes.iter().filter(|p| p.compute_entry.is_some()).count();
        let mut sizes = vec![
            vk::DescriptorPoolSize::default().ty(vk::DescriptorType::UNIFORM_BUFFER).descriptor_count(count.max(1) as u32),
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(sampler_descriptors.max(1) as u32),
        ];
        if storage_descriptors > 0 {
            sizes.push(vk::DescriptorPoolSize::default().ty(vk::DescriptorType::STORAGE_IMAGE).descriptor_count(storage_descriptors as u32));
        }
        let max_sets = (if has_uniforms { count } else { 0 } + tech.passes.len() + compute_passes).max(1) as u32;
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
            if pass.compute_entry.is_some() {
                unsafe { self.build_compute_pass(device, bundle, pass, samples_color[k], samples_depth[k])? };
                continue;
            }
            let mut res = PassRes { samples_color: samples_color[k], samples_depth: samples_depth[k], num_vertices: pass.num_vertices, ..PassRes::default() };
            // Só vale com um espelho (alvo de 8 bits); sem versão sRGB do formato, a flag é ignorada como no ReShade.
            res.srgb_shadow = pass.srgb_write && self.frame_srgb.image != vk::Image::null();

            // Para onde escreve.
            let some: Vec<&str> = pass.render_targets.iter().flatten().map(String::as_str).collect();
            let (output, formats, extent) = if some.is_empty() {
                if pass.render_targets.len() > 1 {
                    return Err(unsupported(format!("pass {k}: several attachments for the frame")));
                }
                let format = if res.srgb_shadow { convert::srgb_counterpart(working_format).unwrap_or(working_format) } else { working_format };
                // Só um passe que limpa dispensa o que havia: sem blend e tela cheia ainda pode dar `discard` (o SMAA do iMMERSE).
                res.preserves_frame = !pass.clear;
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
                    if image.levels > 1 && pass.generate_mipmaps {
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
                let is_depth = tex.semantic.as_deref() == Some("DEPTH");
                let view = match (tex.semantic.as_deref(), m.samplers[sb.sampler as usize].srgb) {
                    (Some("DEPTH"), _) => self.depth_view,
                    (Some("COLOR"), true) if self.copy_srgb.image != vk::Image::null() => self.copy_srgb.view,
                    (Some("COLOR"), _) => self.copy.images[0].view,
                    _ => self.textures[index].view,
                };
                let layout = if is_depth { vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL } else { vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL };
                let image_info = [vk::DescriptorImageInfo::default().sampler(self.samplers[sb.sampler as usize]).image_view(view).image_layout(layout)];
                let write = vk::WriteDescriptorSet::default()
                    .dst_set(res.set1)
                    .dst_binding(sb.binding)
                    .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                    .image_info(&image_info);
                unsafe { device.update_descriptor_sets(&[write], &[]) };
            }

            let vertex = bundle.shader(&pass.vertex_entry).ok_or_else(|| Error::Invalid(format!("no SPIR-V for {:?}", pass.vertex_entry)))?;
            let pixel = bundle.shader(&pass.pixel_entry).ok_or_else(|| Error::Invalid(format!("no SPIR-V for {:?}", pass.pixel_entry)))?;
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
                    for view in &frame_views {
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

        self.pending_init = unsafe { self.initialize_textures(device, memory, bundle, target, queue)? };

        // Diagnóstico: com `LINUX_RESHADE_PROFILE_PASSES` no ambiente, cada passe leva carimbos de tempo (ver `pass_times`).
        if std::env::var_os("LINUX_RESHADE_PROFILE_PASSES").is_some() && self.profile.is_none() {
            let info = vk::QueryPoolCreateInfo::default().query_type(vk::QueryType::TIMESTAMP).query_count(2 * self.passes.len() as u32 + 1);
            if let Ok(pool) = unsafe { device.create_query_pool(&info, None) } {
                let labels = bundle.manifest.techniques.get(technique).map(|t| t.passes.iter().enumerate().map(|(k, p)| format!("{k}: {}", p.compute_entry.as_deref().unwrap_or(&p.pixel_entry))).collect()).unwrap_or_default();
                self.profile = Some(PassProfile { pool, labels });
            }
        }
        for (i, &cb) in self.command_buffers.iter().enumerate() {
            let rec = record::Recording {
                device,
                passes: &self.passes,
                copy: self.copy.images[0].image,
                copy_srgb: (self.copy_srgb.image != vk::Image::null()).then_some(self.copy_srgb.image),
                shadow: (self.frame_srgb.image != vk::Image::null()).then_some(self.frame_srgb.image),
                work: (self.work.image != vk::Image::null()).then_some(self.work.image),
                depth: self.depth_barrier,
                frame: target.images[i],
                frame_extent: target.extent,
                rest_layout: target.rest_layout,
                set0: self.sets0.get(i).copied(),
                profile: self.profile.as_ref().map(|p| p.pool),
            };
            unsafe { record::record(&rec, cb, i)? };
        }
        Ok(())
    }

    /// Cria um passe de compute: sets 1 (samplers) e 2 (imagens de armazenamento), pipeline e despacho.
    pub(super) unsafe fn build_compute_pass(
        &mut self,
        device: &ash::Device,
        bundle: &Bundle,
        pass: &fx_bundle::manifest::Pass,
        samples_color: bool,
        samples_depth: bool,
    ) -> Result<(), Error> {
        let m = &bundle.manifest;
        let entry = pass.compute_entry.as_deref().unwrap_or_default();
        let dispatch = pass.dispatch.ok_or_else(|| Error::Invalid("passe de compute sem tamanho de despacho".into()))?;
        // Como não há render pass, o passe fica como "escreve texturas": não mexe no frame nem invalida a cópia dele.
        self.passes.push(PassRes { samples_color, samples_depth, output: Output::Textures, compute: Some(ComputeRes { dispatch, ..ComputeRes::default() }), ..PassRes::default() });
        let k = self.passes.len() - 1;

        let sampler_bindings: Vec<_> = pass
            .samplers
            .iter()
            .map(|sb| {
                vk::DescriptorSetLayoutBinding::default()
                    .binding(sb.binding)
                    .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                    .descriptor_count(1)
                    .stage_flags(vk::ShaderStageFlags::COMPUTE)
            })
            .collect();
        let storage_bindings: Vec<_> = pass
            .storages
            .iter()
            .map(|sb| {
                vk::DescriptorSetLayoutBinding::default()
                    .binding(sb.binding)
                    .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
                    .descriptor_count(1)
                    .stage_flags(vk::ShaderStageFlags::COMPUTE)
            })
            .collect();
        let set1_layout = unsafe { device.create_descriptor_set_layout(&vk::DescriptorSetLayoutCreateInfo::default().bindings(&sampler_bindings), None)? };
        self.passes[k].set1_layout = set1_layout;
        let set2_layout = unsafe { device.create_descriptor_set_layout(&vk::DescriptorSetLayoutCreateInfo::default().bindings(&storage_bindings), None)? };
        if let Some(c) = self.passes[k].compute.as_mut() {
            c.set2_layout = set2_layout;
        }
        let layouts = [self.set0_layout, set1_layout, set2_layout];
        let pipeline_layout = unsafe { device.create_pipeline_layout(&vk::PipelineLayoutCreateInfo::default().set_layouts(&layouts), None)? };
        self.passes[k].pipeline_layout = pipeline_layout;
        let sets = unsafe {
            device.allocate_descriptor_sets(&vk::DescriptorSetAllocateInfo::default().descriptor_pool(self.descriptor_pool).set_layouts(&[set1_layout, set2_layout]))?
        };
        let (set1, set2) = (sets[0], sets[1]);
        self.passes[k].set1 = set1;
        if let Some(c) = self.passes[k].compute.as_mut() {
            c.set2 = set2;
        }

        for sb in &pass.samplers {
            let (tex, index) = sampled_texture(bundle, sb.sampler)?;
            let is_depth = tex.semantic.as_deref() == Some("DEPTH");
            let view = match (tex.semantic.as_deref(), m.samplers[sb.sampler as usize].srgb) {
                (Some("DEPTH"), _) => self.depth_view,
                (Some("COLOR"), true) if self.copy_srgb.image != vk::Image::null() => self.copy_srgb.view,
                (Some("COLOR"), _) => self.copy.images[0].view,
                _ => self.textures[index].view,
            };
            let layout = if is_depth { vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL } else { vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL };
            let image_info = [vk::DescriptorImageInfo::default().sampler(self.samplers[sb.sampler as usize]).image_view(view).image_layout(layout)];
            let write = vk::WriteDescriptorSet::default().dst_set(set1).dst_binding(sb.binding).descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER).image_info(&image_info);
            unsafe { device.update_descriptor_sets(&[write], &[]) };
        }

        // Uma view (um nível) por imagem de armazenamento.
        let mut written: Vec<vk::Image> = Vec::new();
        for sb in &pass.storages {
            let storage = m.storages.get(sb.storage as usize).ok_or_else(|| Error::Invalid(format!("storage {}", sb.storage)))?;
            let (index, tex) = m
                .textures
                .iter()
                .enumerate()
                .find(|(_, t)| t.unique_name == storage.texture)
                .ok_or_else(|| Error::Invalid(format!("textura {:?}", storage.texture)))?;
            let image = &self.textures[index];
            if image.image == vk::Image::null() {
                return Err(unsupported(format!("storage image {:?}: the texture {:?} has no image", storage.name, tex.name)));
            }
            if storage.level >= image.levels {
                return Err(Error::Invalid(format!("storage {:?}: level {} of a texture with {} levels", storage.name, storage.level, image.levels)));
            }
            let view_type = match tex.kind {
                fx_bundle::manifest::TextureType::D1 => vk::ImageViewType::TYPE_1D,
                fx_bundle::manifest::TextureType::D2 => vk::ImageViewType::TYPE_2D,
                fx_bundle::manifest::TextureType::D3 => vk::ImageViewType::TYPE_3D,
            };
            let info = vk::ImageViewCreateInfo::default()
                .image(image.image)
                .view_type(view_type)
                .format(convert::texture_format(tex.format))
                .subresource_range(vk::ImageSubresourceRange { base_mip_level: storage.level, level_count: 1, ..COLOR_RANGE });
            let view = unsafe { device.create_image_view(&info, None)? };
            self.storage_views.push(view);
            let image_info = [vk::DescriptorImageInfo::default().image_view(view).image_layout(vk::ImageLayout::GENERAL)];
            let write = vk::WriteDescriptorSet::default().dst_set(set2).dst_binding(sb.binding).descriptor_type(vk::DescriptorType::STORAGE_IMAGE).image_info(&image_info);
            unsafe { device.update_descriptor_sets(&[write], &[]) };
            if !written.contains(&image.image) {
                written.push(image.image);
            }
            // Como o ReShade: uma textura com níveis de mipmap que um passe de compute escreve (no nível 0) ganha os níveis
            // seguintes logo depois do passe. Efeitos como o DOF do Marty escrevem a cor de fundo e de frente por compute e depois a
            // leem em níveis mais altos quando o desfoque é grande; sem isso esses níveis ficam vazios e a imagem sai preta.
            if image.levels > 1 && storage.level == 0 && pass.generate_mipmaps && !self.passes[k].mips.iter().any(|c| c.image == image.image) {
                let linear = !matches!(tex.format, TextureFormat::R32f | TextureFormat::Rg32f | TextureFormat::Rgba32f);
                self.passes[k].mips.push(MipChain { image: image.image, extent: vk::Extent2D { width: tex.width, height: tex.height }, levels: image.levels, linear });
            }
        }
        if let Some(c) = self.passes[k].compute.as_mut() {
            c.images = written;
        }

        let code = bundle.shader(entry).ok_or_else(|| Error::Invalid(format!("no SPIR-V for {entry:?}")))?;
        self.passes[k].pipeline = unsafe { pipeline::create_compute_pipeline(device, pipeline_layout, code, entry)? };
        Ok(())
    }

    /// Leva as texturas ao layout de repouso (`SHADER_READ_ONLY_OPTIMAL`): as intermediárias zeradas, as de arquivo
    /// com os pixels do bundle. Uma submissão única, no momento da criação.
    pub(super) unsafe fn initialize_textures(
        &self,
        device: &ash::Device,
        memory: &vk::PhysicalDeviceMemoryProperties,
        bundle: &Bundle,
        target: &Target<'_>,
        queue: Option<vk::Queue>,
    ) -> Result<Option<PendingInit>, Error> {
        let m = &bundle.manifest;
        let mut clear: Vec<vk::Image> = Vec::new();
        let mut upload: Vec<usize> = Vec::new();
        for (i, (image, t)) in self.textures.iter().zip(&m.textures).enumerate() {
            if image.image == vk::Image::null() {
                continue;
            }
            if !is_file(t) {
                // Intermediária: só quem a criou a zera (uma compartilhada já vem inicializada pela etapa anterior).
                if self.init_here.get(i).copied().unwrap_or(false) {
                    clear.push(image.image);
                }
            } else {
                upload.push(i);
            }
        }
        if clear.is_empty() && upload.is_empty() {
            return Ok(None);
        }
        let mut staging: Vec<resources::UniformBuffer> = Vec::new();
        let result = unsafe { self.record_texture_init(device, memory, bundle, target, &clear, &upload, &mut staging) };
        match (result, queue) {
            (Ok(cb), Some(queue)) => {
                let pending = PendingInit { cb, staging };
                unsafe { pending.submit(device, self.command_pool, queue) }?;
                Ok(None)
            }
            (Ok(cb), None) => Ok(Some(PendingInit { cb, staging })),
            (Err(e), _) => {
                for mut buffer in staging {
                    unsafe { buffer.destroy(device) };
                }
                Err(e)
            }
        }
    }

    /// Envia à fila a inicialização das texturas que a criação deixou pendente (se deixou) e espera terminar. Para efeitos
    /// criados sem fila; precisa rodar na thread que usa a fila.
    ///
    /// # Safety
    /// `queue` da família do alvo, sem outro uso concorrente durante a chamada; a GPU não executa este efeito.
    pub unsafe fn finish_init(&mut self, device: &ash::Device, queue: vk::Queue) -> Result<(), Error> {
        match self.pending_init.take() {
            Some(pending) => unsafe { pending.submit(device, self.command_pool, queue) },
            None => Ok(()),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) unsafe fn record_texture_init(
        &self,
        device: &ash::Device,
        memory: &vk::PhysicalDeviceMemoryProperties,
        bundle: &Bundle,
        target: &Target<'_>,
        clear: &[vk::Image],
        upload: &[usize],
        staging: &mut Vec<resources::UniformBuffer>,
    ) -> Result<vk::CommandBuffer, Error> {
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
                let image = bundle.image(i).expect("there is only a GPU image for a texture with an image");
                let pixels = convert::pack_pixels(t.format, &image.rgba).ok_or_else(|| unsupported(format!("texture {:?}", t.name)))?;
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
            Ok(cb)
        }
    }
}
