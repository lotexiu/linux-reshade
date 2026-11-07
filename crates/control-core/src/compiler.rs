//! Compilação de efeitos sob demanda: chama o `fx2spv` para produzir o bundle de um efeito numa
//! resolução e guarda o resultado num cache.
//!
//! Um bundle só serve à resolução para a qual foi compilado, então o diretório do cache leva o tamanho
//! no nome (`Sepia-1920x1080`). A escolha que a layer recebe é o *modelo* `<cache>/Sepia-{W}x{H}`.

use config::{Config, SearchPath};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;
use i18n::{fmt, tr};

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

/// `Nome-1920x1080` -> `("Nome", 1920, 1080)`.
fn parse_bundle_name(file_name: &str) -> Option<(String, u32, u32)> {
    let (name, size) = file_name.rsplit_once('-')?;
    let (w, h) = size.split_once('x')?;
    valid_name(name).then_some(())?;
    Some((name.to_string(), w.parse().ok()?, h.parse().ok()?))
}

/// O nome de um diretório do cache é o de um bundle (`Nome-1920x1080`)?
pub fn is_bundle_name(file_name: &str) -> bool {
    parse_bundle_name(file_name).is_some()
}

/// Quais diretórios do cache apagar. Pura: recebe o que existe e devolve o que sobra de lado.
pub(crate) fn prune_plan(entries: &[CacheEntry], in_use: &std::collections::HashSet<(String, u32, u32)>, keep_per_effect: usize) -> Vec<PathBuf> {
    let mut doomed = Vec::new();
    let mut by_effect: std::collections::BTreeMap<String, Vec<&CacheEntry>> = std::collections::BTreeMap::new();
    for entry in entries {
        if entry.file_name.ends_with(".tmp") {
            doomed.push(entry.path.clone()); // sobra de uma compilação interrompida
        } else if let Some((name, _, _)) = parse_bundle_name(&entry.file_name) {
            by_effect.entry(name).or_default().push(entry);
        } // o que não é um bundle nosso não é tocado
    }
    for (_, mut bundles) in by_effect {
        bundles.sort_by_key(|e| std::cmp::Reverse(e.modified)); // os mais novos primeiro
        for entry in bundles.into_iter().skip(keep_per_effect) {
            if !in_use.contains(&parse_bundle_name(&entry.file_name).expect(tr("already read"))) {
                doomed.push(entry.path.clone());
            }
        }
    }
    doomed
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

/// Executa o comando, repetindo um pouco se o executável estiver "ocupado" (`ETXTBSY`): acontece
/// quando ele acabou de ser escrito (por exemplo, o `fx2spv` sendo recompilado) e algum `fork`
/// concorrente ainda segura o arquivo aberto para escrita. Passa em milissegundos.
fn run_retrying_busy(command: &mut Command) -> std::io::Result<std::process::Output> {
    let mut attempts = 0;
    loop {
        match command.output() {
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && attempts < 20 => {
                attempts += 1;
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            other => return other,
        }
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Onde o compilador anota de quais arquivos vieram as imagens do bundle (um caminho por linha).
const IMAGE_SOURCES: &str = "images/sources.txt";

/// As imagens do bundle ainda correspondem aos arquivos de origem (nenhum foi editado depois da compilação)?
fn images_up_to_date(dir: &Path, compiled: SystemTime) -> bool {
    let Ok(list) = std::fs::read_to_string(dir.join(IMAGE_SOURCES)) else { return true };
    list.lines().all(|path| modified(Path::new(path)).is_some_and(|edited| edited <= compiled))
}

/// O arquivo `source` dentro das pastas de texturas: o nome exato e, se não houver, sem diferenciar maiúsculas
/// (os efeitos foram escritos no Windows) e com `\` como separador.
/// O otimizador de SPIR-V (`spirv-opt`), se existe e a pessoa não o desligou. Medido em 145 efeitos na GPU real: a saída é a mesma
/// (nenhum passou de 1e-3 de diferença no primeiro quadro) e a GPU gasta uns 5 a 10% menos.
fn optimizer() -> Option<&'static Path> {
    static TOOL: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    TOOL.get_or_init(|| {
        if !Config::load().optimize_shaders {
            return None;
        }
        let candidate = std::env::var_os("LINUX_RESHADE_SPIRV_OPT").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("spirv-opt"));
        Command::new(&candidate).arg("--version").output().ok().filter(|o| o.status.success()).map(|_| candidate)
    })
    .as_deref()
}

/// Otimiza (em várias threads) cada `.spv` do bundle. Um módulo que o otimizador recusa, ou que não valida depois (quando o
/// `spirv-validator` existe), fica como estava.
fn optimize_bundle(dir: &Path, tool: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "spv")).collect();
    let validator = Command::new("spirv-val").arg("--version").output().ok().filter(|o| o.status.success()).is_some();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let workers = std::thread::available_parallelism().map_or(2, |n| n.get()).min(8);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(file) = files.get(i) else { break };
                    let out = file.with_extension("spv.opt");
                    // `--loop-unroll`: os laços marcados `[unroll]` viram sequências, e o deslocamento de `tex2D(s, uv, int2(x, y))` deixa de ser
                    // calculado em tempo de execução: o `spirv-opt` o troca por `ConstOffset`, o único que o Vulkan 1.0 aceita em amostragem
                    // (o gerador do ReShade emite `Offset`, que o driver pode executar de qualquer jeito: um Xid 69 da NVIDIA no
                    // ExposureFusion do iMMERSE). `--skip-validation`: a entrada ainda é a inválida; a saída é validada abaixo.
                    let optimized = Command::new(tool).arg("--skip-validation").arg("--loop-unroll").arg("-O").arg("--target-env=vulkan1.0").arg(file).arg("-o").arg(&out).output().is_ok_and(|o| o.status.success());
                    let valid = optimized && (!validator || Command::new("spirv-val").arg("--target-env").arg("vulkan1.0").arg(&out).output().is_ok_and(|o| o.status.success()));
                    if valid {
                        let _ = std::fs::rename(&out, file);
                    } else {
                        let _ = std::fs::remove_file(&out);
                    }
                }
            });
        }
    });
}

/// O bundle anota quais correções de código (`patches`) entraram nele.
const PATCHES_MARKER: &str = "patches";
/// Marca o bundle cujo SPIR-V já passou pelo otimizador.
const OPTIMIZED_MARKER: &str = "optimized";
/// O que o marcador guarda: muda quando a otimização muda (ex.: `--loop-unroll`), e os bundles feitos pela anterior são refeitos.
const OPTIMIZE_VERSION: &str = "2-loop-unroll";

fn patches_up_to_date(dir: &Path, name: &str, source: &Path) -> bool {
    let wanted = crate::patches::applicable(base_name(name), source);
    let have = std::fs::read_to_string(dir.join(PATCHES_MARKER)).unwrap_or_default();
    have.lines().filter(|l| !l.is_empty()).eq(wanted.iter().copied())
}

fn find_texture(dirs: &[PathBuf], source: &str) -> Option<PathBuf> {
    let relative = source.replace('\\', "/");
    let lower = relative.to_ascii_lowercase();
    for dir in dirs {
        let exact = dir.join(&relative);
        if exact.is_file() {
            return Some(exact);
        }
        let wanted = Path::new(&lower).file_name().map(|n| n.to_string_lossy().into_owned());
        if relative.contains('/') {
            continue; // subpasta: só o nome exato
        }
        let found = std::fs::read_dir(dir).ok()?.flatten().find(|e| Some(e.file_name().to_string_lossy().to_ascii_lowercase()) == wanted && e.path().is_file());
        if let Some(entry) = found {
            return Some(entry.path());
        }
    }
    None
}

impl Compiler {
    /// A configuração do processo atual: ver [`from_config`](Self::from_config).
    pub fn from_env() -> Result<Self, String> {
        Self::from_config(&Config::load())
    }

    /// Monta o compilador a partir da configuração do usuário (`docs/presets.md`). Sem pasta de dados nem
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
            (config.effect_paths.clone(), config.effect_paths.clone())
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
            config.texture_paths.clone()
        } else {
            ["third_party/reshade-shaders-legacy/Textures", "third_party/reshade-shaders-slim/Textures"]
                .iter()
                .filter_map(|p| in_repo(p))
                .map(SearchPath::flat)
                .collect()
        };
        Ok(Self { fx2spv, effects, includes, textures, cache: config.cache_dir.clone() })
    }

    /// As pastas de efeitos de verdade (com as subpastas das entradas `**`), na ordem de busca.
    ///
    /// Percorrer as subpastas custa (milhares de arquivos, às vezes num disco lento) e muita coisa pergunta isso por efeito
    /// (`source`, `source_modified`): o resultado guardado vale enquanto nenhuma das pastas mudou (uma subpasta nova
    /// muda a data da pasta que a contém).
    pub fn effect_dirs(&self) -> Vec<PathBuf> {
        use std::sync::Mutex;
        type Cached = (Vec<config::SearchPath>, Vec<(PathBuf, Option<SystemTime>)>);
        static CACHE: Mutex<Option<Cached>> = Mutex::new(None);
        let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((key, dirs)) = cache.as_ref()
            && *key == self.effects
            && dirs.iter().all(|(d, stamp)| modified(d) == *stamp)
        {
            return dirs.iter().map(|(d, _)| d.clone()).collect();
        }
        let dirs = config::expand(&self.effects);
        *cache = Some((self.effects.clone(), dirs.iter().map(|d| (d.clone(), modified(d))).collect()));
        dirs
    }

    /// Identifica as pastas de busca de efeitos e de texturas: muda quando o usuário troca as pastas, e o que foi
    /// concluído com as antigas (efeito sem imagem, por exemplo) deixa de valer.
    pub fn search_fingerprint(&self) -> String {
        format!("{:?}|{:?}", self.effects, self.textures)
    }

    /// O `.fx` de um efeito, procurando nos diretórios configurados.
    pub fn source(&self, name: &str) -> Result<PathBuf, CompileError> {
        if !valid_name(name) {
            return Err(CompileError::BadName(name.to_string()));
        }
        let name = base_name(name); // uma chave com definições aponta para o mesmo `.fx`
        let dirs = self.effect_dirs();
        dirs.iter()
            .map(|d| d.join(format!("{name}.fx")))
            .find(|p| p.is_file())
            .ok_or_else(|| CompileError::UnknownEffect { name: name.to_string(), searched: dirs })
    }

    /// Os nomes de efeito disponíveis (arquivos `.fx` nos diretórios configurados), sem repetição e em ordem.
    /// Cada nome vem com o diretório de onde sai, para o usuário saber qual versão será usada.
    ///
    /// Quem pergunta isso de meio em meio segundo (o painel) não paga a leitura de mais de mil arquivos: o resultado fica guardado
    /// e só é refeito quando a data de alguma das pastas muda (um arquivo novo muda a da pasta onde entra).
    pub fn available(&self) -> Vec<(String, PathBuf)> {
        use std::sync::Mutex;
        type Cached = (Vec<config::SearchPath>, Vec<(PathBuf, Option<SystemTime>)>, Vec<(String, PathBuf)>);
        static CACHE: Mutex<Option<Cached>> = Mutex::new(None);
        let dirs = self.effect_dirs();
        let stamps: Vec<(PathBuf, Option<SystemTime>)> = dirs.iter().map(|d| (d.clone(), modified(d))).collect();
        let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((key, old, found)) = cache.as_ref()
            && *key == self.effects
            && *old == stamps
        {
            return found.clone();
        }
        let found = self.scan_available(&dirs);
        *cache = Some((self.effects.clone(), stamps, found.clone()));
        found
    }

    fn scan_available(&self, dirs: &[PathBuf]) -> Vec<(String, PathBuf)> {
        let mut found: std::collections::BTreeMap<String, PathBuf> = std::collections::BTreeMap::new();
        for dir in dirs {
            let Ok(entries) = std::fs::read_dir(dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(name) = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".fx")) else { continue };
                // O primeiro diretório da lista vence, como em `source`.
                let is_file = entry.file_type().is_ok_and(|t| t.is_file()) || path.is_file(); // o segundo, para links
                if valid_name(name) && is_file {
                    found.entry(name.to_string()).or_insert_with(|| dir.clone());
                }
            }
        }
        found.into_iter().collect()
    }

    /// Apaga do cache os bundles antigos: de cada efeito ficam os `keep_per_effect` mais recentes, e nunca
    /// se apaga um que esteja em `in_use` (resoluções de jogos abertos). Também remove restos de
    /// compilações interrompidas (`*.tmp`). Devolve quantos diretórios apagou.
    ///
    /// Redimensionar uma janela cria dezenas de resoluções de passagem; sem isto o cache só cresceria.
    pub fn prune(&self, in_use: &std::collections::HashSet<(String, u32, u32)>, keep_per_effect: usize) -> usize {
        let Ok(entries) = std::fs::read_dir(&self.cache) else { return 0 };
        let found: Vec<CacheEntry> = entries
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                let file_name = path.file_name()?.to_str()?.to_string();
                let age_key = modified(&path.join("manifest.json")).or_else(|| modified(&path));
                Some(CacheEntry { path, file_name, modified: age_key? })
            })
            .collect();
        let doomed = prune_plan(&found, in_use, keep_per_effect);
        doomed.iter().filter(|p| std::fs::remove_dir_all(p).is_ok()).count()
    }

    /// Apaga os compilados de um efeito (todas as resoluções e variantes de macros): o vigia os refaz na próxima olhada. Devolve
    /// quantos apagou.
    pub fn invalidate(&self, name: &str) -> usize {
        let Ok(entries) = std::fs::read_dir(&self.cache) else { return 0 };
        entries
            .flatten()
            .filter(|e| e.file_name().to_str().and_then(parse_bundle_name).is_some_and(|(n, _, _)| n == name))
            .filter(|e| std::fs::remove_dir_all(e.path()).is_ok())
            .count()
    }

    pub fn bundle_dir(&self, name: &str, width: u32, height: u32) -> PathBuf {
        self.cache.join(format!("{name}-{width}x{height}"))
    }

    /// O modelo de caminho que a layer recebe: `<cache>/<Nome>-{W}x{H}`.
    pub fn template(&self, name: &str) -> String {
        self.cache.join(format!("{name}-{{W}}x{{H}}")).to_string_lossy().into_owned()
    }

    /// O nome do efeito de um modelo feito por [`template`](Self::template); `None` se a escolha
    /// atual não foi feita por nome (um diretório literal, por exemplo).
    pub fn name_from_template(&self, template: &str) -> Option<String> {
        let rest = Path::new(template).strip_prefix(&self.cache).ok()?.to_str()?;
        let name = rest.strip_suffix("-{W}x{H}")?;
        valid_name(name).then(|| name.to_string())
    }

    /// O bundle de `name` nesta resolução já existe e não está desatualizado em relação ao `.fx`?
    pub fn is_current(&self, name: &str, width: u32, height: u32) -> bool {
        let Ok(source) = self.source(name) else { return false };
        let dir = self.bundle_dir(name, width, height);
        // Um compilador mais novo que o bundle (o formato ganhou campos, um defeito foi corrigido) invalida o cache.
        let tool = modified(&self.fx2spv);
        match (modified(&dir.join("manifest.json")), modified(&source)) {
            (Some(compiled), Some(edited)) => {
                compiled >= edited && tool.is_none_or(|t| compiled >= t) && images_up_to_date(&dir, compiled) && patches_up_to_date(&dir, name, &source) && (optimizer().is_none() || std::fs::read_to_string(dir.join(OPTIMIZED_MARKER)).is_ok_and(|v| v == OPTIMIZE_VERSION))
            }
            _ => false,
        }
    }

    /// Quando o `.fx` do efeito foi editado pela última vez.
    pub fn source_modified(&self, name: &str) -> Option<SystemTime> {
        self.source(name).ok().and_then(|p| modified(&p))
    }

    /// Algum bundle já compilado de `name` (o mais recente), em qualquer resolução e com quaisquer macros (as variantes
    /// `Nome.m<hash>` do jogo aberto contam). Serve para ler os controles do efeito sem compilar nada: são os mesmos.
    pub fn any_bundle(&self, name: &str) -> Option<PathBuf> {
        std::fs::read_dir(&self.cache)
            .ok()?
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                let (n, _, _) = parse_bundle_name(path.file_name()?.to_str()?)?;
                (base_name(&n) == name).then_some(())?;
                Some((modified(&path.join("manifest.json"))?, path))
            })
            .max()
            .map(|(_, path)| path)
    }

    /// Um bundle do efeito escolhido por modelo (`.../Sepia-{W}x{H}`), para ler os controles dele. Os
    /// controles são os mesmos em toda resolução, então serve o de um jogo aberto; sem nenhum, usa
    /// [`DEFAULT_SIZE`] (compilando se preciso).
    pub fn schema_bundle(&self, template: &str, live: &[ipc::LiveLayer]) -> Result<PathBuf, CompileError> {
        let name = self.name_from_template(template).ok_or_else(|| CompileError::Io(fmt("{}: not a choice by name", &[&template])))?;
        let (width, height) = live
            .iter()
            .flat_map(|l| &l.status.swapchains)
            .find(|s| !matches!(s.effect, ipc::EffectState::Unavailable(_)))
            .map_or(DEFAULT_SIZE, |s| (s.width, s.height));
        Ok(self.ensure(&name, width, height)?.dir_owned())
    }

    /// Garante um bundle atual de `name` para `width`x`height`: usa o do cache se o `.fx` não mudou
    /// desde a compilação, senão compila. (Mudanças só num `#include` não são notadas.)
    pub fn ensure(&self, name: &str, width: u32, height: u32) -> Result<Ensured, CompileError> {
        self.ensure_with(name, width, height, &[])
    }

    /// Como [`ensure`](Self::ensure), compilando com `defines` (macros do preset e do efeito). Cada combinação tem o
    /// seu bundle (ver [`variant_key`]).
    pub fn ensure_with(&self, name: &str, width: u32, height: u32, defines: &[(String, String)]) -> Result<Ensured, CompileError> {
        let source = self.source(name)?;
        let key = variant_key(name, defines);
        let dir = self.bundle_dir(&key, width, height);
        if self.is_current(&key, width, height) {
            return Ok(Ensured::Cached(dir));
        }
        std::fs::create_dir_all(&self.cache).map_err(|e| CompileError::Io(format!("{}: {e}", self.cache.display())))?;
        // O bundle é montado numa pasta de trabalho e só entra no lugar pronto (já otimizado, com as imagens): o compilador grava
        // primeiro o SPIR-V que o Vulkan não perdoa, e uma layer que carregasse o bundle nessa hora (um jogo abrindo enquanto o
        // daemon recompila) mandaria à GPU o código inválido: Xid 69 da NVIDIA, com o jogo congelado.
        let work = self.cache.join("building").join(format!("{}.{}", dir.file_name().unwrap_or_default().to_string_lossy(), std::process::id()));
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(work.parent().unwrap_or(&self.cache)).map_err(|e| CompileError::Io(format!("{}: {e}", self.cache.display())))?;
        let mut command = Command::new(&self.fx2spv);
        command.arg("--width").arg(width.to_string()).arg("--height").arg(height.to_string());
        for include in config::expand(&self.includes) {
            command.arg("-I").arg(include);
        }
        for (name, value) in defines {
            if !valid_define(name, value) {
                return Err(CompileError::BadName(format!("{name}={value}")));
            }
            command.arg("-D").arg(format!("{name}={value}"));
        }
        // Correções de código que o Vulkan não perdoa (ver `patches`): compila uma cópia corrigida, e os `#include` relativos
        // continuam a valer pelo diretório do arquivo original.
        let applied = crate::patches::applicable(base_name(name), &source);
        let source = if applied.is_empty() {
            source
        } else {
            let text = std::fs::read_to_string(&source).map_err(|e| CompileError::Io(format!("{}: {e}", source.display())))?;
            let (patched, _) = crate::patches::apply(base_name(name), &text);
            // Uma pasta de trabalho com links para tudo o que há ao lado do original e a cópia corrigida no lugar dele: os `#include`
            // relativos resolvem pelo mesmo caminho de sempre (o mesmo cabeçalho por dois caminhos entraria duas vezes).
            let staging = self.cache.join("patched").join(dir.file_name().unwrap_or_default());
            let _ = std::fs::remove_dir_all(&staging);
            std::fs::create_dir_all(&staging).map_err(|e| CompileError::Io(format!("{}: {e}", staging.display())))?;
            let copy = staging.join(format!("{}.fx", base_name(name)));
            if let Some(parent) = source.parent() {
                for entry in std::fs::read_dir(parent).map_err(|e| CompileError::Io(format!("{}: {e}", parent.display())))?.flatten() {
                    if entry.path() != source {
                        let _ = std::os::unix::fs::symlink(entry.path(), staging.join(entry.file_name()));
                    }
                }
            }
            std::fs::write(&copy, patched).map_err(|e| CompileError::Io(format!("{}: {e}", copy.display())))?;
            copy
        };
        command.arg("-o").arg(&work).arg(&source);
        let output = run_retrying_busy(&mut command).map_err(|e| CompileError::Io(format!("{}: {e}", self.fx2spv.display())))?;
        if !output.status.success() {
            let _ = std::fs::remove_dir_all(&work);
            return Err(CompileError::Tool {
                status: output.status.to_string(),
                output: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        let _ = std::fs::write(work.join(PATCHES_MARKER), applied.join("\n"));
        if let Some(tool) = optimizer() {
            optimize_bundle(&work, tool);
            let _ = std::fs::write(work.join(OPTIMIZED_MARKER), OPTIMIZE_VERSION);
        }
        if let Err(e) = self.write_images(&work) {
            // Um bundle sem as imagens não serve e não pode passar por "atual" na próxima olhada.
            let _ = std::fs::remove_dir_all(&work);
            return Err(e);
        }
        // Publica: troca a pasta de uma vez (a antiga sai do caminho com um rename, nunca é editada no lugar).
        let old = work.with_extension(format!("old{}", std::process::id()));
        let had_old = dir.exists() && std::fs::rename(&dir, &old).is_ok();
        if let Err(e) = std::fs::rename(&work, &dir) {
            if had_old {
                let _ = std::fs::rename(&old, &dir);
            }
            let _ = std::fs::remove_dir_all(&work);
            return Err(CompileError::Io(format!("{}: {e}", dir.display())));
        }
        if had_old {
            let _ = std::fs::remove_dir_all(&old);
        }
        Ok(Ensured::Compiled(dir))
    }

    /// Decodifica as imagens das texturas de arquivo do bundle recém-compilado e as grava nele.
    fn write_images(&self, dir: &Path) -> Result<(), CompileError> {
        let _ = std::fs::remove_dir_all(dir.join("images")); // de uma compilação anterior
        let Ok(text) = std::fs::read_to_string(dir.join("manifest.json")) else { return Ok(()) };
        let Ok(manifest) = fx_bundle::Manifest::parse(&text) else { return Ok(()) }; // quem carrega o bundle reclama
        let dirs = config::expand(&self.textures);
        let mut sources = Vec::new();
        for (index, texture) in manifest.textures.iter().enumerate() {
            let Some(source) = fx_bundle::images::source_of(texture) else { continue };
            let path = find_texture(&dirs, source).ok_or_else(|| {
                let searched: Vec<_> = dirs.iter().map(|d| d.display().to_string()).collect();
                CompileError::Texture(fmt("could not find the image {} of the texture {} (searched in: {})", &[&format!("{:?}", source), &format!("{:?}", texture.name), &(searched.join(", "))]))
            })?;
            let bytes = std::fs::read(&path).map_err(|e| CompileError::Texture(format!("{}: {e}", path.display())))?;
            let decoded = image_file::decode(source, &bytes).map_err(|e| CompileError::Texture(format!("{}: {e}", path.display())))?;
            let image = fx_bundle::images::TextureImage { width: decoded.width, height: decoded.height, rgba: decoded.pixels };
            // O ReShade estica a imagem ao tamanho que o efeito declara para a textura.
            let image = if u64::from(texture.width) * u64::from(texture.height) * 4 <= fx_bundle::images::MAX_IMAGE_BYTES { image.resized(texture.width, texture.height) } else { image };
            fx_bundle::images::write(dir, index, &image).map_err(|e| CompileError::Io(format!("{}: {e}", dir.display())))?;
            sources.push(path.display().to_string());
        }
        if !sources.is_empty() {
            std::fs::write(dir.join(IMAGE_SOURCES), sources.join("\n")).map_err(|e| CompileError::Io(format!("{}: {e}", dir.display())))?;
        }
        Ok(())
    }
}

/// Um ambiente de teste com um `fx2spv` de mentira, compartilhado com os testes do vigia.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    pub struct Fake {
        pub root: PathBuf,
        pub compiler: Compiler,
    }

    /// Escreve `manifest.json` no diretório dado por `-o` e anota os argumentos em `calls.log`.
    pub const OK_TOOL: &str = r#"#!/bin/sh
out=""; prev=""
for a in "$@"; do [ "$prev" = "-o" ] && out="$a"; prev="$a"; done
echo "$@" >> "$(dirname "$0")/calls.log"
mkdir -p "$out" && echo '{}' > "$out/manifest.json"
"#;

    /// Um `fx2spv` de mentira que entrega os bundles reais de teste do postfx (Sepia, Invert, ...) no lugar do
    /// que compilaria o `.fx` de mesmo nome; um efeito sem bundle de teste falha. Também anota `calls.log`.
    pub fn bundles_tool() -> String {
        let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../postfx/tests/data");
        format!(
            "#!/bin/sh\nout=\"\"; prev=\"\"; for a in \"$@\"; do [ \"$prev\" = \"-o\" ] && out=\"$a\"; prev=\"$a\"; src=\"$a\"; done\n\
             echo \"$@\" >> \"$(dirname \"$0\")/calls.log\"\n\
             name=$(basename \"$src\" .fx)\n[ -d \"{d}/$name\" ] || {{ echo \"sem bundle de teste para $name\" >&2; exit 1; }}\n\
             mkdir -p \"$out\" && cp -r \"{d}/$name/.\" \"$out/\"\n",
            d = data.display()
        )
    }

    /// Como [`fake`], com `bundles_tool` e os efeitos `Invert` e `Quebrado` (este não tem bundle de teste).
    pub fn fake_with_bundles(name: &str) -> Fake {
        let f = fake(name, &bundles_tool());
        for fx in ["Invert", "Quebrado", "Depth"] {
            fs::write(f.root.join(format!("effects/{fx}.fx")), "// de mentira").unwrap();
        }
        f
    }

    pub fn calls(f: &Fake) -> Vec<String> {
        fs::read_to_string(f.root.join("calls.log")).map(|t| t.lines().map(String::from).collect()).unwrap_or_default()
    }

    pub fn fake(name: &str, tool_script: &str) -> Fake {
        let root = std::env::temp_dir().join(format!("lr-compiler-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("effects")).unwrap();
        fs::write(root.join("effects/Sepia.fx"), "// fx de mentira").unwrap();
        let tool = root.join("fake-fx2spv");
        fs::write(&tool, tool_script).unwrap();
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
        let compiler = Compiler {
            fx2spv: tool,
            effects: vec![SearchPath::flat(root.join("effects"))],
            includes: vec![SearchPath::flat(root.join("inc"))],
            textures: vec![SearchPath::flat(root.join("textures"))],
            cache: root.join("cache"),
        };
        Fake { root, compiler }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use std::fs;
    use std::time::Duration;

    /// 1x1 RGBA transparente.
    const PNG_1X1: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08,
        0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x60, 0x60, 0x60, 0x60, 0x00, 0x00,
        0x00, 0x05, 0x00, 0x01, 0xA5, 0xF6, 0x45, 0x40, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82
    ];

    #[test]
    fn the_image_of_a_file_texture_is_decoded_into_the_bundle() {
        let f = fake_with_bundles("texture-file");
        fs::write(f.root.join("effects/Tex.fx"), "// de mentira").unwrap();
        // O nome pedido pelo efeito (img.png) acha o arquivo mesmo com outras maiúsculas.
        fs::create_dir_all(f.root.join("textures")).unwrap();
        fs::write(f.root.join("textures/IMG.PNG"), PNG_1X1).unwrap();
        let ensured = f.compiler.ensure("Tex", 8, 4).unwrap();
        let image = fs::read(fx_bundle::images::path(ensured.dir(), 0)).unwrap();
        let image = fx_bundle::images::decode(&image).unwrap();
        assert_eq!((image.width, image.height, image.rgba.len()), (4, 2, 32), "esticada ao tamanho que o manifesto declara para a textura");
        assert!(f.compiler.is_current("Tex", 8, 4));

        // O PNG foi editado depois: o bundle deixa de estar atual e é refeito.
        let later = SystemTime::now() + Duration::from_secs(120);
        fs::File::options().write(true).open(f.root.join("textures/IMG.PNG")).unwrap().set_modified(later).unwrap();
        assert!(!f.compiler.is_current("Tex", 8, 4));
    }

    #[test]
    fn a_missing_texture_image_fails_the_compile_and_leaves_no_bundle() {
        let f = fake_with_bundles("texture-missing");
        fs::write(f.root.join("effects/Tex.fx"), "// de mentira").unwrap();
        let err = f.compiler.ensure("Tex", 8, 4).unwrap_err();
        assert!(matches!(err, CompileError::Texture(_)), "{err}");
        assert!(err.to_string().contains("img.png"), "{err}");
        assert!(!f.compiler.bundle_dir("Tex", 8, 4).exists(), "não fica um bundle sem a imagem");
        assert!(!f.compiler.is_current("Tex", 8, 4));
    }

    #[test]
    fn a_texture_that_is_not_an_image_fails_the_compile() {
        let f = fake_with_bundles("texture-bad");
        fs::write(f.root.join("effects/Tex.fx"), "// de mentira").unwrap();
        fs::create_dir_all(f.root.join("textures")).unwrap();
        fs::write(f.root.join("textures/img.png"), b"nao sou um png").unwrap();
        let err = f.compiler.ensure("Tex", 8, 4).unwrap_err();
        assert!(matches!(err, CompileError::Texture(_)), "{err}");
    }

    #[test]
    fn definitions_make_a_separate_bundle_with_a_stable_key() {
        assert_eq!(variant_key("Sepia", &[]), "Sepia");
        let a = variant_key("Sepia", &[("A".into(), "1".into()), ("B".into(), "2".into())]);
        let b = variant_key("Sepia", &[("B".into(), "2".into()), ("A".into(), "1".into())]);
        assert_eq!(a, b, "a ordem não importa");
        assert_ne!(a, variant_key("Sepia", &[("A".into(), "1".into())]));
        assert!(a.starts_with("Sepia.m") && valid_name(&a), "{a}");
        assert_eq!(base_name(&a), "Sepia");
        assert_eq!(base_name("Sepia"), "Sepia");
        assert_eq!(base_name("a.b"), "a.b", "um ponto no nome não é uma definição");
        assert_eq!(base_name("Film.mcafebabe"), "Film");
    }

    #[test]
    fn compiling_with_definitions_passes_them_and_keeps_a_separate_bundle() {
        let f = fake("defines", OK_TOOL);
        let defs = vec![("QUALITY".to_string(), "2".to_string()), ("FAST".to_string(), "1".to_string())];
        let with = f.compiler.ensure_with("Sepia", 800, 600, &defs).unwrap();
        let plain = f.compiler.ensure("Sepia", 800, 600).unwrap();
        assert_ne!(with.dir(), plain.dir());
        let calls = calls(&f);
        assert!(calls[0].contains("-D QUALITY=2") && calls[0].contains("-D FAST=1"), "{}", calls[0]);
        assert!(!calls[1].contains("-D"), "{}", calls[1]);
        // O mesmo pedido de novo usa o cache, e a data do .fx vale para as duas variantes.
        assert!(matches!(f.compiler.ensure_with("Sepia", 800, 600, &defs).unwrap(), Ensured::Cached(_)));
        assert!(f.compiler.is_current(&variant_key("Sepia", &defs), 800, 600));
        // Definição que não é um identificador não chega à linha de comando.
        assert!(f.compiler.ensure_with("Sepia", 1, 1, &[("A B".into(), "1".into())]).is_err());
        assert!(f.compiler.ensure_with("Sepia", 1, 1, &[("A".into(), "x\ny".into())]).is_err());
    }

    #[test]
    fn names_cannot_escape_the_cache() {
        for ok in ["Sepia", "Film_Grain-2", "a.b", "Adventurous MLUT", "Look (2)"] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in ["", "../x", "a/b", "/etc/passwd", ".oculto", " a", "a ", "a;b", "a&b", "é", "a\\b"] {
            assert!(!valid_name(bad), "{bad}");
        }
        let f = fake("names", OK_TOOL);
        assert!(matches!(f.compiler.ensure("../Sepia", 10, 10), Err(CompileError::BadName(_))));
        assert!(calls(&f).is_empty(), "nem chegou a chamar a ferramenta");
    }

    #[test]
    fn the_template_roundtrips_to_the_effect_name() {
        let f = fake("template", OK_TOOL);
        let t = f.compiler.template("Sepia");
        assert!(t.ends_with("/Sepia-{W}x{H}"), "{t}");
        assert_eq!(f.compiler.name_from_template(&t), Some("Sepia".into()));
        assert_eq!(f.compiler.name_from_template("/outro/lugar/Sepia-{W}x{H}"), None, "fora do cache");
        assert_eq!(f.compiler.name_from_template(&f.compiler.bundle_dir("Sepia", 10, 10).to_string_lossy()), None, "diretório literal");
    }

    #[test]
    fn compiles_once_then_uses_the_cache() {
        let f = fake("cache", OK_TOOL);
        let first = f.compiler.ensure("Sepia", 1920, 1080).unwrap();
        assert_eq!(first, Ensured::Compiled(f.compiler.bundle_dir("Sepia", 1920, 1080)));
        assert!(first.dir().join("manifest.json").is_file());

        let second = f.compiler.ensure("Sepia", 1920, 1080).unwrap();
        assert_eq!(second, Ensured::Cached(f.compiler.bundle_dir("Sepia", 1920, 1080)));
        assert_eq!(calls(&f).len(), 1, "a segunda vez não chamou a ferramenta");

        // Outra resolução é outro bundle.
        assert!(matches!(f.compiler.ensure("Sepia", 1914, 1048).unwrap(), Ensured::Compiled(_)));
        assert_eq!(calls(&f).len(), 2);
    }

    #[test]
    fn the_tool_receives_the_size_includes_output_and_source() {
        let f = fake("args", OK_TOOL);
        f.compiler.ensure("Sepia", 800, 600).unwrap();
        let call = &calls(&f)[0];
        assert!(call.starts_with("--width 800 --height 600 -I "), "{call}");
        assert!(call.contains("/inc -o "), "{call}");
        assert!(call.ends_with("/effects/Sepia.fx"), "{call}");
        assert!(call.contains("/cache/building/Sepia-800x600."), "o bundle é montado numa pasta de trabalho: {call}");
    }

    #[test]
    fn the_bundle_only_appears_in_place_once_it_is_complete() {
        // Quem olha o cache enquanto se compila (a layer de um jogo que abre) não pode ver o bundle pela metade.
        let f = fake("atomic", OK_TOOL);
        f.compiler.ensure("Sepia", 800, 600).unwrap();
        let call = &calls(&f)[0];
        let out = call.split(" -o ").nth(1).unwrap().split(' ').next().unwrap();
        assert!(!out.ends_with("/cache/Sepia-800x600"), "o compilador escreveu direto no lugar final: {out}");
        assert!(f.compiler.bundle_dir("Sepia", 800, 600).join("manifest.json").is_file());
        assert!(!std::path::Path::new(out).exists(), "a pasta de trabalho some depois de publicar");
    }

    #[test]
    fn a_known_source_defect_is_compiled_from_a_patched_copy_and_the_bundle_goes_stale_without_the_marker() {
        let f = fake("patched", OK_TOOL);
        let original = "for(uint b = 1; b <= AF_POINTS; b <<= 1, barrier())";
        fs::write(f.root.join("effects/MartysMods_DEPTHOFFIELD.fx"), original).unwrap();
        assert!(matches!(f.compiler.ensure("MartysMods_DEPTHOFFIELD", 64, 64).unwrap(), Ensured::Compiled(_)));
        let call = &calls(&f)[0];
        assert!(call.contains("/cache/patched/MartysMods_DEPTHOFFIELD-64x64/MartysMods_DEPTHOFFIELD.fx"), "{call}");
        assert!(f.root.join("cache/patched/MartysMods_DEPTHOFFIELD-64x64/Sepia.fx").exists(), "o que há ao lado do original aparece na pasta de trabalho");
        assert_eq!(fs::read_to_string(f.root.join("effects/MartysMods_DEPTHOFFIELD.fx")).unwrap(), original, "o original não muda");
        assert!(fs::read_to_string(f.root.join("cache/patched/MartysMods_DEPTHOFFIELD-64x64/MartysMods_DEPTHOFFIELD.fx")).unwrap().contains("b < AF_POINTS"));
        assert!(f.compiler.is_current("MartysMods_DEPTHOFFIELD", 64, 64));
        // Um bundle de antes das correções (sem a anotação) fica desatualizado.
        fs::remove_file(f.root.join("cache/MartysMods_DEPTHOFFIELD-64x64/patches")).unwrap();
        assert!(!f.compiler.is_current("MartysMods_DEPTHOFFIELD", 64, 64));
    }

    #[test]
    fn the_spirv_optimizer_keeps_every_module_valid_and_the_bundle_goes_stale_without_the_marker() {
        let Some(tool) = optimizer() else {
            eprintln!("sem spirv-opt; teste ignorado");
            return;
        };
        let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../postfx/tests/data/Sepia");
        let dir = std::env::temp_dir().join(format!("lr-opt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for e in std::fs::read_dir(&data).unwrap().flatten() {
            std::fs::copy(e.path(), dir.join(e.file_name())).unwrap();
        }
        optimize_bundle(&dir, tool);
        let bundle = fx_bundle::Bundle::load(&dir).expect("o bundle otimizado carrega e valida");
        assert!(!bundle.manifest.techniques.is_empty());
        assert!(std::fs::read_dir(&dir).unwrap().flatten().all(|e| e.path().extension().is_none_or(|x| x != "opt")), "sem sobras temporárias");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn editing_the_source_triggers_a_recompile() {
        let f = fake("stale", OK_TOOL);
        f.compiler.ensure("Sepia", 64, 64).unwrap();
        // O .fx fica mais novo que o bundle compilado.
        let later = SystemTime::now() + Duration::from_secs(60);
        fs::File::options().write(true).open(f.root.join("effects/Sepia.fx")).unwrap().set_modified(later).unwrap();
        assert!(matches!(f.compiler.ensure("Sepia", 64, 64).unwrap(), Ensured::Compiled(_)));
        assert_eq!(calls(&f).len(), 2);
    }

    #[test]
    fn a_failing_tool_reports_what_it_printed() {
        let f = fake("fail", "#!/bin/sh\necho 'Sepia.fx(3, 1): erro de sintaxe' >&2\nexit 1\n");
        match f.compiler.ensure("Sepia", 10, 10) {
            Err(CompileError::Tool { output, .. }) => assert!(output.contains("erro de sintaxe"), "{output}"),
            other => panic!("{other:?}"),
        }
        assert!(!f.compiler.bundle_dir("Sepia", 10, 10).join("manifest.json").exists());
    }

    #[test]
    fn available_lists_effects_once_and_prefers_the_first_directory() {
        let f = fake("available", OK_TOOL);
        let second = f.root.join("outros");
        fs::create_dir_all(&second).unwrap();
        fs::write(second.join("Sepia.fx"), "// repetido").unwrap();
        fs::write(second.join("Vibrance.fx"), "//").unwrap();
        fs::write(second.join("ReShade.fxh"), "// header, não é um efeito").unwrap();
        fs::write(second.join("notas.txt"), "x").unwrap();
        fs::write(second.join(".oculto.fx"), "x").unwrap();
        let mut compiler = f.compiler;
        compiler.effects.push(SearchPath::flat(second.clone()));
        let names: Vec<_> = compiler.available().into_iter().map(|(n, d)| (n, d == second)).collect();
        assert_eq!(names, vec![("Sepia".to_string(), false), ("Vibrance".to_string(), true)]);
    }

    #[test]
    fn recursive_search_path_finds_effects_in_subfolders_and_sees_new_ones() {
        let f = fake("recursive", OK_TOOL);
        let pack = f.root.join("pacote");
        fs::create_dir_all(pack.join("cores")).unwrap();
        fs::create_dir_all(pack.join(".git")).unwrap();
        fs::write(pack.join("cores/Vibrance.fx"), "//").unwrap();
        fs::write(pack.join(".git/Oculto.fx"), "//").unwrap();
        let mut compiler = f.compiler;
        compiler.effects = vec![SearchPath { dir: pack.clone(), recursive: true }];
        assert_eq!(compiler.source("Vibrance").unwrap(), pack.join("cores/Vibrance.fx"));
        assert!(compiler.source("Oculto").is_err(), "pastas ocultas ficam de fora");
        // Um efeito colocado depois que o compilador existe é visto sem recriá-lo (o daemon vive muito tempo).
        assert!(compiler.source("Novo").is_err());
        fs::create_dir_all(pack.join("novos")).unwrap();
        fs::write(pack.join("novos/Novo.fx"), "//").unwrap();
        assert_eq!(compiler.source("Novo").unwrap(), pack.join("novos/Novo.fx"));
        let names: Vec<_> = compiler.available().into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["Novo", "Vibrance"]);
    }

    fn entry(name: &str, age_secs: u64) -> CacheEntry {
        CacheEntry {
            path: PathBuf::from("/cache").join(name),
            file_name: name.to_string(),
            modified: SystemTime::now() - Duration::from_secs(age_secs),
        }
    }

    fn doomed(entries: &[CacheEntry], in_use: &[(&str, u32, u32)], keep: usize) -> Vec<String> {
        let in_use = in_use.iter().map(|(n, w, h)| (n.to_string(), *w, *h)).collect();
        let mut names: Vec<_> =
            prune_plan(entries, &in_use, keep).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    #[test]
    fn prune_keeps_the_newest_of_each_effect() {
        let entries = [
            entry("Sepia-100x100", 400),
            entry("Sepia-200x200", 300),
            entry("Sepia-300x300", 200),
            entry("Sepia-400x400", 100),
            entry("Invert-100x100", 900), // outro efeito: tem a própria cota
        ];
        assert_eq!(doomed(&entries, &[], 2), ["Sepia-100x100", "Sepia-200x200"]);
        assert!(doomed(&entries, &[], 4).is_empty(), "dentro da cota nada é apagado");
    }

    #[test]
    fn prune_never_removes_what_an_open_game_uses() {
        let entries = [entry("Sepia-100x100", 400), entry("Sepia-200x200", 300), entry("Sepia-300x300", 200)];
        assert_eq!(doomed(&entries, &[("Sepia", 100, 100)], 1), ["Sepia-200x200"], "o mais velho está em uso e fica");
    }

    #[test]
    fn prune_clears_interrupted_compilations_and_ignores_strangers() {
        let entries = [entry("Sepia-100x100.tmp", 10), entry("anotacoes", 999), entry("Sepia-1x", 999), entry("../Sepia-5x5", 999)];
        assert_eq!(doomed(&entries, &[], 0), ["Sepia-100x100.tmp"], "só o .tmp; o que não é bundle nosso não se toca");
    }

    #[test]
    fn prune_removes_real_directories_and_reports_how_many() {
        let f = fake("prune-real", OK_TOOL);
        for (i, (w, h)) in [(100, 100), (200, 200), (300, 300)].into_iter().enumerate() {
            f.compiler.ensure("Sepia", w, h).unwrap();
            // Envelhece em ordem: o 100x100 é o mais antigo e o 300x300, o mais novo.
            let manifest = f.compiler.bundle_dir("Sepia", w, h).join("manifest.json");
            let at = SystemTime::now() - Duration::from_secs(1000 - i as u64 * 100);
            fs::File::options().write(true).open(&manifest).unwrap().set_modified(at).unwrap();
        }
        // Fica 1 por cota (o 300x300, o mais novo), e o 100x100, o mais velho, está em uso por um jogo.
        let in_use = std::collections::HashSet::from([("Sepia".to_string(), 100, 100)]);
        assert_eq!(f.compiler.prune(&in_use, 1), 1, "só o 200x200 é apagado");
        let exists = |w, h| f.compiler.bundle_dir("Sepia", w, h).exists();
        assert!(exists(100, 100), "em uso");
        assert!(!exists(200, 200), "velho e sem uso");
        assert!(exists(300, 300), "o mais novo");
        assert_eq!(f.compiler.prune(&in_use, 1), 0, "uma segunda passada não acha mais nada");
    }

    #[test]
    fn the_schema_bundle_comes_from_an_open_game_or_falls_back_to_the_default_size() {
        use ipc::{EffectState, LayerStatus, LiveLayer, SwapchainStatus};
        let f = fake("schema", OK_TOOL);
        let template = f.compiler.template("Sepia");
        let sc = |w, h, e| SwapchainStatus { width: w, height: h, format: 44, images: 3, effect: e };
        let game = |sw: Vec<SwapchainStatus>| LiveLayer { id: "x".into(), status: LayerStatus { perf: None, depth: Vec::new(), depth_passes: String::new(), techniques: Vec::new(), exe: String::new(), app: "A".into(), swapchains: sw } };

        // Sem jogo aberto: o tamanho padrão.
        assert_eq!(f.compiler.schema_bundle(&template, &[]).unwrap(), f.compiler.bundle_dir("Sepia", 1920, 1080));
        // Com jogo: a resolução dele (ignorando swapchains que não aceitam efeito).
        let live = [game(vec![sc(640, 480, EffectState::Unavailable("sRGB".into())), sc(1280, 720, EffectState::Waiting)])];
        assert_eq!(f.compiler.schema_bundle(&template, &live).unwrap(), f.compiler.bundle_dir("Sepia", 1280, 720));
        // Uma escolha que não é por nome não tem o que resolver.
        assert!(f.compiler.schema_bundle("/qualquer/bundle", &[]).is_err());
    }

    #[test]
    fn any_bundle_also_finds_a_variant_compiled_with_macros() {
        let f = fake("any-variant", OK_TOOL);
        let defines = vec![("QUALITY".to_string(), "2".to_string())];
        f.compiler.ensure_with("Sepia", 100, 100, &defines).unwrap();
        let key = variant_key("Sepia", &defines);
        assert_eq!(f.compiler.any_bundle("Sepia"), Some(f.compiler.bundle_dir(&key, 100, 100)));
    }

    #[test]
    fn any_bundle_finds_the_most_recent_compiled_resolution() {
        let f = fake("any", OK_TOOL);
        assert_eq!(f.compiler.any_bundle("Sepia"), None, "nada compilado");
        for (i, (w, h)) in [(100, 100), (200, 200)].into_iter().enumerate() {
            f.compiler.ensure("Sepia", w, h).unwrap();
            let manifest = f.compiler.bundle_dir("Sepia", w, h).join("manifest.json");
            let at = SystemTime::now() - Duration::from_secs(500 - i as u64 * 100);
            fs::File::options().write(true).open(&manifest).unwrap().set_modified(at).unwrap();
        }
        f.compiler.ensure("Invert", 1, 1).ok(); // outro efeito (sem .fx: erro ignorado) não atrapalha
        assert_eq!(f.compiler.any_bundle("Sepia"), Some(f.compiler.bundle_dir("Sepia", 200, 200)));
        assert_eq!(f.compiler.any_bundle("Invert"), None);
    }

    #[test]
    fn an_unknown_effect_lists_where_it_looked() {
        let f = fake("unknown", OK_TOOL);
        match f.compiler.ensure("Nada", 10, 10) {
            Err(CompileError::UnknownEffect { searched, .. }) => assert_eq!(searched, f.compiler.effect_dirs()),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_missing_tool_is_an_io_error_not_a_panic() {
        let mut f = fake("missing", OK_TOOL);
        f.compiler.fx2spv = f.root.join("nao-existe");
        assert!(matches!(f.compiler.ensure("Sepia", 10, 10), Err(CompileError::Io(_))));
    }

    /// Com o fx2spv de verdade (se foi compilado), o resultado tem de ser um bundle que carrega.
    #[test]
    fn the_real_tool_produces_a_loadable_bundle() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let tool = repo.join("build/fx2spv/fx2spv");
        if !tool.is_file() {
            eprintln!("sem {}; rode scripts/dev.sh fx-tool para exercitar este teste", tool.display());
            return;
        }
        let cache = std::env::temp_dir().join(format!("lr-compiler-{}-real", std::process::id()));
        let _ = fs::remove_dir_all(&cache);
        let compiler = Compiler { fx2spv: tool, effects: vec![SearchPath::flat(repo.join("effects"))], includes: vec![], textures: vec![], cache };
        let ensured = compiler.ensure("Sepia", 64, 48).expect("compilar o Sépia de verdade");
        let bundle = fx_bundle::Bundle::load(ensured.dir()).expect("o bundle compilado carrega");
        assert_eq!((bundle.manifest.target.width, bundle.manifest.target.height), (64, 48));
    }
}
