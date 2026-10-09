//! `lrctl`: o app de controle do linux-reshade.
//!
//! - `lrctl on|off|toggle|status`: liga/desliga os efeitos.
//! - `lrctl daemon`: fica residente e reage ao atalho global.
//! - `lrctl uniforms|get|set|reset`: lê e altera os parâmetros do efeito em tempo real.

mod action;
mod backend;
mod bindings;
mod cli;
mod daemon;
mod params;

use action::Action;
use cli::Command;
use fx_bundle::Bundle;
use std::path::Path;
use std::process::ExitCode;

fn fail(message: impl std::fmt::Display) -> ExitCode {
    eprintln!("lrctl: {message}");
    ExitCode::FAILURE
}

fn load(dir: &Path) -> Result<Bundle, ExitCode> {
    Bundle::load(dir).map_err(|e| fail(format!("{}: {e}", dir.display())))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let env_bundle = std::env::var_os("LINUX_RESHADE_BUNDLE").map(Into::into);
    let command = match cli::parse(&args, env_bundle) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("lrctl: {e}\n\n{}", cli::USAGE);
            return ExitCode::from(2);
        }
    };
    let state = match ipc::Writer::create() {
        Ok(w) => w,
        Err(e) => return fail(format!("não consegui abrir {}: {e}", ipc::state_path().display())),
    };

    let action = match command {
        Command::On => Some(Action::EffectsOn),
        Command::Off => Some(Action::EffectsOff),
        Command::Toggle => Some(Action::ToggleEffects),
        Command::Status => None,
        Command::Daemon => {
            return match daemon::run(&state) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => fail(e),
            };
        }
        Command::Uniforms { bundle } => {
            let bundle = match load(&bundle) { Ok(b) => b, Err(code) => return code };
            for row in params::describe(&bundle, &state) {
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
            return ExitCode::SUCCESS;
        }
        Command::Get { bundle, name } => {
            let bundle = match load(&bundle) { Ok(b) => b, Err(code) => return code };
            return match params::get(&bundle, &state, &name) {
                Ok(v) => { println!("{}", params::fmt_values(&v)); ExitCode::SUCCESS }
                Err(e) => fail(e),
            };
        }
        Command::Set { bundle, name, values } => {
            let bundle = match load(&bundle) { Ok(b) => b, Err(code) => return code };
            let parsed = match params::parse_values(&values) { Ok(v) => v, Err(e) => return fail(e) };
            return match params::set(&bundle, &state, &name, &parsed) {
                Ok(change) => {
                    println!("{name}: {} -> {}", params::fmt_values(&change.old), params::fmt_values(&change.new));
                    for w in &change.warnings { eprintln!("aviso: {w}"); }
                    ExitCode::SUCCESS
                }
                Err(e) => fail(e),
            };
        }
        Command::Reset { bundle } => {
            let bundle = match load(&bundle) { Ok(b) => b, Err(code) => return code };
            return match params::reset(&bundle, &state) {
                Ok(()) => { println!("parâmetros de volta aos valores padrão"); ExitCode::SUCCESS }
                Err(e) => fail(e),
            };
        }
    };

    match action {
        Some(action) => println!("{}", action.apply(&state)),
        None => println!("{}", action::Applied { effects_enabled: state.effects_enabled() }),
    }
    ExitCode::SUCCESS
}
