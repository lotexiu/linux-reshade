//! Modo residente: registra os atalhos globais, aplica a ação de cada evento e, em paralelo, mantém
//! compilados os efeitos que os jogos abertos precisam.

use control_core::compiler::Compiler;
use crate::{backend, bindings};
use control_core::{action::Action, watch};
use shortcuts::{Context, Error};

/// Onde o daemon anota as teclas que registrou (id=tecla), para saber, na próxima vez, quais o sistema precisa esquecer.
fn registered_file(config: &config::Config) -> std::path::PathBuf {
    config.cache_dir.parent().unwrap_or(&config.cache_dir).join("shortcuts-bound.ini")
}

fn read_registered(file: &std::path::Path) -> Vec<(String, String)> {
    ini::Ini::load(file).map(|i| i.entries("bound").into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()).unwrap_or_default()
}

/// O sistema só usa a tecla sugerida na primeira vez que vê um atalho: o que mudou de tecla (ou perdeu a tecla) é esquecido
/// antes de registrar. Devolve quantos o sistema esqueceu.
fn forget_changed(previous: &[(String, String)], wanted: &[(&'static str, String)]) -> usize {
    // Sem anotação de antes (versões que ainda não anotavam), vale a tecla de fábrica: a do ligar/desligar já estava registrada.
    let before = |id: &str| previous.iter().find(|(p, _)| p == id).map(|(_, t)| t.clone()).or_else(|| (id == "toggle-effects").then(|| config::default_key("KeyEffects").to_string()));
    let mut stale: Vec<String> = wanted.iter().filter(|(id, trigger)| before(id).is_some_and(|b| &b != trigger)).map(|(id, _)| (*id).to_string()).collect();
    // O que não tem tecla agora (tivesse ou não antes): esquecê-lo é inofensivo se o sistema nem o conhecia.
    for id in bindings::all_ids() {
        if !wanted.iter().any(|(w, _)| *w == id) && !stale.iter().any(|s| s == id) {
            stale.push(id.to_string());
        }
    }
    let ids: Vec<&str> = stale.iter().map(String::as_str).collect();
    shortcuts_portal::forget(&ids)
}

pub fn run(state: &ipc::Writer) -> Result<(), Error> {
    let config = config::Config::load();
    let wanted = bindings::wanted(&config);
    let dispatcher = bindings::dispatcher(&wanted)?;
    let registered = registered_file(&config);
    let forgotten = forget_changed(&read_registered(&registered), &wanted);
    if forgotten > 0 {
        control_core::logln!("{forgotten} atalho(s) com tecla nova ou sem tecla esquecido(s) pelo sistema");
    }
    let mut backend = backend::select()?;
    control_core::logln!("backend de atalhos: {}", backend.name());

    for bound in backend.bind(&dispatcher.global_shortcuts())? {
        // A combinação em vigor pode ser diferente da sugerida: o usuário pode tê-la trocado.
        control_core::logln!("atalho {:?}: {}", bound.id, bound.trigger_description);
    }
    // Anota o que foi registrado (sempre: um id que perde a tecla some da lista).
    let mut note = ini::Ini::new();
    for (id, trigger) in &wanted {
        note.set("bound", id, trigger);
    }
    let _ = std::fs::create_dir_all(registered.parent().unwrap_or(std::path::Path::new(".")));
    let _ = note.save(&registered);
    // A troca de tecla no painel vale na hora: o daemon reinicia a si mesmo ao ver o `config.ini` mudar.
    spawn_config_watch(&config);

    std::thread::scope(|scope| {
        // O vigia precisa do fx2spv; sem ele o atalho continua funcionando e só a compilação automática fica de fora.
        match Compiler::from_env() {
            Ok(compiler) => {
                control_core::logln!("vigiando os jogos com a layer: compila os efeitos escolhidos por nome para cada resolução");
                let config = config::Config::load();
                scope.spawn(move || watch::run(&compiler, state, config.presets_dir, &config.global_macros));
            }
            Err(e) => control_core::elogln!("aviso: sem compilação automática de efeitos ({e})"),
        }

        // Ainda não há janela: só atalhos globais valem.
        let ctx = Context { window_visible: false };
        let presets_dir = config.presets_dir.clone();
        loop {
            let event = backend.next_event()?;
            if let Some(action) = dispatcher.resolve(&event, &ctx) {
                report(*action, state, presets_dir.as_deref());
            }
        }
    })
}

fn report(action: Action, state: &ipc::Writer, presets_dir: Option<&std::path::Path>) {
    control_core::logln!("{action:?} -> {}", action.run(state, presets_dir));
}

/// Olha o `config.ini` a cada 2 s; se as teclas mudaram, o daemon se reexecuta (os atalhos são registrados na partida).
fn spawn_config_watch(config: &config::Config) {
    let Some(path) = config.config_path() else { return };
    let keys_now = |c: &config::Config| config::KEY_NAMES.iter().map(|(k, _)| c.key(k)).collect::<Vec<_>>();
    let start = keys_now(config);
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(2));
        if std::fs::metadata(&path).is_err() {
            continue;
        }
        if keys_now(&config::Config::load()) != start {
            control_core::logln!("teclas de atalho mudaram no config.ini: reiniciando o daemon");
            let Ok(exe) = std::env::current_exe() else { return };
            use std::os::unix::process::CommandExt;
            let error = std::process::Command::new(exe).args(std::env::args().skip(1)).exec();
            control_core::elogln!("não consegui reiniciar o daemon: {error}");
            return;
        }
    });
}
