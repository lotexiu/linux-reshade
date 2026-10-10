//! Editar presets: o que `lrctl preset` e, depois, a interface fazem. Só mexe em arquivos de preset (quem
//! os aplica nos jogos é o vigia, que relê o arquivo quando ele muda).

use crate::compiler::{Compiler, valid_name};
use crate::params;
use fx_bundle::{Bundle, uniforms};
use preset::game::{self, Source};
use preset::{Preset, TechId};
use std::path::{Path, PathBuf};

/// Qual preset editar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// O que vale para este jogo (o dele, ou o global se ele não tem um).
    Game(String),
    /// Um arquivo qualquer (criado se não existir).
    File(PathBuf),
}

/// O arquivo de preset de um alvo.
pub fn resolve(presets_dir: &Path, target: &Target) -> Result<PathBuf, String> {
    match target {
        Target::File(path) => Ok(path.clone()),
        Target::Game(id) => {
            if !game::valid_game_id(id) {
                return Err(format!("id de jogo inválido: {id:?}"));
            }
            game::find(presets_dir, id).map(|f| f.path).ok_or_else(|| {
                format!("{id}: não há preset (nem o dele, nem {}/{}); crie um com 'lrctl preset new <nome> --game {id}'", presets_dir.display(), game::GLOBAL_PRESET)
            })
        }
    }
}

/// Lê o preset; um arquivo que não existe é um preset vazio (será criado ao gravar).
pub fn load(path: &Path) -> Result<Preset, String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Preset::parse(&String::from_utf8_lossy(&bytes))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Preset::new()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: TechId,
    pub enabled: bool,
}

/// Todas as técnicas que o preset conhece, na ordem de execução, com a flag de ligada.
pub fn rows(preset: &Preset) -> Vec<Row> {
    let mut known = preset.sorting();
    for t in preset.techniques() {
        if !known.iter().any(|k| k.matches(&t)) {
            known.push(t);
        }
    }
    let on = preset.techniques();
    known.into_iter().map(|id| Row { enabled: on.iter().any(|t| t.matches(&id)), id }).collect()
}

/// Uma técnica dada pelo usuário: `Nome@Arquivo.fx`, ou só `Nome` (que vale como `Nome@Nome.fx` quando é nova).
pub fn parse_technique(preset: &Preset, text: &str) -> TechId {
    let id = TechId::parse(text);
    if id.file.is_some() {
        return id;
    }
    // Já conhecida pelo nome: usa o arquivo que o preset tem.
    if let Some(known) = rows(preset).into_iter().find(|r| r.id.name == id.name) {
        return known.id;
    }
    TechId { file: Some(format!("{}.fx", id.name)), ..id }
}

/// Cria `<Presets>/<jogo>/<nome>.ini` copiando o preset que vale para o jogo (ou vazio) e o escolhe em `game.ini`.
pub fn new_for_game(presets_dir: &Path, game_id: &str, name: &str) -> Result<PathBuf, String> {
    let file = if name.to_ascii_lowercase().ends_with(".ini") { name.to_string() } else { format!("{name}.ini") };
    if !game::valid_preset_file(&file) {
        return Err(format!("nome de preset inválido: {name:?}"));
    }
    let dir = game::game_dir(presets_dir, game_id).ok_or_else(|| format!("id de jogo inválido: {game_id:?}"))?;
    let path = dir.join(&file);
    if path.exists() {
        return Err(format!("{} já existe", path.display()));
    }
    let text = match game::find(presets_dir, game_id) {
        Some(found) => std::fs::read(&found.path).map_err(|e| format!("{}: {e}", found.path.display()))?,
        None => b"Techniques=\n".to_vec(),
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    game::set_game_preset(presets_dir, game_id, Some(&file)).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Escolhe o preset do jogo (`Some(arquivo)`), ou volta a seguir o global (`None`).
pub fn use_preset(presets_dir: &Path, game_id: &str, name: Option<&str>) -> Result<(), String> {
    let file = name.map(|n| if n.to_ascii_lowercase().ends_with(".ini") { n.to_string() } else { format!("{n}.ini") });
    if let Some(file) = &file {
        let path = game::game_dir(presets_dir, game_id).ok_or_else(|| format!("id de jogo inválido: {game_id:?}"))?.join(file);
        if !path.is_file() {
            return Err(format!("{} não existe", path.display()));
        }
    }
    game::set_game_preset(presets_dir, game_id, file.as_deref()).map_err(|e| e.to_string())
}

/// De onde veio o preset de um jogo, para mostrar ao usuário.
pub fn describe_source(presets_dir: &Path, game_id: &str) -> Option<(PathBuf, Source)> {
    game::find(presets_dir, game_id).map(|f| (f.path, f.source))
}

/// Grava o valor de um uniform no preset, conferindo nome, tipo e número de componentes contra o bundle do
/// efeito (compilado agora se ainda não há um no cache) e avisando de valor fora da faixa sugerida.
pub fn set_uniform(preset: &mut Preset, compiler: &Compiler, file: &str, uniform: &str, values: &[f64]) -> Result<Vec<String>, String> {
    let name = file.strip_suffix(".fx").unwrap_or(file);
    if !valid_name(name) {
        return Err(format!("nome de efeito inválido: {file:?}"));
    }
    let mut warnings = Vec::new();
    let dir = match compiler.any_bundle(name) {
        Some(dir) => dir,
        None => compiler.ensure(name, crate::compiler::DEFAULT_SIZE.0, crate::compiler::DEFAULT_SIZE.1).map_err(|e| e.to_string())?.dir_owned(),
    };
    let bundle = Bundle::load(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let u = uniforms::find(&bundle.manifest, uniform).ok_or_else(|| {
        let known: Vec<_> = bundle.manifest.uniforms.iter().filter(|u| !uniforms::is_runtime_source(u)).map(|u| u.name.as_str()).collect();
        format!("{file} não tem o parâmetro {uniform:?} (existem: {})", known.join(", "))
    })?;
    uniforms::encode(u, values).map_err(|e| e.to_string())?;
    warnings.extend(params::range_warnings(u, values));
    preset.set_value(&format!("{name}.fx"), uniform, &params::fmt_values(values));
    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::testing::fake_with_bundles;
    use std::fs;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lr-presets-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn rows_list_every_known_technique_in_order_with_their_state() {
        let p = Preset::parse("Techniques=B@b.fx,D@d.fx\nTechniqueSorting=A@a.fx,B@b.fx,C@c.fx\n");
        let r: Vec<_> = rows(&p).into_iter().map(|r| (r.id.to_string(), r.enabled)).collect();
        assert_eq!(r, [("A@a.fx".into(), false), ("B@b.fx".into(), true), ("C@c.fx".into(), false), ("D@d.fx".into(), true)]);
    }

    #[test]
    fn a_bare_technique_name_uses_the_known_file_or_assumes_its_own() {
        let p = Preset::parse("Techniques=Nightvision@NightVision.fx\n");
        assert_eq!(parse_technique(&p, "Nightvision").to_string(), "Nightvision@NightVision.fx");
        assert_eq!(parse_technique(&p, "Sepia").to_string(), "Sepia@Sepia.fx");
        assert_eq!(parse_technique(&p, "X@y.fx").to_string(), "X@y.fx");
    }

    #[test]
    fn the_game_target_finds_its_preset_or_says_how_to_create_one() {
        let d = dir("resolve");
        let err = resolve(&d, &Target::Game("Jogo".into())).unwrap_err();
        assert!(err.contains("lrctl preset new"), "{err}");
        fs::write(d.join("preset.ini"), "Techniques=\n").unwrap();
        assert_eq!(resolve(&d, &Target::Game("Jogo".into())).unwrap(), d.join("preset.ini"));
        assert!(resolve(&d, &Target::Game("../x".into())).is_err());
        assert_eq!(resolve(&d, &Target::File("/x/y.ini".into())).unwrap(), PathBuf::from("/x/y.ini"));
    }

    #[test]
    fn a_new_game_preset_copies_the_current_one_and_becomes_the_choice() {
        let d = dir("new");
        fs::write(d.join("preset.ini"), "; global\nTechniques=Sepia@Sepia.fx\n").unwrap();
        let path = new_for_game(&d, "Jogo", "Noturno").unwrap();
        assert_eq!(path, d.join("Jogo/Noturno.ini"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "; global\nTechniques=Sepia@Sepia.fx\n", "herdou o atual, comentários e tudo");
        assert_eq!(game::find(&d, "Jogo").unwrap().path, path);
        assert!(new_for_game(&d, "Jogo", "Noturno").is_err(), "não sobrescreve");
        assert!(new_for_game(&d, "Jogo", "../x").is_err());
        // Sem nenhum preset para herdar, nasce vazio.
        let empty = dir("new-empty");
        let path = new_for_game(&empty, "Outro", "A.ini").unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), "Techniques=\n");
    }

    #[test]
    fn using_a_preset_needs_it_to_exist_and_none_goes_back_to_the_global() {
        let d = dir("use");
        fs::write(d.join("preset.ini"), "").unwrap();
        assert!(use_preset(&d, "Jogo", Some("Nada")).is_err());
        new_for_game(&d, "Jogo", "A").unwrap();
        new_for_game(&d, "Jogo", "B").unwrap();
        use_preset(&d, "Jogo", Some("A")).unwrap();
        assert_eq!(game::find(&d, "Jogo").unwrap().path, d.join("Jogo/A.ini"));
        use_preset(&d, "Jogo", None).unwrap();
        assert_eq!(game::find(&d, "Jogo").unwrap().source, Source::Global);
    }

    #[test]
    fn setting_a_uniform_checks_it_against_the_bundle_when_there_is_one() {
        let f = fake_with_bundles("presets-set");
        let mut p = Preset::parse("Techniques=Sepia@Sepia.fx\n"); // o Sépia ainda não está compilado: o set compila
        assert!(set_uniform(&mut p, &f.compiler, "Sepia.fx", "Strength", &[0.25]).unwrap().is_empty());
        assert_eq!(p.value("Sepia.fx", "Strength"), Some("0.25"));
        let warnings = set_uniform(&mut p, &f.compiler, "Sepia", "Strength", &[3.0]).unwrap();
        assert_eq!(warnings.len(), 1, "fora da faixa sugerida: avisa e grava");
        assert!(set_uniform(&mut p, &f.compiler, "Sepia.fx", "Nada", &[1.0]).unwrap_err().contains("Strength"));
        assert!(set_uniform(&mut p, &f.compiler, "Sepia.fx", "Strength", &[1.0, 2.0]).is_err(), "componentes demais");
        assert!(set_uniform(&mut p, &f.compiler, "../x.fx", "A", &[1.0]).is_err());
        // Efeito ainda não compilado: compila para conferir; sem o parâmetro, recusa.
        let err = set_uniform(&mut p, &f.compiler, "Invert.fx", "Qualquer", &[1.0]).unwrap_err();
        assert!(err.contains("não tem o parâmetro"), "{err}");
        assert!(f.compiler.any_bundle("Invert").is_some());
        assert!(set_uniform(&mut p, &f.compiler, "Inexistente.fx", "A", &[1.0]).is_err());
        assert_eq!(p.value("Invert.fx", "Qualquer"), None, "nada foi gravado");
    }
}
