//! Registro de falhas da layer: um pânico de Rust dentro de um jogo vira um `abort` sem mensagem visível. Com
//! `LINUX_RESHADE_LOG`, o gancho de pânico grava a mensagem, o local e a pilha no log antes de a falha seguir.

use crate::log::log;
use std::sync::Once;

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
    });
}
