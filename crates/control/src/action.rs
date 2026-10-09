//! O que o app de controle sabe fazer. Atalhos e CLI disparam as mesmas ações, para o
//! comportamento ficar num lugar só.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    ToggleEffects,
    EffectsOn,
    EffectsOff,
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
        }
        Applied { effects_enabled: state.effects_enabled() }
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
