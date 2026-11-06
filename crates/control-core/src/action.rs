//! O que o app de controle sabe fazer. Atalhos e CLI disparam as mesmas ações, para o
//! comportamento ficar num lugar só.

use std::fmt;
use i18n::{fmt, tr};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    ToggleEffects,
    EffectsOn,
    EffectsOff,
    /// Passa os jogos abertos para o preset seguinte/anterior (só no daemon, que sabe quais jogos estão abertos).
    NextPreset,
    PreviousPreset,
    /// Pede uma captura de tela a cada jogo com a layer aberta.
    Screenshot,
    /// Liga/desliga uma técnica pelo atalho dela (o número é a posição na lista que o daemon montou; quem executa é o daemon).
    Technique(usize),
    /// Recompila os efeitos dos jogos abertos (apaga os compilados deles; o vigia refaz).
    ReloadEffects,
}

impl Action {
    /// Executa a ação sobre o estado compartilhado e devolve o que mudou, para exibir.
    pub fn apply(self, state: &ipc::Writer) -> Applied {
        match self {
            Action::ToggleEffects => {
                state.toggle_effects();
            }
            Action::EffectsOn => state.set_effects_enabled(true),
            Action::EffectsOff => state.set_effects_enabled(false),
            // Sem jogos para olhar (a CLI): nada a mudar.
            Action::NextPreset | Action::PreviousPreset | Action::Screenshot | Action::Technique(_) | Action::ReloadEffects => {}
        }
        Applied { effects_enabled: state.effects_enabled() }
    }
}

impl Action {
    /// O que a ação faz no daemon: as de efeitos mexem no estado compartilhado; as de preset trocam o preset de cada jogo com a
    /// layer aberta. Devolve o texto para o log.
    pub fn run(self, state: &ipc::Writer, presets_dir: Option<&std::path::Path>, screenshot_before: bool) -> String {
        if matches!(self, Action::Technique(_) | Action::ReloadEffects) {
            return tr("action handled by the daemon").into();
        }
        if self == Action::Screenshot {
            return match ipc::shot::request_screenshot(screenshot_before) {
                Ok(n) => fmt("screenshot requested (#{})", &[&n]),
                Err(e) => fmt("could not request the screenshot: {}", &[&e]),
            };
        }
        let delta = match self {
            Action::NextPreset => 1,
            Action::PreviousPreset => -1,
            other => return other.apply(state).to_string(),
        };
        let Some(dir) = presets_dir else { return tr("no presets folder").into() };
        let mut games: Vec<String> = ipc::live_layers().unwrap_or_default().iter().map(|l| l.status.game_id().to_string()).filter(|g| !g.is_empty()).collect();
        games.sort();
        games.dedup();
        if games.is_empty() {
            return tr("no game with the layer open").into();
        }
        games
            .iter()
            .map(|game| match crate::presets::cycle_preset(dir, game, delta) {
                Ok(name) => fmt("{}: preset {}", &[&game, &name]),
                Err(why) => why,
            })
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// Estado depois de uma ação.
pub struct Applied {
    pub effects_enabled: bool,
}

impl fmt::Display for Applied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", fmt("effects: {}", &[&if self.effects_enabled { tr("enabled") } else { tr("disabled") }]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn writer(name: &str) -> ipc::Writer {
        let dir = std::env::temp_dir().join(format!("lr-control-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        ipc::Writer::create_at(&dir.join("state")).unwrap()
    }

    #[test]
    fn actions_change_shared_state() {
        let w = writer("actions");
        assert!(Action::EffectsOn.apply(&w).effects_enabled);
        assert!(!Action::ToggleEffects.apply(&w).effects_enabled);
        assert!(Action::ToggleEffects.apply(&w).effects_enabled);
        assert!(!Action::EffectsOff.apply(&w).effects_enabled);
    }
}
