//! Tabela de atalhos do app. Para acrescentar um atalho, acrescente uma linha aqui e, se for
//! uma ação nova, uma variante em `Action`.
//!
//! Nunca sugerir `CTRL+ALT+F1`..`F12`: o kernel troca de terminal virtual (VT), e com NVIDIA
//! proprietária no Wayland isso já travou a máquina de desenvolvimento (2026-10-09, hipótese).
//! Atenção ao digitar: `CTRL+SHIFT+F10` (o nosso) e `CTRL+ALT+F10` (tty10) ficam lado a lado.

use crate::action::Action;
use shortcuts::{Binding, Dispatcher, Error, Scope, Shortcut};

fn shortcut(id: &str, description: &str, trigger: &str) -> Shortcut {
    Shortcut { id: id.into(), description: description.into(), preferred_trigger: Some(trigger.into()) }
}

pub fn dispatcher() -> Result<Dispatcher<Action>, Error> {
    Dispatcher::new(vec![
        Binding::on_press(
            // O id é guardado pelo sistema: não renomear depois de publicado.
            shortcut("toggle-effects", "linux-reshade: ligar/desligar efeitos", "CTRL+SHIFT+F10"),
            Scope::Global,
            Action::ToggleEffects,
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_valid_and_exposes_the_global_shortcut() {
        let ids: Vec<_> = dispatcher().unwrap().global_shortcuts().into_iter().map(|s| s.id).collect();
        assert_eq!(ids, ["toggle-effects"]);
    }
}
