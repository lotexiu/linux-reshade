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
        let layers = ipc::live_layers().map_err(|e| format!("não consegui listar as layers: {e}"))?;
        let found = layers::find(&layers, query)?.clone();
        let channel = state.channel(&found.id).map_err(|e| format!("não consegui abrir o canal de {query}: {e}"))?;
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
            Target::All(_) => "todos os jogos".into(),
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
                ChannelSelection::NoEffect => Err(format!("{} está sem efeito; escolha um com 'lrctl use <Nome> --layer ...'", self.label())),
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
            return fail("não sei onde ficam seus dados (HOME e XDG_DATA_HOME vazios): passe a pasta");
        };
        let dir = match std::path::absolute(&dir) {
            Ok(d) => d,
            Err(e) => return fail(format!("{}: {e}", dir.display())),
        };
        match config::init(&dir, &env) {
            Ok(done) => {
                for path in &done.created {
                    println!("criado: {}", path.display());
                }
                match &done.location {
                    Some(file) => println!("escolhida como pasta de dados (ponteiro em {})", file.display()),
                    None => println!("pasta pronta, mas não achei onde gravar o ponteiro (HOME/XDG_CONFIG_HOME): use LINUX_RESHADE_DATA={}", dir.display()),
                }
                println!("coloque os efeitos em {}/Shaders e as imagens em {}/Textures", dir.display(), dir.display());
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
    format!("{} ({}x{}, técnica(s): {})", bundle.manifest.source, t.width, t.height, names.join(", "))
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
            println!("as capturas vão para {}", config.screenshot_dir.display());
            ExitCode::SUCCESS
        }
        Command::Daemon => match daemon::run(state) {
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
                Err(e) => return fail(format!("não consegui listar as layers: {e}")),
            }
            ExitCode::SUCCESS
        }
        Command::Config { init, dir } => config_command(init, dir),
        Command::DepthPick { pick, layer } => depth_pick(pick, layer),
        Command::Preset { action, game, file } => crate::preset_cmd::run(action, game, file),
        Command::Effects => match Compiler::from_env() {
            Ok(compiler) => {
                let available = compiler.available();
                if available.is_empty() {
                    println!("nenhum efeito encontrado em: {}", compiler.effect_dirs().iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join(", "));
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
                println!("vigiando os jogos com a layer (Ctrl+C para sair)");
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
                    Some(source) => println!("{:<22} {:<8} (preenchido pelo runtime: {source})", row.name, row.type_name),
                    None => {
                        let range = row.range.map(|(a, b)| format!("  [{} .. {}]", params::fmt_number(a), params::fmt_number(b))).unwrap_or_default();
                        let label = row.label.as_deref().map(|l| format!("  \"{l}\"")).unwrap_or_default();
                        let default = if row.value == row.default { String::new() } else { format!("  (padrão {})", params::fmt_values(&row.default)) };
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
                println!("parâmetros de volta aos valores padrão ({})", target.label());
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
            println!("nenhum efeito escolhido (a layer volta a usar LINUX_RESHADE_BUNDLE, se o jogo tiver)");
        }
        Target::Game { channel, .. } => {
            channel.publish_no_effect();
            println!("{}: sem efeito (mesmo que haja um escolhido para todos)", target.label());
        }
    }
    ExitCode::SUCCESS
}

/// `lrctl use inherit --layer X`: o jogo volta a seguir a escolha de todos.
fn inherit(target: &Target) -> ExitCode {
    if let Target::Game { channel, .. } = target {
        channel.inherit();
        println!("{}: volta a seguir a escolha de todos os jogos", target.label());
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
    let Some(text) = abs.to_str() else { return fail("o caminho do bundle não é UTF-8") };
    if let Err(e) = target.selection_store().publish_selection(text) {
        return fail(format!("não consegui publicar a escolha: {e}"));
    }
    println!("efeito escolhido para {}: {}", target.label(), describe_bundle(&bundle));
    println!("a layer o carrega no próximo frame com os efeitos ligados (o tamanho precisa ser o do jogo)");
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
        println!("{name} {w}x{h}: {}", if *fresh { "compilado" } else { "já estava compilado" });
    }
    println!("efeito escolhido para {}: {name}", target.label());
    if outcome.prepared.is_empty() {
        println!("nenhum jogo com a layer aberto agora; ele é compilado quando um aparecer (o 'lrctl daemon' ou 'lrctl watch' precisa estar rodando)");
    } else {
        println!("o vigia (daemon ou watch) compila as resoluções novas que aparecerem");
    }
    ExitCode::SUCCESS
}

fn current(target: &Target) -> ExitCode {
    if let Target::Game { layer, global, .. } = target {
        match global.layer_selection(&layer.id) {
            ChannelSelection::Path(_) => println!("{}: escolha própria", target.label()),
            ChannelSelection::NoEffect => {
                println!("{}: sem efeito (escolha própria)", target.label());
                return ExitCode::SUCCESS;
            }
            ChannelSelection::Inherit | ChannelSelection::Unstable => println!("{}: sem escolha própria; segue a de todos os jogos", target.label()),
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
            Some(name) => println!("{name} (por nome; um bundle por resolução em {path})"),
            None => println!("{path} (modelo por resolução)"),
        },
        SelectionRead::Path(path) => match load(Path::new(&path)) {
            Ok(bundle) => println!("{path}\n  {}", describe_bundle(&bundle)),
            Err(e) => println!("{path}\n  (não carrega agora: {e})"),
        },
        SelectionRead::Cleared => {
            println!("nenhum efeito escolhido");
            if let Some(env) = std::env::var_os("LINUX_RESHADE_BUNDLE") {
                println!("(LINUX_RESHADE_BUNDLE={})", Path::new(&env).display());
            }
        }
        SelectionRead::Unstable => println!("a escolha está sendo atualizada; tente de novo"),
    }
}

/// `lrctl depth pick ...`: escreve o pedido que a layer do jogo lê (ver `depth.md`).
fn depth_pick(pick: DepthPick, layer: Option<String>) -> ExitCode {
    let layers = match ipc::live_layers() {
        Ok(l) => l,
        Err(e) => return fail(format!("não consegui listar as layers: {e}")),
    };
    let targets: Vec<LiveLayer> = match layer {
        Some(query) => match layers::find(&layers, &query) {
            Ok(l) => vec![l.clone()],
            Err(e) => return fail(e),
        },
        None => layers,
    };
    if targets.is_empty() {
        return fail("nenhum jogo com a layer ativa");
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
                DepthPick::Auto => "profundidade automática".to_string(),
                DepthPick::Candidate(n) => format!("candidata {n}"),
                DepthPick::Pass(n) => format!("profundidade depois do passe {n}"),
                DepthPick::Clear(n) => format!("profundidade antes da limpeza {n}"),
            }),
            Err(e) => return fail(format!("{}: {e}", l.id)),
        }
    }
    ExitCode::SUCCESS
}
