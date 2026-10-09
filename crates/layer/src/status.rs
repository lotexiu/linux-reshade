//! O que a layer conta ao app de controle: o jogo, e o tamanho e o estado do efeito de cada swapchain.
//!
//! Há um arquivo de status por processo (ver `ipc::status`), criado no primeiro swapchain e apagado
//! quando a última instância Vulkan é destruída. Só se escreve quando alguma coisa mudou.

use crate::log::log;
use ipc::{LayerStatus, StatusWriter, SwapchainStatus};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

struct State {
    app: String,
    /// `(handle do swapchain, estado)`, na ordem de criação.
    swapchains: Vec<(u64, SwapchainStatus)>,
    writer: Option<StatusWriter>,
    /// Já tentamos criar o arquivo e falhou: não insiste (e já logou).
    writer_failed: bool,
}

static STATE: Mutex<State> = Mutex::new(State { app: String::new(), swapchains: Vec::new(), writer: None, writer_failed: false });
static INSTANCES: AtomicUsize = AtomicUsize::new(0);

/// Registra uma instância Vulkan criada. `app` é o nome que o jogo deu (ou vazio).
pub fn instance_created(app: &str) {
    INSTANCES.fetch_add(1, Ordering::AcqRel);
    let mut state = STATE.lock().unwrap();
    if state.app.is_empty() || state.app == "?" {
        state.app = if app.is_empty() || app == "?" { process_name() } else { app.to_string() };
    }
}

/// A última instância foi destruída: o arquivo de status deixa de fazer sentido.
pub fn instance_destroyed() {
    if INSTANCES.fetch_sub(1, Ordering::AcqRel) == 1 {
        let mut state = STATE.lock().unwrap();
        state.swapchains.clear();
        state.writer = None; // o Drop apaga o arquivo
        state.writer_failed = false;
    }
}

fn process_name() -> String {
    std::env::current_exe().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_else(|| "?".into())
}

fn publish(state: &mut State) {
    if state.writer.is_none() && !state.writer_failed {
        match StatusWriter::create() {
            Ok(w) => state.writer = Some(w),
            Err(e) => {
                log!("não consegui criar o arquivo de status: {e}");
                state.writer_failed = true;
            }
        }
    }
    if let Some(writer) = &state.writer {
        let status = LayerStatus { app: state.app.clone(), swapchains: state.swapchains.iter().map(|(_, s)| s.clone()).collect() };
        if let Err(e) = writer.publish(&status) {
            log!("não consegui publicar o status: {e}");
        }
    }
}

/// Registra ou atualiza o estado de um swapchain. Não escreve se nada mudou.
pub fn update(handle: u64, status: SwapchainStatus) {
    let mut state = STATE.lock().unwrap();
    match state.swapchains.iter_mut().find(|(h, _)| *h == handle) {
        Some((_, current)) if *current == status => return,
        Some((_, current)) => *current = status,
        None => state.swapchains.push((handle, status)),
    }
    publish(&mut state);
}

/// O swapchain foi destruído.
pub fn remove(handle: u64) {
    let mut state = STATE.lock().unwrap();
    let before = state.swapchains.len();
    state.swapchains.retain(|(h, _)| *h != handle);
    if state.swapchains.len() != before {
        publish(&mut state);
    }
}
