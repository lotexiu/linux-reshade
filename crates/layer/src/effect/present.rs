//! O que acontece a cada present: aplicar a cadeia (`process`) e as consultas sobre os swapchains.

use super::*;

/// Trata um present: atualiza o estado do efeito do swapchain (sempre) e, com os efeitos ligados,
/// aplica o efeito. Devolve o semáforo que o present deve esperar no lugar dos originais; `None`
/// significa "apresente como o jogo pediu".
///
/// # Safety
/// `info` deve ser o `VkPresentInfoKHR` recebido do jogo, e `queue` a fila do present.
pub unsafe fn process(data: &DeviceData, queue: vk::Queue, info: &vk::PresentInfoKHR<'_>, enabled: bool, allow_resignal: bool) -> Option<vk::Semaphore> {
    // Vários swapchains num mesmo present compartilham a lista de espera: fora do caso comum.
    if info.swapchain_count != 1 {
        return None;
    }
    unsafe {
        reap_semaphores(&data.device, false);
        let swapchain = *info.p_swapchains;
        let index = *info.p_image_indices as usize;

        let mut swapchains = data.swapchains.lock().unwrap();
        let fx = swapchains.get_mut(&swapchain.as_raw())?;
        CHAIN_BUILDING.store(matches!(fx.fx, Fx::Building(_)), std::sync::atomic::Ordering::Relaxed);

        // Mesmo com os efeitos desligados: é aqui que se descobre que falta um bundle para esta
        // resolução, e o app de controle só sabe a tempo de compilá-lo se a layer avisar já.
        let epoch = bundle::epoch();
        if fx.seen_epoch != epoch {
            fx.on_epoch(data, epoch);
        }
        if !enabled || fx.unusable.is_some() || index >= fx.images.len() {
            if !enabled && crate::stats::enabled() {
                // Sem efeitos rodando, a medição da GPU de antes não vale mais.
                crate::stats::set_gpu(Vec::new());
            }
            return None;
        }

        // Uma etapa pede profundidade: liga a observação (custo zero enquanto nenhuma pede).
        let needs_depth = steps_need_depth(&fx.steps);
        if needs_depth {
            data.depth.need();
        }
        // A profundidade escolhida mudou (outra imagem, outro layout, apareceu ou sumiu): reconstrói a cadeia.
        if needs_depth && let Fx::Ready(built) = &fx.fx {
            let now = data.depth.chosen().map(|d| (d.image, d.layout));
            if built.depth_key != now {
                let old = built.depth_key;
                fx.retire(&data.device);
                data.depth.chain_rebuilds.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                *data.depth.last_key_change.lock().unwrap() = format!("{old:x?} -> {now:x?}");
                log!("profundidade para os efeitos mudou: cadeia será reconstruída");
            }
        }

        if needs_depth && matches!(fx.fx, Fx::Failed(_)) && fx.failed_depth != data.depth.chosen().map(|d| (d.image, d.layout)) {
            fx.fx = Fx::Unbuilt;
        }

        if matches!(fx.fx, Fx::Unbuilt) {
            if fx.steps.is_empty() {
                return None; // sem bundle para esta resolução: nada a desenhar
            }
            let family = *data.queues.lock().unwrap().get(&queue.as_raw())?;
            // A criação (a compilação dos shaders pelo driver leva segundos) roda noutra thread: o jogo continua sem o efeito
            // até ela terminar, em vez de parar.
            let depth = if steps_need_depth(&fx.steps) { data.depth.chosen() } else { None };
            let args = BuildArgs {
                device: data.device.clone(),
                memory: data.memory,
                images: fx.images.clone(),
                format: fx.format,
                extent: fx.extent,
                steps: fx.steps.clone(),
                depth,
                queue_family: family,
                set_loader_data: data.set_loader_data,
            };
            let signature = signature(&fx.steps);
            // Uma cadeia aposentada que tem efeitos iguais aos novos (mesmo bundle, técnica e profundidade) os cede: só o que mudou é
            // criado, e o que não mudou segue com o histórico (os efeitos temporais não "piscam" porque outro foi ligado).
            let depth_key_now = depth.map(|d| (d.image, d.layout));
            let mut plan: Option<SendPlan> = None;
            if let Some(old) = fx.retiring.as_mut()
                && old.depth_key == depth_key_now
                && old.fences.len() == fx.images.len()
            {
                let chain_steps: Vec<Step<'_>> = fx.steps.iter().map(|s| Step { bundle: &s.bundle, technique: s.technique }).collect();
                if old.chain.shared_mut().merge_scan(&chain_steps) {
                    let mut available: Vec<Option<(u64, usize)>> = old.chain.keys().into_iter().map(Some).collect();
                    let reuse: Vec<Option<usize>> = fx
                        .steps
                        .iter()
                        .map(|s| {
                            let position = available.iter().position(|k| *k == Some((s.bundle.id, s.technique)));
                            if let Some(p) = position {
                                available[p] = None;
                            }
                            position
                        })
                        .collect();
                    if reuse.iter().any(Option::is_some) {
                        plan = Some(SendPlan(reuse, old.chain.take_shared()));
                    }
                }
            }
            let incremental = plan.is_some();
            let steps_for_job = fx.steps.clone();
            let work = move || {
                SendOut(match plan {
                    Some(SendPlan(reuse, shared)) => build_partial(args, reuse, shared),
                    None => build_chain(args).map(Output::Full).map_err(|why| Failure { why, shared: None }),
                })
            };
            match std::thread::Builder::new().name("lr-build".into()).spawn(work) {
                Ok(thread) => {
                    log!("criação da cadeia iniciada noutra thread ({} etapa(s){})", fx.steps.len(), if incremental { ", só as que mudaram" } else { "" });
                    fx.fx = Fx::Building(Job { thread: Some(thread), steps: steps_for_job, incremental, signature, depth_image: depth.map(|d| d.image), depth_key: depth_key_now, started: Instant::now() });
                }
                Err(e) => {
                    log!("não consegui criar a thread da cadeia: {e}");
                    fx.set_state(EffectState::Failed(format!("thread: {e}")));
                    fx.fx = Fx::Failed(format!("thread: {e}"));
                }
            }
            // Com uma cadeia aposentada o quadro segue com ela; sem nenhuma, passa sem efeito.
            if fx.retiring.is_none() {
                return None;
            }
        }
        let build_finished = matches!(&fx.fx, Fx::Building(job) if job.thread.as_ref().is_some_and(|t| t.is_finished()));
        if matches!(&fx.fx, Fx::Building(_)) && !build_finished && fx.retiring.is_none() {
            return None; // ainda criando: o quadro passa sem o efeito
        }
        if build_finished && let Fx::Building(job) = &mut fx.fx {
            let started = job.started;
            let built_with = job.depth_key;
            let job_steps = std::mem::take(&mut job.steps);
            let incremental = job.incremental;
            let result = job.thread.take().map(|t| t.join()).unwrap_or(Err(Box::new("sem thread")));
            let outcome: Result<Built, (String, Option<postfx::shared::Shared>)> = match result {
                Ok(SendOut(Ok(Output::Full(built)))) => Ok(built),
                Ok(SendOut(Ok(Output::Partial(partial)))) => assemble_partial(fx.retiring.take(), partial, &job_steps, built_with, &data.device),
                Ok(SendOut(Err(failure))) => Err((failure.why, failure.shared)),
                Err(_) => Err(("the chain creation thread failed".to_string(), None)),
            };
            fx.fx = match outcome {
                Ok(mut built) => {
                    // A inicialização das texturas (só as dos efeitos novos) precisa da fila: aqui, na thread que a usa.
                    match crate::queue_lock::with(queue.as_raw(), || built.chain.finish_init(&data.device, queue)) {
                        Ok(()) => {
                            log!("efeito criado para o swapchain {:#x} ({} imagens) em {} ms", swapchain.as_raw(), fx.images.len(), started.elapsed().as_millis());
                            log!("cadeia de {} etapa(s) criada", built.chain.len());
                            crate::status::set_techniques(
                                job_steps
                                    .iter()
                                    .enumerate()
                                    .map(|(i, step)| TechniqueStatus {
                                        name: step.bundle.manifest.techniques.get(step.technique).map(|t| t.name.clone()).unwrap_or_default(),
                                        state: match built.skipped.iter().find(|(s, _)| *s == i) {
                                            Some((_, why)) => TechniqueState::Skipped(why.clone()),
                                            None => TechniqueState::Applied,
                                        },
                                    })
                                    .collect(),
                            );
                            for (step, ignored) in built.chain.ignored_sources() {
                                log!("etapa {step}: uniform sem fonte no runtime (fica no valor padrão): {ignored}");
                            }
                            fx.set_state(EffectState::Ready);
                            if incremental {
                                // A escolha pode ter mudado de novo durante o trabalho: olha-a outra vez no próximo present.
                                fx.seen_epoch = u64::MAX;
                            }
                            Fx::Ready(built)
                        }
                        Err(e) => {
                            let why = e.to_string();
                            log!("efeito indisponível no swapchain {:#x}: {why}", swapchain.as_raw());
                            let mut dead = Fx::Ready(built);
                            SwapchainFx::destroy_fx(&mut dead, &data.device);
                            fx.failed_depth = built_with;
                            fx.set_state(EffectState::Failed(why.clone()));
                            Fx::Failed(why)
                        }
                    }
                }
                Err((why, shared)) => {
                    log!("efeito indisponível no swapchain {:#x}: {why}", swapchain.as_raw());
                    // A cadeia aposentada também apelida as texturas compartilhadas: sai antes delas.
                    fx.drop_retiring(&data.device);
                    if let Some(mut shared) = shared {
                        shared.destroy(&data.device);
                    }
                    fx.failed_depth = built_with;
                    fx.set_state(EffectState::Failed(why.clone()));
                    Fx::Failed(why)
                }
            };
        }
        // A cadeia nova pronta (ou que falhou) substitui a aposentada.
        if matches!(fx.fx, Fx::Ready(_) | Fx::Failed(_)) && fx.retiring.is_some() {
            fx.bury_retiring(&data.device);
        }
        if !fx.graveyard.is_empty() {
            fx.flush_graveyard(&data.device, false);
        }
        let now = Instant::now();
        let previous = fx.last_present.replace(now);
        let built = match &mut fx.fx {
            Fx::Ready(built) => built,
            // Numa criação incremental a cadeia aposentada cedeu as texturas compartilhadas à thread de criação (que as altera e liga os efeitos
            // novos a elas): desenhar com a cadeia velha agora usaria imagens que outra thread mexe. O quadro passa sem efeito até a nova ficar pronta.
            Fx::Building(job) if job.incremental || quiet_swap() => return None,
            _ => match fx.retiring.as_mut() {
                Some(built) => built,
                None => return None,
            },
        };

        // A execução anterior desta imagem precisa ter terminado antes de reenviar o mesmo buffer.
        let fence = built.fences[index];
        if data.device.wait_for_fences(&[fence], true, FENCE_TIMEOUT_NS).is_err() || data.device.reset_fences(&[fence]).is_err() {
            log!("efeito pulado: a execução anterior da imagem {index} não terminou");
            return None;
        }

        // Medição de desempenho (só quando pedida): lê o que a GPU gravou da última vez que usou esta imagem e garante o cronômetro.
        let measure = crate::stats::enabled();
        if measure {
            if built.gpu.is_none() {
                let family = data.queues.lock().unwrap().get(&queue.as_raw()).copied();
                let names: Vec<String> = built
                    .origin
                    .iter()
                    .map(|&i| fx.steps.get(i).and_then(|s| s.bundle.manifest.techniques.get(s.technique)).map_or_else(|| format!("step {i}"), |t| t.name.clone()))
                    .collect();
                built.gpu = family.and_then(|f| GpuTimer::new(data, f, fx.images.len(), names));
            }
            if let Some(timer) = built.gpu.as_mut() {
                timer.collect(&data.device, index);
                if data.presents.load(std::sync::atomic::Ordering::Relaxed) % 30 == 0 {
                    crate::stats::set_gpu(timer.steps.clone());
                }
            }
        }

        // A fence garante que a GPU terminou com o buffer de uniforms desta imagem: pode reescrevê-lo.
        // Primeiro os parâmetros de cada etapa (a imagem inteira do buffer)...
        let own = crate::control::channel().map(|c| c as &dyn ipc::ParamSource);
        let global = crate::control::reader().map(|r| r as &dyn ipc::ParamSource);
        for (step, runtime) in built.params.iter_mut().enumerate() {
            match &mut runtime.source {
                StepSource::Published(state) => {
                    if (own.is_some() || global.is_some()) && let Some(bytes) = state.next(own, global, runtime.params_id) {
                        built.chain.write_uniforms(step, index, 0, bytes);
                    }
                }
                StepSource::Plan(Some(bytes)) => built.chain.write_uniforms(step, index, 0, bytes),
                StepSource::Plan(None) => {}
            }
        }
        // ...depois os valores do runtime (tempo etc.), que valem mais que qualquer parâmetro.
        built.chain.set_active_control(active_control().as_deref());
        built.chain.update_sources(index, &frame_info(now, previous, data.presents.load(std::sync::atomic::Ordering::Relaxed)));

        let waits = std::slice::from_raw_parts(info.p_wait_semaphores, info.wait_semaphore_count as usize);
        // ALL_COMMANDS: nada do efeito pode começar antes de o jogo terminar de desenhar o frame.
        let stages = vec![vk::PipelineStageFlags::ALL_COMMANDS; waits.len()];
        let mut command_buffers = built.chain.command_buffers(index);
        let mut timed = false;
        if measure && let Some(timer) = &built.gpu && timer.stamps.get(index).is_some_and(|c| c.len() == command_buffers.len() + 1) {
            // Um carimbo antes do primeiro efeito e um depois de cada um.
            let stamps = &timer.stamps[index];
            let mut all = Vec::with_capacity(command_buffers.len() * 2 + 1);
            all.push(stamps[0]);
            for (i, cb) in command_buffers.iter().enumerate() {
                all.push(*cb);
                all.push(stamps[i + 1]);
            }
            command_buffers = all;
            timed = true;
        }
        // A profundidade que a cadeia lê é a foto: se nenhuma regra copiou neste quadro, copia agora (como o ReShade faz na hora dos
        // efeitos), com o layout de agora, no mesmo envio e antes dos efeitos.
        if let (Some(key), Some(snapshot)) = (built.depth_key, data.depth.snapshot_raw())
            && key.0 == snapshot
        {
            let family = data.queues.lock().unwrap().get(&queue.as_raw()).copied().unwrap_or(0);
            match crate::depth::copy_for_present(data, family, index) {
                crate::depth::PresentCopy::Recorded(cb) => command_buffers.insert(0, cb),
                crate::depth::PresentCopy::NotNeeded => {}
                crate::depth::PresentCopy::Unavailable => {
                    // Sem cópia nunca gravada a foto está sem layout: não dá para os efeitos a lerem. A fence já foi resetada: um lote
                    // vazio a sinaliza de novo.
                    if !data.depth.snapshot_written.load(std::sync::atomic::Ordering::Relaxed) {
                        let _ = crate::queue_lock::with(queue.as_raw(), || data.device.queue_submit(queue, &[vk::SubmitInfo::default()], fence));
                        return None;
                    }
                }
            }
        }
        // Modo "ressinalizar": o envio dos efeitos espera os semáforos do jogo e os sinaliza DE NOVO, e o present segue esperando os mesmos
        // semáforos que o jogo entregou (a lista de espera do present não é trocada). O DXVK novo (3.x) usa `VK_EXT_swapchain_maintenance1`
        // (fence de apresentação) e `present_id2`, que contam com o que o present espera de fato. Padrão: trocar pelo nosso semáforo.
        let resignal = allow_resignal && data.depth.resignal_on() && !waits.is_empty();
        let signal: Vec<vk::Semaphore> = if resignal { waits.to_vec() } else { vec![built.semaphores[index]] };
        let submit = vk::SubmitInfo::default()
            .wait_semaphores(waits)
            .wait_dst_stage_mask(&stages)
            .command_buffers(&command_buffers)
            .signal_semaphores(&signal);
        match crate::queue_lock::with(queue.as_raw(), || data.device.queue_submit(queue, &[submit], fence)) {
            Ok(()) => {
                if timed && let Some(timer) = built.gpu.as_mut() {
                    timer.written[index] = true;
                }
                if resignal {
                    data.depth.resignal_applied.store(true, std::sync::atomic::Ordering::Relaxed);
                    None
                } else {
                    Some(signal[0])
                }
            }
            Err(e) => {
                log!("vkQueueSubmit do efeito falhou: erro {}", e.as_raw());
                // A fence já foi resetada; um lote vazio a sinaliza de novo para não travar o frame seguinte.
                let _ = crate::queue_lock::with(queue.as_raw(), || data.device.queue_submit(queue, &[vk::SubmitInfo::default()], fence));
                None
            }
        }
    }
}

/// Alguma etapa amostra a profundidade do jogo (semântica `DEPTH` num sampler de um passe da técnica)?
pub(super) fn steps_need_depth(steps: &[ResolvedStep]) -> bool {
    steps.iter().any(|s| {
        let m = &s.bundle.manifest;
        let Some(t) = m.techniques.get(s.technique) else { return false };
        t.passes.iter().flat_map(|p| &p.samplers).any(|b| {
            m.samplers.get(b.sampler as usize).is_some_and(|sampler| m.textures.iter().any(|tex| tex.unique_name == sampler.texture && tex.semantic.as_deref() == Some("DEPTH")))
        })
    })
}

/// O jogo vai destruir uma imagem de profundidade: os efeitos que a usam saem antes (têm uma view dela).
pub fn on_depth_image_destroyed(data: &DeviceData, image: u64) {
    let mut swapchains = data.swapchains.lock().unwrap();
    for fx in swapchains.values_mut() {
        if fx.retiring.as_ref().is_some_and(|b| b.depth_key.is_some_and(|k| k.0 == image)) {
            unsafe { fx.drop_retiring(&data.device) };
        }
        if matches!(&fx.fx, Fx::Ready(b) if b.depth_key.is_some_and(|k| k.0 == image)) || matches!(&fx.fx, Fx::Building(j) if j.depth_image == Some(image)) {
            unsafe { SwapchainFx::destroy_fx(&mut fx.fx, &data.device) };
            fx.fx = Fx::Unbuilt;
            log!("efeito descartado: o jogo destruiu a imagem de profundidade que ele usava");
        }
    }
}

/// A imagem que o swapchain apresenta agora (para a captura de tela); `None` se o swapchain não aceita o uso extra.
pub fn frame_target(data: &DeviceData, swapchain: u64, index: usize) -> Option<crate::screenshot::Target> {
    let swapchains = data.swapchains.lock().unwrap();
    let fx = swapchains.get(&swapchain)?;
    if fx.unusable.is_some() {
        return None;
    }
    Some(crate::screenshot::Target { image: *fx.images.get(index)?, extent: fx.extent, format: fx.format })
}

/// O tamanho do maior swapchain do dispositivo (a "tela" a que a profundidade deve se parecer); 0x0 se não há.
pub fn largest_extent(data: &DeviceData) -> (u32, u32) {
    data.swapchains.lock().unwrap().values().map(|fx| (fx.extent.width, fx.extent.height)).max_by_key(|(w, h)| u64::from(*w) * u64::from(*h)).unwrap_or((0, 0))
}

/// A imagem é de um swapchain deste dispositivo?
pub fn is_swapchain_image(data: &DeviceData, image: u64) -> bool {
    data.swapchains.lock().unwrap().values().any(|fx| fx.images.iter().any(|i| i.as_raw() == image))
}
