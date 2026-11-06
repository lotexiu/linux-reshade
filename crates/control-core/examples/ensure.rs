//! `cargo run --release -p control-core --example ensure -- <Efeito> [largura altura]`: apaga o compilado do efeito, compila de novo
//! pelo mesmo caminho do daemon (fx2spv + spirv-opt) e valida cada SPIR-V do bundle com o `spirv-val`. Sai com erro se algum for
//! inválido para o Vulkan 1.0.
use control_core::compiler::Compiler;
use std::process::Command;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(name) = args.next() else {
        eprintln!("uso: ensure <Efeito> [largura altura]");
        std::process::exit(2);
    };
    let (w, h) = (args.next().and_then(|v| v.parse().ok()).unwrap_or(1920), args.next().and_then(|v| v.parse().ok()).unwrap_or(1080));
    let compiler = Compiler::from_env().expect("compilador");
    compiler.invalidate(&name);
    let started = std::time::Instant::now();
    let dir = compiler.ensure(&name, w, h).unwrap_or_else(|e| panic!("{e}")).dir_owned();
    println!("compilado em {:.1} s: {}", started.elapsed().as_secs_f64(), dir.display());
    let mut bad = 0;
    for entry in std::fs::read_dir(&dir).unwrap().flatten().filter(|e| e.path().extension().is_some_and(|x| x == "spv")) {
        let out = Command::new("spirv-val").args(["--target-env", "vulkan1.0"]).arg(entry.path()).output().expect("spirv-val");
        if !out.status.success() {
            bad += 1;
            eprintln!("INVÁLIDO {}: {}", entry.file_name().to_string_lossy(), String::from_utf8_lossy(&out.stderr).lines().next().unwrap_or(""));
        }
    }
    println!("{bad} módulo(s) inválido(s)");
    std::process::exit(i32::from(bad > 0));
}
