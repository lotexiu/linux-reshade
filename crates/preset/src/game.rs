//! Qual preset vale para um jogo: a pasta dele em `<Presets>/<Jogo>/` e o preset global.
//!
//! ```text
//! <Presets>/preset.ini              vale para jogo sem preset próprio
//! <Presets>/<Jogo>/game.ini         preset=Noturno.ini       (um arquivo da própria pasta do jogo)
//! <Presets>/<Jogo>/Noturno.ini      os presets do jogo
//! ```
//!
//! Nada aqui escreve no disco além de [`set_game_preset`], que o `lrctl`/`lrpanel` chamam.

use ini::Ini;
use std::io;
use std::path::{Path, PathBuf};
use i18n::fmt;

pub const GAME_FILE: &str = "game.ini";
pub const GLOBAL_PRESET: &str = "preset.ini";
const PRESET_KEY: &str = "preset";
const DEPTH_KEY: &str = "depth";

/// De onde veio o preset escolhido.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// `game.ini` do jogo.
    Game,
    /// `preset.ini` global, porque o jogo não tem preset próprio.
    Global,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub path: PathBuf,
    pub source: Source,
}

/// Um id de jogo vira nome de pasta: só serve se não puder sair de `<Presets>`.
pub fn valid_game_id(game: &str) -> bool {
    !game.is_empty() && game != "." && game != ".." && !game.starts_with('.') && !game.contains(['/', '\\', '\0'])
}

/// Um arquivo de preset é só um nome de arquivo `.ini` dentro da pasta do jogo.
pub fn valid_preset_file(name: &str) -> bool {
    valid_game_id(name) && name.to_ascii_lowercase().ends_with(".ini") && name != GAME_FILE
}

pub fn game_dir(presets_dir: &Path, game: &str) -> Option<PathBuf> {
    valid_game_id(game).then(|| presets_dir.join(game))
}

/// O preset que vale para `game`: o escolhido em `game.ini` (se o arquivo existe) e, na falta, o global.
pub fn find(presets_dir: &Path, game: &str) -> Option<Found> {
    if let Some(dir) = game_dir(presets_dir, game)
        && let Ok(ini) = Ini::load(&dir.join(GAME_FILE))
        && let Some(name) = ini.get("", PRESET_KEY).filter(|n| valid_preset_file(n))
    {
        let path = dir.join(name);
        if path.is_file() {
            return Some(Found { path, source: Source::Game });
        }
    }
    let global = presets_dir.join(GLOBAL_PRESET);
    global.is_file().then_some(Found { path: global, source: Source::Global })
}

/// Escolhe o preset de um jogo (cria `game.ini` e a pasta se preciso; o resto do arquivo é preservado).
/// `None` volta a seguir o preset global.
pub fn set_game_preset(presets_dir: &Path, game: &str, preset: Option<&str>) -> io::Result<()> {
    let dir = game_dir(presets_dir, game).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, fmt("invalid game id: {}", &[&format!("{:?}", game)])))?;
    let file = dir.join(GAME_FILE);
    let mut ini = Ini::load(&file).unwrap_or_default();
    match preset {
        Some(name) if valid_preset_file(name) => ini.set("", PRESET_KEY, name),
        Some(name) => return Err(io::Error::new(io::ErrorKind::InvalidInput, fmt("invalid preset name: {}", &[&format!("{:?}", name)]))),
        None => {
            ini.remove("", PRESET_KEY);
        }
    }
    ini.save(&file)
}

/// A escolha de profundidade do jogo gravada em `game.ini` (`depth=pass:2`, `depth=candidate:1`), como texto. `None`: automática.
pub fn depth_choice(presets_dir: &Path, game: &str) -> Option<String> {
    let dir = game_dir(presets_dir, game)?;
    let ini = Ini::load(&dir.join(GAME_FILE)).ok()?;
    ini.get("", DEPTH_KEY).map(str::trim).filter(|v| !v.is_empty()).map(String::from)
}

/// Grava (ou apaga, com `None`) a escolha de profundidade do jogo. O resto do `game.ini` é preservado; sem `game.ini`
/// e sem escolha, não cria nada.
pub fn set_depth_choice(presets_dir: &Path, game: &str, choice: Option<&str>) -> io::Result<()> {
    let dir = game_dir(presets_dir, game).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, fmt("invalid game id: {}", &[&format!("{:?}", game)])))?;
    let file = dir.join(GAME_FILE);
    if choice.is_none() && !file.exists() {
        return Ok(());
    }
    let mut ini = Ini::load(&file).unwrap_or_default();
    match choice {
        Some(text) => ini.set("", DEPTH_KEY, text),
        None => {
            ini.remove("", DEPTH_KEY);
        }
    }
    std::fs::create_dir_all(&dir)?;
    ini.save(&file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Dir(PathBuf);
    impl Dir {
        fn new(name: &str) -> Self {
            let d = std::env::temp_dir().join(format!("preset-game-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&d);
            fs::create_dir_all(&d).unwrap();
            Self(d)
        }
        fn write(&self, rel: &str, text: &str) {
            let p = self.0.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_depth_choice_is_kept_in_game_ini_next_to_the_preset() {
        let d = Dir::new("depth");
        d.write("Jogo/game.ini", "preset=Noturno.ini\n");
        assert_eq!(depth_choice(&d.0, "Jogo"), None);
        set_depth_choice(&d.0, "Jogo", Some("pass:2")).unwrap();
        assert_eq!(depth_choice(&d.0, "Jogo").as_deref(), Some("pass:2"));
        d.write("Jogo/Noturno.ini", "Techniques=\n");
        assert_eq!(find(&d.0, "Jogo").unwrap().source, Source::Game, "o preset escolhido continua valendo");
        set_depth_choice(&d.0, "Jogo", None).unwrap();
        assert_eq!(depth_choice(&d.0, "Jogo"), None);
        // Sem escolha e sem game.ini: nada é criado.
        set_depth_choice(&d.0, "Novo", None).unwrap();
        assert!(!d.0.join("Novo").exists());
        assert!(set_depth_choice(&d.0, "../x", Some("pass:1")).is_err());
    }

    #[test]
    fn the_game_ini_picks_a_preset_in_the_games_folder() {
        let d = Dir::new("pick");
        d.write("RocketLeague/game.ini", "preset=Noturno.ini\n");
        d.write("RocketLeague/Noturno.ini", "Techniques=\n");
        d.write("preset.ini", "Techniques=\n");
        let found = find(&d.0, "RocketLeague").unwrap();
        assert_eq!(found, Found { path: d.0.join("RocketLeague/Noturno.ini"), source: Source::Game });
    }

    #[test]
    fn without_a_game_preset_the_global_one_applies() {
        let d = Dir::new("global");
        d.write("preset.ini", "Techniques=\n");
        d.write("vkcube/game.ini", "; sem preset\n");
        for game in ["vkcube", "OutroJogo"] {
            assert_eq!(find(&d.0, game), Some(Found { path: d.0.join("preset.ini"), source: Source::Global }), "{game}");
        }
    }

    #[test]
    fn a_missing_file_falls_back_and_no_preset_at_all_is_none() {
        let d = Dir::new("missing");
        d.write("Jogo/game.ini", "preset=Sumiu.ini\n");
        assert_eq!(find(&d.0, "Jogo"), None);
        d.write("preset.ini", "");
        assert_eq!(find(&d.0, "Jogo").unwrap().source, Source::Global);
    }

    #[test]
    fn ids_and_file_names_cannot_leave_the_presets_folder() {
        let d = Dir::new("escape");
        d.write("preset.ini", "");
        d.write("Jogo/game.ini", "preset=../preset.ini\n");
        assert_eq!(find(&d.0, "Jogo").unwrap().source, Source::Global, "o caminho com .. é ignorado");
        for bad in ["", ".", "..", "../x", "a/b", ".oculto", "a\\b"] {
            assert!(!valid_game_id(bad), "{bad:?}");
            assert!(find(&d.0, bad).is_none_or(|f| f.source == Source::Global));
        }
        assert!(valid_game_id("Idols of Ash"));
        assert!(valid_preset_file("Cores vivas.ini"));
        assert!(!valid_preset_file("game.ini") && !valid_preset_file("x.txt") && !valid_preset_file("../x.ini"));
        assert!(set_game_preset(&d.0, "../x", Some("a.ini")).is_err());
        assert!(set_game_preset(&d.0, "Jogo", Some("../a.ini")).is_err());
    }

    #[test]
    fn choosing_a_preset_keeps_the_rest_of_game_ini() {
        let d = Dir::new("set");
        d.write("Jogo/game.ini", "; meu jogo\nid=Jogo.exe\n");
        set_game_preset(&d.0, "Jogo", Some("A.ini")).unwrap();
        let text = fs::read_to_string(d.0.join("Jogo/game.ini")).unwrap();
        assert_eq!(text, "; meu jogo\nid=Jogo.exe\npreset=A.ini\n");
        set_game_preset(&d.0, "Jogo", None).unwrap();
        assert_eq!(fs::read_to_string(d.0.join("Jogo/game.ini")).unwrap(), "; meu jogo\nid=Jogo.exe\n");
        // Jogo novo: cria a pasta.
        set_game_preset(&d.0, "Novo", Some("B.ini")).unwrap();
        assert!(d.0.join("Novo/game.ini").is_file());
    }
}
