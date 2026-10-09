//! `lrctl`: escreve o estado compartilhado lido pela layer.
//!
//! - `lrctl on|off|toggle|status`: age uma vez e sai.
//! - `lrctl daemon`: fica residente e reage aos atalhos globais.

mod action;
mod backend;
mod bindings;
mod daemon;

use action::Action;
use std::process::ExitCode;

const USAGE: &str = "uso: lrctl <on|off|toggle|status|daemon>";

fn main() -> ExitCode {
    let command = std::env::args().nth(1).unwrap_or_else(|| "status".into());
    let state = match ipc::Writer::create() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("lrctl: não consegui abrir {}: {e}", ipc::state_path().display());
            return ExitCode::FAILURE;
        }
    };

    let action = match command.as_str() {
        "on" => Some(Action::EffectsOn),
        "off" => Some(Action::EffectsOff),
        "toggle" => Some(Action::ToggleEffects),
        "status" => None,
        "daemon" => {
            return match daemon::run(&state) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("lrctl: {e}");
                    ExitCode::FAILURE
                }
            };
        }
        _ => {
            eprintln!("lrctl: comando desconhecido {command:?}\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    match action {
        Some(action) => println!("{}", action.apply(&state)),
        None => println!("{}", action::Applied { effects_enabled: state.effects_enabled() }),
    }
    ExitCode::SUCCESS
}
