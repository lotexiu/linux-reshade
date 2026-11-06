//! Interpretação da linha de comando, sem efeitos colaterais (para ser testada sem terminal).

use std::path::PathBuf;
use control_core::i18n::{fmt, tr};

pub const USAGE: &str = "\
usage: lrctl <command> [options]

  on | off | toggle | status      turn the effects on/off (or show the state)
  daemon                          stay resident and react to the global shortcut
  daemon-ensure                   start the daemon, or replace the one left over from an older build
  daemon-stop                     stop the running daemon

  use <Name>                      choose an effect by name (e.g. Sepia); it is compiled for the
                                  resolution of each open game, and for new ones that show up
  use <bundle-directory>          choose an already compiled bundle (fixed at one resolution)
  use none                        undo the choice (with --layer: no effect only in that game)
  use inherit --layer <game>      the game goes back to following the choice for all
  current                         show the chosen effect
  effects                         list the effects that can be chosen by name
  layers                          show the games with the layer active: resolution, format, state
  watch                           only compile, without the global shortcut (the daemon already does that)
  setup [--remove]                install (or remove) the Vulkan layer for the user: the library goes to ~/.local/share/linux-reshade/lib
  flatpak [list]                  list the Flatpak apps and whether linux-reshade is on for each one
  flatpak enable|disable <app-id> turn on/undo the setup of a Flatpak app (layer extension and permissions)
  flatpak global on|off           turn on/undo for ALL Flatpak apps (one app opts out with LINUX_RESHADE=0)

  stats on|off                    turn the layer's performance measurement on/off (shown in 'layers' and in the panel's Performance tab)
  screenshot                      capture the screen of each game with the layer (with and without effects); the daemon/watch writes the PNGs

  depth pick candidate <N>        choose the Nth depth candidate from the 'lrctl layers' list (from 1); with
                                  --layer <game> it applies only to that game
  depth pick pass <N>             use the depth right after the Nth pass that draws it (from 1), by copy
  depth pick clear <N>            use the depth right before its Nth clear in the frame (like ReShade), by copy
  depth pick auto                 go back to the automatic choice

  config                          show where the effects, textures, presets and cache are
  config init [folder]            prepare the data folder (Shaders, Textures, Presets, config.ini);
                                  without <folder>: ~/.local/share/linux-reshade
                                  and choose it; it survives reinstalling the game or the machine

  uniforms                        list the effect's parameters, with current value and range
  get <name>                      show the current value of a parameter
  set <name> <value...>           change a parameter (vectors: 0.1 0.2 0.3 or 0.1,0.2,0.3)
  reset                           put all parameters back to the default values

  preset show                     the preset's techniques (execution order; [x] on, [ ] off)
  preset enable|disable <technique> turn on or off (Name@File.fx, or just Name)
  preset move <technique> <position> change the position in the execution order (starts at 1)
  preset set <File.fx> <parameter> <value...>
                                  write a parameter's value into the preset
  preset macros [File.fx] [NAME=VALUE,...]
                                  show or set the preset's preprocessor definitions (or just the effect's); empty
                                  clears; changing them recompiles the affected effects
  preset new <name> --game <id>   create a preset for the game, copying what applies now, and choose it
  preset use <name|none> --game <id>
                                  choose the game's preset; none goes back to the global preset
                                  (the daemon applies the preset to open games as soon as the file changes)

options:
  --game <id>                     the game for the 'preset' commands: edits the preset that applies to it
  --file <file.ini>               or any preset file
  --layer <game>                  applies only to one open game, given by process number (PID column
                                  of 'lrctl layers') or name; works in use, current, uniforms, get, set
                                  and reset. Without it the command applies to all games. The game's
                                  choice and parameters take priority over the ones for all.
  --bundle <directory>            the effect for the parameter commands; without it, the one chosen with
                                  'use' applies and, failing that, the LINUX_RESHADE_BUNDLE variable";

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Status,
    On,
    Off,
    Toggle,
    Daemon,
    /// Inicia o daemon desta build, ou troca o que sobrou de uma build anterior.
    DaemonEnsure,
    /// Encerra o daemon em execução (e o processo dos atalhos dele).
    DaemonStop,
    /// Só os atalhos globais (o daemon inicia este processo quando há um jogo aberto).
    Keys,
    /// Pede uma captura de tela a cada jogo com a layer aberta (o daemon ou o `watch` grava os PNG).
    Screenshot,
    /// Liga ou desliga a medição de desempenho da layer (aparece em `layers`).
    Stats(bool),
    Layers,
    /// Instala a camada do Vulkan para o usuário (a biblioteca numa pasta fixa e o manifesto), ou a tira (`false`).
    Setup(bool),
    /// Liga o linux-reshade a jogos Flatpak (ver `control_core::flatpak`).
    Flatpak(FlatpakAction),
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

#[derive(Debug, PartialEq, Eq)]
pub enum FlatpakAction {
    /// Lista os apps Flatpak e o estado de cada um.
    List,
    /// Liga (`true`) ou desfaz (`false`) a configuração de um app.
    Set { app: String, on: bool },
    /// Liga (`true`) ou desfaz (`false`) a configuração global (todos os apps).
    Global(bool),
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
    env.ok_or_else(|| tr("no effect chosen: use 'lrctl use <directory>', --bundle <directory> or set LINUX_RESHADE_BUNDLE").to_string())
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
            bundle = Some(PathBuf::from(it.next().ok_or(tr("--bundle needs a directory"))?));
        } else if let Some(dir) = a.strip_prefix("--bundle=") {
            bundle = Some(PathBuf::from(dir));
        } else if a == "--layer" {
            layer = Some(it.next().ok_or(tr("--layer needs the process number or the game name (see 'lrctl layers')"))?.clone());
        } else if let Some(name) = a.strip_prefix("--layer=") {
            layer = Some(name.to_string());
        } else if a == "--game" {
            game = Some(it.next().ok_or(tr("--game needs the game id"))?.clone());
        } else if let Some(id) = a.strip_prefix("--game=") {
            game = Some(id.to_string());
        } else if a == "--file" {
            file = Some(PathBuf::from(it.next().ok_or(tr("--file needs an .ini file"))?));
        } else if let Some(path) = a.strip_prefix("--file=") {
            file = Some(PathBuf::from(path));
        } else {
            rest.push(a.clone());
        }
    }
    let Some((command, tail)) = rest.split_first() else { return Ok(Command::Status) };
    let no_args = |c: Command| if tail.is_empty() { Ok(c) } else { Err(fmt("'{}' takes no arguments", &[&command])) };
    if layer.is_some() && !matches!(command.as_str(), "use" | "current" | "uniforms" | "get" | "set" | "reset" | "depth") {
        return Err(fmt("--layer does not apply to '{}'", &[&command]));
    }
    if (game.is_some() || file.is_some()) && command != "preset" {
        return Err(fmt("--game and --file only apply to 'preset', not to '{}'", &[&command]));
    }
    if game.is_some() && file.is_some() {
        return Err(tr("use --game or --file, not both").into());
    }
    match command.as_str() {
        "depth" => {
            let number = |n: &str| n.parse::<u32>().ok().filter(|n| *n > 0).ok_or_else(|| fmt("invalid number: {} (starts at 1)", &[&format!("{:?}", n)]));
            let pick = match tail.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
                ["pick", "candidate", n] => DepthPick::Candidate(number(n)?),
                ["pick", "pass", n] => DepthPick::Pass(number(n)?),
                ["pick", "clear", n] => DepthPick::Clear(number(n)?),
                ["pick", "auto"] => DepthPick::Auto,
                _ => return Err(tr("usage: lrctl depth pick candidate <N> | pass <N> | auto  (--layer <game> optional)").into()),
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
                    _ => return Err(fmt("invalid position: {} (starts at 1)", &[&format!("{:?}", pos)])),
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
                _ => return Err(tr("usage: lrctl preset show | enable|disable <technique> | move <technique> <position> | macros [File.fx] [NAME=VALUE,...] | set <File.fx> <parameter> <value...> | new <name> | use <name|none>  (with --game <id> or --file <file>)").into()),
            };
            Ok(Command::Preset { action, game, file })
        }
        "status" => no_args(Command::Status),
        "on" => no_args(Command::On),
        "off" => no_args(Command::Off),
        "toggle" => no_args(Command::Toggle),
        "daemon" => no_args(Command::Daemon),
        "daemon-ensure" => no_args(Command::DaemonEnsure),
        "daemon-stop" => no_args(Command::DaemonStop),
        "keys" => no_args(Command::Keys),
        "screenshot" => no_args(Command::Screenshot),
        "setup" => match tail {
            [] => Ok(Command::Setup(true)),
            [x] if x == "--remove" => Ok(Command::Setup(false)),
            _ => Err("uso: lrctl setup [--remove]".to_string()),
        },
        "flatpak" => match tail {
            [] => Ok(Command::Flatpak(FlatpakAction::List)),
            [x] if x == "list" => Ok(Command::Flatpak(FlatpakAction::List)),
            [x, app] if x == "enable" => Ok(Command::Flatpak(FlatpakAction::Set { app: app.clone(), on: true })),
            [x, app] if x == "disable" => Ok(Command::Flatpak(FlatpakAction::Set { app: app.clone(), on: false })),
            [x, y] if x == "global" && (y == "on" || y == "off") => Ok(Command::Flatpak(FlatpakAction::Global(y == "on"))),
            _ => Err("uso: lrctl flatpak [list] | enable <app-id> | disable <app-id> | global on|off".to_string()),
        },
        "stats" => match tail {
            [x] if x == "on" => Ok(Command::Stats(true)),
            [x] if x == "off" => Ok(Command::Stats(false)),
            _ => Err("uso: lrctl stats on|off".to_string()),
        },
        "current" => no_args(Command::Current { layer }),
        "layers" => no_args(Command::Layers),
        "effects" => no_args(Command::Effects),
        "watch" => no_args(Command::Watch),
        "config" => match tail {
            [] => Ok(Command::Config { init: false, dir: None }),
            [init] if init == "init" => Ok(Command::Config { init: true, dir: None }),
            [init, dir] if init == "init" && !dir.starts_with('-') => Ok(Command::Config { init: true, dir: Some(PathBuf::from(dir)) }),
            _ => Err(tr("usage: lrctl config | lrctl config init [folder]").into()),
        },
        "use" => match tail {
            [none] if none == "none" => Ok(Command::Use { bundle: None, layer }),
            [inherit] if inherit == "inherit" => match layer {
                Some(layer) => Ok(Command::Inherit { layer }),
                None => Err(tr("'use inherit' needs --layer <game>").into()),
            },
            [dir] => Ok(Command::Use { bundle: Some(PathBuf::from(dir)), layer }),
            _ => Err(tr("usage: lrctl use <Name|bundle-directory> | lrctl use none | lrctl use inherit --layer <game>").into()),
        },
        "uniforms" => no_args(Command::Uniforms { bundle, layer }),
        "reset" => no_args(Command::Reset { bundle, layer }),
        "get" => match tail {
            [name] => Ok(Command::Get { bundle, layer, name: name.clone() }),
            _ => Err(tr("usage: lrctl get <name>").into()),
        },
        "set" => match tail {
            [name, values @ ..] if !values.is_empty() => {
                Ok(Command::Set { bundle, layer, name: name.clone(), values: values.to_vec() })
            }
            _ => Err(tr("usage: lrctl set <name> <value...>").into()),
        },
        other => Err(fmt("unknown command: {}", &[&format!("{:?}", other)])),
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
