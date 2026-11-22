//! O que sai de uso mas só pode ser destruído mais tarde: os semáforos aposentados e as criações de cadeia descartadas em segundo plano.

use super::*;

/// Espera as limpezas em segundo plano acabarem (até `timeout`). Chamada antes de destruir um swapchain ou o dispositivo: a cadeia
/// em criação guarda views das imagens dele.
pub fn wait_cleanups(timeout: std::time::Duration) {
    let started = Instant::now();
    while PENDING_CLEANUPS.load(std::sync::atomic::Ordering::Acquire) > 0 && started.elapsed() < timeout {
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

/// Descarta uma criação em andamento sem parar o jogo: outra thread espera o fim da criação e destrói o resultado (que a GPU
/// nunca usou).
pub(super) fn discard_job_in_background(fx: &mut Fx, device: &ash::Device) {
    let Fx::Building(mut job) = std::mem::replace(fx, Fx::Unbuilt) else { return };
    let Some(thread) = job.thread.take() else { return };
    let device = device.clone();
    PENDING_CLEANUPS.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    let spawned = std::thread::Builder::new().name("lr-cleanup".into()).spawn({
        let device = device.clone();
        move || {
            if let Ok(SendOut(Ok(Output::Full(built)))) = thread.join() {
                let mut dead = Fx::Ready(built);
                // SAFETY: a cadeia acabou de ser criada e nunca foi enviada à GPU.
                unsafe { SwapchainFx::destroy_fx(&mut dead, &device) };
            }
            PENDING_CLEANUPS.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
        }
    });
    if spawned.is_err() {
        PENDING_CLEANUPS.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

pub(crate) fn retire_semaphores(device: &ash::Device, semaphores: impl Iterator<Item = vk::Semaphore>) {
    let key = device.handle().as_raw();
    let mut retired = RETIRED_SEMAPHORES.lock().unwrap();
    retired.extend(semaphores.map(|s| (key, s, SEMAPHORE_RETIRE_PRESENTS)));
}

/// Conta um present para os semáforos aposentados deste dispositivo e destrói os que chegaram ao fim (todos, com `all`).
pub unsafe fn reap_semaphores(device: &ash::Device, all: bool) {
    let key = device.handle().as_raw();
    let mut retired = RETIRED_SEMAPHORES.lock().unwrap();
    retired.retain_mut(|(owner, semaphore, left)| {
        if *owner != key {
            return true;
        }
        *left = left.saturating_sub(1);
        if all || *left == 0 {
            unsafe { device.destroy_semaphore(*semaphore, None) };
            false
        } else {
            true
        }
    });
}
