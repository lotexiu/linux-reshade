//! A configuração de onde moram os dados do usuário (ver `docs/presets.md`).
//!
//! ```text
//! ~/.config/linux-reshade/location.ini   data_dir=/mnt/D/reshade       (o único arquivo fora da pasta de dados)
//! <data_dir>/config.ini                  [GENERAL] EffectSearchPaths=./Shaders/**  ...
//! <data_dir>/Shaders  Textures  Presets
//! ```
//!
//! Pasta de dados: `LINUX_RESHADE_DATA` > `location.ini` > a padrão (`~/.local/share/linux-reshade`), que só
//! vale se já existir (`lrctl config init` a cria). Ordem de decisão de cada pasta dentro dela: variável de
//! ambiente > `config.ini` > padrão dentro da pasta de dados.
//! Sem pasta de dados e sem variáveis, nada é configurado (os caminhos vêm vazios) e quem chama decide o
//! que fazer (o `lrctl` usa os efeitos do clone do repositório). Caminhos relativos valem em relação à pasta
//! do `config.ini`, e não do jogo: a pasta de dados pode ser copiada para outra máquina.

use ini::Ini;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

/// Profundidade máxima da busca recursiva (`**`): protege de árvores gigantes e de laços.
const MAX_DEPTH: usize = 8;

/// Uma entrada de lista de busca: a pasta e se as subpastas também entram (`pasta/**`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchPath {
    pub dir: PathBuf,
    pub recursive: bool,
}

impl SearchPath {
    /// Só esta pasta, sem subpastas.
    pub fn flat(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into(), recursive: false }
    }

    /// Interpreta uma entrada do `config.ini`: `~/` vira a home, relativo é medido a partir de `base`, e um
    /// `/**` ou `\**` no fim liga a busca recursiva.
    pub fn parse(text: &str, base: &Path, home: Option<&Path>) -> Self {
        let text = text.trim();
        let (text, recursive) = match text.strip_suffix("**") {
            Some(rest) => (rest.trim_end_matches(['/', '\\']), true),
            None => (text, false),
        };
        let text = if text.is_empty() { "." } else { text };
        let path = match (text.strip_prefix("~/"), home) {
            (Some(rest), Some(home)) => home.join(rest),
            _ if text == "~" && home.is_some() => home.unwrap().to_path_buf(),
            _ => PathBuf::from(text),
        };
        let dir = if path.is_absolute() { path } else { base.join(path) };
        Self { dir: normalize(&dir), recursive }
    }

    /// As pastas que esta entrada cobre: ela mesma e, se recursiva, as subpastas (ordem alfabética, sem as
    /// ocultas como `.git`, sem seguir links simbólicos).
    pub fn dirs(&self) -> Vec<PathBuf> {
        let mut out = vec![self.dir.clone()];
        if self.recursive {
            collect(&self.dir, 1, &mut out);
        }
        out
    }
}

fn collect(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut subdirs: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()) && !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| e.path())
        .collect();
    subdirs.sort();
    for sub in subdirs {
        out.push(sub.clone());
        collect(&sub, depth + 1, out);
    }
}

/// Tira os `.` e `..` de um caminho sem tocar no disco.
fn normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir if matches!(out.components().next_back(), Some(Component::Normal(_))) => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Todas as pastas de uma lista, na ordem, sem repetir.
pub fn expand(paths: &[SearchPath]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for p in paths {
        for d in p.dirs() {
            if !out.contains(&d) {
                out.push(d);
            }
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// A pasta de dados, se alguma foi escolhida.
    pub data_dir: Option<PathBuf>,
    /// O `config.ini` lido (se existia).
    pub config_file: Option<PathBuf>,
    pub effect_paths: Vec<SearchPath>,
    pub texture_paths: Vec<SearchPath>,
    /// Onde ficam `preset.ini` e as pastas de cada jogo.
    pub presets_dir: Option<PathBuf>,
    /// Definições de pré-processador valendo para todos os efeitos (`PreprocessorDefinitions` do `config.ini`, como no
    /// `ReShade.ini`): `NOME=VALOR,OUTRO`. Os presets e os efeitos as sobrepõem.
    pub global_macros: String,
    /// Onde ficam os bundles compilados.
    pub cache_dir: PathBuf,
}

/// Como o ambiente é lido: injetado, para os testes não dependerem da máquina.
pub type Env<'a> = &'a dyn Fn(&str) -> Option<OsString>;

pub const LOCATION_DIR: &str = "linux-reshade";

fn home(env: Env) -> Option<PathBuf> {
    env("HOME").map(PathBuf::from)
}

/// O `location.ini`: o ponteiro para a pasta de dados.
pub fn location_file(env: Env) -> Option<PathBuf> {
    let base = env("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| home(env).map(|h| h.join(".config")))?;
    Some(base.join(LOCATION_DIR).join("location.ini"))
}

/// A pasta de dados padrão, para quem nunca escolheu outra: `$XDG_DATA_HOME/linux-reshade`, ou
/// `~/.local/share/linux-reshade`.
pub fn default_data_dir(env: Env) -> Option<PathBuf> {
    let base = env("XDG_DATA_HOME").map(PathBuf::from).or_else(|| home(env).map(|h| h.join(".local/share")))?;
    Some(base.join(LOCATION_DIR))
}

fn default_cache(env: Env) -> PathBuf {
    let base = env("XDG_CACHE_HOME").map(PathBuf::from).or_else(|| home(env).map(|h| h.join(".cache"))).unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join(LOCATION_DIR).join("bundles")
}

/// Lê uma lista de caminhos de uma variável de ambiente (separada por `:`).
fn env_paths(env: Env, var: &str, home: Option<&Path>) -> Option<Vec<SearchPath>> {
    let value = env(var)?;
    let base = Path::new("");
    Some(std::env::split_paths(&value).map(|p| SearchPath::parse(&p.to_string_lossy(), base, home)).collect())
}

impl Config {
    /// A configuração do processo atual (variáveis de ambiente reais).
    pub fn load() -> Self {
        Self::resolve(&|name| std::env::var_os(name))
    }

    /// Variáveis: `LINUX_RESHADE_DATA` (pasta de dados), `LINUX_RESHADE_CONFIG` (outro `config.ini`),
    /// `LINUX_RESHADE_EFFECTS`, `LINUX_RESHADE_TEXTURES`, `LINUX_RESHADE_PRESETS`, `LINUX_RESHADE_CACHE`.
    pub fn resolve(env: Env) -> Self {
        let home = home(env);
        let home = home.as_deref();

        let data_dir = env("LINUX_RESHADE_DATA").map(PathBuf::from).or_else(|| {
            let location = location_file(env)?;
            let ini = Ini::load(&location).ok()?;
            let value = ini.get("", "data_dir").filter(|v| !v.is_empty())?;
            Some(SearchPath::parse(value, location.parent()?, home).dir)
        });
        // Sem escolha nenhuma, a padrão vale desde que já tenha sido criada: um clone recém-baixado, sem
        // `init`, continua usando os efeitos do repositório.
        let data_dir = data_dir.or_else(|| default_data_dir(env).filter(|d| d.is_dir()));

        let config_file = env("LINUX_RESHADE_CONFIG").map(PathBuf::from).or_else(|| data_dir.as_ref().map(|d| d.join("config.ini")));
        let ini = config_file.as_deref().and_then(|f| Ini::load(f).ok());
        // O que for relativo no config.ini é medido a partir da pasta dele.
        let base = config_file.as_deref().and_then(Path::parent).map(Path::to_path_buf).or_else(|| data_dir.clone());
        let configured = config_file.as_ref().zip(ini.as_ref()).map(|(f, _)| f.clone());

        let list = |key: &str, default: &str| -> Vec<SearchPath> {
            let Some(base) = &base else { return Vec::new() };
            let items = ini.as_ref().map(|i| i.get_list("GENERAL", key)).filter(|l| !l.is_empty()).unwrap_or_else(|| vec![default.to_string()]);
            items.iter().map(|s| SearchPath::parse(s, base, home)).collect()
        };
        let single = |key: &str, default: &str| -> Option<PathBuf> {
            let base = base.as_ref()?;
            let value = ini.as_ref().and_then(|i| i.get("GENERAL", key)).filter(|v| !v.is_empty()).unwrap_or(default);
            Some(SearchPath::parse(value, base, home).dir)
        };

        let effect_paths = env_paths(env, "LINUX_RESHADE_EFFECTS", home).unwrap_or_else(|| list("EffectSearchPaths", "./Shaders/**"));
        let texture_paths = env_paths(env, "LINUX_RESHADE_TEXTURES", home).unwrap_or_else(|| list("TextureSearchPaths", "./Textures/**"));
        let presets_dir = env("LINUX_RESHADE_PRESETS").map(PathBuf::from).or_else(|| single("PresetsPath", "./Presets"));
        let cache_dir = env("LINUX_RESHADE_CACHE")
            .map(PathBuf::from)
            .or_else(|| ini.as_ref().and_then(|i| i.get("GENERAL", "IntermediateCachePath")).filter(|v| !v.is_empty()).zip(base.as_ref()).map(|(v, b)| SearchPath::parse(v, b, home).dir))
            .unwrap_or_else(|| default_cache(env));

        let global_macros = env("LINUX_RESHADE_DEFINES").map(|v| v.to_string_lossy().into_owned()).or_else(|| ini.as_ref().and_then(|i| i.get("GENERAL", "PreprocessorDefinitions")).map(String::from)).unwrap_or_default();
        Self { data_dir, config_file: configured, effect_paths, texture_paths, presets_dir, global_macros, cache_dir }
    }

    /// Nada foi configurado: sem pasta de dados e sem lista de efeitos por variável.
    pub fn is_configured(&self) -> bool {
        !self.effect_paths.is_empty()
    }

    /// A pasta de um jogo dentro dos presets (pode não existir ainda).
    pub fn game_dir(&self, game: &str) -> Option<PathBuf> {
        Some(self.presets_dir.as_ref()?.join(game))
    }

    /// As pastas de efeitos na ordem de busca: a do jogo (`<jogo>/Shaders`, se existir) vem antes das globais,
    /// e o primeiro arquivo com um nome vence.
    pub fn effect_dirs(&self, game: Option<&str>) -> Vec<PathBuf> {
        self.layered(game, "Shaders", &self.effect_paths)
    }

    pub fn texture_dirs(&self, game: Option<&str>) -> Vec<PathBuf> {
        self.layered(game, "Textures", &self.texture_paths)
    }

    fn layered(&self, game: Option<&str>, folder: &str, global: &[SearchPath]) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        if let Some(dir) = game.and_then(|g| self.game_dir(g)).map(|d| d.join(folder)).filter(|d| d.is_dir()) {
            paths.push(SearchPath { dir, recursive: true });
        }
        paths.extend(global.iter().cloned());
        expand(&paths)
    }
}

/// O que `init` criou.
#[derive(Debug, Default)]
pub struct Initialized {
    pub created: Vec<PathBuf>,
    pub location: Option<PathBuf>,
}

/// Prepara uma pasta de dados: cria as pastas, um `config.ini` com os padrões (se não existir) e aponta o
/// `location.ini` para ela. Não apaga nem sobrescreve nada que já exista, a não ser o ponteiro.
pub fn init(data_dir: &Path, env: Env) -> io::Result<Initialized> {
    let mut done = Initialized::default();
    for sub in ["", "Shaders", "Textures", "Presets"] {
        let dir = data_dir.join(sub);
        if !dir.is_dir() {
            std::fs::create_dir_all(&dir)?;
            done.created.push(dir);
        }
    }
    let file = data_dir.join("config.ini");
    if !file.exists() {
        let mut ini = Ini::new();
        ini.set("GENERAL", "EffectSearchPaths", "./Shaders/**");
        ini.set("GENERAL", "TextureSearchPaths", "./Textures/**");
        ini.set("GENERAL", "PresetsPath", "./Presets");
        ini.save(&file)?;
        done.created.push(file);
    }
    if let Some(location) = location_file(env) {
        let mut ini = Ini::load(&location).unwrap_or_default();
        ini.set("", "data_dir", &data_dir.to_string_lossy());
        ini.save(&location)?;
        done.location = Some(location);
    }
    Ok(done)
}

#[cfg(test)]
mod tests;
