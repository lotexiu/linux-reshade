//! Preferências da interface (não do preset nem dos jogos): `~/.config/linux-reshade/ui.ini`.
//!
//! Por enquanto uma só: se pergunta antes de apagar um preset. Quando a janela ganhar a aba de configurações, é
//! lá que ela se liga e desliga de novo.

use ini::Ini;
use std::path::PathBuf;

const KEY_CONFIRM_DELETE: &str = "confirm_delete_preset";

fn file() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join(config::LOCATION_DIR).join("ui.ini"))
}

/// Pergunta antes de apagar um preset? Sim, até a pessoa pedir para não perguntar mais.
pub fn confirm_delete_preset() -> bool {
    let Some(file) = file() else { return true };
    Ini::load(&file).ok().and_then(|i| i.get("", KEY_CONFIRM_DELETE).map(|v| v != "false")).unwrap_or(true)
}

pub fn set_confirm_delete_preset(ask: bool) {
    let Some(file) = file() else { return };
    let mut ini = Ini::load(&file).unwrap_or_default();
    ini.set("", KEY_CONFIRM_DELETE, if ask { "true" } else { "false" });
    let _ = ini.save(&file);
}
