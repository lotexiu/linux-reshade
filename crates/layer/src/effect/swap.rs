//! O ciclo de vida da cadeia de um swapchain: construir quando preciso, trocar, aposentar e destruir.

use super::*;

impl SwapchainFx {
    pub fn new(handle: u64, images: Vec<vk::Image>, format: vk::Format, extent: vk::Extent2D, unusable: Option<String>) -> Self {
        let state = unusable.clone().map_or(EffectState::Idle, EffectState::Unavailable);
        let fx = Self { handle, images, format, extent, unusable, fx: Fx::Unbuilt, retiring: None, graveyard: Vec::new(), seen_epoch: u64::MAX, failed_depth: None, steps: Vec::new(), state, last_present: None };
        crate::status::update(handle, fx.status());
        fx
    }

    pub(super) fn status(&self) -> SwapchainStatus {
        SwapchainStatus {
            width: self.extent.width,
            height: self.extent.height,
            format: self.format.as_raw(),
            images: self.images.len() as u32,
            effect: self.state.clone(),
        }
    }

    /// Muda o estado e, se mudou, conta ao app de controle.
    pub(super) fn set_state(&mut self, state: EffectState) {
        if self.state != state {
            self.state = state;
            crate::status::update(self.handle, self.status());
        }
    }

    /// A escolha de efeito mudou (ou é a primeira vez): resolve o bundle desta resolução e descarta o
    /// que foi construído com outro. Não toca na GPU além disso; a construção é preguiçosa.
    pub(super) unsafe fn on_epoch(&mut self, data: &DeviceData, epoch: u64) {
        self.seen_epoch = epoch;
        if self.unusable.is_some() {
            return;
        }
        let (steps, state) = match bundle::resolve(self.extent.width, self.extent.height) {
            Resolved::None => (Vec::new(), EffectState::Idle),
            Resolved::Waiting(dirs) => {
                let dirs: Vec<_> = dirs.iter().map(|d| d.display().to_string()).collect();
                log!("swapchain {}x{}: ainda não há bundle, esperando {}", self.extent.width, self.extent.height, dirs.join(", "));
                (Vec::new(), EffectState::Waiting)
            }
            Resolved::Invalid(why) => (Vec::new(), EffectState::Failed(why)),
            Resolved::Ready(steps) => (steps, EffectState::Ready),
        };
        let new_signature = signature(&steps);
        if let Fx::Ready(built) = &self.fx {
            if built.signature != new_signature {
                log!("efeito do swapchain {:#x} descartado: o bundle mudou (a cadeia é outra)", self.handle);
                crate::status::set_techniques(Vec::new());
                if !steps.is_empty() && (hard_swap() || kinds(&self.steps) == kinds(&steps)) {
                    // Mesmos efeitos e técnicas, outras variantes: foi uma mudança de definições (macros). Como no ReShade, os efeitos saem de
                    // vez e voltam quando a cadeia nova ficar pronta (um piscar), sem a velha e a nova coexistindo.
                    log!("definições mudaram (mesmos efeitos, outras variantes): troca completa, sem a cadeia velha");
                    unsafe { self.drop_everything(&data.device) };
                } else {
                    // A cadeia velha segue desenhando até a nova ficar pronta (ligar/desligar efeito não deve piscar).
                    self.retire(&data.device);
                }
            } else if let Some(built) = match &mut self.fx { Fx::Ready(b) => Some(b), _ => None } {
                // A mesma cadeia: só os uniforms do plano podem ter mudado.
                log!("plano: mesma cadeia, só os valores dos uniforms foram atualizados");
                for (runtime, &from) in built.params.iter_mut().zip(&built.origin) {
                    if let (StepSource::Plan(bytes), Some(StepParams::Plan(new))) = (&mut runtime.source, steps.get(from).map(|s| &s.params)) {
                        *bytes = new.clone();
                    }
                }
            }
        }
        if let Fx::Building(job) = &self.fx
            && job.signature != new_signature
        {
            if job.incremental {
                // Termina e instala (as texturas compartilhadas estão na thread); depois da instalação a escolha é olhada de novo e a
                // diferença vira o próximo trabalho.
            } else {
                log!("criação da cadeia descartada: a escolha mudou");
                // Sem esperar a thread terminar (a primeira compilação leva segundos e parava o jogo): ela se desfaz sozinha.
                discard_job_in_background(&mut self.fx, &data.device);
            }
        }
        // Uma falha anterior era da cadeia antiga: com outra vale tentar de novo.
        if matches!(self.fx, Fx::Failed(_)) && signature(&self.steps) != new_signature {
            self.fx = Fx::Unbuilt;
        }
        if steps.is_empty() {
            // Nenhum efeito para esta resolução: nada mais a desenhar.
            unsafe { self.drop_retiring(&data.device) };
        }
        self.steps = steps;
        let state = match &self.fx {
            Fx::Failed(why) => EffectState::Failed(why.clone()),
            _ => state,
        };
        self.set_state(state);
    }

    /// `LINUX_RESHADE_SAFE_SWAP=1`: espera a GPU ficar ociosa antes de destruir uma cadeia de efeitos.
    pub(super) fn safe_swap() -> bool {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ON.get_or_init(|| std::env::var_os("LINUX_RESHADE_SAFE_SWAP").is_some_and(|v| v != "0"))
    }

    /// Libera os recursos de GPU.
    ///
    /// # Safety
    /// A GPU não pode estar executando trabalho deste swapchain (o chamador espera o dispositivo ficar ocioso).
    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        unsafe {
            self.drop_retiring(device);
            self.flush_graveyard(device, true);
            Self::destroy_fx(&mut self.fx, device);
        }
        self.fx = Fx::Failed("destroyed".into());
        crate::status::remove(self.handle);
    }

    /// A cadeia em uso sai de cena, mas continua desenhando até a próxima ficar pronta.
    pub(super) fn retire(&mut self, device: &ash::Device) {
        if let Fx::Ready(_) = &self.fx {
            let old = std::mem::replace(&mut self.fx, Fx::Unbuilt);
            // Uma cadeia já aposentada que ainda não foi trocada sai de vez (só uma espera).
            unsafe { self.bury_retiring(device) };
            if let Fx::Ready(built) = old {
                self.retiring = Some(built);
            }
        }
    }

    /// Como `drop_retiring`, mas com `LINUX_RESHADE_LATE_FREE=1` a cadeia só é destruída uns 300 presents depois.
    pub(super) unsafe fn bury_retiring(&mut self, device: &ash::Device) {
        if late_free() && let Some(built) = self.retiring.take() {
            log!("cadeia aposentada guardada: será destruída em {LATE_FREE_PRESENTS} presents");
            self.graveyard.push((built, LATE_FREE_PRESENTS));
        } else {
            unsafe { self.drop_retiring(device) };
        }
    }

    /// Conta um present para as cadeias guardadas e destrói as que chegaram ao fim (todas, com `all`).
    pub(super) unsafe fn flush_graveyard(&mut self, device: &ash::Device, all: bool) {
        let mut i = 0;
        while i < self.graveyard.len() {
            self.graveyard[i].1 = self.graveyard[i].1.saturating_sub(1);
            if all || self.graveyard[i].1 == 0 {
                let (built, _) = self.graveyard.remove(i);
                let mut old = Fx::Ready(built);
                unsafe { Self::destroy_fx(&mut old, device) };
                log!("cadeia guardada destruída");
            } else {
                i += 1;
            }
        }
    }

    /// Destrói já a cadeia em uso, a aposentada e as guardadas (espera as fences de cada uma); o swapchain fica sem efeito até criar outra.
    pub(super) unsafe fn drop_everything(&mut self, device: &ash::Device) {
        unsafe {
            if Self::safe_swap() {
                let _ = device.device_wait_idle();
            }
            self.drop_retiring(device);
            self.flush_graveyard(device, true);
            if matches!(self.fx, Fx::Ready(_)) {
                Self::destroy_fx(&mut self.fx, device);
            }
        }
        self.fx = Fx::Unbuilt;
    }

    pub(super) unsafe fn drop_retiring(&mut self, device: &ash::Device) {
        if let Some(built) = self.retiring.take() {
            // Diagnóstico dos Xid 69 (erro de classe da GPU) que aparecem depois de trocar a cadeia de efeitos: com
            // `LINUX_RESHADE_SAFE_SWAP=1` a GPU fica ociosa antes de a cadeia antiga ser destruída. Se os erros somem, a causa é um
            // recurso destruído enquanto a GPU ainda o usava.
            if Self::safe_swap() {
                let _ = unsafe { device.device_wait_idle() };
            }
            log!("cadeia aposentada: destruindo os recursos dela (espera as fences)");
            let mut old = Fx::Ready(built);
            unsafe { Self::destroy_fx(&mut old, device) };
            log!("cadeia aposentada destruída");
        }
    }

    pub(super) unsafe fn destroy_fx(fx: &mut Fx, device: &ash::Device) {
        let built = match std::mem::replace(fx, Fx::Unbuilt) {
            Fx::Ready(built) => Some(built),
            // Uma criação em andamento: espera a thread terminar e descarta o resultado (nada o usou).
            Fx::Building(mut job) => match job.thread.take().and_then(|t| t.join().ok()) {
                Some(SendOut(Ok(Output::Full(built)))) => Some(built),
                // Os efeitos novos saem antes das texturas compartilhadas que apelidam; a cadeia aposentada (que também as apelida)
                // já saiu antes (ver `drop_retiring` nos chamadores).
                Some(SendOut(Ok(Output::Partial(mut partial)))) => {
                    unsafe {
                        for effect in partial.effects.iter_mut().flatten() {
                            effect.destroy(device);
                        }
                        partial.shared.destroy(device);
                    }
                    None
                }
                Some(SendOut(Err(mut failure))) => {
                    if let Some(shared) = failure.shared.as_mut() {
                        unsafe { shared.destroy(device) };
                    }
                    None
                }
                None => None,
            },
            _ => None,
        };
        if let Some(mut built) = built {
            unsafe {
                // Só o trabalho da própria cadeia precisa terminar (suas fences). `vkDeviceWaitIdle` exige que nenhuma outra thread
                // use NENHUMA fila durante a espera, e o jogo submete de outras threads: era a suspeita dos erros de GPU (Xid 69)
                // ao trocar de efeitos.
                if !built.fences.is_empty() {
                    let _ = device.wait_for_fences(&built.fences, true, 5_000_000_000);
                }
                if let Some(timer) = built.gpu.as_mut() {
                    timer.destroy(device);
                }
                built.chain.destroy(device);
                // Um present já enfileirado ainda pode esperar o semáforo do efeito: ele sai alguns presents depois.
                retire_semaphores(device, built.semaphores.drain(..));
                for f in built.fences.drain(..) {
                    device.destroy_fence(f, None);
                }
            }
        }
    }

}
