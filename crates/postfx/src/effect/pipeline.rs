//! Render pass e pipeline de um passe do efeito.

use super::convert;
use ash::vk;
use fx_bundle::manifest::Pass;

/// Cache de pipelines do driver (`VkPipelineCache`), um por dispositivo, guardado em arquivo entre execuções.
///
/// Criar os pipelines de uma cadeia custa segundos (o driver compila o SPIR-V para a GPU); com o cache em disco, a
/// segunda vez que o mesmo efeito aparece (outra sessão do jogo, trocar de preset, mudar a resolução) é quase de graça.
/// O driver confere o cabeçalho (GPU, versão do driver) e ignora dados que não são dele, então um arquivo velho ou de
/// outra placa é inofensivo.
pub mod cache {
    use ash::vk::{self, Handle};
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Mutex, OnceLock};

    static PATH: OnceLock<PathBuf> = OnceLock::new();
    static CACHES: Mutex<Option<HashMap<u64, vk::PipelineCache>>> = Mutex::new(None);
    static DIRTY: AtomicBool = AtomicBool::new(false);

    /// Onde o cache fica no disco (a primeira chamada vale). Sem isto, `LINUX_RESHADE_PIPELINE_CACHE` decide; sem nenhum
    /// dos dois, não há cache em disco (o `VkPipelineCache` ainda existe, só na memória do processo).
    pub fn set_path(path: PathBuf) {
        let _ = PATH.set(path);
    }

    fn path() -> Option<PathBuf> {
        PATH.get().cloned().or_else(|| std::env::var_os("LINUX_RESHADE_PIPELINE_CACHE").map(PathBuf::from))
    }

    /// O cache do dispositivo (criado, com os dados do arquivo, na primeira vez). `null` se a criação falhar: o driver
    /// aceita isso e simplesmente não guarda nada.
    pub fn get(device: &ash::Device) -> vk::PipelineCache {
        let mut caches = CACHES.lock().unwrap_or_else(|e| e.into_inner());
        let map = caches.get_or_insert_with(HashMap::new);
        if let Some(c) = map.get(&device.handle().as_raw()) {
            return *c;
        }
        let data = path().and_then(|p| std::fs::read(p).ok()).unwrap_or_default();
        let info = vk::PipelineCacheCreateInfo::default().initial_data(&data);
        // SAFETY: `device` válido; `data` vive até o fim da chamada.
        let cache = unsafe { device.create_pipeline_cache(&info, None) }
            .or_else(|_| unsafe { device.create_pipeline_cache(&vk::PipelineCacheCreateInfo::default(), None) })
            .unwrap_or(vk::PipelineCache::null());
        map.insert(device.handle().as_raw(), cache);
        cache
    }

    /// Avisa que um pipeline novo entrou no cache (o arquivo precisa ser regravado).
    pub(crate) fn touched() {
        DIRTY.store(true, Ordering::Relaxed);
    }

    fn write(path: &Path, data: &[u8]) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension(format!("tmp{}", std::process::id()));
        if std::fs::write(&tmp, data).is_ok() && std::fs::rename(&tmp, path).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }

    /// Grava o cache do dispositivo no arquivo, se algo mudou desde a última gravação.
    pub fn save(device: &ash::Device) {
        let Some(path) = path() else { return };
        if !DIRTY.swap(false, Ordering::Relaxed) {
            return;
        }
        let cache = get(device);
        if cache == vk::PipelineCache::null() {
            return;
        }
        // SAFETY: `cache` é deste `device`.
        if let Ok(data) = unsafe { device.get_pipeline_cache_data(cache) } {
            write(&path, &data);
        }
    }

    /// Grava e destrói o cache do dispositivo (antes de destruí-lo).
    ///
    /// # Safety
    /// Nenhum pipeline está sendo criado neste dispositivo agora.
    pub unsafe fn release(device: &ash::Device) {
        DIRTY.store(true, Ordering::Relaxed);
        save(device);
        let mut caches = CACHES.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = caches.as_mut().and_then(|m| m.remove(&device.handle().as_raw()))
            && c != vk::PipelineCache::null()
        {
            unsafe { device.destroy_pipeline_cache(c, None) };
        }
    }
}

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
    // O passe só pode descartar o conteúdo se limpa. Cobrir a tela com um triângulo sem blend NÃO basta: o shader pode dar
    // `discard` nos pixels que não quer mudar (o SMAA do iMMERSE só escreve onde há borda e conta com o quadro do jogo por baixo),
    // e com DONT_CARE o resto virava preto.
    let load_op = if pass.clear { vk::AttachmentLoadOp::CLEAR } else { vk::AttachmentLoadOp::LOAD };
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
        let result = device.create_graphics_pipelines(cache::get(device), &[info], None);
        cache::touched();
        device.destroy_shader_module(vert, None);
        device.destroy_shader_module(frag, None);
        result.map(|p| p[0]).map_err(|(_, e)| e)
    }
}

/// Cria o pipeline de compute de um passe.
///
/// # Safety
/// `device` válido; `code` é SPIR-V válido com o entry point `entry`.
pub unsafe fn create_compute_pipeline(device: &ash::Device, layout: vk::PipelineLayout, code: &[u32], entry: &str) -> Result<vk::Pipeline, vk::Result> {
    unsafe {
        let module = device.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(code), None)?;
        let name = std::ffi::CString::new(entry).map_err(|_| vk::Result::ERROR_INITIALIZATION_FAILED)?;
        let stage = vk::PipelineShaderStageCreateInfo::default().stage(vk::ShaderStageFlags::COMPUTE).module(module).name(&name);
        let info = vk::ComputePipelineCreateInfo::default().stage(stage).layout(layout);
        let result = device.create_compute_pipelines(cache::get(device), &[info], None);
        cache::touched();
        device.destroy_shader_module(module, None);
        result.map(|p| p[0]).map_err(|(_, e)| e)
    }
}
