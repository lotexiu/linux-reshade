//! Observação da profundidade do jogo (etapa D1 do `docs/depth.md`): só olha, não muda nada no que o jogo desenha.
//!
//! A layer acompanha as imagens de profundidade, os passes de renderização que as usam e os desenhos feitos em
//! cada passe, junta o que os command buffers enviados à fila fizeram num quadro (como o ReShade faz em
//! `vkQueueSubmit`) e, no present, publica as candidatas a "profundidade da cena" para o app de controle.
//!
//! Só trabalha quando ligado (`LINUX_RESHADE_DEPTH_OBSERVE=1` nesta etapa; depois, quando o plano tem uma técnica
//! que lê profundidade): desligado, cada gancho é uma leitura atômica e a chamada segue para o próximo elo.
//! A criação de imagens, views, framebuffers e render passes é sempre anotada: é rara e não dá para recuperar depois.

mod hooks;
pub mod observe;

pub use hooks::{end_frame, intercept_name};

use ash::vk;
use observe::{FrameStats, ImageInfo, LoadOp, PassRecord, StoreOp};
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// Como os pipelines gráficos comparam a profundidade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Compare {
    #[default]
    Unknown,
    Less,
    Greater,
    Other,
}

/// O que um render pass faz com a profundidade.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RenderPassInfo {
    /// Índice do anexo de profundidade (nos anexos do framebuffer).
    pub depth_attachment: u32,
    pub load: LoadOp,
    pub store: StoreOp,
    /// O layout em que o passe deixa o anexo de profundidade (o `finalLayout` do render pass).
    pub final_layout: i32,
}

/// O que se gravou num command buffer.
#[derive(Debug, Default)]
pub(crate) struct CmdState {
    pub passes: Vec<PassRecord>,
    pub current: Option<PassRecord>,
    pub compare: Compare,
    /// Mudanças de layout de imagens de profundidade, na ordem de gravação (passes e barreiras).
    pub layouts: Vec<(u64, i32)>,
}

/// Funções do próximo elo que não estão na tabela 1.0 do `ash` (podem não existir no dispositivo).
#[derive(Default)]
pub(crate) struct NextFns {
    pub create_render_pass2: Option<vk::PFN_vkCreateRenderPass2>,
    pub cmd_begin_render_pass2: Option<vk::PFN_vkCmdBeginRenderPass2>,
    pub cmd_end_render_pass2: Option<vk::PFN_vkCmdEndRenderPass2>,
    pub cmd_begin_rendering: Option<vk::PFN_vkCmdBeginRendering>,
    pub cmd_end_rendering: Option<vk::PFN_vkCmdEndRendering>,
    pub queue_submit2: Option<vk::PFN_vkQueueSubmit2>,
    pub cmd_set_depth_compare_op: Option<vk::PFN_vkCmdSetDepthCompareOp>,
    pub cmd_draw_indirect_count: Option<vk::PFN_vkCmdDrawIndirectCount>,
    pub cmd_draw_indexed_indirect_count: Option<vk::PFN_vkCmdDrawIndexedIndirectCount>,
    pub cmd_pipeline_barrier2: Option<vk::PFN_vkCmdPipelineBarrier2>,
}

/// A profundidade da cena que os efeitos podem usar agora.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chosen {
    pub image: u64,
    pub format: i32,
    /// Layout em que a imagem está depois do último envio à fila.
    pub layout: i32,
}

pub struct Depth {
    pub enabled: AtomicBool,
    /// Armado (`LINUX_RESHADE_DEPTH=1` ao abrir o jogo): a layer acrescenta `SAMPLED` ao uso das imagens de profundidade
    /// na criação, para os efeitos poderem amostrar a original. Precisa estar armado antes de o jogo criar as imagens.
    pub armed: bool,
    /// Teste (`LINUX_RESHADE_DEPTH_FORCE_STORE=1`): os render passes que descartam a profundidade ao fim passam a guardá-la,
    /// para jogos como o vkcube poderem ser amostrados. Só vale para render passes criados depois de armado.
    pub force_store: bool,
    pub(crate) formats: Mutex<HashMap<i32, bool>>,
    /// Layout atual de cada imagem de profundidade (depois do último envio à fila).
    pub(crate) image_layouts: Mutex<HashMap<u64, i32>>,
    pub(crate) next: NextFns,
    pub(crate) images: Mutex<HashMap<u64, ImageInfo>>,
    /// view -> imagem (só views de imagens de profundidade).
    pub(crate) views: Mutex<HashMap<u64, u64>>,
    /// framebuffer -> views dos anexos (só os que têm alguma view de profundidade).
    pub(crate) framebuffers: Mutex<HashMap<u64, Vec<u64>>>,
    pub(crate) render_passes: Mutex<HashMap<u64, RenderPassInfo>>,
    pub(crate) pipelines: Mutex<HashMap<u64, Compare>>,
    pub(crate) cmds: Mutex<HashMap<u64, CmdState>>,
    pub(crate) frame: Mutex<FrameStats>,
    /// A última lista publicada (para só avisar quando muda).
    pub(crate) last_published: Mutex<Vec<observe::Candidate>>,
    /// A candidata escolhida que dá para amostrar (com `SAMPLED` e layout conhecido); `None` se não há.
    pub(crate) chosen: Mutex<Option<Chosen>>,
    /// Quantos pipelines (maior, menor, outro) já foram registrados no log.
    pub(crate) pipelines_logged: Mutex<(usize, usize, usize)>,
    /// O último resumo de passes registrado no log.
    pub(crate) passes_logged: Mutex<String>,
}

/// Carrega do próximo elo a primeira das funções que existir (nome do núcleo e da extensão).
unsafe fn load<T: Copy>(gdpa: vk::PFN_vkGetDeviceProcAddr, device: vk::Device, names: &[&std::ffi::CStr]) -> Option<T> {
    assert_eq!(size_of::<T>(), size_of::<unsafe extern "system" fn()>());
    for name in names {
        if let Some(f) = unsafe { gdpa(device, name.as_ptr()) } {
            // SAFETY: `T` é o tipo de ponteiro de função do comando nomeado (conferido pelo tamanho e pelo uso).
            return Some(unsafe { std::mem::transmute_copy::<unsafe extern "system" fn(), T>(&f) });
        }
    }
    None
}

impl Depth {
    pub fn new(gdpa: vk::PFN_vkGetDeviceProcAddr, device: vk::Device) -> Self {
        let flag = |name: &str| std::env::var_os(name).is_some_and(|v| v != "0");
        let armed = flag("LINUX_RESHADE_DEPTH");
        let force_store = armed && flag("LINUX_RESHADE_DEPTH_FORCE_STORE");
        let on = armed || flag("LINUX_RESHADE_DEPTH_OBSERVE");
        let next = unsafe {
            NextFns {
                create_render_pass2: load(gdpa, device, &[c"vkCreateRenderPass2", c"vkCreateRenderPass2KHR"]),
                cmd_begin_render_pass2: load(gdpa, device, &[c"vkCmdBeginRenderPass2", c"vkCmdBeginRenderPass2KHR"]),
                cmd_end_render_pass2: load(gdpa, device, &[c"vkCmdEndRenderPass2", c"vkCmdEndRenderPass2KHR"]),
                cmd_begin_rendering: load(gdpa, device, &[c"vkCmdBeginRendering", c"vkCmdBeginRenderingKHR"]),
                cmd_end_rendering: load(gdpa, device, &[c"vkCmdEndRendering", c"vkCmdEndRenderingKHR"]),
                queue_submit2: load(gdpa, device, &[c"vkQueueSubmit2", c"vkQueueSubmit2KHR"]),
                cmd_set_depth_compare_op: load(gdpa, device, &[c"vkCmdSetDepthCompareOp", c"vkCmdSetDepthCompareOpEXT"]),
                cmd_draw_indirect_count: load(gdpa, device, &[c"vkCmdDrawIndirectCount", c"vkCmdDrawIndirectCountKHR"]),
                cmd_draw_indexed_indirect_count: load(gdpa, device, &[c"vkCmdDrawIndexedIndirectCount", c"vkCmdDrawIndexedIndirectCountKHR"]),
                cmd_pipeline_barrier2: load(gdpa, device, &[c"vkCmdPipelineBarrier2", c"vkCmdPipelineBarrier2KHR"]),
            }
        };
        Self {
            enabled: AtomicBool::new(on),
            armed,
            force_store,
            formats: Mutex::new(HashMap::new()),
            image_layouts: Mutex::new(HashMap::new()),
            next,
            images: Mutex::new(HashMap::new()),
            views: Mutex::new(HashMap::new()),
            framebuffers: Mutex::new(HashMap::new()),
            render_passes: Mutex::new(HashMap::new()),
            pipelines: Mutex::new(HashMap::new()),
            cmds: Mutex::new(HashMap::new()),
            frame: Mutex::new(FrameStats::default()),
            last_published: Mutex::new(Vec::new()),
            chosen: Mutex::new(None),
            pipelines_logged: Mutex::new((usize::MAX, 0, 0)),
            passes_logged: Mutex::new(String::new()),
        }
    }

    pub fn is_on(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Liga a observação (um efeito do plano precisa de profundidade). Barato e idempotente.
    pub fn need(&self) {
        self.enabled.store(true, Ordering::Relaxed);
    }

    /// A profundidade que os efeitos podem amostrar neste momento.
    pub fn chosen(&self) -> Option<Chosen> {
        *self.chosen.lock().unwrap()
    }
}

/// `VkFormat` de profundidade (com ou sem estêncil)?
pub(crate) fn is_depth_format(format: vk::Format) -> bool {
    matches!(
        format,
        vk::Format::D16_UNORM
            | vk::Format::X8_D24_UNORM_PACK32
            | vk::Format::D32_SFLOAT
            | vk::Format::D16_UNORM_S8_UINT
            | vk::Format::D24_UNORM_S8_UINT
            | vk::Format::D32_SFLOAT_S8_UINT
    )
}

pub(crate) fn load_op(op: vk::AttachmentLoadOp) -> LoadOp {
    match op {
        vk::AttachmentLoadOp::LOAD => LoadOp::Load,
        vk::AttachmentLoadOp::CLEAR => LoadOp::Clear,
        vk::AttachmentLoadOp::DONT_CARE => LoadOp::DontCare,
        _ => LoadOp::None,
    }
}

pub(crate) fn store_op(op: vk::AttachmentStoreOp) -> StoreOp {
    match op {
        vk::AttachmentStoreOp::STORE => StoreOp::Store,
        vk::AttachmentStoreOp::DONT_CARE => StoreOp::DontCare,
        _ => StoreOp::None,
    }
}

pub(crate) fn compare_of(op: vk::CompareOp) -> Compare {
    match op {
        vk::CompareOp::GREATER | vk::CompareOp::GREATER_OR_EQUAL => Compare::Greater,
        vk::CompareOp::LESS | vk::CompareOp::LESS_OR_EQUAL => Compare::Less,
        _ => Compare::Other,
    }
}
