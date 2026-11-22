//! As limpezas da profundidade ("copiar antes de limpar", como o ReShade) e o que impede de partir um passe (consultas, renderização condicional, transform feedback).

use super::*;

/// Registra uma limpeza (ou recomeço) da imagem observada. Devolve se é a que a pessoa (ou a regra automática) quer copiar.
pub(super) fn register_clear_event(data: &DeviceData, image: u64, splittable: bool) -> bool {
    if image == 0 || image != data.depth.watched_image.load(Ordering::Relaxed) {
        return false;
    }
    let ordinal = data.depth.clear_counter.fetch_add(1, Ordering::Relaxed) + 1;
    let draws_before = data.depth.draws_since_event.swap(0, Ordering::Relaxed);
    data.depth.frame_clears.lock().unwrap().push(crate::depth::ClearEvent { draws_before, splittable });
    ordinal == data.depth.snapshot_clear.load(Ordering::Relaxed)
}

/// Grava a cópia da profundidade de `image` (no layout `layout`) para a foto da layer, fora de um passe de renderização.
pub(super) unsafe fn snapshot_now(data: &DeviceData, cmd: vk::CommandBuffer, image: u64, layout: vk::ImageLayout) {
    let Some(info) = data.depth.images.lock().unwrap().get(&image).copied() else { return };
    let Some((dst, format)) = data.depth.snapshot.lock().unwrap().as_ref().map(|s| (s.image, s.format)) else { return };
    if layout == vk::ImageLayout::UNDEFINED {
        return;
    }
    if info.samples > 1 {
        // MSAA: o resolve desenha a amostra 0 de cada pixel na foto de uma amostra.
        if format != crate::depth::resolve::RESOLVED_FORMAT || !unsafe { crate::depth::resolve::record(data, cmd, vk::Image::from_raw(image), layout, vk::Format::from_raw(info.format), info.width, info.height) } {
            return;
        }
    } else {
        if format.as_raw() != info.format {
            return;
        }
        unsafe { crate::depth::snapshot::record_copy(data, cmd, vk::Image::from_raw(image), layout, dst, format, info.width, info.height) };
    }
    data.depth.copied_frame.store(true, Ordering::Relaxed);
    if !data.depth.snapshot_written.swap(true, Ordering::Relaxed) {
        let (clear, pass) = (data.depth.snapshot_clear.load(Ordering::Relaxed), data.depth.snapshot_pass.load(Ordering::Relaxed));
        if clear > 0 {
            log!("foto da profundidade gravada antes da limpeza {clear} (primeira vez)");
        } else {
            log!("foto da profundidade gravada (primeira vez; passe {})", if pass == ALL_PASSES { "todos".to_string() } else { pass.to_string() });
        }
    }
}

/// Guarda uma cópia do `VkRenderingInfo` se o passe é sobre a imagem observada (para partir o passe numa limpeza).
pub(super) unsafe fn remember_rendering(data: &DeviceData, cmd: vk::CommandBuffer, info: &vk::RenderingInfo<'_>, image: u64) {
    unsafe {
        let own = |a: &vk::RenderingAttachmentInfo<'_>| {
            let mut c = *a;
            c.p_next = std::ptr::null();
            // SAFETY: sem `p_next`, a struct não referencia nada além dela mesma.
            std::mem::transmute::<vk::RenderingAttachmentInfo<'_>, vk::RenderingAttachmentInfo<'static>>(c)
        };
        let colors = if info.p_color_attachments.is_null() { Vec::new() } else { std::slice::from_raw_parts(info.p_color_attachments, info.color_attachment_count as usize).iter().map(own).collect() };
        let depth = (!info.p_depth_attachment.is_null()).then(|| own(&*info.p_depth_attachment));
        let stencil = (!info.p_stencil_attachment.is_null()).then(|| own(&*info.p_stencil_attachment));
        // Só dá para partir um passe simples: sem estruturas encadeadas e sem suspender/retomar.
        let simple = info.p_next.is_null() && !info.flags.intersects(vk::RenderingFlags::SUSPENDING | vk::RenderingFlags::RESUMING);
        let copy = simple.then(|| crate::depth::RenderingCopy { flags: info.flags, render_area: info.render_area, layer_count: info.layer_count, view_mask: info.view_mask, colors, depth, stencil, image });
        if let Some(state) = data.depth.cmds.lock(cmd.as_raw()).get_mut(&cmd.as_raw()) {
            state.rendering = copy;
        }
    }
}

/// Parte o passe de renderização dinâmica em gravação: fecha, copia a profundidade, reabre o mesmo passe sem apagar nada.
pub(super) unsafe fn split_rendering(data: &DeviceData, cmd: vk::CommandBuffer, copy: &crate::depth::RenderingCopy) -> bool {
    unsafe {
        let (Some(end), Some(begin)) = (data.depth.next.cmd_end_rendering, data.depth.next.cmd_begin_rendering) else { return false };
        let Some(depth) = copy.depth else { return false };
        end(cmd);
        snapshot_now(data, cmd, copy.image, depth.image_layout);
        // Reabre com `LOAD`: o conteúdo (cor e profundidade) segue como estava.
        let keep = |a: &vk::RenderingAttachmentInfo<'static>| {
            let mut c = *a;
            c.load_op = vk::AttachmentLoadOp::LOAD;
            c
        };
        let colors: Vec<vk::RenderingAttachmentInfo<'static>> = copy.colors.iter().map(keep).collect();
        let depth = keep(&depth);
        let stencil = copy.stencil.as_ref().map(keep);
        let mut info = vk::RenderingInfo::default().flags(copy.flags).render_area(copy.render_area).layer_count(copy.layer_count).view_mask(copy.view_mask).color_attachments(&colors).depth_attachment(&depth);
        if let Some(stencil) = &stencil {
            info = info.stencil_attachment(stencil);
        }
        begin(cmd, &info);
        true
    }
}

pub(super) unsafe extern "system" fn cmd_clear_attachments(cmd: vk::CommandBuffer, count: u32, p_attachments: *const vk::ClearAttachment, rect_count: u32, p_rects: *const vk::ClearRect) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        if data.depth.is_on() && !p_attachments.is_null() && std::slice::from_raw_parts(p_attachments, count as usize).iter().any(|a| a.aspect_mask.intersects(vk::ImageAspectFlags::DEPTH | vk::ImageAspectFlags::STENCIL)) {
            data.depth.frame_clear_attachments.fetch_add(1, Ordering::Relaxed);
            // A limpeza é dentro do passe em gravação: se é sobre a imagem observada, é uma limpeza dela.
            let (image, copy, scopes) = match data.depth.cmds.lock(cmd.as_raw()).get(&cmd.as_raw()) {
                Some(state) => (state.current.map_or(0, |p| p.image), state.rendering.clone(), state.open_scopes),
                None => (0, None, 0),
            };
            // Partir o passe só se pedido: o ReShade não copia dentro de um passe ativo (é inválido, e consultas ou renderização condicional
            // abertas no passe não sobrevivem ao fim dele). O "antes da limpeza N" explícito da pessoa vale como pedido.
            let wanted = copy.is_some() && (data.depth.split_on() || data.depth.pick.lock().unwrap().clear.is_some());
            // Só se parte com o passe válido de partir: nenhuma consulta, renderização condicional ou transform feedback aberta nele.
            let allow = wanted && scopes == 0;
            if wanted && !allow && image != 0 {
                if data.depth.splits_skipped.fetch_add(1, Ordering::Relaxed) == 0 {
                    log!("passe não partido: há consulta, renderização condicional ou transform feedback aberto nele ({scopes}); a cópia da hora dos efeitos cobre");
                }
            }
            if image != 0 && register_clear_event(&data, image, allow) && allow && let Some(copy) = copy {
                data.depth.splits_done.fetch_add(1, Ordering::Relaxed);
                split_rendering(&data, cmd, &copy);
            }
        }
        (data.device.fp_v1_0().cmd_clear_attachments)(cmd, count, p_attachments, rect_count, p_rects);
    }
}

pub(super) unsafe extern "system" fn cmd_clear_depth_stencil_image(
    cmd: vk::CommandBuffer,
    image: vk::Image,
    layout: vk::ImageLayout,
    p_value: *const vk::ClearDepthStencilValue,
    count: u32,
    p_ranges: *const vk::ImageSubresourceRange,
) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        if data.depth.is_on() && data.depth.images.lock().unwrap().contains_key(&image.as_raw()) {
            data.depth.frame_clear_images.fetch_add(1, Ordering::Relaxed);
            // Fora de um passe: dá para copiar logo antes.
            if register_clear_event(&data, image.as_raw(), true) {
                snapshot_now(&data, cmd, image.as_raw(), layout);
            }
        }
        (data.device.fp_v1_0().cmd_clear_depth_stencil_image)(cmd, image, layout, p_value, count, p_ranges);
    }
}

/// Conta um escopo que precisa terminar na mesma instância de passe (consulta, renderização condicional, transform feedback).
pub(super) fn scope_changed(data: &DeviceData, cmd: vk::CommandBuffer, opened: bool) {
    if !data.depth.is_on() {
        return;
    }
    if let Some(state) = data.depth.cmds.lock(cmd.as_raw()).get_mut(&cmd.as_raw()) {
        state.open_scopes = if opened { state.open_scopes + 1 } else { state.open_scopes.saturating_sub(1) };
    }
}

pub(super) unsafe extern "system" fn cmd_begin_query(cmd: vk::CommandBuffer, pool: vk::QueryPool, query: u32, flags: vk::QueryControlFlags) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        scope_changed(&data, cmd, true);
        (data.device.fp_v1_0().cmd_begin_query)(cmd, pool, query, flags);
    }
}

pub(super) unsafe extern "system" fn cmd_end_query(cmd: vk::CommandBuffer, pool: vk::QueryPool, query: u32) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        scope_changed(&data, cmd, false);
        (data.device.fp_v1_0().cmd_end_query)(cmd, pool, query);
    }
}

pub(super) unsafe extern "system" fn cmd_begin_query_indexed(cmd: vk::CommandBuffer, pool: vk::QueryPool, query: u32, flags: vk::QueryControlFlags, index: u32) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        let Some(next) = data.depth.next.cmd_begin_query_indexed else { return };
        scope_changed(&data, cmd, true);
        next(cmd, pool, query, flags, index);
    }
}

pub(super) unsafe extern "system" fn cmd_end_query_indexed(cmd: vk::CommandBuffer, pool: vk::QueryPool, query: u32, index: u32) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        let Some(next) = data.depth.next.cmd_end_query_indexed else { return };
        scope_changed(&data, cmd, false);
        next(cmd, pool, query, index);
    }
}

pub(super) unsafe extern "system" fn cmd_begin_conditional_rendering(cmd: vk::CommandBuffer, p_begin: *const vk::ConditionalRenderingBeginInfoEXT<'_>) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        let Some(next) = data.depth.next.cmd_begin_conditional_rendering else { return };
        scope_changed(&data, cmd, true);
        next(cmd, p_begin);
    }
}

pub(super) unsafe extern "system" fn cmd_end_conditional_rendering(cmd: vk::CommandBuffer) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        let Some(next) = data.depth.next.cmd_end_conditional_rendering else { return };
        scope_changed(&data, cmd, false);
        next(cmd);
    }
}

pub(super) unsafe extern "system" fn cmd_begin_transform_feedback(cmd: vk::CommandBuffer, first: u32, count: u32, p_buffers: *const vk::Buffer, p_offsets: *const vk::DeviceSize) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        let Some(next) = data.depth.next.cmd_begin_transform_feedback else { return };
        scope_changed(&data, cmd, true);
        next(cmd, first, count, p_buffers, p_offsets);
    }
}

pub(super) unsafe extern "system" fn cmd_end_transform_feedback(cmd: vk::CommandBuffer, first: u32, count: u32, p_buffers: *const vk::Buffer, p_offsets: *const vk::DeviceSize) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        let Some(next) = data.depth.next.cmd_end_transform_feedback else { return };
        scope_changed(&data, cmd, false);
        next(cmd, first, count, p_buffers, p_offsets);
    }
}
