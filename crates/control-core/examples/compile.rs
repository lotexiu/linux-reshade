//! Compila um efeito como o painel e o daemon fazem (com as pastas de efeitos e texturas da configuração e os efeitos que vêm com o programa) e
//! imprime a pasta do bundle: `cargo run -p control-core --example compile -- <Efeito> <largura> <altura>` (use `LINUX_RESHADE_CACHE` para não
//! misturar com o cache do usuário).

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [name, width, height] = args.as_slice() else {
        eprintln!("uso: compile <Efeito> <largura> <altura>");
        std::process::exit(2);
    };
    let (Ok(width), Ok(height)) = (width.parse::<u32>(), height.parse::<u32>()) else {
        eprintln!("largura e altura precisam ser números");
        std::process::exit(2);
    };
    let compiler = match control_core::compiler::Compiler::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    match compiler.ensure(name, width, height) {
        Ok(ensured) => println!("{}", ensured.dir_owned().display()),
        Err(e) => {
            eprintln!("{name}: {e}");
            std::process::exit(1);
        }
    }
}
