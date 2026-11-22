//! Os passes de renderização: o começo e o fim de cada um (clássicos e dinâmicos), de onde saem os registros que a observação usa.

use super::*;

pub(super) unsafe extern "system" fn begin_command_buffer(cmd: vk::CommandBuffer, p_begin_info: *const vk::CommandBufferBeginInfo<'_>) -> vk::Result {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return vk::Result::ERROR_DEVICE_LOST };
        if data.depth.is_on() {
            data.depth.cmds.lock(cmd.as_raw()).insert(cmd.as_raw(), CmdState::default());
        }
        (data.device.fp_v1_0().begin_command_buffer)(cmd, p_begin_info)
    }
}

/// Começa o registro de um passe com profundidade, se o render pass tem uma.
pub(super) fn start_pass(data: &DeviceData, cmd: vk::CommandBuffer, render_pass: vk::RenderPass, views: Option<Vec<u64>>, framebuffer: vk::Framebuffer, area: vk::Rect2D, clear_depth: impl Fn(u32) -> Option<f32>) {
    let rp = data.depth.render_passes.lock().unwrap().get(&render_pass.as_raw()).copied();
    let Some(rp) = rp else { return };
    let views = views.or_else(|| data.depth.framebuffers.lock().unwrap().get(&framebuffer.as_raw()).cloned());
    let Some(view) = views.and_then(|v| v.get(rp.depth_attachment as usize).copied()) else { return };
    let Some(image) = data.depth.views.lock().unwrap().get(&view).copied() else { return };
    let record = PassRecord {
        image,
        load: rp.load,
        store: rp.store,
        clear_depth: if rp.load == LoadOp::Clear { clear_depth(rp.depth_attachment) } else { None },
        area: (area.extent.width, area.extent.height),
        end_layout: rp.final_layout,
        ..PassRecord::default()
    };
    if let Some(state) = data.depth.cmds.lock(cmd.as_raw()).get_mut(&cmd.as_raw()) {
        state.current = Some(record);
    }
}

/// As views dos anexos dadas no início do passe (framebuffer sem imagens), se houver.
pub(super) unsafe fn imageless_views(mut next: *const std::ffi::c_void) -> Option<Vec<u64>> {
    unsafe {
        while !next.is_null() {
            let base = &*(next as *const vk::BaseInStructure<'_>);
            if base.s_type == vk::StructureType::RENDER_PASS_ATTACHMENT_BEGIN_INFO {
                let info = &*(next as *const vk::RenderPassAttachmentBeginInfo<'_>);
                if info.p_attachments.is_null() {
                    return None;
                }
                return Some(std::slice::from_raw_parts(info.p_attachments, info.attachment_count as usize).iter().map(|v| v.as_raw()).collect());
            }
            next = base.p_next as *const std::ffi::c_void;
        }
        None
    }
}

pub(super) unsafe fn on_begin_render_pass(data: &DeviceData, cmd: vk::CommandBuffer, begin: *const vk::RenderPassBeginInfo<'_>) {
    unsafe {
        if !data.depth.is_on() || begin.is_null() {
            return;
        }
        let b = &*begin;
        let clears: &[vk::ClearValue] = if b.p_clear_values.is_null() { &[] } else { std::slice::from_raw_parts(b.p_clear_values, b.clear_value_count as usize) };
        let views = imageless_views(b.p_next);
        if let Some(pass) = data.depth.screen.begin_pass(b.render_pass.as_raw(), b.framebuffer.as_raw(), views.clone())
            && let Some(state) = data.depth.cmds.lock(cmd.as_raw()).get_mut(&cmd.as_raw())
        {
            state.screen_current = Some(pass);
        }
        start_pass(data, cmd, b.render_pass, views, b.framebuffer, b.render_area, |i| clears.get(i as usize).map(|c| c.depth_stencil.depth));
    }
}

pub(super) unsafe extern "system" fn cmd_begin_render_pass(cmd: vk::CommandBuffer, p_begin: *const vk::RenderPassBeginInfo<'_>, contents: vk::SubpassContents) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        on_begin_render_pass(&data, cmd, p_begin);
        (data.device.fp_v1_0().cmd_begin_render_pass)(cmd, p_begin, contents);
    }
}

pub(super) unsafe extern "system" fn cmd_begin_render_pass2(cmd: vk::CommandBuffer, p_begin: *const vk::RenderPassBeginInfo<'_>, p_subpass: *const vk::SubpassBeginInfo<'_>) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        let Some(next) = data.depth.next.cmd_begin_render_pass2 else { return };
        on_begin_render_pass(&data, cmd, p_begin);
        next(cmd, p_begin, p_subpass);
    }
}

/// Fecha o passe que estava sendo gravado e o guarda no command buffer.
pub(super) fn finish_pass(data: &DeviceData, cmd: vk::CommandBuffer) -> Option<PassRecord> {
    if !data.depth.is_on() {
        return None;
    }
    let mut cmds = data.depth.cmds.lock(cmd.as_raw());
    let state = cmds.get_mut(&cmd.as_raw())?;
    if let Some(screen) = state.screen_current.take() {
        state.screen_passes.push(screen);
    }
    state.rendering = None;
    let pass = state.current.take()?;
    if pass.end_layout != 0 {
        state.layouts.push((pass.image, pass.end_layout));
    }
    state.passes.push(pass);
    Some(pass)
}

/// Depois do fim de um passe da profundidade em observação: se é o passe pedido, grava a foto da profundidade aqui.
pub(super) unsafe fn after_pass(data: &DeviceData, cmd: vk::CommandBuffer, pass: Option<PassRecord>) {
    let Some(pass) = pass else { return };
    let wanted = data.depth.snapshot_pass.load(Ordering::Relaxed);
    if wanted == 0 || pass.image != data.depth.watched_image.load(Ordering::Relaxed) || pass.end_layout == 0 {
        return;
    }
    // O passe de número `wanted` (de 1) da imagem neste quadro, na ordem de gravação; `ALL_PASSES`: depois de todos (o resultado é o
    // do último), para uma imagem MSAA, cujo resolve não pode esperar saber qual é o último passe.
    let counted = data.depth.pass_counter.fetch_add(1, Ordering::Relaxed) + 1;
    if wanted != ALL_PASSES && counted != wanted {
        return;
    }
    unsafe { snapshot_now(data, cmd, pass.image, vk::ImageLayout::from_raw(pass.end_layout)) };
}

pub(super) unsafe extern "system" fn cmd_end_render_pass(cmd: vk::CommandBuffer) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        let finished = finish_pass(&data, cmd);
        (data.device.fp_v1_0().cmd_end_render_pass)(cmd);
        after_pass(&data, cmd, finished);
    }
}

pub(super) unsafe extern "system" fn cmd_end_render_pass2(cmd: vk::CommandBuffer, p_end: *const vk::SubpassEndInfo<'_>) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        let Some(next) = data.depth.next.cmd_end_render_pass2 else { return };
        let finished = finish_pass(&data, cmd);
        next(cmd, p_end);
        after_pass(&data, cmd, finished);
    }
}

pub(super) unsafe extern "system" fn cmd_begin_rendering(cmd: vk::CommandBuffer, p_info: *const vk::RenderingInfo<'_>) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        let Some(next) = data.depth.next.cmd_begin_rendering else { return };
        // O que o passe novo vai passar adiante (corrigido, se for o caso: ver `fixed` abaixo).
        let mut fixed: Option<(vk::RenderingAttachmentInfo<'_>, Option<vk::RenderingAttachmentInfo<'_>>)> = None;
        let mut fixed_info;
        let mut p_info = p_info;
        if data.depth.is_on() && !p_info.is_null() {
            let info = &*p_info;
            if let Some(state) = data.depth.cmds.lock(cmd.as_raw()).get_mut(&cmd.as_raw()) {
                state.open_scopes = 0;
            }
            if !info.p_color_attachments.is_null() {
                let colors = std::slice::from_raw_parts(info.p_color_attachments, info.color_attachment_count as usize);
                if let Some(a) = colors.iter().find(|a| data.depth.screen.is_view(a.image_view.as_raw()))
                    && let Some(state) = data.depth.cmds.lock(cmd.as_raw()).get_mut(&cmd.as_raw())
                {
                    state.screen_current = Some(crate::depth::screen::ScreenPass { kind: crate::depth::screen::Kind::Pass, load: load_op(a.load_op), draws: 0 });
                }
            }
            if !info.p_depth_attachment.is_null() {
                let a = &*info.p_depth_attachment;
                let image = data.depth.views.lock().unwrap().get(&a.image_view.as_raw()).copied();
                if let Some(image) = image {
                    // Partir este passe numa limpeza (ver `split_rendering`) fecha a primeira metade com o `storeOp` do jogo: com
                    // `DONT_CARE` a profundidade (e o estêncil) ficaria indefinida e a segunda metade a leria com `LOAD`. Na imagem observada,
                    // o passe passa a guardar (o que a maioria dos drivers já faz de qualquer forma).
                    if data.depth.split_on() && image == data.depth.watched_image.load(Ordering::Relaxed) {
                        let keeps = |op: vk::AttachmentStoreOp| !matches!(op, vk::AttachmentStoreOp::DONT_CARE | vk::AttachmentStoreOp::NONE);
                        let stencil_a = (!info.p_stencil_attachment.is_null()).then(|| *info.p_stencil_attachment);
                        if !keeps(a.store_op) || stencil_a.is_some_and(|s| !keeps(s.store_op)) {
                            let mut d = *a;
                            d.store_op = vk::AttachmentStoreOp::STORE;
                            let s = stencil_a.map(|mut s| {
                                s.store_op = vk::AttachmentStoreOp::STORE;
                                s
                            });
                            fixed = Some((d, s));
                        }
                    }
                    if let Some((d, s)) = &fixed {
                        fixed_info = *info;
                        fixed_info.p_depth_attachment = d;
                        if let Some(s) = s {
                            fixed_info.p_stencil_attachment = s;
                        }
                        p_info = &fixed_info;
                    }
                    let info = &*p_info;
                    let a = &*info.p_depth_attachment;
                    let load = load_op(a.load_op);
                    let record = PassRecord {
                        image,
                        load,
                        store: store_op(a.store_op),
                        clear_depth: (load == LoadOp::Clear).then(|| a.clear_value.depth_stencil.depth),
                        area: (info.render_area.extent.width, info.render_area.extent.height),
                        end_layout: a.image_layout.as_raw(),
                        ..PassRecord::default()
                    };
                    if let Some(state) = data.depth.cmds.lock(cmd.as_raw()).get_mut(&cmd.as_raw()) {
                        state.current = Some(record);
                    }
                    // Um passe que recomeça a profundidade (`Clear` ou `DontCare`) é uma limpeza dela; copia antes, fora do passe.
                    if matches!(load, LoadOp::Clear | LoadOp::DontCare) && register_clear_event(&data, image, true) {
                        snapshot_now(&data, cmd, image, a.image_layout);
                    }
                    remember_rendering(&data, cmd, info, image);
                }
            }
        }
        next(cmd, p_info);
    }
}

pub(super) unsafe extern "system" fn cmd_end_rendering(cmd: vk::CommandBuffer) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        let Some(next) = data.depth.next.cmd_end_rendering else { return };
        let finished = finish_pass(&data, cmd);
        next(cmd);
        after_pass(&data, cmd, finished);
    }
}
