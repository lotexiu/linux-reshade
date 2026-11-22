//! Os ganchos Vulkan do observador de profundidade. Cada um repassa a chamada ao próximo elo e, se o observador
//! está ligado, anota o que importa. Nada aqui muda o que o jogo envia.

use super::observe::{ImageInfo, LoadOp, PassRecord, StoreOp};
use super::{Compare, CmdState, RenderPassInfo, compare_of, is_depth_format, load_op, store_op};
use crate::log::log;
use crate::proc_addr::erase;
use crate::state::{DEVICES, DeviceData};
use ash::vk::{self, Handle};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use vk_layer::dispatch::dispatch_key;

/// Uma referência ao dado do dispositivo sem contar referências (cada gancho de desenho pagava duas operações atômicas e uma
/// busca num mapa). Vale enquanto o dispositivo existe, o que o Vulkan já exige de quem usa o command buffer.
pub(super) struct DeviceRef(*const DeviceData);

impl std::ops::Deref for DeviceRef {
    type Target = DeviceData;

    fn deref(&self) -> &DeviceData {
        // SAFETY: o registro mantém o `Arc` vivo até `vkDestroyDevice`, e usar um command buffer depois disso é erro do jogo.
        unsafe { &*self.0 }
    }
}

thread_local! {
    /// O último dispositivo visto por esta thread: `(chave de dispatch, geração, dado)`.
    static LAST_DEVICE: std::cell::RefCell<Option<(usize, u64, Arc<DeviceData>)>> = const { std::cell::RefCell::new(None) };
}

fn data_of(raw: u64) -> Option<DeviceRef> {
    // SAFETY: `raw` é um handle despachável entregue pelo loader.
    let key = unsafe { dispatch_key(raw) };
    let generation = crate::state::DEVICES_GENERATION.load(Ordering::Acquire);
    LAST_DEVICE.with(|cell| {
        let mut cache = cell.borrow_mut();
        if let Some((k, g, data)) = cache.as_ref()
            && *k == key
            && *g == generation
        {
            return Some(DeviceRef(Arc::as_ptr(data)));
        }
        let data = DEVICES.get(key)?;
        let reference = DeviceRef(Arc::as_ptr(&data));
        *cache = Some((key, generation, data));
        Some(reference)
    })
}

/// O gancho de um comando de profundidade, pelo nome (e pelos apelidos de extensão).
pub fn intercept_name(name: &[u8]) -> vk::PFN_vkVoidFunction {
    match name {
        b"vkCreateImage" => erase(create_image as vk::PFN_vkCreateImage),
        b"vkDestroyImage" => erase(destroy_image as vk::PFN_vkDestroyImage),
        b"vkCreateImageView" => erase(create_image_view as vk::PFN_vkCreateImageView),
        b"vkCreateFramebuffer" => erase(create_framebuffer as vk::PFN_vkCreateFramebuffer),
        b"vkCreateRenderPass" => erase(create_render_pass as vk::PFN_vkCreateRenderPass),
        b"vkCreateRenderPass2" | b"vkCreateRenderPass2KHR" => erase(create_render_pass2 as vk::PFN_vkCreateRenderPass2),
        b"vkCreateGraphicsPipelines" => erase(create_graphics_pipelines as vk::PFN_vkCreateGraphicsPipelines),
        b"vkBeginCommandBuffer" => erase(begin_command_buffer as vk::PFN_vkBeginCommandBuffer),
        b"vkCmdBeginRenderPass" => erase(cmd_begin_render_pass as vk::PFN_vkCmdBeginRenderPass),
        b"vkCmdBeginRenderPass2" | b"vkCmdBeginRenderPass2KHR" => erase(cmd_begin_render_pass2 as vk::PFN_vkCmdBeginRenderPass2),
        b"vkCmdEndRenderPass" => erase(cmd_end_render_pass as vk::PFN_vkCmdEndRenderPass),
        b"vkCmdEndRenderPass2" | b"vkCmdEndRenderPass2KHR" => erase(cmd_end_render_pass2 as vk::PFN_vkCmdEndRenderPass2),
        b"vkCmdBeginRendering" | b"vkCmdBeginRenderingKHR" => erase(cmd_begin_rendering as vk::PFN_vkCmdBeginRendering),
        b"vkCmdEndRendering" | b"vkCmdEndRenderingKHR" => erase(cmd_end_rendering as vk::PFN_vkCmdEndRendering),
        b"vkCmdDraw" => erase(cmd_draw as vk::PFN_vkCmdDraw),
        b"vkCmdDrawIndexed" => erase(cmd_draw_indexed as vk::PFN_vkCmdDrawIndexed),
        b"vkCmdDrawIndirect" => erase(cmd_draw_indirect as vk::PFN_vkCmdDrawIndirect),
        b"vkCmdDrawIndexedIndirect" => erase(cmd_draw_indexed_indirect as vk::PFN_vkCmdDrawIndexedIndirect),
        b"vkCmdDrawIndirectCount" | b"vkCmdDrawIndirectCountKHR" => erase(cmd_draw_indirect_count as vk::PFN_vkCmdDrawIndirectCount),
        b"vkCmdDrawIndexedIndirectCount" | b"vkCmdDrawIndexedIndirectCountKHR" => {
            erase(cmd_draw_indexed_indirect_count as vk::PFN_vkCmdDrawIndexedIndirectCount)
        }
        b"vkCmdBindPipeline" => erase(cmd_bind_pipeline as vk::PFN_vkCmdBindPipeline),
        b"vkCmdPipelineBarrier" => erase(cmd_pipeline_barrier as vk::PFN_vkCmdPipelineBarrier),
        b"vkCmdPipelineBarrier2" | b"vkCmdPipelineBarrier2KHR" => erase(cmd_pipeline_barrier2 as vk::PFN_vkCmdPipelineBarrier2),
        b"vkCmdExecuteCommands" => erase(cmd_execute_commands as vk::PFN_vkCmdExecuteCommands),
        b"vkCmdSetDepthCompareOp" | b"vkCmdSetDepthCompareOpEXT" => erase(cmd_set_depth_compare_op as vk::PFN_vkCmdSetDepthCompareOp),
        b"vkCmdClearAttachments" => erase(cmd_clear_attachments as vk::PFN_vkCmdClearAttachments),
        b"vkCmdClearDepthStencilImage" => erase(cmd_clear_depth_stencil_image as vk::PFN_vkCmdClearDepthStencilImage),
        b"vkCmdBlitImage" => erase(cmd_blit_image as vk::PFN_vkCmdBlitImage),
        b"vkCmdCopyImage" => erase(cmd_copy_image as vk::PFN_vkCmdCopyImage),
        b"vkCmdResolveImage" => erase(cmd_resolve_image as vk::PFN_vkCmdResolveImage),
        b"vkCmdClearColorImage" => erase(cmd_clear_color_image as vk::PFN_vkCmdClearColorImage),
        b"vkCmdBeginQuery" => erase(cmd_begin_query as vk::PFN_vkCmdBeginQuery),
        b"vkCmdEndQuery" => erase(cmd_end_query as vk::PFN_vkCmdEndQuery),
        b"vkCmdBeginQueryIndexedEXT" => erase(cmd_begin_query_indexed as vk::PFN_vkCmdBeginQueryIndexedEXT),
        b"vkCmdEndQueryIndexedEXT" => erase(cmd_end_query_indexed as vk::PFN_vkCmdEndQueryIndexedEXT),
        b"vkCmdBeginConditionalRenderingEXT" => erase(cmd_begin_conditional_rendering as vk::PFN_vkCmdBeginConditionalRenderingEXT),
        b"vkCmdEndConditionalRenderingEXT" => erase(cmd_end_conditional_rendering as vk::PFN_vkCmdEndConditionalRenderingEXT),
        b"vkCmdBeginTransformFeedbackEXT" => erase(cmd_begin_transform_feedback as vk::PFN_vkCmdBeginTransformFeedbackEXT),
        b"vkCmdEndTransformFeedbackEXT" => erase(cmd_end_transform_feedback as vk::PFN_vkCmdEndTransformFeedbackEXT),
        b"vkQueueSubmit" => erase(queue_submit as vk::PFN_vkQueueSubmit),
        b"vkQueueSubmit2" | b"vkQueueSubmit2KHR" => erase(queue_submit2 as vk::PFN_vkQueueSubmit2),
        _ => None,
    }
}

mod barriers;
mod clears;
mod copies;
mod draws;
mod frame;
mod objects;
mod passes;

pub use frame::{destroy, end_frame};

use barriers::*;
use clears::*;
use copies::*;
use draws::*;
use frame::*;
use objects::*;
use passes::*;

/// `snapshot_pass` com este valor: a foto sai depois de cada passe da imagem (a do último fica).
const ALL_PASSES: u32 = u32::MAX;

/// Quantos quadros seguidos o desafiante precisa vencer para a candidata de profundidade em uso mudar.
const STICKY_SWITCH_FRAMES: u32 = 120;

/// Quantos quadros seguidos a limpeza da cena precisa mudar para a escolha automática mudar.
const AUTO_CLEAR_SWITCH_FRAMES: u32 = 8;

/// Quantos quadros seguidos sem um passe que recomeça a imagem fazem a regra automática largar a foto.
const AUTO_SCENE_FORGET_FRAMES: u32 = 600;
