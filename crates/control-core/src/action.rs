//! O que o app de controle sabe fazer. Atalhos e CLI disparam as mesmas ações, para o
//! comportamento ficar num lugar só.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    ToggleEffects,
    EffectsOn,
    EffectsOff,
    /// Passa os jogos abertos para o preset seguinte/anterior (só no daemon, que sabe quais jogos estão abertos).
    NextPreset,
    PreviousPreset,
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
            Action::NextPreset | Action::PreviousPreset => {}
        }
        Applied { effects_enabled: state.effects_enabled() }
    }
}

impl Action {
    /// O que a ação faz no daemon: as de efeitos mexem no estado compartilhado; as de preset trocam o preset de cada jogo com a
    /// layer aberta. Devolve o texto para o log.
    pub fn run(self, state: &ipc::Writer, presets_dir: Option<&std::path::Path>) -> String {
        let delta = match self {
            Action::NextPreset => 1,
            Action::PreviousPreset => -1,
            other => return other.apply(state).to_string(),
        };
        let Some(dir) = presets_dir else { return "sem pasta de presets".into() };
        let mut games: Vec<String> = ipc::live_layers().unwrap_or_default().iter().map(|l| l.status.game_id().to_string()).filter(|g| !g.is_empty()).collect();
        games.sort();
        games.dedup();
        if games.is_empty() {
            return "nenhum jogo com a layer aberto".into();
        }
        games
            .iter()
            .map(|game| match crate::presets::cycle_preset(dir, game, delta) {
                Ok(name) => format!("{game}: preset {name}"),
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
        write!(f, "efeitos: {}", if self.effects_enabled { "ligados" } else { "desligados" })
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
