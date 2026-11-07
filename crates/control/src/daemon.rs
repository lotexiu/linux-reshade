//! Modo residente: registra os atalhos globais, aplica a ação de cada evento e, em paralelo, mantém
//! compilados os efeitos que os jogos abertos precisam.

use control_core::compiler::Compiler;
use crate::{backend, bindings};
use control_core::{action::Action, watch};
use shortcuts::{Context, Error};
use control_core::i18n::{fmt, tr};

/// Onde o daemon anota as teclas que registrou (id=tecla), para saber, na próxima vez, quais o sistema precisa esquecer.
fn registered_file(config: &config::Config) -> std::path::PathBuf {
    config.cache_dir.parent().unwrap_or(&config.cache_dir).join("shortcuts-bound.ini")
}

fn read_registered(file: &std::path::Path) -> Vec<(String, String)> {
    ini::Ini::load(file).map(|i| i.entries("bound").into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()).unwrap_or_default()
}

/// O sistema só usa a tecla sugerida na primeira vez que vê um atalho: o que mudou de tecla (ou perdeu a tecla) é esquecido
/// antes de registrar. Devolve quantos o sistema esqueceu.
fn forget_changed(previous: &[(String, String)], wanted: &[(String, String)]) -> usize {
    // Sem anotação de antes (versões que ainda não anotavam), vale a tecla de fábrica: a do ligar/desligar já estava registrada.
    let before = |id: &str| previous.iter().find(|(p, _)| p == id).map(|(_, t)| t.clone()).or_else(|| (id == "toggle-effects").then(|| config::default_key("KeyEffects").to_string()));
    let mut stale: Vec<String> = wanted.iter().filter(|(id, trigger)| before(id).is_some_and(|b| &b != trigger)).map(|(id, _)| id.clone()).collect();
    // O que não tem tecla agora (tivesse ou não antes): esquecê-lo é inofensivo se o sistema nem o conhecia. Os atalhos de técnica
    // que sumiram dos presets só aparecem na anotação de antes.
    let known = bindings::all_ids().into_iter().map(String::from).chain(previous.iter().map(|(id, _)| id.clone()));
    for id in known {
        if !wanted.iter().any(|(w, _)| *w == id) && !stale.contains(&id) {
            stale.push(id);
        }
    }
    let ids: Vec<&str> = stale.iter().map(String::as_str).collect();
    shortcuts_portal::forget(&ids)
}

/// O processo dos atalhos (`lrctl keys`) enquanto algum jogo com a layer está aberto.
static KEYS_CHILD: std::sync::Mutex<Option<std::process::Child>> = std::sync::Mutex::new(None);

fn stop_keys() {
    if let Some(mut child) = KEYS_CHILD.lock().unwrap_or_else(|e| e.into_inner()).take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// O daemon: vigia os jogos (compila os efeitos que eles precisam) e liga os atalhos globais SÓ enquanto há um jogo com a
/// layer aberto. Registrar os atalhos o tempo todo tomava F1, F2... de todos os outros programas: o sistema entrega a tecla
/// ao daemon, não à janela em foco. Os atalhos moram num processo à parte (`lrctl keys`): ao encerrá-lo, o sistema os solta.
/// `LINUX_RESHADE_KEYS_ALWAYS=1` volta a registrá-los sempre.
pub fn run(state: &ipc::Writer) -> Result<(), Error> {
    // Um daemon só: o painel (AppImage) o inicia quando não há, e dois registrariam os atalhos em dobro.
    control_core::install::ensure_desktop_entry();
    let Some(lock) = ipc::daemon_lock::acquire() else {
        control_core::logln!("{}", tr("the daemon is already running; this one exits"));
        return Ok(());
    };
    // Diz de que build é: o painel troca o daemon que sobrou de uma versão anterior (ver `install::ensure_daemon_current`).
    if let Some(stamp) = std::env::current_exe().ok().and_then(|exe| ipc::daemon_lock::exe_stamp(&exe)) {
        ipc::daemon_lock::publish(&lock, &stamp);
    }
    let _lock = lock;
    let config = config::Config::load();
    let techs = config.presets_dir.as_deref().map(control_core::tech_keys::scan).unwrap_or_default();
    // A troca de tecla no painel vale na hora: o daemon reinicia a si mesmo ao ver o `config.ini` mudar.
    spawn_config_watch(&config, &techs);
    let always = std::env::var_os("LINUX_RESHADE_KEYS_ALWAYS").is_some_and(|v| v != "0");

    std::thread::scope(|scope| {
        // O vigia precisa do fx2spv; sem ele o atalho continua funcionando e só a compilação automática fica de fora.
        match Compiler::from_env() {
            Ok(compiler) => {
                control_core::logln!("{}", tr("watching the games with the layer: compiling the effects chosen by name for each resolution"));
                let config = config::Config::load();
                scope.spawn(move || watch::run(&compiler, state, config.presets_dir, &config.global_macros, &config.screenshot_dir));
            }
            Err(e) => control_core::elogln!("{}", fmt("warning: no automatic effect compilation ({})", &[&e])),
        }

        let exe = std::env::current_exe().map_err(|e| Error::Backend(e.to_string()))?;
        let mut missing_since: Option<std::time::Instant> = None;
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            let present = always || !ipc::live_layers().unwrap_or_default().is_empty();
            let mut slot = KEYS_CHILD.lock().unwrap_or_else(|e| e.into_inner());
            if slot.as_mut().is_some_and(|c| c.try_wait().ok().flatten().is_some()) {
                control_core::logln!("o processo dos atalhos terminou");
                *slot = None;
            }
            match (present, slot.is_some()) {
                (true, false) => {
                    missing_since = None;
                    match std::process::Command::new(&exe).arg("keys").spawn() {
                        Ok(child) => {
                            control_core::logln!("{}", tr("game open: global shortcuts on"));
                            *slot = Some(child);
                        }
                        Err(e) => control_core::elogln!("{}", fmt("could not start the shortcuts: {}", &[&e])),
                    }
                }
                // Um jogo que fecha e abre em seguida (trocar de mapa) não deve soltar as teclas por um instante.
                (false, true) => {
                    let since = *missing_since.get_or_insert_with(std::time::Instant::now);
                    if since.elapsed() >= std::time::Duration::from_secs(3) {
                        drop(slot);
                        stop_keys();
                        missing_since = None;
                        control_core::logln!("{}", tr("no game open: global shortcuts released (F1, F2... go back to the other programs)"));
                    }
                }
                _ => missing_since = None,
            }
        }
    })
}

/// Os atalhos globais (processo `lrctl keys`): registra no sistema e aplica a ação de cada evento até ser encerrado.
pub fn run_keys(state: &ipc::Writer) -> Result<(), Error> {
    // Se o daemon morrer, este processo não pode ficar segurando as teclas.
    let parent = rustix::process::getppid();
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
        if rustix::process::getppid() != parent {
            std::process::exit(0);
        }
    });
    let config = config::Config::load();
    let techs = config.presets_dir.as_deref().map(control_core::tech_keys::scan).unwrap_or_default();
    let wanted = bindings::wanted(&config, &techs);
    let dispatcher = bindings::dispatcher(&wanted, &techs)?;
    let registered = registered_file(&config);
    let forgotten = forget_changed(&read_registered(&registered), &wanted);
    if forgotten > 0 {
        control_core::logln!("{}", fmt("{} shortcut(s) with a new key or no key forgotten by the system", &[&forgotten]));
    }
    control_core::install::ensure_desktop_entry();
    let mut backend = backend::select()?;
    control_core::logln!("{}", fmt("shortcuts backend: {}", &[&backend.name()]));

    let mut bound_now = backend.bind(&dispatcher.global_shortcuts())?;
    // O KDE só usa a tecla sugerida na primeira vez que vê o id: se o id já existia sem tecla (limpo, ou registrado por outro programa),
    // o atalho volta SEM tecla e a tecla do painel não funciona. Um atalho que devia ter tecla e voltou sem nenhuma é esquecido e
    // registrado de novo, uma vez.
    let keyless: Vec<String> = bound_now.iter().filter(|b| b.trigger_description.trim().is_empty() && wanted.iter().any(|(id, _)| *id == b.id)).map(|b| b.id.clone()).collect();
    if !keyless.is_empty() {
        let ids: Vec<&str> = keyless.iter().map(String::as_str).collect();
        let forgotten = shortcuts_portal::forget(&ids);
        control_core::logln!("{}", fmt("{} shortcut(s) came back without a key; forgotten {} and registering again", &[&keyless.len(), &forgotten]));
        if forgotten > 0 {
            bound_now = backend.bind(&dispatcher.global_shortcuts())?;
        }
    }
    for bound in bound_now {
        // A combinação em vigor pode ser diferente da sugerida: o usuário pode tê-la trocado.
        control_core::logln!("{}", fmt("shortcut {}: {}", &[&format!("{:?}", bound.id), &bound.trigger_description]));
    }
    // Anota o que foi registrado (sempre: um id que perde a tecla some da lista).
    let mut note = ini::Ini::new();
    for (id, trigger) in &wanted {
        note.set("bound", id, trigger);
    }
    let _ = std::fs::create_dir_all(registered.parent().unwrap_or(std::path::Path::new(".")));
    let _ = note.save(&registered);

    // Ainda não há janela: só atalhos globais valem.
    let ctx = Context { window_visible: false };
    let presets_dir = config.presets_dir.clone();
    let screenshot_before = config.screenshot_before;
    let reloader = Compiler::from_env().ok();
    loop {
        let event = backend.next_event()?;
        if let Some(action) = dispatcher.resolve(&event, &ctx) {
            report(*action, state, presets_dir.as_deref(), screenshot_before, &techs, reloader.as_ref());
        }
    }
}

fn report(action: Action, state: &ipc::Writer, presets_dir: Option<&std::path::Path>, screenshot_before: bool, techs: &[control_core::tech_keys::TechKey], compiler: Option<&Compiler>) {
    let result = match action {
        Action::Technique(i) => match (techs.get(i), presets_dir) {
            (Some(key), Some(dir)) => match control_core::tech_keys::toggle(dir, key) {
                Ok(on) => format!("{} {}", key.technique.name, if on { "ligada" } else { "desligada" }),
                Err(why) => why,
            },
            _ => tr("unknown technique").into(),
        },
        Action::ReloadEffects => reload_effects(compiler, presets_dir),
        other => other.run(state, presets_dir, screenshot_before),
    };
    control_core::logln!("{action:?} -> {result}");
}

/// Apaga os compilados dos efeitos dos presets dos jogos abertos (e do global): o vigia os compila de novo (pega o `.fx` editado).
fn reload_effects(compiler: Option<&Compiler>, presets_dir: Option<&std::path::Path>) -> String {
    let (Some(compiler), Some(dir)) = (compiler, presets_dir) else { return tr("no compiler or presets folder").into() };
    let mut names: Vec<String> = Vec::new();
    let mut presets: Vec<std::path::PathBuf> = vec![dir.join("preset.ini")];
    for layer in ipc::live_layers().unwrap_or_default() {
        if let Some(found) = preset::game::find(dir, layer.status.game_id()) {
            presets.push(found.path);
        }
    }
    for path in presets {
        if let Ok(preset) = preset::Preset::load(&path) {
            for id in preset.techniques() {
                if let Some(file) = id.file {
                    let name = file.trim_end_matches(".fx").to_string();
                    if !names.contains(&name) {
                        names.push(name);
                    }
                }
            }
        }
    }
    let removed: usize = names.iter().map(|n| compiler.invalidate(n)).sum();
    fmt("{} compiled bundle(s) of {} effect(s) deleted; the watcher redoes them", &[&removed, &names.len()])
}

/// Olha o `config.ini` a cada 2 s; se as teclas mudaram, o daemon se reexecuta (os atalhos são registrados na partida).
fn spawn_config_watch(config: &config::Config, techs: &[control_core::tech_keys::TechKey]) {
    let Some(path) = config.config_path() else { return };
    // Reinicia por qualquer ajuste que o daemon lê só na partida: teclas, pastas e capturas.
    let keys_now = |c: &config::Config| {
        let mut v: Vec<String> = config::KEY_NAMES.iter().map(|(k, _)| c.key(k)).collect();
        v.push(format!("{:?}|{:?}|{:?}|{:?}|{}", c.effect_paths, c.texture_paths, c.presets_dir, c.screenshot_dir, c.screenshot_before));
        v
    };
    // Os atalhos de técnica moram nos presets: se mudam, o daemon também se reinicia.
    let tech_signature = |dir: &Option<std::path::PathBuf>| dir.as_deref().map(|d| control_core::tech_keys::scan(d).into_iter().map(|k| (k.id, k.trigger)).collect::<Vec<_>>()).unwrap_or_default();
    let (start, start_techs, presets_dir) = (keys_now(config), tech_signature(&config.presets_dir), config.presets_dir.clone());
    let _ = techs;
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(2));
        let config_changed = std::fs::metadata(&path).is_ok() && keys_now(&config::Config::load()) != start;
        if config_changed || tech_signature(&presets_dir) != start_techs {
            control_core::logln!("{}", tr("shortcut keys changed (config.ini or presets): restarting the daemon"));
            let Ok(exe) = std::env::current_exe() else { return };
            stop_keys(); // o exec mantém o pid: o processo dos atalhos antigo não perceberia que o daemon mudou
            use std::os::unix::process::CommandExt;
            let error = std::process::Command::new(exe).args(std::env::args().skip(1)).exec();
            control_core::elogln!("{}", fmt("could not restart the daemon: {}", &[&error]));
            return;
        }
    });
}
