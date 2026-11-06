//! Atalhos de técnica: o `Key<técnica>=código,ctrl,shift,alt` que o ReShade guarda no preset liga e desliga aquela técnica. Aqui
//! eles viram atalhos globais (o daemon os registra) e a ação de ligar/desligar.

use preset::{KeyCombo, Preset, TechId};
use std::path::{Path, PathBuf};
use crate::i18n::tr;

/// Um atalho de técnica achado num preset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TechKey {
    /// O id do atalho no portal: único por preset e técnica.
    pub id: String,
    /// O jogo dono do preset (`None`: o preset global).
    pub game: Option<String>,
    pub technique: TechId,
    /// A tecla no formato XDG (`CTRL+F11`).
    pub trigger: String,
}

fn sanitize(text: &str) -> String {
    text.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' }).collect()
}

fn keys_of(preset: &Preset, game: Option<&str>, out: &mut Vec<TechKey>) {
    let mut ids = preset.sorting();
    for id in preset.techniques() {
        if !ids.iter().any(|known| known.matches(&id)) {
            ids.push(id);
        }
    }
    for id in ids {
        let Some(KeyCombo { code, ctrl, shift, alt }) = preset.technique_key(&id) else { continue };
        let Some(trigger) = shortcuts::trigger::from_vk(code, ctrl, shift, alt) else { continue };
        let owner = game.map_or_else(|| "global".to_string(), sanitize);
        out.push(TechKey { id: format!("tech.{owner}.{}", sanitize(&id.name)), game: game.map(String::from), technique: id, trigger });
    }
}

/// Os atalhos de técnica do preset global e do preset que vale para cada jogo.
pub fn scan(presets_dir: &Path) -> Vec<TechKey> {
    let mut out = Vec::new();
    if let Ok(preset) = Preset::load(&presets_dir.join("preset.ini")) {
        keys_of(&preset, None, &mut out);
    }
    let mut games: Vec<String> = std::fs::read_dir(presets_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    games.sort();
    for game in games {
        if let Some(found) = preset::game::find(presets_dir, &game)
            && found.source == preset::game::Source::Game
            && let Ok(preset) = Preset::load(&found.path)
        {
            keys_of(&preset, Some(&game), &mut out);
        }
    }
    out
}

/// O arquivo de preset que o atalho edita.
fn path_of(presets_dir: &Path, key: &TechKey) -> Option<PathBuf> {
    match &key.game {
        Some(game) => preset::game::find(presets_dir, game).map(|f| f.path),
        None => Some(presets_dir.join("preset.ini")),
    }
}

/// Liga a técnica se estava desligada e desliga se estava ligada, e grava o preset (o vigia aplica). Devolve o novo estado.
pub fn toggle(presets_dir: &Path, key: &TechKey) -> Result<bool, String> {
    let path = path_of(presets_dir, key).ok_or(tr("the game has no preset"))?;
    let mut preset = Preset::load(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let on = !preset.is_enabled(&key.technique);
    preset.set_enabled(&key.technique, on);
    preset.save(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(on)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn technique_keys_are_found_in_global_and_game_presets_and_toggle_the_right_file() {
        let dir = std::env::temp_dir().join(format!("lr-techkeys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Jogo")).unwrap();
        std::fs::write(dir.join("preset.ini"), "Techniques=Sepia@Sepia.fx\nKeySepia@Sepia.fx=122,1,0,0\n").unwrap();
        std::fs::write(dir.join("Jogo/game.ini"), "preset=A.ini\n").unwrap();
        std::fs::write(dir.join("Jogo/A.ini"), "Techniques=\nTechniqueSorting=Bloom@Bloom.fx\nKeyBloom@Bloom.fx=222,0,0,0\n").unwrap();
        let keys = scan(&dir);
        assert_eq!(keys.iter().map(|k| (k.id.as_str(), k.trigger.as_str())).collect::<Vec<_>>(), [("tech.global.Sepia", "CTRL+F11"), ("tech.Jogo.Bloom", "apostrophe")]);
        assert!(toggle(&dir, &keys[1]).unwrap(), "estava desligada: liga");
        assert!(std::fs::read_to_string(dir.join("Jogo/A.ini")).unwrap().contains("Techniques=Bloom@Bloom.fx"));
        assert!(!toggle(&dir, &keys[0]).unwrap(), "estava ligada: desliga");
        assert!(!toggle(&dir, &keys[1]).unwrap(), "e de volta");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
