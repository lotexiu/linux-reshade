//! Escolher o efeito que a layer executa.

use crate::compiler::{CompileError, Compiler, Ensured};
use ipc::{LiveLayer, SelectionStore};
use i18n::fmt;

/// O que escolher um efeito por nome fez.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Cada resolução de jogo aberto para a qual o efeito foi preparado: `(largura, altura, recém-compilado)`.
    pub prepared: Vec<(u32, u32, bool)>,
}

/// Escolhe o efeito `name`: compila já para as resoluções das `layers` dadas (assim aparece sem esperar) e
/// publica no `store` o modelo `<cache>/<Nome>-{W}x{H}`. As resoluções que surgirem depois ficam para o
/// vigia. `store` é o canal global (todas as layers vivas) ou o de um jogo (só a layer dele).
pub fn by_name(compiler: &Compiler, store: &dyn SelectionStore, name: &str, layers: &[LiveLayer]) -> Result<Outcome, CompileError> {
    compiler.source(name)?; // valida o nome e confere que o efeito existe antes de mexer em qualquer coisa
    let mut resolutions: Vec<(u32, u32)> = layers
        .iter()
        .flat_map(|l| &l.status.swapchains)
        .filter(|s| !matches!(s.effect, ipc::EffectState::Unavailable(_)))
        .map(|s| (s.width, s.height))
        .collect();
    resolutions.sort_unstable();
    resolutions.dedup();
    prepare(compiler, store, name, &resolutions)
}

/// O mesmo, para resoluções dadas (separado para poder ser testado sem jogos abertos).
pub fn prepare(compiler: &Compiler, store: &dyn SelectionStore, name: &str, resolutions: &[(u32, u32)]) -> Result<Outcome, CompileError> {
    compiler.source(name)?;
    let mut outcome = Outcome::default();
    for &(w, h) in resolutions {
        let fresh = matches!(compiler.ensure(name, w, h)?, Ensured::Compiled(_));
        outcome.prepared.push((w, h, fresh));
    }
    store.publish_selection(&compiler.template(name)).map_err(|e| CompileError::Io(fmt("could not publish the choice: {}", &[&e])))?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::testing::{OK_TOOL, calls, fake};
    use ipc::SelectionRead;

    fn state(root: &std::path::Path) -> ipc::Writer {
        ipc::Writer::create_at(&root.join("state-dir/state")).unwrap()
    }

    #[test]
    fn prepares_each_resolution_and_publishes_the_template() {
        let f = fake("select", OK_TOOL);
        let state = state(&f.root);
        let outcome = prepare(&f.compiler, &state, "Sepia", &[(1920, 1080), (1280, 720)]).unwrap();
        assert_eq!(outcome.prepared, vec![(1920, 1080, true), (1280, 720, true)]);
        assert_eq!(state.selection(), SelectionRead::Path(f.compiler.template("Sepia")));

        // Escolher de novo: já está compilado e só republica.
        let again = prepare(&f.compiler, &state, "Sepia", &[(1920, 1080)]).unwrap();
        assert_eq!(again.prepared, vec![(1920, 1080, false)]);
        assert_eq!(calls(&f).len(), 2, "não recompilou");
    }

    #[test]
    fn an_unknown_effect_changes_nothing() {
        let f = fake("select-unknown", OK_TOOL);
        let state = state(&f.root);
        state.publish_selection("/antiga").unwrap();
        assert!(prepare(&f.compiler, &state, "Nada", &[(10, 10)]).is_err());
        assert!(prepare(&f.compiler, &state, "../x", &[]).is_err());
        assert_eq!(state.selection(), SelectionRead::Path("/antiga".into()), "a escolha anterior continua");
    }

    #[test]
    fn a_compile_failure_does_not_publish_a_choice_that_cannot_work() {
        let f = fake("select-fail", "#!/bin/sh
echo erro >&2
exit 1
");
        let state = state(&f.root);
        assert!(prepare(&f.compiler, &state, "Sepia", &[(10, 10)]).is_err());
        assert_eq!(state.selection(), SelectionRead::Cleared);
    }

    #[test]
    fn with_no_open_game_it_still_publishes_the_choice() {
        let f = fake("select-idle", OK_TOOL);
        let state = state(&f.root);
        assert_eq!(prepare(&f.compiler, &state, "Sepia", &[]).unwrap(), Outcome::default());
        assert_eq!(state.selection(), SelectionRead::Path(f.compiler.template("Sepia")));
    }
}
