//! `lrctl`: o app de controle do linux-reshade.
//!
//! - `lrctl on|off|toggle|status`: liga/desliga os efeitos.
//! - `lrctl daemon`: fica residente e reage ao atalho global.
//! - `lrctl use|current`: escolhe o efeito que a layer carrega.
//! - `lrctl uniforms|get|set|reset`: lê e altera os parâmetros do efeito em tempo real.

mod action;
mod backend;
mod bindings;
mod cli;
mod commands;
mod daemon;
mod params;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = match cli::parse(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("lrctl: {e}\n\n{}", cli::USAGE);
            return ExitCode::from(2);
        }
    };
    let state = match ipc::Writer::create() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("lrctl: não consegui abrir {}: {e}", ipc::state_path().display());
            return ExitCode::FAILURE;
        }
    };
    commands::run(command, &state)
}
