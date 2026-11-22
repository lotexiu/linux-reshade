//! Compilação de efeitos sob demanda: chama o `fx2spv` para produzir o bundle de um efeito numa
//! resolução e guarda o resultado num cache.
//!
//! Um bundle só serve à resolução para a qual foi compilado, então o diretório do cache leva o tamanho
//! no nome (`Sepia-1920x1080`). A escolha que a layer recebe é o *modelo* `<cache>/Sepia-{W}x{H}`.

use crate::util::modified;
use config::{Config, SearchPath};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;
use i18n::{fmt, tr};

mod build;
mod cache;
mod search;
#[cfg(test)]
pub(crate) mod testing;
#[cfg(test)]
mod tests;
pub use cache::is_bundle_name;

use build::*;
#[cfg(test)]
use cache::*;
use search::*;

pub struct Compiler {
    /// O executável do `fx2spv`.
    pub fx2spv: PathBuf,
    /// Onde procurar `Nome.fx`, em ordem. As pastas `**` são expandidas a cada uso: um shader novo aparece
    /// sem reiniciar o programa.
    pub effects: Vec<SearchPath>,
    /// Diretórios de `#include` (por exemplo, o do `ReShade.fxh`).
    pub includes: Vec<SearchPath>,
    /// Onde procurar as imagens das texturas que vêm de arquivo (`source = "lut.png"`), em ordem.
    pub textures: Vec<SearchPath>,
    /// Onde ficam os bundles compilados.
    pub cache: PathBuf,
}

#[derive(Debug)]
pub enum CompileError {
    /// O nome tem caracteres que não pertencem a um nome de efeito.
    BadName(String),
    UnknownEffect { name: String, searched: Vec<PathBuf> },
    /// O `fx2spv` terminou com erro; `output` é o que ele escreveu.
    Tool { status: String, output: String },
    /// Uma textura de arquivo não foi achada ou não pôde ser lida.
    Texture(String),
    Io(String),
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompileError::BadName(n) => write!(f, "{}", fmt("{} is not a valid effect name (letters, digits, _ - .)", &[&format!("{n:?}")])),
            CompileError::UnknownEffect { name, searched } => {
                let dirs: Vec<_> = searched.iter().map(|d| d.display().to_string()).collect();
                write!(f, "{}", fmt("could not find the effect {} (searched in: {})", &[&format!("{name:?}"), &dirs.join(", ")]))
            }
            CompileError::Tool { status, output } => write!(f, "{}", fmt("fx2spv failed ({}):\n{}", &[status, &output.trim_end()])),
            CompileError::Texture(why) => write!(f, "{why}"),
            CompileError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for CompileError {}

/// O que `ensure` fez.
#[derive(Debug, PartialEq, Eq)]
pub enum Ensured {
    /// Já havia um bundle atual.
    Cached(PathBuf),
    /// Foi (re)compilado agora.
    Compiled(PathBuf),
}

impl Ensured {
    pub fn dir_owned(self) -> PathBuf {
        match self {
            Ensured::Cached(d) | Ensured::Compiled(d) => d,
        }
    }

    #[cfg(test)]
    pub fn dir(&self) -> &Path {
        match self {
            Ensured::Cached(d) | Ensured::Compiled(d) => d,
        }
    }
}

/// Definições de pré-processador (`NOME`, `VALOR`) com que um efeito é compilado.
pub type Defines = Vec<(String, String)>;

/// A chave de um efeito compilado com certas definições: o próprio nome sem definições, e `Nome.m<hash>` com elas
/// (o hash é das definições em ordem alfabética, então a mesma combinação cai no mesmo bundle). Serve de nome de
/// bundle no cache e de modelo de caminho para a layer, que não precisa saber o que as definições significam.
pub fn variant_key(name: &str, defines: &[(String, String)]) -> String {
    if defines.is_empty() {
        return name.to_string();
    }
    let mut sorted: Vec<String> = defines.iter().map(|(n, v)| format!("{n}={v}")).collect();
    sorted.sort();
    let hash = sorted.join("\u{1}").bytes().fold(0x811C_9DC5u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193));
    format!("{name}.m{hash:08x}")
}

/// O nome do efeito de uma chave de [`variant_key`] (tira o `.m<hash>`, se houver).
pub fn base_name(key: &str) -> &str {
    match key.rsplit_once(".m") {
        Some((base, hash)) if hash.len() == 8 && hash.bytes().all(|b| b.is_ascii_hexdigit()) && !base.is_empty() => base,
        _ => key,
    }
}

/// Resolução usada para compilar um bundle só para ler os controles quando não há jogo aberto.
pub const DEFAULT_SIZE: (u32, u32) = (1920, 1080);

/// Um diretório do cache, como o `prune` o vê.
pub(crate) struct CacheEntry {
    pub path: PathBuf,
    pub file_name: String,
    pub modified: SystemTime,
}

/// Um nome de efeito é só letras, números, `_`, `-` e `.` (e não começa com ponto). Isso impede que
/// `../x` ou um caminho absoluto escapem do cache ao virarem um diretório.
/// Uma definição que pode ir na linha de comando do compilador: nome de identificador e valor sem quebra de linha.
pub fn valid_define(name: &str, value: &str) -> bool {
    !name.is_empty()
        && name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !value.contains(['\n', '\r', '\0'])
}

/// Um nome de efeito: letras, números, `_`, `-`, `.` e espaço (os LUTs `Nome MLUT.fx` têm espaços), `(`, `)` e `+`. Nada de `/`, `\\` nem
/// ponto no começo: o nome vira um diretório do cache e não pode escapar dele.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.starts_with(' ')
        && !name.ends_with(' ')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ' ' | '(' | ')' | '+'))
}

/// Onde o compilador anota de quais arquivos vieram as imagens do bundle (um caminho por linha).
const IMAGE_SOURCES: &str = "images/sources.txt";

/// O bundle anota quais correções de código (`patches`) entraram nele.
const PATCHES_MARKER: &str = "patches";

/// Marca o bundle cujo SPIR-V já passou pelo otimizador.
const OPTIMIZED_MARKER: &str = "optimized";

/// O que o marcador guarda: muda quando a otimização muda (ex.: `--loop-unroll`), e os bundles feitos pela anterior são refeitos.
const OPTIMIZE_VERSION: &str = "2-loop-unroll";

impl Compiler {
    /// A configuração do processo atual: ver [`from_config`](Self::from_config).
    pub fn from_env() -> Result<Self, String> {
        Self::from_config(&Config::load())
    }

    /// Monta o compilador a partir da configuração do usuário (`docs/usuario/presets.md`). Sem pasta de dados nem
    /// lista de efeitos, usa os padrões de um clone do repositório (`target/release/lrctl` dentro dele).
    /// Variáveis próprias daqui: `LINUX_RESHADE_FX2SPV` e `LINUX_RESHADE_INCLUDES` (lista separada por `:`).
    pub fn from_config(config: &Config) -> Result<Self, String> {
        let repo = crate::install::resource_root();
        let in_repo = |rel: &str| repo.as_ref().map(|r| r.join(rel)).filter(|p| p.exists());

        let fx2spv = std::env::var_os("LINUX_RESHADE_FX2SPV")
            .map(PathBuf::from)
            .or_else(|| in_repo("build/fx2spv/fx2spv"))
            .ok_or(tr("could not find fx2spv: run 'scripts/dev.sh fx-tool' or set LINUX_RESHADE_FX2SPV"))?;

        let (effects, default_includes) = if config.is_configured() {
            // Os `#include` (ReShade.fxh e cia.) moram junto dos efeitos, como no ReShade.
            // Os efeitos que vêm com o programa (FPSCounter, os Basic*...) entram por último: o que a pessoa tem com o mesmo nome vale
            // mais, e quem já tinha uma pasta de dados antes deles existirem os recebe sem copiar nada.
            let mut effects = config.effect_paths.clone();
            effects.extend(in_repo("effects").map(SearchPath::flat).filter(|b| !effects.iter().any(|e| e.dir == b.dir)));
            (effects, config.effect_paths.clone())
        } else {
            let community: Vec<SearchPath> = ["third_party/reshade-shaders-legacy/Shaders", "third_party/reshade-shaders-slim/Shaders"]
                .iter()
                .filter_map(|p| in_repo(p))
                .map(SearchPath::flat)
                .collect();
            // Os headers ficam no ramo "slim"; os efeitos clássicos, no "legacy" (ver tools/fx2spv/deps.sh).
            let effects = in_repo("effects").map(SearchPath::flat).into_iter().chain(community.clone()).collect();
            (effects, community.into_iter().rev().collect())
        };
        let includes = match std::env::var_os("LINUX_RESHADE_INCLUDES") {
            Some(v) => std::env::split_paths(&v).map(SearchPath::flat).collect(),
            None => default_includes,
        };
        let textures = if config.is_configured() {
            let mut textures = config.texture_paths.clone();
            textures.extend(in_repo("effects/Textures").map(SearchPath::flat).filter(|b| !textures.iter().any(|t| t.dir == b.dir)));
            textures
        } else {
            ["third_party/reshade-shaders-legacy/Textures", "third_party/reshade-shaders-slim/Textures"]
                .iter()
                .filter_map(|p| in_repo(p))
                .map(SearchPath::flat)
                .collect()
        };
        Ok(Self { fx2spv, effects, includes, textures, cache: config.cache_dir.clone() })
    }
}
