//! `lrctl config`: onde o programa foi buscar cada coisa e o que encontrou lá.

use config::{Config, SearchPath};
use std::path::{Path, PathBuf};

/// Quantos arquivos com `extension` há nas pastas (sem entrar em subpastas além das dadas).
fn count_files(dirs: &[PathBuf], extension: &str) -> usize {
    dirs.iter()
        .filter_map(|d| std::fs::read_dir(d).ok())
        .flat_map(|entries| entries.flatten())
        .filter(|e| e.path().extension().is_some_and(|x| x == extension) && e.path().is_file())
        .count()
}

fn list_line(label: &str, paths: &[SearchPath], extension: &str) -> String {
    if paths.is_empty() {
        return format!("{label:<16}(nenhum)\n");
    }
    let mut out = String::new();
    for (i, p) in paths.iter().enumerate() {
        let dirs = p.dirs();
        let shown = format!("{}{}", p.dir.display(), if p.recursive { "/**" } else { "" });
        let found = if p.dir.is_dir() {
            format!("{} pasta(s), {} arquivo(s) .{extension}", dirs.len(), count_files(&dirs, extension))
        } else {
            "não existe".to_string()
        };
        out.push_str(&format!("{:<16}{shown}  →  {found}\n", if i == 0 { label } else { "" }));
    }
    out
}

fn exists(path: &Path) -> &'static str {
    if path.exists() { "existe" } else { "não existe" }
}

/// O resumo que `lrctl config` imprime.
pub fn describe(config: &Config, location_file: Option<&Path>) -> String {
    let mut out = String::new();
    match &config.data_dir {
        Some(dir) => out.push_str(&format!("{:<16}{}  ({})\n", "pasta de dados", dir.display(), exists(dir))),
        None => out.push_str(&format!("{:<16}(nenhuma escolhida)\n", "pasta de dados")),
    }
    if let Some(file) = location_file {
        out.push_str(&format!("{:<16}{}  ({})\n", "ponteiro", file.display(), exists(file)));
    }
    if let Some(file) = &config.config_file {
        out.push_str(&format!("{:<16}{}\n", "config.ini", file.display()));
    }
    out.push_str(&list_line("efeitos", &config.effect_paths, "fx"));
    out.push_str(&list_line("texturas", &config.texture_paths, "png"));
    match &config.presets_dir {
        Some(dir) => {
            let games = std::fs::read_dir(dir).map(|e| e.flatten().filter(|e| e.path().is_dir()).count()).unwrap_or(0);
            let found = if dir.is_dir() { format!("{games} pasta(s) de jogo") } else { "não existe".to_string() };
            out.push_str(&format!("{:<16}{}  →  {found}\n", "presets", dir.display()));
        }
        None => out.push_str(&format!("{:<16}(nenhum)\n", "presets")),
    }
    out.push_str(&format!("{:<16}{}\n", "cache", config.cache_dir.display()));
    if !config.is_configured() {
        out.push_str("\nnenhuma pasta de dados escolhida: vale o que está no clone do repositório.\nPara escolher uma: lrctl config init <pasta>\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_what_was_found() {
        let root = std::env::temp_dir().join(format!("lr-location-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Shaders/cores")).unwrap();
        std::fs::create_dir_all(root.join("Presets/RocketLeague")).unwrap();
        std::fs::write(root.join("Shaders/A.fx"), "").unwrap();
        std::fs::write(root.join("Shaders/cores/B.fx"), "").unwrap();
        std::fs::write(root.join("Shaders/ReShade.fxh"), "").unwrap();
        let env = |k: &str| (k == "LINUX_RESHADE_DATA").then(|| root.clone().into_os_string());
        let config = Config::resolve(&env);
        let text = describe(&config, None);
        assert!(text.contains("2 pasta(s), 2 arquivo(s) .fx"), "{text}");
        assert!(text.contains("1 pasta(s) de jogo"), "{text}");
        assert!(text.contains("Textures/**  →  não existe"), "{text}");
        assert!(!text.contains("nenhuma pasta de dados escolhida"), "{text}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn says_so_when_nothing_is_configured() {
        let env = |_: &str| None;
        let text = describe(&Config::resolve(&env), None);
        assert!(text.contains("lrctl config init"), "{text}");
    }
}
