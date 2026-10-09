//! Interpretação da linha de comando, sem efeitos colaterais (para ser testada sem terminal).

use std::path::PathBuf;

pub const USAGE: &str = "\
uso: lrctl <comando> [opções]

  on | off | toggle | status      liga/desliga os efeitos (ou mostra o estado)
  daemon                          fica residente e reage ao atalho global

  use <diretório-do-bundle>       escolhe o efeito que a layer carrega (sem editar o Steam)
  use none                        desfaz a escolha
  current                         mostra o efeito escolhido

  uniforms                        lista os parâmetros do efeito, com valor atual e faixa
  get <nome>                      mostra o valor atual de um parâmetro
  set <nome> <valor...>           altera um parâmetro (vetores: 0.1 0.2 0.3 ou 0.1,0.2,0.3)
  reset                           volta todos os parâmetros aos valores padrão

opções dos comandos de parâmetros:
  --bundle <diretório>            o efeito; sem isso, vale o escolhido com 'use' e, na falta dele,
                                  a variável LINUX_RESHADE_BUNDLE";

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Status,
    On,
    Off,
    Toggle,
    Daemon,
    /// Escolhe o efeito (`Some`) ou desfaz a escolha (`None`).
    Use { bundle: Option<PathBuf> },
    Current,
    /// Os comandos de parâmetros levam o `--bundle` explícito, se houve; quem decide o padrão é [`resolve_bundle`].
    Uniforms { bundle: Option<PathBuf> },
    Get { bundle: Option<PathBuf>, name: String },
    Set { bundle: Option<PathBuf>, name: String, values: Vec<String> },
    Reset { bundle: Option<PathBuf> },
}

/// Qual bundle os comandos de parâmetros usam: o `--bundle` explícito; senão o escolhido com `use`;
/// senão o da variável de ambiente.
pub fn resolve_bundle(explicit: Option<PathBuf>, selected: ipc::SelectionRead, env: Option<PathBuf>) -> Result<PathBuf, String> {
    if let Some(path) = explicit {
        return Ok(path);
    }
    if let ipc::SelectionRead::Path(path) = selected {
        return Ok(PathBuf::from(path));
    }
    env.ok_or_else(|| "nenhum efeito escolhido: use 'lrctl use <diretório>', --bundle <diretório> ou defina LINUX_RESHADE_BUNDLE".to_string())
}

/// `args` sem o nome do programa.
pub fn parse(args: &[String]) -> Result<Command, String> {
    // Separa `--bundle X` (em qualquer posição) do resto.
    let mut bundle: Option<PathBuf> = None;
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
    let Some((command, tail)) = rest.split_first() else { return Ok(Command::Status) };
    let no_args = |c: Command| if tail.is_empty() { Ok(c) } else { Err(format!("'{command}' não aceita argumentos")) };
    match command.as_str() {
        "status" => no_args(Command::Status),
        "on" => no_args(Command::On),
        "off" => no_args(Command::Off),
        "toggle" => no_args(Command::Toggle),
        "daemon" => no_args(Command::Daemon),
        "current" => no_args(Command::Current),
        "use" => match tail {
            [none] if none == "none" => Ok(Command::Use { bundle: None }),
            [dir] => Ok(Command::Use { bundle: Some(PathBuf::from(dir)) }),
            _ => Err("uso: lrctl use <diretório-do-bundle> | lrctl use none".into()),
        },
        "uniforms" => no_args(Command::Uniforms { bundle }),
        "reset" => no_args(Command::Reset { bundle }),
        "get" => match tail {
            [name] => Ok(Command::Get { bundle, name: name.clone() }),
            _ => Err("uso: lrctl get <nome>".into()),
        },
        "set" => match tail {
            [name, values @ ..] if !values.is_empty() => {
                Ok(Command::Set { bundle, name: name.clone(), values: values.to_vec() })
            }
            _ => Err("uso: lrctl set <nome> <valor...>".into()),
        },
        other => Err(format!("comando desconhecido: {other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipc::SelectionRead;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn p(path: &str) -> Option<PathBuf> {
        Some(PathBuf::from(path))
    }

    #[test]
    fn no_arguments_means_status() {
        assert_eq!(parse(&[]), Ok(Command::Status));
    }

    #[test]
    fn simple_commands_take_no_arguments() {
        assert_eq!(parse(&args(&["toggle"])), Ok(Command::Toggle));
        assert_eq!(parse(&args(&["daemon"])), Ok(Command::Daemon));
        assert_eq!(parse(&args(&["current"])), Ok(Command::Current));
        assert!(parse(&args(&["on", "x"])).is_err());
    }

    #[test]
    fn use_selects_or_clears_the_effect() {
        assert_eq!(parse(&args(&["use", "/b/Sepia"])), Ok(Command::Use { bundle: p("/b/Sepia") }));
        assert_eq!(parse(&args(&["use", "none"])), Ok(Command::Use { bundle: None }));
        assert!(parse(&args(&["use"])).is_err());
        assert!(parse(&args(&["use", "a", "b"])).is_err());
    }

    #[test]
    fn the_explicit_bundle_flag_is_kept_for_the_parameter_commands() {
        assert_eq!(parse(&args(&["uniforms"])), Ok(Command::Uniforms { bundle: None }));
        assert_eq!(parse(&args(&["--bundle", "/x", "uniforms"])), Ok(Command::Uniforms { bundle: p("/x") }));
        assert_eq!(parse(&args(&["uniforms", "--bundle=/y"])), Ok(Command::Uniforms { bundle: p("/y") }));
    }

    #[test]
    fn the_bundle_is_resolved_from_flag_then_selection_then_environment() {
        let selected = || SelectionRead::Path("/escolhido".into());
        assert_eq!(resolve_bundle(p("/flag"), selected(), p("/env")), Ok(PathBuf::from("/flag")), "a flag vence tudo");
        assert_eq!(resolve_bundle(None, selected(), p("/env")), Ok(PathBuf::from("/escolhido")), "depois o efeito escolhido");
        assert_eq!(resolve_bundle(None, SelectionRead::Cleared, p("/env")), Ok(PathBuf::from("/env")), "por fim o ambiente");
        assert_eq!(resolve_bundle(None, SelectionRead::Unstable, p("/env")), Ok(PathBuf::from("/env")));
        let err = resolve_bundle(None, SelectionRead::Cleared, None).unwrap_err();
        assert!(err.contains("lrctl use"), "{err}");
    }

    #[test]
    fn set_keeps_every_value_as_text() {
        assert_eq!(
            parse(&args(&["set", "Color", "0.1", "0.2,0.3"])),
            Ok(Command::Set { bundle: None, name: "Color".into(), values: args(&["0.1", "0.2,0.3"]) })
        );
        assert!(parse(&args(&["set", "Color"])).is_err(), "sem valor");
        assert!(parse(&args(&["get"])).is_err());
        assert!(parse(&args(&["get", "a", "b"])).is_err());
    }

    #[test]
    fn unknown_commands_and_dangling_flags_are_errors() {
        assert!(parse(&args(&["voar"])).is_err());
        assert!(parse(&args(&["uniforms", "--bundle"])).is_err());
    }
}
