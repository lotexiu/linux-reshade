//! O envio à fila e o fim do quadro: junta o que os command buffers gravaram, escolhe a candidata de profundidade e publica o resultado.

use super::*;

/// Junta ao quadro o que os command buffers enviados gravaram.
pub(super) fn merge_submitted(data: &DeviceData, buffers: &[vk::CommandBuffer]) {
    if !data.depth.is_on() || buffers.is_empty() {
        return;
    }
    let (passes, layouts): (Vec<PassRecord>, Vec<(u64, i32)>) = {
        let mut passes = Vec::new();
        let mut layouts = Vec::new();
        for b in buffers {
            let cmds = data.depth.cmds.lock(b.as_raw());
            if let Some(s) = cmds.get(&b.as_raw()) {
                passes.extend(s.passes.iter().copied());
                layouts.extend(s.layouts.iter().copied());
                data.depth.screen.push_frame(s.screen_passes.iter().copied());
            }
        }
        (passes, layouts)
    };
    // O layout de cada imagem segue a ordem dos envios (a ordem em que a GPU os executa).
    if !layouts.is_empty() {
        let mut map = data.depth.image_layouts.lock().unwrap();
        for (image, layout) in layouts {
            map.insert(image, layout);
        }
    }
    if passes.is_empty() {
        return;
    }
    let images = data.depth.images.lock().unwrap().clone();
    data.depth.frame.lock().unwrap().merge(&passes, &images);
}

pub(super) unsafe extern "system" fn queue_submit(queue: vk::Queue, count: u32, p_submits: *const vk::SubmitInfo<'_>, fence: vk::Fence) -> vk::Result {
    unsafe {
        let Some(data) = data_of(queue.as_raw()) else { return vk::Result::ERROR_DEVICE_LOST };
        if data.depth.is_on() && !p_submits.is_null() {
            for submit in std::slice::from_raw_parts(p_submits, count as usize) {
                if !submit.p_command_buffers.is_null() {
                    merge_submitted(&data, std::slice::from_raw_parts(submit.p_command_buffers, submit.command_buffer_count as usize));
                }
            }
        }
        crate::queue_lock::with(queue.as_raw(), || (data.device.fp_v1_0().queue_submit)(queue, count, p_submits, fence))
    }
}

pub(super) unsafe extern "system" fn queue_submit2(queue: vk::Queue, count: u32, p_submits: *const vk::SubmitInfo2<'_>, fence: vk::Fence) -> vk::Result {
    unsafe {
        let Some(data) = data_of(queue.as_raw()) else { return vk::Result::ERROR_DEVICE_LOST };
        let Some(next) = data.depth.next.queue_submit2 else { return vk::Result::ERROR_EXTENSION_NOT_PRESENT };
        if data.depth.is_on() && !p_submits.is_null() {
            for submit in std::slice::from_raw_parts(p_submits, count as usize) {
                if !submit.p_command_buffer_infos.is_null() {
                    let buffers: Vec<vk::CommandBuffer> = std::slice::from_raw_parts(submit.p_command_buffer_infos, submit.command_buffer_info_count as usize).iter().map(|i| i.command_buffer).collect();
                    merge_submitted(&data, &buffers);
                }
            }
        }
        crate::queue_lock::with(queue.as_raw(), || next(queue, count, p_submits, fence))
    }
}

/// Lê o pedido da pessoa (`lrctl depth ...`) de tempos em tempos: `/dev/shm/.../depth-pick-<id da layer>`, com linhas
/// `candidate=N` e/ou `pass=N`.
pub(super) fn poll_pick(data: &DeviceData) {
    if data.depth.frames.fetch_add(1, Ordering::Relaxed) % 20 != 0 {
        return;
    }
    let Some(id) = crate::status::id() else { return };
    let text = std::fs::read_to_string(ipc::state_dir().join(format!("depth-pick-{id}"))).unwrap_or_default();
    let number = |key: &str| text.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('=')?.trim().parse::<u32>().ok()).filter(|n| *n > 0);
    let pick = crate::depth::Pick { candidate: number("candidate"), pass: number("pass"), clear: number("clear") };
    // Os ajustes de política (`lrctl depth policy`) valem na hora.
    let policy_text = std::fs::read_to_string(ipc::state_dir().join(format!("depth-policy-{id}"))).unwrap_or_default();
    let flag = |key: &str| policy_text.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('=').map(|v| v.trim() == "on").filter(|_| matches!(l.split_once('=').map(|(_, v)| v.trim()), Some("on" | "off"))));
    let policy = crate::depth::Policy { split: flag("split"), copy: flag("copy"), snapshot: flag("snapshot"), resignal: flag("resignal") };
    {
        let mut current = data.depth.policy.lock().unwrap();
        if *current != policy {
            log!("política da profundidade: {:?}", policy);
            *current = policy;
        }
    }
    let mut current = data.depth.pick.lock().unwrap();
    if *current != pick {
        log!("pedido para a profundidade: candidata {:?}, passe {:?}, limpeza {:?}", pick.candidate, pick.pass, pick.clear);
        *current = pick;
    }
}

/// Fim de um quadro (chamado no present): escolhe a candidata e publica a lista para o app de controle.
pub fn end_frame(data: &DeviceData, frame: (u32, u32)) {
    if !data.depth.is_on() {
        return;
    }
    unsafe { crate::depth::snapshot::reap_retired(data) };
    data.depth.pass_counter.store(0, Ordering::Relaxed);
    data.depth.clear_counter.store(0, Ordering::Relaxed);
    data.depth.draws_since_event.store(0, Ordering::Relaxed);
    let clear_events = std::mem::take(&mut *data.depth.frame_clears.lock().unwrap());
    let clear_count = clear_events.len();
    poll_pick(data);
    // O present já usou a marca deste quadro (`copy_for_present`, antes de `end_frame`): começa a do próximo.
    data.depth.copied_frame.store(false, Ordering::Relaxed);
    let (frame_draws, frame_depth_draws) = (data.depth.frame_draws.swap(0, Ordering::Relaxed), data.depth.frame_depth_draws.swap(0, Ordering::Relaxed));
    let (frame_clear_attachments, frame_clear_images) = (data.depth.frame_clear_attachments.swap(0, Ordering::Relaxed), data.depth.frame_clear_images.swap(0, Ordering::Relaxed));
    let stats = std::mem::take(&mut *data.depth.frame.lock().unwrap());
    let screen_summary = data.depth.screen.take_summary();
    if stats.is_empty() {
        return;
    }
    let layouts = data.depth.image_layouts.lock().unwrap().clone();
    let mut candidates = stats.candidates(frame, &layouts);
    // O pedido da pessoa manda sobre a heurística. O número da candidata vira a IMAGEM daquele momento e ela fica: a ordem da lista
    // muda entre quadros e cenas, e o número sozinho alternava entre imagens diferentes. Se a imagem some, volta à heurística.
    let pick = *data.depth.pick.lock().unwrap();
    let mut picked_by_number = false;
    {
        let mut picked = data.depth.picked_image.lock().unwrap();
        match pick.candidate {
            Some(n) => {
                if picked.0 != n {
                    let c = candidates.get(n as usize - 1);
                    *picked = (n, c.map_or(0, |c| c.image), c.map_or((0, 0, 0), |c| (c.width, c.height, c.format)));
                }
                // O jogo recria imagens (mapas de sombra em cascata, trocas de qualidade): se a escolhida sumiu, a que tem o
                // mesmo tamanho e formato (a de mais desenhos) herda o pedido. Sem isso a escolha ia e voltava a cada recriação.
                if picked.1 == 0 || !candidates.iter().any(|c| c.image == picked.1) {
                    let sig = picked.2;
                    picked.1 = candidates.iter().filter(|c| (c.width, c.height, c.format) == sig && sig.0 != 0).max_by_key(|c| c.draws).map_or(0, |c| c.image);
                }
                // Só o que não dá para amostrar de jeito nenhum (MSAA sem `SAMPLED`) é recusado; as outras ("proporção diferente", "sem
                // desenhos") a pessoa pode escolher: a heurística não as escolheria, e o resultado pode não ser o esperado (o painel avisa
                // em amarelo). O pedido recusado fica guardado e a heurística decide.
                if picked.1 != 0 && candidates.iter().any(|c| c.image == picked.1 && c.unusable.is_some_and(crate::depth::observe::is_blocking)) {
                    if data.depth.refused_pick.swap(picked.1, Ordering::Relaxed) != picked.1 {
                        log!("pedido ignorado: a candidata escolhida não serve para os efeitos");
                    }
                } else if picked.1 != 0 {
                    let image = picked.1;
                    for c in candidates.iter_mut() {
                        c.chosen = c.image == image;
                    }
                    picked_by_number = true;
                }
            }
            None => *picked = (0, 0, (0, 0, 0)),
        }
    }
    // Histerese: a imagem em uso só perde o lugar depois de o desafiante vencer vários quadros seguidos (não vale para um pedido).
    if !picked_by_number {
        let winner = candidates.iter().find(|c| c.chosen).map_or(0, |c| c.image);
        let mut sticky = data.depth.sticky.lock().unwrap();
        let keeps = sticky.0 != 0 && sticky.0 != winner && candidates.iter().any(|c| c.image == sticky.0 && c.unusable.is_none());
        if keeps {
            if sticky.1 == winner {
                sticky.2 += 1;
            } else {
                sticky.1 = winner;
                sticky.2 = 1;
            }
            if sticky.2 >= STICKY_SWITCH_FRAMES {
                *sticky = (winner, 0, 0);
            } else {
                let current = sticky.0;
                for c in candidates.iter_mut() {
                    c.chosen = c.image == current;
                }
            }
        } else {
            *sticky = (winner, 0, 0);
        }
    }
    let active = candidates.iter().find(|c| c.chosen).cloned();
    let now_image = active.as_ref().map_or(0, |c| c.image);
    data.depth.watched_image.store(now_image, Ordering::Relaxed);
    if data.depth.last_image.swap(now_image, Ordering::Relaxed) != now_image {
        data.depth.image_changes.fetch_add(1, Ordering::Relaxed);
    }

    // O que os efeitos recebem: a original (amostrável, com layout conhecido) ou, se pediram um passe, a foto dele.
    let linear = |format: i32| {
        unsafe { data.instance.instance.get_physical_device_format_properties(data.physical_device, vk::Format::from_raw(format)) }
            .optimal_tiling_features
            .contains(vk::FormatFeatureFlags::SAMPLED_IMAGE_FILTER_LINEAR)
    };
    let mut chosen = None;
    data.depth.snapshot_pass.store(0, Ordering::Relaxed);
    data.depth.snapshot_clear.store(0, Ordering::Relaxed);
    if let Some(c) = &active {
        // A regra automática da limpeza (como o "copiar antes de limpar" do ReShade): entre as limpezas da profundidade do
        // quadro, a primeira que apaga uma parte relevante deles é a da cena (a de mais desenhos vem depois, e o índice dela varia). O número da limpeza é estável de um quadro para o outro (o de
        // passes não é: o DXVK os divide como quer), e a escolha só muda depois de vários quadros discordando.
        let auto_clear = || {
            let mut state = data.depth.auto_clear.lock().unwrap();
            if state.0 != c.image {
                *state = (c.image, 0, 0, 0);
            }
            let best = crate::depth::observe::scene_clear(&clear_events.iter().map(|e| (e.draws_before, e.splittable)).collect::<Vec<_>>()).unwrap_or(0);
            if state.1 == 0 {
                // Ainda sem escolha: só adota uma limpeza que aparece (a mesma) em vários quadros seguidos. Uma que aparece só de vez em
                // quando (a câmera, uma tela) faria a profundidade piscar entre a foto e a imagem ao vivo.
                if best != 0 && state.2 == best {
                    state.3 += 1;
                } else if best != 0 {
                    state.2 = best;
                    state.3 = 1;
                } else {
                    state.2 = 0;
                    state.3 = 0;
                }
                if state.3 >= AUTO_CLEAR_SWITCH_FRAMES {
                    crate::log::log!("profundidade: limpeza automática = {best} (de {clear_count}; desenhos antes de cada: {:?})", clear_events.iter().map(|e| e.draws_before).collect::<Vec<_>>());
                    *state = (c.image, best, best, 0);
                }
            } else if best == 0 {
                // Sem limpezas neste quadro: só esquece a escolha depois de muito tempo.
                state.3 += 1;
                if state.3 >= AUTO_SCENE_FORGET_FRAMES {
                    *state = (c.image, 0, 0, 0);
                }
            } else if best == state.1 {
                state.2 = best;
                state.3 = 0;
            } else if best == state.2 {
                state.3 += 1;
                if state.3 >= AUTO_CLEAR_SWITCH_FRAMES {
                    crate::log::log!("profundidade: limpeza automática mudou de {} para {best} (desenhos antes de cada: {:?})", state.1, clear_events.iter().map(|e| e.draws_before).collect::<Vec<_>>());
                    *state = (c.image, best, best, 0);
                }
            } else {
                state.2 = best;
                state.3 = 1;
            }
            // O quadro pode ter menos limpezas que o número escolhido (o Rocket League alterna entre 2 e 3, por exemplo): o pedido vale pelo que o
            // último quadro teve (a última limpeza que existe), e a foto continua sendo gravada todo quadro. Antes a regra não copiava nesses quadros,
            // a profundidade ficava congelada na foto antiga por 30 quadros e depois a escolha era largada e refeita (a profundidade ao vivo,
            // errada, aparecia nos 8 quadros seguintes).
            (state.1 > 0).then_some(if clear_count > 0 { state.1.min(clear_count as u32) } else { state.1 })
        };
        // Um pedido da pessoa (passe ou limpeza) manda; senão a regra da limpeza; senão a do passe (jogos sem limpezas que se
        // possa partir, como os de passes de renderização clássicos).
        let msaa = c.samples > 1;
        // Uma imagem MSAA só serve resolvida: a foto depois de TODOS os passes (o último vale), sem a regra de limpeza.
        // Diagnóstico dos Xid 69: `LINUX_RESHADE_DEPTH_NO_SNAPSHOT=1` não parte passes nem copia a profundidade (os efeitos leem a imagem do
        // jogo, que pode estar velha ou limpa). Se os erros da GPU somem, a causa é a cópia dentro dos command buffers do jogo.
        let no_snapshot = !data.depth.snapshot_on();
        let wanted_clear = if no_snapshot || pick.pass.is_some() || msaa { None } else { pick.clear.or_else(auto_clear) };
        let auto_pass = || {
            // Com histerese: depois de usar a foto de um passe, só volta à imagem original se o jogo ficar muito tempo sem
            // recomeçar a imagem; o número do passe acompanha o último visto. Sem isso, a escolha trocava entre foto e
            // original a cada poucos quadros e a cadeia era reconstruída toda vez.
            let mut state = data.depth.auto_scene.lock().unwrap();
            if state.0 != c.image {
                *state = (c.image, None, 0);
            }
            match (state.1, crate::depth::observe::scene_pass(stats.passes_of(c.image))) {
                (_, Some(p)) => *state = (c.image, Some(p), 0),
                (Some(_), None) => {
                    state.2 += 1;
                    if state.2 >= AUTO_SCENE_FORGET_FRAMES {
                        *state = (c.image, None, 0);
                    }
                }
                (None, None) => {}
            }
            state.1
        };
        let wanted_pass = if no_snapshot { None } else if msaa { Some(ALL_PASSES) } else if wanted_clear.is_some() { None } else { pick.pass.or_else(auto_pass) };
        // Modo cópia (o do ReShade): sempre há foto, e a cópia da hora dos efeitos a renova quando nenhuma regra copiou no quadro.
        let copy_live = data.depth.copy_on() && !no_snapshot && !msaa && c.sampled && c.layout != 0;
        *data.depth.live.lock().unwrap() = copy_live.then_some(crate::depth::Live { image: c.image, format: c.format, width: c.width, height: c.height });
        let snapshot = (wanted_pass.is_some() || wanted_clear.is_some() || copy_live).then_some(()).and_then(|()| {
            // A foto: precisa da imagem da foto, e a imagem original precisa ter TRANSFER_SRC (armado). MSAA: foto de uma amostra.
            let format = if msaa { crate::depth::resolve::RESOLVED_FORMAT } else { vk::Format::from_raw(c.format) };
            unsafe { crate::depth::snapshot::ensure(data, format, c.width, c.height, msaa) }
        });
        match snapshot {
            Some(image) => {
                data.depth.snapshot_pass.store(wanted_pass.unwrap_or(0), Ordering::Relaxed);
                data.depth.snapshot_clear.store(wanted_clear.unwrap_or(0), Ordering::Relaxed);
                // Antes da primeira foto a imagem não tem layout: os efeitos só a recebem depois.
                if data.depth.snapshot_written.load(Ordering::Relaxed) || copy_live {
                    chosen = Some(crate::depth::Chosen { image: image.as_raw(), format: c.format, layout: vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL.as_raw(), width: c.width, height: c.height, linear: linear(c.format) });
                }
            }
            None if c.sampled && c.layout != 0 => {
                chosen = Some(crate::depth::Chosen { image: c.image, format: c.format, layout: c.layout, width: c.width, height: c.height, linear: linear(c.format) });
            }
            None => {}
        }
    }
    *data.depth.chosen.lock().unwrap() = chosen;

    let mut last = data.depth.last_published.lock().unwrap();
    // Para entender o que o jogo desenha na profundidade (e achar o ponto certo de usá-la): os passes da escolhida.
    let mut passes_summary = String::new();
    if let Some(c) = &active {
        let briefs: Vec<String> = stats
            .passes_of(c.image)
            .iter()
            .enumerate()
            .filter(|(_, p)| p.draws > 0 || !matches!(p.load, crate::depth::observe::LoadOp::Load))
            .map(|(i, p)| format!("{}:{}d/{}i {:?}->{:?}", i + 1, p.draws, p.indirect, p.load, p.store))
            .collect();
        passes_summary = briefs.join(" ");
        if !screen_summary.is_empty() {
            passes_summary = format!("tela: {screen_summary} | {passes_summary}");
        }
        // Os contadores mudam a cada quadro: só se atualizam de 90 em 90, para não reescrever o status sem parar.
        {
            let mut diag = data.depth.diag.lock().unwrap();
            if data.depth.diag_frames.fetch_add(1, Ordering::Relaxed) % 90 == 0 || diag.is_empty() {
                *diag = format!(
                    "trocas da imagem: {}, reconstruções da cadeia: {}; um quadro: {} desenhos ({} em passes com profundidade), limpezas de profundidade fora de passe: {} de anexos e {} de imagens; limpeza escolhida: {} de {} [desenhos antes de cada uma: {}]; última mudança que reconstruiu: {}; presents sem efeito/com efeito: {}/{}; passes partidos/pulados: {}/{}",
                    data.depth.image_changes.load(Ordering::Relaxed),
                    data.depth.chain_rebuilds.load(Ordering::Relaxed),
                    frame_draws,
                    frame_depth_draws,
                    frame_clear_attachments,
                    frame_clear_images,
                    data.depth.snapshot_clear.load(Ordering::Relaxed),
                    clear_count,
                    clear_events.iter().map(|e| format!("{}{}", e.draws_before, if e.splittable { "" } else { "!" })).collect::<Vec<_>>().join(" "),
                    data.depth.last_key_change.lock().unwrap(),
                    data.depth.fx_presents[0].load(Ordering::Relaxed),
                    data.depth.fx_presents[1].load(Ordering::Relaxed),
                    data.depth.splits_done.load(Ordering::Relaxed),
                    data.depth.splits_skipped.load(Ordering::Relaxed),
                );
            }
            passes_summary = format!("{} | {passes_summary}", *diag);
        }
        let mut logged = data.depth.passes_logged.lock().unwrap();
        // O resumo muda quase a cada quadro em alguns jogos: só se registra de vez em quando.
        if *logged != passes_summary && data.depth.diag_frames.load(Ordering::Relaxed) % 90 == 1 {
            log!("passes da profundidade escolhida: {passes_summary}");
            *logged = passes_summary.clone();
        }
    }
    if *last != candidates {
        if let Some(c) = &active {
            let key = |c: &crate::depth::observe::Candidate| (c.width, c.height, c.format, c.load, c.store, c.reversed);
            if last.iter().find(|p| p.chosen).map(key) != Some(key(c)) {
                log!(
                    "profundidade escolhida: {}x{} formato {} · {} desenhos, {} vértices, {} passe(s) · começa com {:?}, termina com {:?} · invertida: {:?}",
                    c.width,
                    c.height,
                    c.format,
                    c.draws,
                    c.vertices,
                    c.passes,
                    c.load,
                    c.store,
                    c.reversed
                );
            }
        }
        // Para entender jogos em que a comparação de profundidade não aparece: o que os pipelines criados declaram.
        {
            let pipelines = data.depth.pipelines.read().unwrap();
            let count = |c: Compare| pipelines.values().filter(|v| **v == c).count();
            let (greater, less, other) = (count(Compare::Greater), count(Compare::Less), count(Compare::Other));
            let mut logged = data.depth.pipelines_logged.lock().unwrap();
            if *logged != (greater, less, other) {
                *logged = (greater, less, other);
                log!("pipelines com teste de profundidade fixo: {greater} comparam 'maior', {less} 'menor', {other} outros");
            }
        }
        *last = candidates.clone();
    }
    crate::status::set_depth(candidates.iter().map(to_status).collect(), passes_summary);
}

/// Libera o que a observação criou (a imagem da foto). Chamado ao destruir o dispositivo.
pub fn destroy(data: &DeviceData) {
    unsafe { crate::depth::snapshot::destroy(data) };
}

pub(super) fn to_status(c: &crate::depth::observe::Candidate) -> ipc::DepthStatus {
    let load = match c.load {
        LoadOp::Load => "load",
        LoadOp::Clear => "clear",
        LoadOp::DontCare => "dont_care",
        LoadOp::None => "none",
    };
    let store = match c.store {
        StoreOp::Store => "store",
        StoreOp::DontCare => "dont_care",
        StoreOp::None => "none",
    };
    ipc::DepthStatus {
        width: c.width,
        height: c.height,
        format: c.format,
        samples: c.samples,
        draws: c.draws,
        vertices: c.vertices,
        passes: c.passes,
        load: load.into(),
        store: store.into(),
        reversed: c.reversed,
        last_pass: c.last_pass,
        chosen: c.chosen,
        automatic: c.automatic,
        unusable: c.unusable.map(String::from),
        sampled: c.sampled,
        layout: c.layout,
    }
}
