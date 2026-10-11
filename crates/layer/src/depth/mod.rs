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
mod resolve;
mod screen;
mod snapshot;

pub use hooks::{destroy, end_frame, intercept_name};

use ash::vk;
use observe::{FrameStats, ImageInfo, LoadOp, PassRecord, StoreOp};
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

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
#[derive(Default)]
pub(crate) struct CmdState {
    pub passes: Vec<PassRecord>,
    pub current: Option<PassRecord>,
    pub compare: Compare,
    /// Mudanças de layout de imagens de profundidade, na ordem de gravação (passes e barreiras).
    pub layouts: Vec<(u64, i32)>,
    /// O passe em gravação que desenha na imagem do swapchain, e os que já terminaram.
    pub screen_current: Option<screen::ScreenPass>,
    /// O passe de renderização dinâmica em gravação sobre a imagem de profundidade observada (para poder parti-lo).
    pub rendering: Option<RenderingCopy>,
    pub screen_passes: Vec<screen::ScreenPass>,
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
    pub width: u32,
    pub height: u32,
    /// O formato aceita filtro linear ao ser amostrado.
    pub linear: bool,
}

/// O que a pessoa pediu para a escolha da profundidade (vem de um arquivo em `/dev/shm`, escrito por `lrctl depth`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Pick {
    /// Usa a N-ésima candidata (de 1) da lista publicada em vez da escolhida pela heurística.
    pub candidate: Option<u32>,
    /// Usa o estado da profundidade logo depois do N-ésimo passe (de 1) que a usa: uma cópia é feita nesse ponto.
    pub pass: Option<u32>,
    /// Usa o estado da profundidade logo antes da N-ésima limpeza (de 1) dela no quadro, como o ReShade ("copiar antes de limpar").
    pub clear: Option<u32>,
}

/// Uma limpeza (ou recomeço) da profundidade observada no quadro, na ordem da gravação.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ClearEvent {
    /// Desenhos na imagem desde a limpeza anterior: o conteúdo que a limpeza vai apagar.
    pub draws_before: u32,
    /// Dá para copiar a profundidade antes dela (renderização dinâmica sem estruturas extras, ou fora de um passe).
    pub splittable: bool,
}

/// Uma cópia do `VkRenderingInfo` do passe em gravação, para reabri-lo depois de uma cópia no meio dele.
#[derive(Clone)]
pub(crate) struct RenderingCopy {
    pub flags: vk::RenderingFlags,
    pub render_area: vk::Rect2D,
    pub layer_count: u32,
    pub view_mask: u32,
    pub colors: Vec<vk::RenderingAttachmentInfo<'static>>,
    pub depth: Option<vk::RenderingAttachmentInfo<'static>>,
    pub stencil: Option<vk::RenderingAttachmentInfo<'static>>,
    /// A imagem de profundidade do passe.
    pub image: u64,
}

/// A imagem própria da layer onde se guarda a cópia da profundidade de um passe.
pub(crate) struct Snapshot {
    pub image: vk::Image,
    pub memory: vk::DeviceMemory,
    pub format: vk::Format,
    pub width: u32,
    pub height: u32,
    /// Se a foto é o resultado de um resolve de profundidade MSAA: a view e o framebuffer onde o resolve desenha.
    pub resolve: Option<resolve::ResolveTarget>,
}

pub struct Depth {
    pub enabled: AtomicBool,
    /// Armado (`LINUX_RESHADE_DEPTH=1` ao abrir o jogo): a layer acrescenta `SAMPLED` ao uso das imagens de profundidade
    /// na criação, para os efeitos poderem amostrar a original. Precisa estar armado antes de o jogo criar as imagens.
    pub armed: bool,
    /// Teste (`LINUX_RESHADE_DEPTH_FORCE_STORE=1`): os render passes que descartam a profundidade ao fim passam a guardá-la,
    /// para jogos como o vkcube poderem ser amostrados. Só vale para render passes criados depois de armado.
    pub force_store: bool,
    pub(crate) formats: Mutex<HashMap<i32, u64>>,
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
    pub(crate) screen: screen::Screen,
    pub(crate) cmds: Mutex<HashMap<u64, CmdState>>,
    pub(crate) frame: Mutex<FrameStats>,
    /// A última lista publicada (para só avisar quando muda).
    pub(crate) last_published: Mutex<Vec<observe::Candidate>>,
    /// A candidata escolhida que dá para amostrar (com `SAMPLED` e layout conhecido); `None` se não há.
    pub(crate) chosen: Mutex<Option<Chosen>>,
    /// O que a pessoa pediu (candidata ou passe); vazio = heurística.
    pub(crate) pick: Mutex<Pick>,
    /// A imagem da candidata em uso (para o gancho de fim de passe saber qual observar), e o passe de onde copiar (0 = nenhum).
    pub(crate) watched_image: AtomicU64,
    pub(crate) snapshot_pass: AtomicU32,
    /// Quantos passes da imagem observada já terminaram de ser gravados neste quadro.
    pub(crate) pass_counter: AtomicU32,
    pub(crate) snapshot: Mutex<Option<Snapshot>>,
    /// O passe que resolve a profundidade MSAA (criado quando o jogo usa MSAA).
    pub(crate) resolver: Mutex<Option<resolve::Resolver>>,
    /// Fotos substituídas, ainda vivas por um tempo: command buffers já gravados (e ainda não executados) podem referenciá-las.
    pub(crate) retired: Mutex<Vec<(Snapshot, u32)>>,
    /// A foto já foi gravada ao menos uma vez (antes disso a imagem está sem layout e não pode ser lida).
    pub(crate) snapshot_written: AtomicBool,
    /// Quadros desde a última leitura do arquivo do pedido.
    pub(crate) frames: AtomicU32,
    /// Quantos pipelines (maior, menor, outro) já foram registrados no log.
    pub(crate) pipelines_logged: Mutex<(usize, usize, usize)>,
    /// O último resumo de passes registrado no log.
    pub(crate) passes_logged: Mutex<String>,
    /// Diagnóstico (aparece em `lrctl layers`): quantas vezes a imagem de profundidade escolhida trocou e quantas vezes a
    /// cadeia de efeitos foi reconstruída por causa disso.
    pub(crate) image_changes: AtomicU32,
    pub(crate) chain_rebuilds: AtomicU32,
    /// Presents com e sem o efeito aplicado (um efeito que some em alguns quadros aparece como oscilação).
    pub(crate) fx_presents: [AtomicU32; 2],
    pub(crate) last_image: AtomicU64,
    /// Diagnóstico por quadro: desenhos vistos, os que caíram num passe com profundidade, e limpezas de profundidade fora de
    /// passes (`vkCmdClearAttachments` e `vkCmdClearDepthStencilImage`).
    /// A regra automática do passe da cena, com histerese: `(imagem, passe, quadros seguidos sem passe)`. Trocar entre a foto e a
    /// imagem original a cada poucos quadros reconstruía a cadeia sem parar.
    /// Limpezas de profundidade da imagem observada: o número pedido (0 = nenhum), a contagem do quadro, os desenhos desde a
    /// última e a lista do quadro; e a escolha automática com histerese `(imagem, escolhida, candidata, quadros seguidos)`.
    /// A candidata em uso com histerese: `(imagem em uso, desafiante, quadros seguidos em que o desafiante vence)`. Alguns jogos
    /// (o Roblox) têm várias imagens de profundidade de contagens parecidas e a vencedora trocava de um quadro para outro.
    pub(crate) sticky: Mutex<(u64, u64, u32)>,
    /// A candidata que a pessoa escolheu pelo número, resolvida para a imagem no momento do pedido: `(número pedido, imagem)`. A
    /// lista de candidatas muda de ordem entre quadros (e entre cenas), então o número sozinho trocava de imagem.
    pub(crate) picked_image: Mutex<(u32, u64, (u32, u32, i32))>,
    pub(crate) snapshot_clear: AtomicU32,
    pub(crate) clear_counter: AtomicU32,
    pub(crate) draws_since_event: AtomicU32,
    pub(crate) frame_clears: Mutex<Vec<ClearEvent>>,
    pub(crate) auto_clear: Mutex<(u64, u32, u32, u32)>,
    pub(crate) auto_scene: Mutex<(u64, Option<u32>, u32)>,
    pub(crate) diag: Mutex<String>,
    /// A última mudança da profundidade que reconstruiu a cadeia (diagnóstico).
    pub(crate) last_key_change: Mutex<String>,
    pub(crate) diag_frames: AtomicU32,
    pub(crate) frame_draws: AtomicU32,
    pub(crate) frame_depth_draws: AtomicU32,
    pub(crate) frame_clear_attachments: AtomicU32,
    pub(crate) frame_clear_images: AtomicU32,
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
        // Armado pela variável de ambiente ou pelo marcador que o app de controle deixa para jogos cujo preset usa profundidade.
        let armed = flag("LINUX_RESHADE_DEPTH") || ipc::depth_arm_wanted(&crate::status::game_id());
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
            screen: screen::Screen::default(),
            cmds: Mutex::new(HashMap::new()),
            frame: Mutex::new(FrameStats::default()),
            last_published: Mutex::new(Vec::new()),
            chosen: Mutex::new(None),
            pick: Mutex::new(Pick::default()),
            watched_image: AtomicU64::new(0),
            snapshot_pass: AtomicU32::new(0),
            pass_counter: AtomicU32::new(0),
            snapshot: Mutex::new(None),
            resolver: Mutex::new(None),
            retired: Mutex::new(Vec::new()),
            snapshot_written: AtomicBool::new(false),
            frames: AtomicU32::new(0),
            pipelines_logged: Mutex::new((usize::MAX, 0, 0)),
            passes_logged: Mutex::new(String::new()),
            image_changes: AtomicU32::new(0),
            chain_rebuilds: AtomicU32::new(0),
            fx_presents: [AtomicU32::new(0), AtomicU32::new(0)],
            last_image: AtomicU64::new(0),
            sticky: Mutex::new((0, 0, 0)),
            picked_image: Mutex::new((0, 0, (0, 0, 0))),
            snapshot_clear: AtomicU32::new(0),
            clear_counter: AtomicU32::new(0),
            draws_since_event: AtomicU32::new(0),
            frame_clears: Mutex::new(Vec::new()),
            auto_clear: Mutex::new((0, 0, 0, 0)),
            auto_scene: Mutex::new((0, None, 0)),
            diag: Mutex::new(String::new()),
            last_key_change: Mutex::new(String::new()),
            diag_frames: AtomicU32::new(0),
            frame_draws: AtomicU32::new(0),
            frame_depth_draws: AtomicU32::new(0),
            frame_clear_attachments: AtomicU32::new(0),
            frame_clear_images: AtomicU32::new(0),
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
