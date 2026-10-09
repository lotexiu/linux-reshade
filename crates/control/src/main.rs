//! `lrctl`: escreve o estado compartilhado lido pela layer. Será substituído/estendido pelo app
//! de controle com atalhos globais; a CLI continua útil para testes e scripts.

use std::process::ExitCode;

fn main() -> ExitCode {
    let command = std::env::args().nth(1).unwrap_or_default();
    let writer = match ipc::Writer::create() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("lrctl: não consegui abrir {}: {e}", ipc::state_path().display());
            return ExitCode::FAILURE;
        }
    };
    match command.as_str() {
        "on" => writer.set_effects_enabled(true),
        "off" => writer.set_effects_enabled(false),
        "toggle" => {
            writer.toggle_effects();
        }
        "status" | "" => {}
        other => {
            eprintln!("lrctl: comando desconhecido {other:?}. Uso: lrctl <on|off|toggle|status>");
            return ExitCode::from(2);
        }
    }
    println!("efeitos: {}", if writer.effects_enabled() { "ligados" } else { "desligados" });
    ExitCode::SUCCESS
}
