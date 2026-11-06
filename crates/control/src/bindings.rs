//! Tabela de atalhos do app: cada tecla de `config::KEY_NAMES` vira um atalho global ligado a uma ação. As teclas vêm do
//! `config.ini` (seção `[INPUT]`, editável no painel); a de ligar/desligar tem uma de fábrica.
//!
//! Nunca sugerir `CTRL+ALT+F1`..`F12`: o kernel troca de terminal virtual (VT), e com NVIDIA
//! proprietária no Wayland isso já travou a máquina de desenvolvimento (2026-10-09, hipótese).
//! Atenção ao digitar: `CTRL+SHIFT+F10` (o nosso) e `CTRL+ALT+F10` (tty10) ficam lado a lado.

use control_core::action::Action;
use control_core::tech_keys::TechKey;
use shortcuts::{trigger, Binding, Dispatcher, Error, Scope, Shortcut};
use control_core::i18n::{fmt, tr};

/// O que cada tecla do `config.ini` faz: `(nome no config.ini, id do atalho, ação, descrição)`.
/// Os ids são guardados pelo sistema: não renomear depois de publicados.
const TABLE: [(&str, &str, Action, &str); 5] = [
    ("KeyEffects", "toggle-effects", Action::ToggleEffects, "linux-reshade: turn the effects on/off"),
    ("KeyNextPreset", "next-preset", Action::NextPreset, "linux-reshade: next preset of the game"),
    ("KeyPreviousPreset", "previous-preset", Action::PreviousPreset, "linux-reshade: previous preset of the game"),
    ("KeyScreenshot", "screenshot", Action::Screenshot, "linux-reshade: capture the game screen"),
    ("KeyReload", "reload-effects", Action::ReloadEffects, "linux-reshade: reload the effects"),
];

/// Todos os ids fixos que o programa pode registrar: os que ficam sem tecla são esquecidos pelo sistema.
pub fn all_ids() -> Vec<&'static str> {
    TABLE.iter().map(|(_, id, _, _)| *id).collect()
}

/// Os atalhos pedidos: `(id, tecla no formato XDG)` de cada um que tem tecla (os do `config.ini` e os de técnica dos presets). Uma
/// tecla que não se entende é avisada e ignorada.
pub fn wanted(config: &config::Config, techs: &[TechKey]) -> Vec<(String, String)> {
    let mut wanted: Vec<(String, String)> = TABLE
        .iter()
        .filter_map(|(key, id, _, _)| {
            let text = config.key(key);
            if text.trim().is_empty() {
                return None;
            }
            match trigger::normalize(&text) {
                Ok(t) => Some(((*id).to_string(), t)),
                Err(why) => {
                    control_core::elogln!("aviso: {key}={text:?} ignorado ({why})");
                    None
                }
            }
        })
        .collect();
    wanted.extend(techs.iter().map(|k| (k.id.clone(), k.trigger.clone())));
    wanted
}

/// O despachante dos atalhos pedidos. O atalho de técnica `techs[i]` vira `Action::Technique(i)`.
pub fn dispatcher(wanted: &[(String, String)], techs: &[TechKey]) -> Result<Dispatcher<Action>, Error> {
    Dispatcher::new(
        wanted
            .iter()
            .filter_map(|(id, trigger)| {
                let (action, description) = match TABLE.iter().find(|(_, table_id, _, _)| table_id == id) {
                    Some((_, _, action, description)) => (*action, tr(description).to_string()),
                    None => {
                        let index = techs.iter().position(|k| &k.id == id)?;
                        (Action::Technique(index), fmt("linux-reshade: turn {} on/off ({})", &[&techs[index].technique.name, &techs[index].game.as_deref().unwrap_or(tr("all games"))]))
                    }
                };
                Some(Binding::on_press(Shortcut { id: id.clone(), description, preferred_trigger: Some(trigger.clone()) }, Scope::Global, action))
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with(keys: &[(&str, &str)]) -> config::Config {
        let mut c = config::Config::resolve(&|_| None);
        c.keys = keys.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect();
        c
    }

    #[test]
    fn by_default_only_the_toggle_has_a_key() {
        let w = wanted(&config_with(&[]), &[]);
        assert_eq!(w, [("toggle-effects".to_string(), "CTRL+SHIFT+F10".to_string())]);
        let ids: Vec<_> = dispatcher(&w, &[]).unwrap().global_shortcuts().into_iter().map(|s| s.id).collect();
        assert_eq!(ids, ["toggle-effects"]);
    }

    #[test]
    fn the_config_changes_keys_and_adds_actions_and_the_oem_key_works() {
        let w = wanted(&config_with(&[("KeyEffects", "OEM_7"), ("KeyNextPreset", "Ctrl+Next"), ("KeyPreviousPreset", "")]), &[]);
        assert_eq!(w, [("toggle-effects".to_string(), "apostrophe".to_string()), ("next-preset".to_string(), "CTRL+Next".to_string())]);
    }

    #[test]
    fn an_empty_key_removes_the_shortcut_and_a_bad_one_is_skipped() {
        assert!(wanted(&config_with(&[("KeyEffects", "")]), &[]).is_empty());
        assert!(wanted(&config_with(&[("KeyEffects", "Ctrl+Banana")]), &[]).is_empty());
    }

    #[test]
    fn technique_keys_become_numbered_actions() {
        let techs = vec![TechKey { id: "tech.global.Sepia".into(), game: None, technique: preset::TechId::new("Sepia", "Sepia.fx"), trigger: "F9".into() }];
        let w = wanted(&config_with(&[("KeyEffects", "")]), &techs);
        assert_eq!(w, [("tech.global.Sepia".to_string(), "F9".to_string())]);
        let d = dispatcher(&w, &techs).unwrap();
        let event = shortcuts::Event { id: "tech.global.Sepia".into(), phase: shortcuts::Phase::Pressed, timestamp: 0 };
        assert_eq!(d.resolve(&event, &shortcuts::Context { window_visible: false }), Some(&Action::Technique(0)));
    }
}
