//! `lrctl preset ...`: edita o preset de um jogo (ou um arquivo). A lógica vive em `control_core::presets`.

use crate::cli::PresetAction;
use control_core::compiler::Compiler;
use control_core::presets::{self, Target};
use control_core::params;
use preset::Preset;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use control_core::i18n::{fmt, tr};

fn fail(message: impl std::fmt::Display) -> ExitCode {
    eprintln!("lrctl: {message}");
    ExitCode::FAILURE
}

fn target(game: Option<String>, file: Option<PathBuf>) -> Result<Target, String> {
    match (game, file) {
        (Some(id), None) => Ok(Target::Game(id)),
        (None, Some(path)) => Ok(Target::File(path)),
        _ => Err(tr("say which preset: --game <id> (the one that applies to the game) or --file <file.ini>").into()),
    }
}

fn presets_dir() -> Result<PathBuf, String> {
    config::Config::load().presets_dir.ok_or_else(|| tr("no data folder chosen: run 'lrctl config init'").to_string())
}

fn show(preset: &Preset, path: &Path) {
    println!("{}", path.display());
    let rows = presets::rows(preset);
    if rows.is_empty() {
        println!("{}", tr("  (no techniques)"));
    }
    for (n, row) in rows.iter().enumerate() {
        println!("  {:>2}. [{}] {}", n + 1, if row.enabled { "x" } else { " " }, row.id);
    }
    let macros = preset.macros();
    if !macros.is_empty() {
        println!("  macros: {}", macros.iter().map(|(n, v)| format!("{n}={v}")).collect::<Vec<_>>().join(", "));
    }
}

pub fn run(action: PresetAction, game: Option<String>, file: Option<PathBuf>) -> ExitCode {
    match run_inner(action, game, file) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(e),
    }
}

fn run_inner(action: PresetAction, game: Option<String>, file: Option<PathBuf>) -> Result<(), String> {
    // Escolher/criar o preset de um jogo não edita um arquivo existente: trata antes.
    match &action {
        PresetAction::New(name) => {
            let id = game.ok_or(tr("'preset new' needs --game <id>"))?;
            let path = presets::new_for_game(&presets_dir()?, &id, name)?;
            println!("{}", fmt("created and chosen for {}: {}", &[&id, &path.display()]));
            return Ok(());
        }
        PresetAction::Use(name) => {
            let id = game.ok_or(tr("'preset use' needs --game <id>"))?;
            let dir = presets_dir()?;
            presets::use_preset(&dir, &id, name.as_deref())?;
            match presets::describe_source(&dir, &id) {
                Some((path, _)) => println!("{id} usa {}", path.display()),
                None => println!("{}", fmt("{} has no preset", &[&id])),
            }
            return Ok(());
        }
        _ => {}
    }

    let target = target(game, file)?;
    let path = match &target {
        Target::Game(_) => presets::resolve(&presets_dir()?, &target)?,
        Target::File(p) => p.clone(),
    };
    let mut preset = presets::load(&path)?;
    match action {
        PresetAction::Show => {
            show(&preset, &path);
            return Ok(());
        }
        PresetAction::Enable(t) => {
            let id = presets::parse_technique(&preset, &t);
            preset.set_enabled(&id, true);
        }
        PresetAction::Disable(t) => {
            let id = presets::parse_technique(&preset, &t);
            if !presets::rows(&preset).iter().any(|r| r.id.matches(&id)) {
                return Err(fmt("the preset does not know the technique {}", &[&id]));
            }
            preset.set_enabled(&id, false);
        }
        PresetAction::Move(t, position) => {
            let id = presets::parse_technique(&preset, &t);
            if !preset.move_technique(&id, position - 1) {
                return Err(fmt("the preset does not know the technique {}", &[&id]));
            }
        }
        PresetAction::Set { file, parameter, values } => {
            let values = params::parse_values(&values).map_err(|e| e.to_string())?;
            let compiler = Compiler::from_env()?;
            for warning in presets::set_uniform(&mut preset, &compiler, &file, &parameter, &values)? {
                eprintln!("aviso: {warning}");
            }
        }
        PresetAction::Macros { file, text } => {
            let current = |p: &Preset| match &file {
                Some(f) => p.effect_macros(f),
                None => p.macros(),
            };
            match text {
                None => {
                    let list = panel::macros_text(&current(&preset));
                    println!("{}", if list.is_empty() { tr("(no definitions)").to_string() } else { list });
                    return Ok(());
                }
                Some(text) => {
                    let parsed = preset::parse_macros(&text);
                    if let Some(bad) = parsed.iter().find(|(n, v)| !control_core::compiler::valid_define(n, v)) {
                        return Err(fmt("invalid definition: {}={} (the name must be an identifier)", &[&bad.0, &format!("{:?}", bad.1)]));
                    }
                    match &file {
                        Some(f) => preset.set_effect_macros(f, &parsed),
                        None => preset.set_macros(&parsed),
                    }
                }
            }
        }
        PresetAction::New(_) | PresetAction::Use(_) => unreachable!("tratados acima"),
    }
    preset.save(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    show(&preset, &path);
    Ok(())
}
