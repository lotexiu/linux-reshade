//! Interpretação da linha de comando, sem efeitos colaterais (para ser testada sem terminal).

use std::path::PathBuf;

pub const USAGE: &str = "\
uso: lrctl <comando> [opções]

  on | off | toggle | status      liga/desliga os efeitos (ou mostra o estado)
  daemon                          fica residente e reage ao atalho global

  use <Nome>                      escolhe um efeito pelo nome (ex.: Sepia); é compilado para a
                                  resolução de cada jogo aberto, e para as novas que aparecerem
  use <diretório-do-bundle>       escolhe um bundle já compilado (fixo numa resolução)
  use none                        desfaz a escolha (com --layer: sem efeito só naquele jogo)
  use inherit --layer <jogo>      o jogo volta a seguir a escolha de todos
  current                         mostra o efeito escolhido
  effects                         lista os efeitos que podem ser escolhidos por nome
  layers                          mostra os jogos com a layer ativa: resolução, formato, estado
  watch                           só compila, sem atalho global (o daemon já faz isso)
  screenshot                      captura a tela de cada jogo com a layer (com e sem efeitos); o daemon/watch grava os PNG

  depth pick candidate <N>        escolhe a N-ésima candidata de profundidade da lista de 'lrctl layers' (de 1); com
                                  --layer <jogo> vale só para ele
  depth pick pass <N>             usa a profundidade logo depois do N-ésimo passe que a desenha (de 1), por cópia
  depth pick clear <N>            usa a profundidade logo antes da N-ésima limpeza dela no quadro (como o ReShade), por cópia
  depth pick auto                 volta à escolha automática

  config                          mostra onde estão os efeitos, texturas, presets e cache
  config init [pasta]             prepara a pasta de dados (Shaders, Textures, Presets, config.ini);
                                  sem <pasta>: ~/.local/share/linux-reshade
                                  e a escolhe; sobrevive a reinstalar o jogo ou a máquina

  uniforms                        lista os parâmetros do efeito, com valor atual e faixa
  get <nome>                      mostra o valor atual de um parâmetro
  set <nome> <valor...>           altera um parâmetro (vetores: 0.1 0.2 0.3 ou 0.1,0.2,0.3)
  reset                           volta todos os parâmetros aos valores padrão

  preset show                     as técnicas do preset (ordem de execução; [x] ligada, [ ] desligada)
  preset enable|disable <técnica> liga ou desliga (Nome@Arquivo.fx, ou só Nome)
  preset move <técnica> <posição> muda a posição na ordem de execução (começa em 1)
  preset set <Arquivo.fx> <parâmetro> <valor...>
                                  grava o valor de um parâmetro no preset
  preset macros [Arquivo.fx] [NOME=VALOR,...]
                                  mostra ou define as definições de pré-processador do preset (ou só do efeito); vazio
                                  apaga; mudar recompila os efeitos afetados
  preset new <nome> --game <id>   cria um preset do jogo, copiando o que vale agora, e o escolhe
  preset use <nome|none> --game <id>
                                  escolhe o preset do jogo; none volta ao preset global
                                  (o daemon aplica o preset aos jogos abertos assim que o arquivo muda)

opções:
  --game <id>                     o jogo dos comandos 'preset': edita o preset que vale para ele
  --file <arquivo.ini>            ou um arquivo de preset qualquer
  --layer <jogo>                  vale só para um jogo aberto, dado por número do processo (coluna PID
                                  de 'lrctl layers') ou nome; vale em use, current, uniforms, get, set
                                  e reset. Sem isso o comando vale para todos os jogos. A escolha e os
                                  parâmetros do jogo têm prioridade sobre os de todos.
  --bundle <diretório>            o efeito dos comandos de parâmetros; sem isso, vale o escolhido com
                                  'use' e, na falta dele, a variável LINUX_RESHADE_BUNDLE";

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Status,
    On,
    Off,
    Toggle,
    Daemon,
    /// Pede uma captura de tela a cada jogo com a layer aberta (o daemon ou o `watch` grava os PNG).
    Screenshot,
    Layers,
    Watch,
    Effects,
    /// Mostra a configuração de pastas (`init: false`) ou prepara uma pasta de dados (`dir`, ou a padrão).
    Config { init: bool, dir: Option<PathBuf> },
    /// Escolhe o efeito (`Some`) ou desfaz a escolha (`None`). Com `layer`, só para aquele jogo.
    Use { bundle: Option<PathBuf>, layer: Option<String> },
    /// O jogo `layer` volta a seguir a escolha global.
    Inherit { layer: String },
    Current { layer: Option<String> },
    /// Os comandos de parâmetros levam o `--bundle` explícito, se houve; quem decide o padrão é [`resolve_bundle`].
    Uniforms { bundle: Option<PathBuf>, layer: Option<String> },
    Get { bundle: Option<PathBuf>, layer: Option<String>, name: String },
    Set { bundle: Option<PathBuf>, layer: Option<String>, name: String, values: Vec<String> },
    Reset { bundle: Option<PathBuf>, layer: Option<String> },
    /// Escolhe a candidata de profundidade ou o passe de onde ela é copiada (o jogo precisa ter sido aberto com
    /// `LINUX_RESHADE_DEPTH=1`).
    DepthPick { pick: DepthPick, layer: Option<String> },
    /// Edita um preset: o que vale para `game`, ou o arquivo `file`.
    Preset { action: PresetAction, game: Option<String>, file: Option<PathBuf> },
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum DepthPick {
    Candidate(u32),
    Pass(u32),
    Clear(u32),
    Auto,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PresetAction {
    Show,
    Enable(String),
    Disable(String),
    /// Posição de 1 em diante.
    Move(String, usize),
    Set { file: String, parameter: String, values: Vec<String> },
    New(String),
    /// Mostra (`text: None`) ou define (`Some`, vazio apaga) as definições de pré-processador do preset (`file: None`) ou de um efeito.
    Macros { file: Option<String>, text: Option<String> },
    /// `None`: volta ao preset global.
    Use(Option<String>),
}

/// `lrctl use X` escolhe por *nome* ou por *diretório*? Um diretório que existe, ou qualquer coisa com
/// barra, é um caminho; o resto é o nome de um efeito.
pub fn is_path(arg: &str) -> bool {
    arg.contains('/') || std::path::Path::new(arg).is_dir()
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
    // Separa `--bundle X` e `--layer X` (em qualquer posição) do resto.
    let mut bundle: Option<PathBuf> = None;
    let mut layer: Option<String> = None;
    let mut game: Option<String> = None;
    let mut file: Option<PathBuf> = None;
    let mut rest = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--bundle" {
            bundle = Some(PathBuf::from(it.next().ok_or("--bundle precisa de um diretório")?));
        } else if let Some(dir) = a.strip_prefix("--bundle=") {
            bundle = Some(PathBuf::from(dir));
        } else if a == "--layer" {
            layer = Some(it.next().ok_or("--layer precisa do número do processo ou do nome do jogo (veja 'lrctl layers')")?.clone());
        } else if let Some(name) = a.strip_prefix("--layer=") {
            layer = Some(name.to_string());
        } else if a == "--game" {
            game = Some(it.next().ok_or("--game precisa do id do jogo")?.clone());
        } else if let Some(id) = a.strip_prefix("--game=") {
            game = Some(id.to_string());
        } else if a == "--file" {
            file = Some(PathBuf::from(it.next().ok_or("--file precisa de um arquivo .ini")?));
        } else if let Some(path) = a.strip_prefix("--file=") {
            file = Some(PathBuf::from(path));
        } else {
            rest.push(a.clone());
        }
    }
    let Some((command, tail)) = rest.split_first() else { return Ok(Command::Status) };
    let no_args = |c: Command| if tail.is_empty() { Ok(c) } else { Err(format!("'{command}' não aceita argumentos")) };
    if layer.is_some() && !matches!(command.as_str(), "use" | "current" | "uniforms" | "get" | "set" | "reset" | "depth") {
        return Err(format!("--layer não vale em '{command}'"));
    }
    if (game.is_some() || file.is_some()) && command != "preset" {
        return Err(format!("--game e --file só valem em 'preset', não em '{command}'"));
    }
    if game.is_some() && file.is_some() {
        return Err("use --game ou --file, não os dois".into());
    }
    match command.as_str() {
        "depth" => {
            let number = |n: &str| n.parse::<u32>().ok().filter(|n| *n > 0).ok_or_else(|| format!("número inválido: {n:?} (começa em 1)"));
            let pick = match tail.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
                ["pick", "candidate", n] => DepthPick::Candidate(number(n)?),
                ["pick", "pass", n] => DepthPick::Pass(number(n)?),
                ["pick", "clear", n] => DepthPick::Clear(number(n)?),
                ["pick", "auto"] => DepthPick::Auto,
                _ => return Err("uso: lrctl depth pick candidate <N> | pass <N> | auto  (--layer <jogo> opcional)".into()),
            };
            Ok(Command::DepthPick { pick, layer })
        }
        "preset" => {
            let action = match tail.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
                [] | ["show"] => PresetAction::Show,
                ["enable", t] => PresetAction::Enable((*t).into()),
                ["disable", t] => PresetAction::Disable((*t).into()),
                ["move", t, pos] => match pos.parse::<usize>() {
                    Ok(n) if n >= 1 => PresetAction::Move((*t).into(), n),
                    _ => return Err(format!("posição inválida: {pos:?} (começa em 1)")),
                },
                ["set", f, parameter, values @ ..] if !values.is_empty() => {
                    PresetAction::Set { file: (*f).into(), parameter: (*parameter).into(), values: values.iter().map(|v| (*v).into()).collect() }
                }
                ["new", name] if !name.starts_with('-') => PresetAction::New((*name).into()),
                ["macros"] => PresetAction::Macros { file: None, text: None },
                ["macros", f] if f.ends_with(".fx") => PresetAction::Macros { file: Some((*f).into()), text: None },
                ["macros", text] => PresetAction::Macros { file: None, text: Some((*text).into()) },
                ["macros", f, text] if f.ends_with(".fx") => PresetAction::Macros { file: Some((*f).into()), text: Some((*text).into()) },
                ["use", "none"] => PresetAction::Use(None),
                ["use", name] if !name.starts_with('-') => PresetAction::Use(Some((*name).into())),
                _ => return Err("uso: lrctl preset show | enable|disable <técnica> | move <técnica> <posição> | macros [Arquivo.fx] [NOME=VALOR,...] | set <Arquivo.fx> <parâmetro> <valor...> | new <nome> | use <nome|none>  (com --game <id> ou --file <arquivo>)".into()),
            };
            Ok(Command::Preset { action, game, file })
        }
        "status" => no_args(Command::Status),
        "on" => no_args(Command::On),
        "off" => no_args(Command::Off),
        "toggle" => no_args(Command::Toggle),
        "daemon" => no_args(Command::Daemon),
        "screenshot" => no_args(Command::Screenshot),
        "current" => no_args(Command::Current { layer }),
        "layers" => no_args(Command::Layers),
        "effects" => no_args(Command::Effects),
        "watch" => no_args(Command::Watch),
        "config" => match tail {
            [] => Ok(Command::Config { init: false, dir: None }),
            [init] if init == "init" => Ok(Command::Config { init: true, dir: None }),
            [init, dir] if init == "init" && !dir.starts_with('-') => Ok(Command::Config { init: true, dir: Some(PathBuf::from(dir)) }),
            _ => Err("uso: lrctl config | lrctl config init [pasta]".into()),
        },
        "use" => match tail {
            [none] if none == "none" => Ok(Command::Use { bundle: None, layer }),
            [inherit] if inherit == "inherit" => match layer {
                Some(layer) => Ok(Command::Inherit { layer }),
                None => Err("'use inherit' precisa de --layer <jogo>".into()),
            },
            [dir] => Ok(Command::Use { bundle: Some(PathBuf::from(dir)), layer }),
            _ => Err("uso: lrctl use <Nome|diretório-do-bundle> | lrctl use none | lrctl use inherit --layer <jogo>".into()),
        },
        "uniforms" => no_args(Command::Uniforms { bundle, layer }),
        "reset" => no_args(Command::Reset { bundle, layer }),
        "get" => match tail {
            [name] => Ok(Command::Get { bundle, layer, name: name.clone() }),
            _ => Err("uso: lrctl get <nome>".into()),
        },
        "set" => match tail {
            [name, values @ ..] if !values.is_empty() => {
                Ok(Command::Set { bundle, layer, name: name.clone(), values: values.to_vec() })
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
    fn depth_pick_commands() {
        assert_eq!(parse(&args(&["depth", "pick", "candidate", "2"])), Ok(Command::DepthPick { pick: DepthPick::Candidate(2), layer: None }));
        assert_eq!(parse(&args(&["depth", "pick", "pass", "7", "--layer", "Haste"])), Ok(Command::DepthPick { pick: DepthPick::Pass(7), layer: Some("Haste".into()) }));
        assert_eq!(parse(&args(&["depth", "pick", "auto"])), Ok(Command::DepthPick { pick: DepthPick::Auto, layer: None }));
        assert!(parse(&args(&["depth", "pick", "pass", "0"])).is_err());
        assert!(parse(&args(&["depth", "pick", "pass", "x"])).is_err());
        assert!(parse(&args(&["depth", "pick"])).is_err());
    }

    #[test]
    fn preset_commands_and_their_targets() {
        let c = parse(&args(&["preset", "enable", "Sepia", "--game", "vkcube"]));
        assert_eq!(c, Ok(Command::Preset { action: PresetAction::Enable("Sepia".into()), game: Some("vkcube".into()), file: None }));
        let c = parse(&args(&["preset", "--file=/x/p.ini", "move", "Sepia", "2"]));
        assert_eq!(c, Ok(Command::Preset { action: PresetAction::Move("Sepia".into(), 2), game: None, file: p("/x/p.ini") }));
        assert!(matches!(parse(&args(&["preset"])), Ok(Command::Preset { action: PresetAction::Show, .. })));
        let c = parse(&args(&["preset", "set", "Sepia.fx", "Strength", "0.5"]));
        assert!(matches!(c, Ok(Command::Preset { action: PresetAction::Set { .. }, .. })));
        assert!(matches!(parse(&args(&["preset", "use", "none", "--game", "g"])), Ok(Command::Preset { action: PresetAction::Use(None), .. })));
        assert!(matches!(parse(&args(&["preset", "use", "Noturno", "--game", "g"])), Ok(Command::Preset { action: PresetAction::Use(Some(_)), .. })));
        // Posição começa em 1; nome de preset não é uma opção; as opções só valem em 'preset'.
        let m = parse(&args(&["preset", "macros", "Sepia.fx", "Q=2,FAST", "--game", "g"]));
        assert!(matches!(m, Ok(Command::Preset { action: PresetAction::Macros { file: Some(f), text: Some(t) }, .. }) if f == "Sepia.fx" && t == "Q=2,FAST"));
        assert!(matches!(parse(&args(&["preset", "macros", "--game", "g"])), Ok(Command::Preset { action: PresetAction::Macros { file: None, text: None }, .. })));
        assert!(matches!(parse(&args(&["preset", "macros", "Q=1", "--game", "g"])), Ok(Command::Preset { action: PresetAction::Macros { file: None, text: Some(_) }, .. })));
        assert!(parse(&args(&["preset", "move", "Sepia", "0"])).is_err());
        assert!(parse(&args(&["preset", "new", "--help"])).is_err());
        assert!(parse(&args(&["preset", "set", "Sepia.fx", "Strength"])).is_err());
        assert!(parse(&args(&["use", "Sepia", "--game", "g"])).is_err());
        assert!(parse(&args(&["preset", "show", "--game", "g", "--file", "x.ini"])).is_err());
    }

    #[test]
    fn no_arguments_means_status() {
        assert_eq!(parse(&[]), Ok(Command::Status));
    }

    #[test]
    fn simple_commands_take_no_arguments() {
        assert_eq!(parse(&args(&["toggle"])), Ok(Command::Toggle));
        assert_eq!(parse(&args(&["daemon"])), Ok(Command::Daemon));
        assert_eq!(parse(&args(&["current"])), Ok(Command::Current { layer: None }));
        assert!(parse(&args(&["on", "x"])).is_err());
    }

    #[test]
    fn layers_and_watch_take_no_arguments() {
        assert_eq!(parse(&args(&["layers"])), Ok(Command::Layers));
        assert_eq!(parse(&args(&["watch"])), Ok(Command::Watch));
        assert_eq!(parse(&args(&["effects"])), Ok(Command::Effects));
        assert!(parse(&args(&["layers", "x"])).is_err());
    }

    #[test]
    fn config_shows_or_initialises() {
        assert_eq!(parse(&args(&["config"])), Ok(Command::Config { init: false, dir: None }));
        assert_eq!(parse(&args(&["config", "init", "/mnt/D/reshade"])), Ok(Command::Config { init: true, dir: p("/mnt/D/reshade") }));
        assert_eq!(parse(&args(&["config", "init"])), Ok(Command::Config { init: true, dir: None }));
        // Uma opção não é nome de pasta (`config init --help` já criou uma pasta chamada `--help`).
        assert!(parse(&args(&["config", "init", "--help"])).is_err());
        assert!(parse(&args(&["config", "x"])).is_err());
    }

    #[test]
    fn names_and_paths_are_told_apart() {
        assert!(!is_path("Sepia"));
        assert!(!is_path("FilmicPass"));
        assert!(is_path("build/bundles/Sepia-1920x1080"));
        assert!(is_path("/abs/Sepia"));
        assert!(is_path("./Sepia"));
        assert!(is_path("/tmp"), "um diretório que existe é um caminho");
    }

    #[test]
    fn use_selects_or_clears_the_effect() {
        assert_eq!(parse(&args(&["use", "/b/Sepia"])), Ok(Command::Use { bundle: p("/b/Sepia"), layer: None }));
        assert_eq!(parse(&args(&["use", "none"])), Ok(Command::Use { bundle: None, layer: None }));
        assert!(parse(&args(&["use"])).is_err());
        assert!(parse(&args(&["use", "a", "b"])).is_err());
    }

    fn game(name: &str) -> Option<String> {
        Some(name.to_string())
    }

    #[test]
    fn layer_targets_one_game_in_every_command_that_takes_it() {
        assert_eq!(parse(&args(&["use", "Sepia", "--layer", "1234"])), Ok(Command::Use { bundle: p("Sepia"), layer: game("1234") }));
        assert_eq!(parse(&args(&["--layer=vkcube", "use", "none"])), Ok(Command::Use { bundle: None, layer: game("vkcube") }));
        assert_eq!(parse(&args(&["use", "inherit", "--layer", "7"])), Ok(Command::Inherit { layer: "7".into() }));
        assert_eq!(parse(&args(&["current", "--layer", "7"])), Ok(Command::Current { layer: game("7") }));
        assert_eq!(parse(&args(&["uniforms", "--layer", "7"])), Ok(Command::Uniforms { bundle: None, layer: game("7") }));
        assert_eq!(parse(&args(&["get", "X", "--layer", "7"])), Ok(Command::Get { bundle: None, layer: game("7"), name: "X".into() }));
        assert_eq!(
            parse(&args(&["set", "X", "1", "--layer", "7", "--bundle", "/b"])),
            Ok(Command::Set { bundle: p("/b"), layer: game("7"), name: "X".into(), values: args(&["1"]) })
        );
        assert_eq!(parse(&args(&["reset", "--layer", "7"])), Ok(Command::Reset { bundle: None, layer: game("7") }));
    }

    #[test]
    fn layer_is_refused_where_it_means_nothing_and_inherit_needs_it() {
        assert!(parse(&args(&["toggle", "--layer", "7"])).is_err());
        assert!(parse(&args(&["layers", "--layer", "7"])).is_err());
        assert!(parse(&args(&["use", "inherit"])).is_err(), "herdar não faz sentido para todos os jogos");
        assert!(parse(&args(&["use", "Sepia", "--layer"])).is_err(), "falta o jogo");
    }

    #[test]
    fn the_explicit_bundle_flag_is_kept_for_the_parameter_commands() {
        assert_eq!(parse(&args(&["uniforms"])), Ok(Command::Uniforms { bundle: None, layer: None }));
        assert_eq!(parse(&args(&["--bundle", "/x", "uniforms"])), Ok(Command::Uniforms { bundle: p("/x"), layer: None }));
        assert_eq!(parse(&args(&["uniforms", "--bundle=/y"])), Ok(Command::Uniforms { bundle: p("/y"), layer: None }));
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
            Ok(Command::Set { bundle: None, layer: None, name: "Color".into(), values: args(&["0.1", "0.2,0.3"]) })
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
