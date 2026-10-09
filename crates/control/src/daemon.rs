//! Modo residente: registra os atalhos globais e aplica a ação de cada evento.

use crate::{action::Action, backend, bindings};
use shortcuts::{Context, Error};

pub fn run(state: &ipc::Writer) -> Result<(), Error> {
    let dispatcher = bindings::dispatcher()?;
    let mut backend = backend::select()?;
    println!("backend de atalhos: {}", backend.name());

    for bound in backend.bind(&dispatcher.global_shortcuts())? {
        // A combinação em vigor pode ser diferente da sugerida: o usuário pode tê-la trocado.
        println!("atalho {:?}: {}", bound.id, bound.trigger_description);
    }

    // Ainda não há janela: só atalhos globais valem.
    let ctx = Context { window_visible: false };
    loop {
        let event = backend.next_event()?;
        if let Some(action) = dispatcher.resolve(&event, &ctx) {
            report(*action, state);
        }
    }
}

fn report(action: Action, state: &ipc::Writer) {
    println!("{action:?} -> {}", action.apply(state));
}
