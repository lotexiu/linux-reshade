//! Registro de falhas da layer: um pânico de Rust dentro de um jogo vira um `abort` sem mensagem visível. Com
//! `LINUX_RESHADE_LOG`, o gancho de pânico grava a mensagem, o local e a pilha no log antes de a falha seguir. Com
//! `LINUX_RESHADE_SIGNALS=1` o mesmo vale para SIGSEGV/SIGABRT/SIGBUS/SIGILL (falhas do driver ou do jogo).

use crate::log::log;
use std::sync::Once;

unsafe extern "C" {
    fn signal(signum: i32, handler: usize) -> usize;
    fn raise(signum: i32) -> i32;
}

const SIGILL: i32 = 4;
const SIGABRT: i32 = 6;
const SIGBUS: i32 = 7;
const SIGSEGV: i32 = 11;

/// Grava a pilha no log e deixa o sinal seguir com o tratamento padrão (que mata o processo). Não é seguro dentro de um
/// sinal, mas o processo já está perdido: vale uma tentativa de deixar a causa escrita.
extern "C" fn on_signal(signum: i32) {
    let thread = std::thread::current();
    log!("SINAL {signum} na thread {:?}\n{}", thread.name().unwrap_or("?"), std::backtrace::Backtrace::force_capture());
    unsafe {
        signal(signum, 0);
        raise(signum);
    }
}

static ONCE: Once = Once::new();

/// Instala o gancho de pânico (uma vez). Só faz algo se o log está ligado: sem log, não há onde escrever e o gancho original
/// do processo fica como estava.
pub fn install() {
    ONCE.call_once(|| {
        if std::env::var_os("LINUX_RESHADE_LOG").is_none_or(|v| v.is_empty()) {
            return;
        }
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let thread = std::thread::current();
            log!("PÂNICO na thread {:?}: {info}\n{}", thread.name().unwrap_or("?"), std::backtrace::Backtrace::force_capture());
            previous(info);
        }));
        // Opt-in: jogos do Wine/Proton usam o SIGSEGV para si, e trocar o tratamento deles os quebraria.
        if std::env::var_os("LINUX_RESHADE_SIGNALS").is_some_and(|v| v == "1") {
            for sig in [SIGILL, SIGABRT, SIGBUS, SIGSEGV] {
                unsafe { signal(sig, on_signal as extern "C" fn(i32) as usize) };
            }
        }
    });
}
