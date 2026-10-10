//! Observa os passes que desenham na imagem do swapchain (a "tela"), para achar onde a cena termina e a interface do
//! jogo começa: base do modo "efeito antes da UI" (ver `docs/antes-da-ui.md`). Só observa; não muda nada no jogo.

use super::observe::LoadOp;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

/// Um passe que desenhou numa imagem do swapchain.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ScreenPass {
    pub load: LoadOp,
    pub draws: u32,
}

#[derive(Default)]
pub(crate) struct Screen {
    /// Views de imagens do swapchain.
    views: Mutex<HashSet<u64>>,
    /// framebuffer -> views dos anexos (só os que têm uma view do swapchain).
    framebuffers: Mutex<HashMap<u64, Vec<u64>>>,
    /// render pass -> `loadOp` de cada anexo.
    render_pass_loads: Mutex<HashMap<u64, Vec<LoadOp>>>,
    /// Os passes do quadro, na ordem dos envios à fila.
    frame: Mutex<Vec<ScreenPass>>,
}

impl Screen {
    pub fn add_view(&self, view: u64) {
        self.views.lock().unwrap().insert(view);
    }

    pub fn is_view(&self, view: u64) -> bool {
        self.views.lock().unwrap().contains(&view)
    }

    pub fn add_framebuffer(&self, framebuffer: u64, views: &[u64]) {
        if views.iter().any(|v| self.is_view(*v)) {
            self.framebuffers.lock().unwrap().insert(framebuffer, views.to_vec());
        }
    }

    pub fn add_render_pass(&self, render_pass: u64, loads: Vec<LoadOp>) {
        self.render_pass_loads.lock().unwrap().insert(render_pass, loads);
    }

    /// O passe que começa agora desenha no swapchain? (`views` vem do início do passe se o framebuffer não tem imagens.)
    pub fn begin_pass(&self, render_pass: u64, framebuffer: u64, views: Option<Vec<u64>>) -> Option<ScreenPass> {
        let views = views.or_else(|| self.framebuffers.lock().unwrap().get(&framebuffer).cloned())?;
        let position = views.iter().position(|v| self.is_view(*v))?;
        let load = self.render_pass_loads.lock().unwrap().get(&render_pass).and_then(|l| l.get(position).copied()).unwrap_or(LoadOp::None);
        Some(ScreenPass { load, draws: 0 })
    }

    pub fn push_frame(&self, passes: impl IntoIterator<Item = ScreenPass>) {
        self.frame.lock().unwrap().extend(passes);
    }

    /// Os passes do quadro que acabou, num resumo de uma linha (`vazio` se nenhum desenhou na tela).
    pub fn take_summary(&self) -> String {
        let passes = std::mem::take(&mut *self.frame.lock().unwrap());
        passes.iter().enumerate().map(|(i, p)| format!("{}:{}d {:?}", i + 1, p.draws, p.load)).collect::<Vec<_>>().join(" ")
    }
}
