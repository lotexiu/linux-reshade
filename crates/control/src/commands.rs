//! Execução de cada comando do `lrctl`. Cada função devolve o código de saída e escreve na saída padrão
//! ou de erro; a lógica que merece teste vive em `params` e `cli`.

use crate::action::{self, Action};
use crate::cli::{self, Command};
use crate::compiler::Compiler;
use crate::{daemon, layers, params, watch};
use fx_bundle::Bundle;
use ipc::SelectionRead;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn fail(message: impl std::fmt::Display) -> ExitCode {
    eprintln!("lrctl: {message}");
    ExitCode::FAILURE
}

fn load(dir: &Path) -> Result<Bundle, String> {
    Bundle::load(dir).map_err(|e| format!("{}: {e}", dir.display()))
}

/// O bundle dos comandos de parâmetros: `--bundle`, senão o escolhido com `use`, senão o do ambiente.
fn bundle_for(explicit: Option<PathBuf>, state: &ipc::Writer) -> Result<Bundle, String> {
    let env = std::env::var_os("LINUX_RESHADE_BUNDLE").map(PathBuf::from);
    load(&cli::resolve_bundle(explicit, state.selection(), env)?)
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
        Command::Daemon => match daemon::run(state) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(e),
        },
        Command::Use { bundle: None } => {
            state.clear_selection();
            println!("nenhum efeito escolhido (a layer volta a usar LINUX_RESHADE_BUNDLE, se o jogo tiver)");
            ExitCode::SUCCESS
        }
        Command::Use { bundle: Some(arg) } if !cli::is_path(&arg.to_string_lossy()) => select_by_name(&arg.to_string_lossy(), state),
        Command::Use { bundle: Some(dir) } => select(&dir, state),
        Command::Current => current(state),
        Command::Layers => {
            match ipc::live_layers() {
                Ok(list) => print!("{}", layers::render(&list)),
                Err(e) => return fail(format!("não consegui listar as layers: {e}")),
            }
            ExitCode::SUCCESS
        }
        Command::Watch => match Compiler::from_env() {
            Ok(compiler) => {
                println!("vigiando os jogos com a layer (Ctrl+C para sair)");
                watch::run(&compiler, state)
            }
            Err(e) => fail(e),
        },
        Command::Uniforms { bundle } => with_bundle(bundle, state, |bundle| {
            for row in params::describe(bundle, state) {
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
        Command::Get { bundle, name } => with_bundle(bundle, state, |bundle| match params::get(bundle, state, &name) {
            Ok(v) => {
                println!("{}", params::fmt_values(&v));
                ExitCode::SUCCESS
            }
            Err(e) => fail(e),
        }),
        Command::Set { bundle, name, values } => with_bundle(bundle, state, |bundle| {
            let parsed = match params::parse_values(&values) {
                Ok(v) => v,
                Err(e) => return fail(e),
            };
            match params::set(bundle, state, &name, &parsed) {
                Ok(change) => {
                    println!("{name}: {} -> {}", params::fmt_values(&change.old), params::fmt_values(&change.new));
                    for w in &change.warnings {
                        eprintln!("aviso: {w}");
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => fail(e),
            }
        }),
        Command::Reset { bundle } => with_bundle(bundle, state, |bundle| match params::reset(bundle, state) {
            Ok(()) => {
                println!("parâmetros de volta aos valores padrão");
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

fn with_bundle(explicit: Option<PathBuf>, state: &ipc::Writer, f: impl FnOnce(&Bundle) -> ExitCode) -> ExitCode {
    match bundle_for(explicit, state) {
        Ok(bundle) => f(&bundle),
        Err(e) => fail(e),
    }
}

/// `lrctl use <dir>`: confere que o bundle é válido antes de pedir à layer que o carregue.
fn select(dir: &Path, state: &ipc::Writer) -> ExitCode {
    let abs = match std::fs::canonicalize(dir) {
        Ok(p) => p,
        Err(e) => return fail(format!("{}: {e}", dir.display())),
    };
    let bundle = match load(&abs) {
        Ok(b) => b,
        Err(e) => return fail(e),
    };
    let Some(text) = abs.to_str() else { return fail("o caminho do bundle não é UTF-8") };
    if let Err(e) = state.publish_selection(text) {
        return fail(format!("não consegui publicar a escolha: {e}"));
    }
    println!("efeito escolhido: {}", describe_bundle(&bundle));
    println!("a layer o carrega no próximo frame com os efeitos ligados (o tamanho precisa ser o do jogo)");
    ExitCode::SUCCESS
}

/// `lrctl use Sepia`: escolhe um efeito pelo nome. Compila já para as resoluções dos jogos abertos
/// (assim aparece sem esperar) e publica o modelo `<cache>/Sepia-{W}x{H}`; o vigia cuida das resoluções
/// que surgirem depois.
fn select_by_name(name: &str, state: &ipc::Writer) -> ExitCode {
    let compiler = match Compiler::from_env() {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    if let Err(e) = compiler.source(name) {
        return fail(e);
    }
    let live = ipc::live_layers().unwrap_or_default();
    let mut resolutions: Vec<(u32, u32)> = live
        .iter()
        .flat_map(|l| &l.status.swapchains)
        .filter(|s| !matches!(s.effect, ipc::EffectState::Unavailable(_)))
        .map(|s| (s.width, s.height))
        .collect();
    resolutions.sort_unstable();
    resolutions.dedup();
    for (w, h) in &resolutions {
        match compiler.ensure(name, *w, *h) {
            Ok(done) => println!("{name} {w}x{h}: {}", if matches!(done, crate::compiler::Ensured::Compiled(_)) { "compilado" } else { "já estava compilado" }),
            Err(e) => return fail(e),
        }
    }
    if let Err(e) = state.publish_selection(&compiler.template(name)) {
        return fail(format!("não consegui publicar a escolha: {e}"));
    }
    println!("efeito escolhido: {name}");
    if resolutions.is_empty() {
        println!("nenhum jogo com a layer aberto agora; ele é compilado quando um aparecer (o 'lrctl daemon' ou 'lrctl watch' precisa estar rodando)");
    } else {
        println!("o vigia (daemon ou watch) compila as resoluções novas que aparecerem");
    }
    ExitCode::SUCCESS
}

fn current(state: &ipc::Writer) -> ExitCode {
    match state.selection() {
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
    ExitCode::SUCCESS
}
