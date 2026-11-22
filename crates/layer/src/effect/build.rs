//! A criação de uma cadeia de efeitos (inteira, ou só dos efeitos que faltam), feita numa thread própria.

use super::*;

/// De onde vêm os uniforms de uma etapa nova.
pub(super) fn runtime_for(step: &ResolvedStep) -> StepRuntime {
    StepRuntime {
        params_id: step.bundle.params_id(),
        source: match &step.params {
            StepParams::Legacy => StepSource::Published(ParamState::new(fx_bundle::uniforms::default_block(&step.bundle.manifest))),
            StepParams::Plan(bytes) => StepSource::Plan(bytes.clone()),
        },
    }
}

/// Cria só os efeitos que faltam (ver [`Partial`]) numa thread: os de `reuse` já existem na cadeia aposentada.
pub(super) fn build_partial(a: BuildArgs, reuse: Vec<Option<usize>>, mut shared: postfx::shared::Shared) -> Result<Output, Failure> {
    for (n, step) in a.steps.iter().enumerate() {
        let t = step.bundle.manifest.target;
        if (t.width, t.height) != (a.extent.width, a.extent.height) {
            return Err(Failure { why: format!("step {n}: the bundle was compiled for {}x{} and the swapchain is {}x{}", t.width, t.height, a.extent.width, a.extent.height), shared: Some(shared) });
        }
    }
    let device = &a.device;
    let device_handle = device.handle();
    let set_loader_data = a.set_loader_data;
    let depth = a.depth;
    let target = Target {
        images: &a.images,
        format: a.format,
        extent: a.extent,
        rest_layout: vk::ImageLayout::PRESENT_SRC_KHR,
        queue_family: a.queue_family,
        init_command_buffer: Some(&move |cb| {
            let result = unsafe { set_loader_data(device_handle, cb.as_raw() as *mut std::ffi::c_void) };
            if result == vk::Result::SUCCESS { Ok(()) } else { Err(result) }
        }),
        depth: depth.map(|d| DepthInput {
            image: vk::Image::from_raw(d.image),
            format: vk::Format::from_raw(d.format),
            layout: vk::ImageLayout::from_raw(d.layout),
            extent: vk::Extent2D { width: d.width, height: d.height },
            linear_filter: d.linear,
        }),
    };
    let chain_steps: Vec<Step<'_>> = a.steps.iter().map(|s| Step { bundle: &s.bundle, technique: s.technique }).collect();
    let flags: Vec<bool> = reuse.iter().map(Option::is_some).collect();
    match unsafe { postfx::build_missing(device, &a.memory, &chain_steps, &flags, &target, &mut shared) } {
        Ok((effects, skipped)) => {
            for s in &skipped {
                log!("etapa {} pulada: {}", s.step, s.why);
            }
            Ok(Output::Partial(Partial { effects, skipped, shared, reuse }))
        }
        Err(e) => Err(Failure { why: e.to_string(), shared: Some(shared) }),
    }
}

/// Junta os efeitos reaproveitados da cadeia aposentada com os novos, na ordem das etapas. A cadeia aposentada deixa de existir
/// (o que sobrou dela é destruído); as fences e os semáforos dela passam para a nova.
pub(super) fn assemble_partial(old: Option<Built>, partial: Partial, steps: &[ResolvedStep], depth_key: Option<(u64, i32)>, device: &ash::Device) -> Result<Built, (String, Option<postfx::shared::Shared>)> {
    let Partial { effects: mut new_effects, skipped, shared, reuse } = partial;
    let Some(old) = old else {
        for effect in new_effects.iter_mut().flatten() {
            unsafe { effect.destroy(device) };
        }
        return Err(("the previous chain vanished before the update finished".into(), Some(shared)));
    };
    unsafe {
        if !old.fences.is_empty() {
            let _ = device.wait_for_fences(&old.fences, true, 5_000_000_000);
        }
    }
    let Built { chain, params: old_params, semaphores, fences, gpu, .. } = old;
    if let Some(mut timer) = gpu {
        unsafe { timer.destroy(device) };
    }
    let mut parts = chain.into_parts();
    let mut old_params: Vec<Option<StepRuntime>> = old_params.into_iter().map(Some).collect();
    let (mut effects, mut origin, mut params) = (Vec::new(), Vec::new(), Vec::new());
    for (n, step) in steps.iter().enumerate() {
        if let Some(i) = reuse.get(n).copied().flatten() {
            let (Some(effect), Some(mut runtime)) = (parts.effects.get_mut(i).and_then(Option::take), old_params.get_mut(i).and_then(Option::take)) else { continue };
            // Os uniforms do plano podem ter mudado.
            if let (StepSource::Plan(bytes), StepParams::Plan(new)) = (&mut runtime.source, &step.params) {
                *bytes = new.clone();
            }
            effects.push(effect);
            origin.push(n);
            params.push(runtime);
        } else if let Some(effect) = new_effects.get_mut(n).and_then(Option::take) {
            effects.push(effect);
            origin.push(n);
            params.push(runtime_for(step));
        }
    }
    // O que a cadeia aposentada tinha e a nova não usa.
    for effect in parts.effects.iter_mut().flatten() {
        unsafe { effect.destroy(device) };
    }
    let chain = Chain::from_parts(effects, shared, origin.clone());
    Ok(Built { chain, signature: signature(steps), origin, skipped: skipped.iter().map(|s| (s.step, s.why.clone())).collect(), depth_key, params, semaphores, fences, gpu: None })
}

/// Cria a cadeia (sem usar nenhuma fila: a inicialização das texturas termina depois, na thread do present).
pub(super) fn build_chain(a: BuildArgs) -> Result<Built, String> {
    for (n, step) in a.steps.iter().enumerate() {
        let t = step.bundle.manifest.target;
        if (t.width, t.height) != (a.extent.width, a.extent.height) {
            return Err(format!("step {n}: the bundle was compiled for {}x{} and the swapchain is {}x{}", t.width, t.height, a.extent.width, a.extent.height));
        }
    }
    let device = &a.device;
    let device_handle = device.handle();
    let set_loader_data = a.set_loader_data;
    let depth = a.depth;
    let target = Target {
        images: &a.images,
        format: a.format,
        extent: a.extent,
        rest_layout: vk::ImageLayout::PRESENT_SRC_KHR,
        queue_family: a.queue_family,
        // O command buffer nasce sem o ponteiro de dispatch do loader; as layers abaixo precisam dele.
        init_command_buffer: Some(&move |cb| {
            let result = unsafe { set_loader_data(device_handle, cb.as_raw() as *mut std::ffi::c_void) };
            if result == vk::Result::SUCCESS { Ok(()) } else { Err(result) }
        }),
        depth: depth.map(|d| DepthInput {
            image: vk::Image::from_raw(d.image),
            format: vk::Format::from_raw(d.format),
            layout: vk::ImageLayout::from_raw(d.layout),
            extent: vk::Extent2D { width: d.width, height: d.height },
            linear_filter: d.linear,
        }),
    };
    let steps = &a.steps;
    let chain_steps: Vec<Step<'_>> = steps.iter().map(|s| Step { bundle: &s.bundle, technique: s.technique }).collect();
    unsafe {
        let (chain, skipped) = Chain::new_lenient_with(device, &a.memory, &chain_steps, &target, None).map_err(|e| e.to_string())?;
        for s in &skipped {
            log!("etapa {} pulada: {}", s.step, s.why);
        }
        if chain.is_empty() && !steps.is_empty() {
            return Err(format!("no step is supported ({})", skipped.iter().map(|s| format!("step {}: {}", s.step, s.why)).collect::<Vec<_>>().join("; ")));
        }
        // Os parâmetros seguem as etapas que ficaram na cadeia (as puladas não têm efeito a alimentar).
        let origin: Vec<usize> = (0..chain.len()).filter_map(|i| chain.origin(i)).collect();
        let params = origin.iter().map(|&i| runtime_for(&steps[i])).collect();
        let skipped = skipped.iter().map(|s| (s.step, s.why.clone())).collect();
        let mut built = Built { chain, signature: signature(steps), origin, skipped, depth_key: depth.map(|d| (d.image, d.layout)), params, semaphores: Vec::new(), fences: Vec::new(), gpu: None };
        for _ in &a.images {
            let semaphore = device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None);
            // Nasce sinalizada: a primeira espera por ela não bloqueia.
            let fence = device.create_fence(&vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED), None);
            match (semaphore, fence) {
                (Ok(s), Ok(f)) => {
                    built.semaphores.push(s);
                    built.fences.push(f);
                }
                (s, f) => {
                    // Libera o par que deu certo e tudo o que já foi criado.
                    let error = s.as_ref().err().or(f.as_ref().err()).copied().unwrap_or(vk::Result::ERROR_UNKNOWN);
                    if let Ok(s) = s {
                        device.destroy_semaphore(s, None);
                    }
                    if let Ok(f) = f {
                        device.destroy_fence(f, None);
                    }
                    let mut partial = Fx::Ready(built);
                    SwapchainFx::destroy_fx(&mut partial, device);
                    return Err(format!("failed to create semaphore/fence (error {})", error.as_raw()));
                }
            }
        }
        Ok(built)
    }
}
