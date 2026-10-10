//! Modo residente: registra os atalhos globais, aplica a ação de cada evento e, em paralelo, mantém
//! compilados os efeitos que os jogos abertos precisam.

use control_core::compiler::Compiler;
use crate::{backend, bindings};
use control_core::{action::Action, watch};
use shortcuts::{Context, Error};

pub fn run(state: &ipc::Writer) -> Result<(), Error> {
    let dispatcher = bindings::dispatcher()?;
    let mut backend = backend::select()?;
    println!("backend de atalhos: {}", backend.name());

    for bound in backend.bind(&dispatcher.global_shortcuts())? {
        // A combinação em vigor pode ser diferente da sugerida: o usuário pode tê-la trocado.
        println!("atalho {:?}: {}", bound.id, bound.trigger_description);
    }

    std::thread::scope(|scope| {
        // O vigia precisa do fx2spv; sem ele o atalho continua funcionando e só a compilação automática fica de fora.
        match Compiler::from_env() {
            Ok(compiler) => {
                println!("vigiando os jogos com a layer: compila os efeitos escolhidos por nome para cada resolução");
                let presets = config::Config::load().presets_dir;
                scope.spawn(move || watch::run(&compiler, state, presets));
            }
            Err(e) => eprintln!("aviso: sem compilação automática de efeitos ({e})"),
        }

        // Ainda não há janela: só atalhos globais valem.
        let ctx = Context { window_visible: false };
        loop {
            let event = backend.next_event()?;
            if let Some(action) = dispatcher.resolve(&event, &ctx) {
                report(*action, state);
            }
        }
    })
}

fn report(action: Action, state: &ipc::Writer) {
    println!("{action:?} -> {}", action.apply(state));
}
