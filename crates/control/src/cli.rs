//! Interpretação da linha de comando, sem efeitos colaterais (para ser testada sem terminal).

use std::path::PathBuf;

pub const USAGE: &str = "\
uso: lrctl <comando> [opções]

  on | off | toggle | status      liga/desliga os efeitos (ou mostra o estado)
  daemon                          fica residente e reage ao atalho global

  uniforms                        lista os parâmetros do efeito, com valor atual e faixa
  get <nome>                      mostra o valor atual de um parâmetro
  set <nome> <valor...>           altera um parâmetro (vetores: 0.1 0.2 0.3 ou 0.1,0.2,0.3)
  reset                           volta todos os parâmetros aos valores padrão

opções dos comandos de parâmetros:
  --bundle <diretório>            o efeito (padrão: a variável LINUX_RESHADE_BUNDLE)";

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Status,
    On,
    Off,
    Toggle,
    Daemon,
    Uniforms { bundle: PathBuf },
    Get { bundle: PathBuf, name: String },
    Set { bundle: PathBuf, name: String, values: Vec<String> },
    Reset { bundle: PathBuf },
}

/// `args` sem o nome do programa. `env_bundle` é o valor de `LINUX_RESHADE_BUNDLE`.
pub fn parse(args: &[String], env_bundle: Option<PathBuf>) -> Result<Command, String> {
    // Separa `--bundle X` (em qualquer posição) do resto.
    let mut bundle = env_bundle;
    let mut rest = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--bundle" {
            bundle = Some(PathBuf::from(it.next().ok_or("--bundle precisa de um diretório")?));
        } else if let Some(dir) = a.strip_prefix("--bundle=") {
            bundle = Some(PathBuf::from(dir));
        } else {
            rest.push(a.clone());
        }
    }
    let need_bundle = |what: &str| bundle.clone().ok_or_else(|| format!("'{what}' precisa de um efeito: use --bundle <diretório> ou defina LINUX_RESHADE_BUNDLE"));

    let Some((command, tail)) = rest.split_first() else { return Ok(Command::Status) };
    let no_args = |c: Command| if tail.is_empty() { Ok(c) } else { Err(format!("'{command}' não aceita argumentos")) };
    match command.as_str() {
        "status" => no_args(Command::Status),
        "on" => no_args(Command::On),
        "off" => no_args(Command::Off),
        "toggle" => no_args(Command::Toggle),
        "daemon" => no_args(Command::Daemon),
        "uniforms" => no_args(Command::Uniforms { bundle: need_bundle("uniforms")? }),
        "reset" => no_args(Command::Reset { bundle: need_bundle("reset")? }),
        "get" => match tail {
            [name] => Ok(Command::Get { bundle: need_bundle("get")?, name: name.clone() }),
            _ => Err("uso: lrctl get <nome>".into()),
        },
        "set" => match tail {
            [name, values @ ..] if !values.is_empty() => {
                Ok(Command::Set { bundle: need_bundle("set")?, name: name.clone(), values: values.to_vec() })
            }
            _ => Err("uso: lrctl set <nome> <valor...>".into()),
        },
        other => Err(format!("comando desconhecido: {other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn bundle() -> Option<PathBuf> {
        Some(PathBuf::from("/b"))
    }

    #[test]
    fn no_arguments_means_status() {
        assert_eq!(parse(&[], None), Ok(Command::Status));
    }

    #[test]
    fn simple_commands_take_no_arguments() {
        assert_eq!(parse(&args(&["toggle"]), None), Ok(Command::Toggle));
        assert_eq!(parse(&args(&["daemon"]), None), Ok(Command::Daemon));
        assert!(parse(&args(&["on", "x"]), None).is_err());
    }

    #[test]
    fn bundle_comes_from_the_flag_or_the_environment() {
        assert_eq!(parse(&args(&["uniforms"]), bundle()), Ok(Command::Uniforms { bundle: PathBuf::from("/b") }));
        assert_eq!(
            parse(&args(&["--bundle", "/x", "uniforms"]), bundle()),
            Ok(Command::Uniforms { bundle: PathBuf::from("/x") }),
            "a flag vence a variável"
        );
        assert_eq!(parse(&args(&["uniforms", "--bundle=/y"]), None), Ok(Command::Uniforms { bundle: PathBuf::from("/y") }));
        let err = parse(&args(&["uniforms"]), None).unwrap_err();
        assert!(err.contains("LINUX_RESHADE_BUNDLE"), "{err}");
    }

    #[test]
    fn set_keeps_every_value_as_text() {
        assert_eq!(
            parse(&args(&["set", "Color", "0.1", "0.2,0.3"]), bundle()),
            Ok(Command::Set { bundle: PathBuf::from("/b"), name: "Color".into(), values: args(&["0.1", "0.2,0.3"]) })
        );
        assert!(parse(&args(&["set", "Color"]), bundle()).is_err(), "sem valor");
        assert!(parse(&args(&["get"]), bundle()).is_err());
        assert!(parse(&args(&["get", "a", "b"]), bundle()).is_err());
    }

    #[test]
    fn unknown_commands_and_dangling_flags_are_errors() {
        assert!(parse(&args(&["voar"]), None).is_err());
        assert!(parse(&args(&["uniforms", "--bundle"]), None).is_err());
    }
}
