//! Execução de cada comando do `lrctl`. Cada função devolve o código de saída e escreve na saída padrão
//! ou de erro; a lógica que merece teste vive em `params` e `cli`.

use control_core::action::{self, Action};
use crate::cli::{self, Command, DepthPick};
use control_core::compiler::Compiler;
use crate::daemon;
use control_core::{layers, location, params, select, watch};
use fx_bundle::Bundle;
use ipc::{ChannelSelection, LiveLayer, ParamStore, SelectionRead, SelectionStore};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use control_core::i18n::{fmt, tr};

fn fail(message: impl std::fmt::Display) -> ExitCode {
    eprintln!("lrctl: {message}");
    ExitCode::FAILURE
}

fn load(dir: &Path) -> Result<Bundle, String> {
    Bundle::load(dir).map_err(|e| format!("{}: {e}", dir.display()))
}

/// A quem um comando se dirige: a todos os jogos (canal global) ou a um só (canal próprio dele).
enum Target<'a> {
    All(&'a ipc::Writer),
    Game { layer: LiveLayer, channel: ipc::ChannelWriter, global: &'a ipc::Writer },
}

impl<'a> Target<'a> {
    /// `--layer <jogo>` (ou nenhum: todos). Falha se o jogo não está aberto.
    fn open(layer: Option<&str>, state: &'a ipc::Writer) -> Result<Self, String> {
        let Some(query) = layer else { return Ok(Target::All(state)) };
        let layers = ipc::live_layers().map_err(|e| fmt("could not list the layers: {}", &[&e]))?;
        let found = layers::find(&layers, query)?.clone();
        let channel = state.channel(&found.id).map_err(|e| fmt("could not open the channel of {}: {}", &[&query, &e]))?;
        Ok(Target::Game { layer: found, channel, global: state })
    }

    fn selection_store(&self) -> &dyn SelectionStore {
        match self {
            Target::All(state) => *state,
            Target::Game { channel, .. } => channel,
        }
    }

    /// Os parâmetros do alvo: os globais, ou os do jogo sobre os globais.
    fn with_params<R>(&self, f: impl FnOnce(&dyn ParamStore) -> R) -> R {
        match self {
            Target::All(state) => f(*state),
            Target::Game { channel, global, .. } => f(&params::Layered { own: channel, global }),
        }
    }

    /// As layers a que uma escolha se aplica: todas as vivas, ou só a do jogo.
    fn layers(&self) -> Vec<LiveLayer> {
        match self {
            Target::All(_) => ipc::live_layers().unwrap_or_default(),
            Target::Game { layer, .. } => vec![layer.clone()],
        }
    }

    /// "todos os jogos" ou "vkcube (pid 123)".
    fn label(&self) -> String {
        match self {
            Target::All(_) => tr("all games").into(),
            Target::Game { layer, .. } => {
                let app = if layer.status.app.is_empty() { layer.id.as_str() } else { layer.status.app.as_str() };
                format!("{app} (pid {})", layers::pid_of(&layer.id).unwrap_or("?"))
            }
        }
    }

    /// A escolha de efeito que o alvo tem hoje, como os comandos de parâmetros a entendem.
    fn selection(&self) -> Result<SelectionRead, String> {
        match self {
            Target::All(state) => Ok(state.selection()),
            Target::Game { layer, global, .. } => match global.layer_selection(&layer.id) {
                ChannelSelection::Path(path) => Ok(SelectionRead::Path(path)),
                ChannelSelection::NoEffect => Err(fmt("{} has no effect; choose one with 'lrctl use <Name> --layer ...'", &[&self.label()])),
                ChannelSelection::Inherit | ChannelSelection::Unstable => Ok(global.selection()),
            },
        }
    }
}

/// O bundle dos comandos de parâmetros: `--bundle`, senão o escolhido com `use`, senão o do ambiente.
fn bundle_for(explicit: Option<PathBuf>, target: &Target) -> Result<Bundle, String> {
    let env = std::env::var_os("LINUX_RESHADE_BUNDLE").map(PathBuf::from);
    let dir = cli::resolve_bundle(explicit, target.selection()?, env)?;
    // Uma escolha por nome é um modelo com {W}x{H}: para ler os controles basta o bundle de uma resolução.
    if dir.to_string_lossy().contains("{W}") {
        let compiler = Compiler::from_env()?;
        let schema = compiler.schema_bundle(&dir.to_string_lossy(), &ipc::live_layers().unwrap_or_default()).map_err(|e| e.to_string())?;
        return load(&schema);
    }
    load(&dir)
}

/// `lrctl config` e `lrctl config init [pasta]`.
fn config_command(init: bool, dir: Option<PathBuf>) -> ExitCode {
    let env = |name: &str| std::env::var_os(name);
    if init {
        let Some(dir) = dir.or_else(|| config::default_data_dir(&env)) else {
            return fail(tr("do not know where your data lives (HOME and XDG_DATA_HOME are empty): pass the folder"));
        };
        let dir = match std::path::absolute(&dir) {
            Ok(d) => d,
            Err(e) => return fail(format!("{}: {e}", dir.display())),
        };
        match config::init(&dir, &env) {
            Ok(done) => {
                for path in &done.created {
                    println!("{}", fmt("created: {}", &[&path.display()]));
                }
                match &done.location {
                    Some(file) => println!("{}", fmt("chosen as the data folder (pointer in {})", &[&file.display()])),
                    None => println!("{}", fmt("folder ready, but could not find where to write the pointer (HOME/XDG_CONFIG_HOME): use LINUX_RESHADE_DATA={}", &[&dir.display()])),
                }
                println!("{}", fmt("put the effects in {}/Shaders and the images in {}/Textures", &[&dir.display(), &dir.display()]));
            }
            Err(e) => return fail(format!("{}: {e}", dir.display())),
        }
        return ExitCode::SUCCESS;
    }
    print!("{}", location::describe(&config::Config::load(), config::location_file(&env).as_deref()));
    ExitCode::SUCCESS
}

fn describe_bundle(bundle: &Bundle) -> String {
    let t = bundle.manifest.target;
    let names: Vec<&str> = bundle.manifest.techniques.iter().map(|t| t.name.as_str()).collect();
    fmt("{} ({}x{}, technique(s): {})", &[&bundle.manifest.source, &t.width, &t.height, &names.join(", ")])
}

pub fn run(command: Command, state: &ipc::Writer) -> ExitCode {
    match command {
        Command::On => report(Action::EffectsOn, state),
        Command::Off => report(Action::EffectsOff, state),
        Command::Toggle => report(Action::ToggleEffects, state),
        Command::Status => {
            println!("{}", action::Applied { effects_enabled: state.effects_enabled() });
            ExitCode::SUCCESS
        }
        Command::Screenshot => {
            let config = config::Config::load();
            println!("{}", action::Action::Screenshot.run(state, None, config.screenshot_before));
            println!("{}", fmt("screenshots go to {}", &[&config.screenshot_dir.display()]));
            ExitCode::SUCCESS
        }
        Command::Setup(true) => match control_core::install::install_layer() {
            Ok(lib) => {
                // A pasta de dados (com os efeitos que vêm junto) também, se ainda não há.
                if let Err(e) = control_core::install::first_run() {
                    eprintln!("lrctl: aviso: {e}");
                }
                println!("{}", fmt("layer installed: {}\nmanifest: {}", &[&lib.display(), &control_core::install::manifest_path().display()]));
                ExitCode::SUCCESS
            }
            Err(e) => fail(e),
        },
        Command::Setup(false) => {
            control_core::install::uninstall_layer();
            println!("{}", tr("layer removed"));
            ExitCode::SUCCESS
        }
        Command::Flatpak(action) => flatpak(action),
        Command::Stats(on) => match ipc::set_stats_on(on) {
            Ok(()) => {
                println!("{}", if on { tr("performance measurement on: see it with 'lrctl layers' (or in the panel's Performance tab)") } else { tr("performance measurement off") });
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("lrctl: {e}");
                ExitCode::FAILURE
            }
        },
        Command::Daemon => match daemon::run(state) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(e),
        },
        Command::DaemonEnsure => match control_core::install::ensure_daemon_current() {
            control_core::install::DaemonAction::Kept => {
                println!("{}", tr("the daemon is already from this build"));
                ExitCode::SUCCESS
            }
            control_core::install::DaemonAction::Started => {
                println!("{}", tr("daemon started"));
                ExitCode::SUCCESS
            }
            control_core::install::DaemonAction::Replaced => {
                println!("{}", tr("daemon from an older build replaced by this one"));
                ExitCode::SUCCESS
            }
            control_core::install::DaemonAction::Failed => fail(tr("could not start the daemon")),
        },
        Command::DaemonStop => {
            if !ipc::daemon_lock::is_running() {
                println!("{}", tr("no daemon running"));
                ExitCode::SUCCESS
            } else if control_core::install::stop_daemon() {
                println!("{}", tr("daemon stopped"));
                ExitCode::SUCCESS
            } else {
                fail(tr("the daemon did not stop"))
            }
        }
        Command::Keys => match daemon::run_keys(state) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(e),
        },
        Command::Use { bundle: None, layer } => with_target(layer, state, use_none),
        Command::Inherit { layer } => with_target(Some(layer), state, inherit),
        Command::Use { bundle: Some(arg), layer } if !cli::is_path(&arg.to_string_lossy()) => with_target(layer, state, |t| select_by_name(&arg.to_string_lossy(), t)),
        Command::Use { bundle: Some(dir), layer } => with_target(layer, state, |t| select(&dir, t)),
        Command::Current { layer } => with_target(layer, state, current),
        Command::Layers => {
            match ipc::live_layers() {
                Ok(list) => print!("{}", layers::render(&list)),
                Err(e) => return fail(fmt("could not list the layers: {}", &[&e])),
            }
            ExitCode::SUCCESS
        }
        Command::Config { init, dir } => config_command(init, dir),
        Command::DepthPick { pick, layer } => depth_pick(pick, layer),
        Command::DepthPolicy { changes, layer } => depth_policy(changes, layer),
        Command::Preset { action, game, file } => crate::preset_cmd::run(action, game, file),
        Command::Effects => match Compiler::from_env() {
            Ok(compiler) => {
                let available = compiler.available();
                if available.is_empty() {
                    println!("{}", fmt("no effect found in: {}", &[&compiler.effect_dirs().iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join(", ")]));
                }
                for (name, dir) in available {
                    println!("{name:<24} {}", dir.display());
                }
                ExitCode::SUCCESS
            }
            Err(e) => fail(e),
        },
        Command::Watch => match Compiler::from_env() {
            Ok(compiler) => {
                println!("{}", tr("watching the games with the layer (Ctrl+C to quit)"));
                {
                    let config = config::Config::load();
                    watch::run(&compiler, state, config.presets_dir, &config.global_macros, &config.screenshot_dir)
                }
            }
            Err(e) => fail(e),
        },
        Command::Uniforms { bundle, layer } => with_bundle(bundle, layer, state, |bundle, target| {
            for row in target.with_params(|store| params::describe(bundle, store)) {
                match &row.source {
                    Some(source) => println!("{:<22} {:<8} {}", row.name, row.type_name, fmt("(filled in by the runtime: {})", &[&source])),
                    None => {
                        let range = row.range.map(|(a, b)| format!("  [{} .. {}]", params::fmt_number(a), params::fmt_number(b))).unwrap_or_default();
                        let label = row.label.as_deref().map(|l| format!("  \"{l}\"")).unwrap_or_default();
                        let default = if row.value == row.default { String::new() } else { fmt("  (default {})", &[&params::fmt_values(&row.default)]) };
                        println!("{:<22} {:<8} = {}{default}{range}{label}", row.name, row.type_name, params::fmt_values(&row.value));
                    }
                }
            }
            ExitCode::SUCCESS
        }),
        Command::Get { bundle, layer, name } => with_bundle(bundle, layer, state, |bundle, target| match target.with_params(|store| params::get(bundle, store, &name)) {
            Ok(v) => {
                println!("{}", params::fmt_values(&v));
                ExitCode::SUCCESS
            }
            Err(e) => fail(e),
        }),
        Command::Set { bundle, layer, name, values } => with_bundle(bundle, layer, state, |bundle, target| {
            let parsed = match params::parse_values(&values) {
                Ok(v) => v,
                Err(e) => return fail(e),
            };
            match target.with_params(|store| params::set(bundle, store, &name, &parsed)) {
                Ok(change) => {
                    println!("{name}: {} -> {}  ({})", params::fmt_values(&change.old), params::fmt_values(&change.new), target.label());
                    for w in &change.warnings {
                        eprintln!("aviso: {w}");
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => fail(e),
            }
        }),
        Command::Reset { bundle, layer } => with_bundle(bundle, layer, state, |bundle, target| match target.with_params(|store| params::reset(bundle, store)) {
            Ok(()) => {
                println!("{}", fmt("parameters back to the default values ({})", &[&target.label()]));
                ExitCode::SUCCESS
            }
            Err(e) => fail(e),
        }),
    }
}

fn report(action: Action, state: &ipc::Writer) -> ExitCode {
    println!("{}", action.apply(state));
    ExitCode::SUCCESS
}

fn with_target(layer: Option<String>, state: &ipc::Writer, f: impl FnOnce(&Target) -> ExitCode) -> ExitCode {
    match Target::open(layer.as_deref(), state) {
        Ok(target) => f(&target),
        Err(e) => fail(e),
    }
}

fn with_bundle(explicit: Option<PathBuf>, layer: Option<String>, state: &ipc::Writer, f: impl FnOnce(&Bundle, &Target) -> ExitCode) -> ExitCode {
    with_target(layer, state, |target| match bundle_for(explicit, target) {
        Ok(bundle) => f(&bundle, target),
        Err(e) => fail(e),
    })
}

/// `lrctl use none`: sem escolha global; com `--layer`, sem efeito só naquele jogo.
fn use_none(target: &Target) -> ExitCode {
    match target {
        Target::All(state) => {
            state.clear_selection();
            println!("{}", tr("no effect chosen (the layer goes back to LINUX_RESHADE_BUNDLE, if the game has it)"));
        }
        Target::Game { channel, .. } => {
            channel.publish_no_effect();
            println!("{}", fmt("{}: no effect (even if one is chosen for everyone)", &[&target.label()]));
        }
    }
    ExitCode::SUCCESS
}

/// `lrctl use inherit --layer X`: o jogo volta a seguir a escolha de todos.
fn inherit(target: &Target) -> ExitCode {
    if let Target::Game { channel, .. } = target {
        channel.inherit();
        println!("{}", fmt("{}: follows the choice for all games again", &[&target.label()]));
    }
    ExitCode::SUCCESS
}

/// `lrctl use <dir>`: confere que o bundle é válido antes de pedir à layer que o carregue.
fn select(dir: &Path, target: &Target) -> ExitCode {
    let abs = match std::fs::canonicalize(dir) {
        Ok(p) => p,
        Err(e) => return fail(format!("{}: {e}", dir.display())),
    };
    let bundle = match load(&abs) {
        Ok(b) => b,
        Err(e) => return fail(e),
    };
    let Some(text) = abs.to_str() else { return fail(tr("the bundle path is not UTF-8")) };
    if let Err(e) = target.selection_store().publish_selection(text) {
        return fail(fmt("could not publish the choice: {}", &[&e]));
    }
    println!("{}", fmt("effect chosen for {}: {}", &[&target.label(), &describe_bundle(&bundle)]));
    println!("{}", tr("the layer loads it on the next frame with the effects on (the size must be the game's)"));
    ExitCode::SUCCESS
}

/// `lrctl use Sepia`: escolhe um efeito pelo nome. Compila já para as resoluções dos jogos abertos
/// (assim aparece sem esperar) e publica o modelo `<cache>/Sepia-{W}x{H}`; o vigia cuida das resoluções
/// que surgirem depois.
fn select_by_name(name: &str, target: &Target) -> ExitCode {
    let compiler = match Compiler::from_env() {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let outcome = match select::by_name(&compiler, target.selection_store(), name, &target.layers()) {
        Ok(o) => o,
        Err(e) => return fail(e),
    };
    for (w, h, fresh) in &outcome.prepared {
        println!("{name} {w}x{h}: {}", if *fresh { tr("compiled") } else { tr("was already compiled") });
    }
    println!("{}", fmt("effect chosen for {}: {}", &[&target.label(), &name]));
    if outcome.prepared.is_empty() {
        println!("{}", tr("no game with the layer open right now; it is compiled when one shows up ('lrctl daemon' or 'lrctl watch' must be running)"));
    } else {
        println!("{}", tr("the watcher (daemon or watch) compiles the new resolutions that show up"));
    }
    ExitCode::SUCCESS
}

fn current(target: &Target) -> ExitCode {
    if let Target::Game { layer, global, .. } = target {
        match global.layer_selection(&layer.id) {
            ChannelSelection::Path(_) => println!("{}", fmt("{}: its own choice", &[&target.label()])),
            ChannelSelection::NoEffect => {
                println!("{}", fmt("{}: no effect (its own choice)", &[&target.label()]));
                return ExitCode::SUCCESS;
            }
            ChannelSelection::Inherit | ChannelSelection::Unstable => println!("{}", fmt("{}: no choice of its own; follows the one for all games", &[&target.label()])),
        }
    }
    match target.selection() {
        Ok(selection) => describe_selection(selection),
        Err(e) => return fail(e),
    }
    ExitCode::SUCCESS
}

fn describe_selection(selection: SelectionRead) {
    match selection {
        SelectionRead::Path(path) if path.contains("{W}") => match Compiler::from_env().ok().and_then(|c| c.name_from_template(&path)) {
            Some(name) => println!("{}", fmt("{} (by name; one bundle per resolution in {})", &[&name, &path])),
            None => println!("{}", fmt("{} (template per resolution)", &[&path])),
        },
        SelectionRead::Path(path) => match load(Path::new(&path)) {
            Ok(bundle) => println!("{path}\n  {}", describe_bundle(&bundle)),
            Err(e) => println!("{}", fmt("{}\n  (does not load right now: {})", &[&path, &e])),
        },
        SelectionRead::Cleared => {
            println!("{}", tr("no effect chosen"));
            if let Some(env) = std::env::var_os("LINUX_RESHADE_BUNDLE") {
                println!("(LINUX_RESHADE_BUNDLE={})", Path::new(&env).display());
            }
        }
        SelectionRead::Unstable => println!("{}", tr("the choice is being updated; try again")),
    }
}

/// `lrctl depth pick ...`: escreve o pedido que a layer do jogo lê (ver `depth.md`).
fn depth_pick(pick: DepthPick, layer: Option<String>) -> ExitCode {
    let layers = match ipc::live_layers() {
        Ok(l) => l,
        Err(e) => return fail(fmt("could not list the layers: {}", &[&e])),
    };
    let targets: Vec<LiveLayer> = match layer {
        Some(query) => match layers::find(&layers, &query) {
            Ok(l) => vec![l.clone()],
            Err(e) => return fail(e),
        },
        None => layers,
    };
    if targets.is_empty() {
        return fail(tr("no game with the layer active"));
    }
    for l in targets {
        let wanted = match pick {
            DepthPick::Auto => ipc::DepthPick::Auto,
            DepthPick::Candidate(n) => ipc::DepthPick::Candidate(n),
            DepthPick::Pass(n) => ipc::DepthPick::Pass(n),
            DepthPick::Clear(n) => ipc::DepthPick::Clear(n),
        };
        match ipc::set_depth_pick(&l.id, wanted) {
            Ok(()) => println!("{}: {}", l.status.app, match pick {
                DepthPick::Auto => tr("automatic depth").to_string(),
                DepthPick::Candidate(n) => fmt("candidate {}", &[&n]),
                DepthPick::Pass(n) => fmt("depth after pass {}", &[&n]),
                DepthPick::Clear(n) => fmt("depth before clear {}", &[&n]),
            }),
            Err(e) => return fail(format!("{}: {e}", l.id)),
        }
    }
    ExitCode::SUCCESS
}

/// `lrctl depth policy ...`: muda (ou mostra) os ajustes da profundidade de cada jogo aberto; a layer os lê em instantes.
fn depth_policy(changes: Vec<(String, Option<String>)>, layer: Option<String>) -> ExitCode {
    let layers = match ipc::live_layers() {
        Ok(l) => l,
        Err(e) => return fail(fmt("could not list the layers: {}", &[&e])),
    };
    let targets: Vec<LiveLayer> = match layer {
        Some(query) => match layers::find(&layers, &query) {
            Ok(l) => vec![l.clone()],
            Err(e) => return fail(e),
        },
        None => layers,
    };
    if targets.is_empty() {
        return fail(tr("no game with the layer active"));
    }
    for l in targets {
        for (key, value) in &changes {
            if let Err(e) = ipc::set_depth_policy(&l.id, key, value.as_deref()) {
                return fail(format!("{}: {e}", l.id));
            }
        }
        let now = ipc::depth_policy(&l.id);
        let shown: Vec<String> = ipc::DEPTH_POLICY_KEYS.iter().map(|k| format!("{k}={}", now.iter().find(|(n, _)| n == k).map_or("default", |(_, v)| v.as_str()))).collect();
        println!("{}: {}", l.status.app, shown.join(" "));
    }
    ExitCode::SUCCESS
}

/// `lrctl flatpak ...`: o mesmo que a página "Jogos Flatpak" do painel faz.
fn flatpak(action: cli::FlatpakAction) -> ExitCode {
    use control_core::flatpak::{self, State};
    if !flatpak::available() {
        return fail(tr("the 'flatpak' command is not installed"));
    }
    let result = match action {
        cli::FlatpakAction::List => flatpak::installed_apps().map(|apps| {
            println!("{}", if flatpak::global_enabled() { tr("global setting: on (all apps)") } else { tr("global setting: off") });
            for app in apps {
                let state = match flatpak::state(&app.id) {
                    State::Off => tr("off"),
                    State::On => tr("on"),
                    State::Global => tr("on (global)"),
                    State::DisabledHere => tr("off here only (LINUX_RESHADE=0)"),
                };
                println!("{:<44} {:<28} {state}", app.id, app.name);
            }
        }),
        cli::FlatpakAction::Set { app, on: true } => match flatpak::installed_apps().and_then(|apps| apps.into_iter().find(|a| a.id == app).ok_or(fmt("{}: not an installed Flatpak app", &[&app]))) {
            Ok(found) => flatpak::enable(&found).map(|()| println!("{}", fmt("enabled for {}", &[&app]))),
            Err(e) => Err(e),
        },
        cli::FlatpakAction::Set { app, on: false } => flatpak::disable(&app).map(|()| println!("{}", fmt("undone for {}", &[&app]))),
        cli::FlatpakAction::Global(true) => flatpak::enable_global().map(|()| println!("{}", tr("enabled for all Flatpak apps"))),
        cli::FlatpakAction::Global(false) => flatpak::disable_global().map(|()| println!("{}", tr("undone for all Flatpak apps"))),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(e),
    }
}
