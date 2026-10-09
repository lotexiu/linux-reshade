//! Leitura do estado escrito pelo app de controle (ver o crate `ipc`).

use crate::log::log;
use std::sync::OnceLock;

/// De quantos em quantos presents tentar abrir o estado compartilhado enquanto o app de controle
/// ainda não o criou (uma chamada `open`, barata).
const STATE_RETRY_EVERY: u64 = 120;

/// Depois de aberto, é lido por `load` atômico, sem lock.
static CONTROL: OnceLock<ipc::Reader> = OnceLock::new();

/// O canal com o app de controle, se já foi aberto (por `effects_enabled`, a cada present).
pub fn reader() -> Option<&'static ipc::Reader> {
    CONTROL.get()
}

/// Valor atual da flag de efeitos. Enquanto o app de controle não existe, os efeitos ficam desligados.
pub fn effects_enabled(present_number: u64) -> bool {
    if let Some(reader) = CONTROL.get() {
        return reader.effects_enabled();
    }
    if present_number % STATE_RETRY_EVERY == 1 {
        match ipc::Reader::open() {
            Ok(Some(reader)) => {
                log!("estado compartilhado aberto: {}", ipc::state_path().display());
                return CONTROL.get_or_init(|| reader).effects_enabled();
            }
            Ok(None) => {}
            Err(e) => log!("falha ao abrir o estado compartilhado: {e}"),
        }
    }
    false
}
