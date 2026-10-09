//! Leitura do estado escrito pelo app de controle (ver o crate `ipc`).

use crate::log::log;
use std::sync::OnceLock;

/// De quantos em quantos presents tentar abrir o estado compartilhado enquanto o app de controle
/// ainda não o criou (uma chamada `open`, barata).
const STATE_RETRY_EVERY: u64 = 120;

/// De quantos em quantos presents procurar o canal desta layer enquanto o app de controle não o criou.
/// Procurar é uma chamada `open` que falha (barata); o intervalo curto faz uma escolha por jogo aparecer logo.
const CHANNEL_RETRY_EVERY: u64 = 30;

/// Depois de aberto, é lido por `load` atômico, sem lock.
static CONTROL: OnceLock<ipc::Reader> = OnceLock::new();
/// O canal só desta layer (escolha e parâmetros próprios), criado pelo app de controle quando o usuário
/// configura este jogo. Vale antes do canal global.
static CHANNEL: OnceLock<ipc::ChannelReader> = OnceLock::new();

/// O canal com o app de controle, se já foi aberto (por `effects_enabled`, a cada present).
pub fn reader() -> Option<&'static ipc::Reader> {
    CONTROL.get()
}

/// O canal próprio desta layer, se o app de controle já o criou.
pub fn channel() -> Option<&'static ipc::ChannelReader> {
    CHANNEL.get()
}

/// Procura o canal desta layer de tempos em tempos, até achar. Só existe depois do primeiro swapchain
/// (é quando a layer ganha um identificador).
fn find_channel(present_number: u64) {
    if CHANNEL.get().is_some() || present_number % CHANNEL_RETRY_EVERY != 1 {
        return;
    }
    let Some(id) = crate::status::id() else { return };
    match ipc::ChannelReader::open(&id) {
        Ok(Some(channel)) => {
            log!("canal próprio aberto: {id}");
            let _ = CHANNEL.set(channel);
        }
        Ok(None) => {}
        Err(e) => log!("falha ao abrir o canal próprio: {e}"),
    }
}

/// Valor atual da flag de efeitos. Enquanto o app de controle não existe, os efeitos ficam desligados.
pub fn effects_enabled(present_number: u64) -> bool {
    find_channel(present_number);
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
