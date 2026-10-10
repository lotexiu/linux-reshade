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

/// O nome do executável do jogo, sem extensão. Sob Wine/Proton o processo é o do Wine; o `.exe` do jogo está no
/// primeiro argumento (`Z:\\...\\RocketLeague.exe`), então ele vale mais que o nome do binário.
fn exe_name() -> String {
    let from_cmdline = std::fs::read("/proc/self/cmdline").ok().and_then(|bytes| {
        let first = bytes.split(|b| *b == 0).next()?;
        exe_stem(&String::from_utf8_lossy(first)).filter(|_| first.windows(4).any(|w| w.eq_ignore_ascii_case(b".exe")))
    });
    from_cmdline.or_else(|| exe_stem(&process_name())).unwrap_or_default()
}

/// `Z:\\jogo\\Rocket League.exe` e `/usr/bin/vkcube` viram `Rocket League` e `vkcube`.
fn exe_stem(path: &str) -> Option<String> {
    let file = path.rsplit(['/', '\\']).next()?;
    let stem = match file.rsplit_once('.') {
        Some((stem, ext)) if ext.eq_ignore_ascii_case("exe") => stem,
        _ => file,
    };
    (!stem.is_empty()).then(|| stem.to_string())
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
        let status = LayerStatus { exe: exe_name(), app: state.app.clone(), swapchains: state.swapchains.iter().map(|(_, s)| s.clone()).collect() };
        if let Err(e) = writer.publish(&status) {
            log!("não consegui publicar o status: {e}");
        }
    }
}

/// O identificador desta layer para o app de controle; só existe depois do primeiro swapchain.
pub fn id() -> Option<String> {
    STATE.lock().unwrap().writer.as_ref().map(StatusWriter::id)
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

#[cfg(test)]
mod tests {
    use super::exe_stem;

    #[test]
    fn the_executable_name_loses_the_directory_and_the_exe_extension() {
        assert_eq!(exe_stem("Z:\\home\\u\\Rocket League\\RocketLeague.exe").as_deref(), Some("RocketLeague"));
        assert_eq!(exe_stem("/usr/bin/vkcube").as_deref(), Some("vkcube"));
        assert_eq!(exe_stem("C:\\Game\\GAME.EXE").as_deref(), Some("GAME"));
        assert_eq!(exe_stem("/opt/app.bin").as_deref(), Some("app.bin"), "só .exe é extensão de verdade");
        assert_eq!(exe_stem(""), None);
    }
}
