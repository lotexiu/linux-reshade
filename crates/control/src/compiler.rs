//! Compilação de efeitos sob demanda: chama o `fx2spv` para produzir o bundle de um efeito numa
//! resolução e guarda o resultado num cache.
//!
//! Um bundle só serve à resolução para a qual foi compilado, então o diretório do cache leva o tamanho
//! no nome (`Sepia-1920x1080`). A escolha que a layer recebe é o *modelo* `<cache>/Sepia-{W}x{H}`.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

pub struct Compiler {
    /// O executável do `fx2spv`.
    pub fx2spv: PathBuf,
    /// Onde procurar `Nome.fx`, em ordem.
    pub effects: Vec<PathBuf>,
    /// Diretórios de `#include` (por exemplo, o do `ReShade.fxh`).
    pub includes: Vec<PathBuf>,
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
    Io(String),
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompileError::BadName(n) => write!(f, "{n:?} não é um nome de efeito válido (letras, números, _ - .)"),
            CompileError::UnknownEffect { name, searched } => {
                let dirs: Vec<_> = searched.iter().map(|d| d.display().to_string()).collect();
                write!(f, "não achei o efeito {name:?} (procurei em: {})", dirs.join(", "))
            }
            CompileError::Tool { status, output } => write!(f, "o fx2spv falhou ({status}):\n{}", output.trim_end()),
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
    #[cfg(test)]
    pub fn dir(&self) -> &Path {
        match self {
            Ensured::Cached(d) | Ensured::Compiled(d) => d,
        }
    }
}

/// Um nome de efeito é só letras, números, `_`, `-` e `.` (e não começa com ponto). Isso impede que
/// `../x` ou um caminho absoluto escapem do cache ao virarem um diretório.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
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

impl Compiler {
    /// Lê a configuração do ambiente, com padrões para o uso a partir de um clone do repositório
    /// (`target/release/lrctl` dentro dele). Variáveis: `LINUX_RESHADE_FX2SPV`, `LINUX_RESHADE_EFFECTS`,
    /// `LINUX_RESHADE_INCLUDES` (listas separadas por `:`) e `LINUX_RESHADE_CACHE`.
    pub fn from_env() -> Result<Self, String> {
        let repo = std::env::current_exe().ok().and_then(|exe| exe.ancestors().nth(3).map(Path::to_path_buf));
        let in_repo = |rel: &str| repo.as_ref().map(|r| r.join(rel)).filter(|p| p.exists());
        let list = |var: &str| std::env::var_os(var).map(|v| std::env::split_paths(&v).collect::<Vec<_>>());

        let fx2spv = std::env::var_os("LINUX_RESHADE_FX2SPV")
            .map(PathBuf::from)
            .or_else(|| in_repo("build/fx2spv/fx2spv"))
            .ok_or("não achei o fx2spv: rode 'scripts/dev.sh fx-tool' ou defina LINUX_RESHADE_FX2SPV")?;
        let community: Vec<PathBuf> = ["third_party/reshade-shaders-legacy/Shaders", "third_party/reshade-shaders-slim/Shaders"]
            .iter()
            .filter_map(|p| in_repo(p))
            .collect();
        let effects = list("LINUX_RESHADE_EFFECTS").unwrap_or_else(|| in_repo("effects").into_iter().chain(community.clone()).collect());
        // Os headers ficam no ramo "slim"; os efeitos clássicos, no "legacy" (ver tools/fx2spv/deps.sh).
        let includes = list("LINUX_RESHADE_INCLUDES").unwrap_or_else(|| community.iter().rev().cloned().collect());
        let cache = std::env::var_os("LINUX_RESHADE_CACHE").map(PathBuf::from).unwrap_or_else(|| {
            let base = std::env::var_os("XDG_CACHE_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
                .unwrap_or_else(|| PathBuf::from("/tmp"));
            base.join("linux-reshade").join("bundles")
        });
        Ok(Self { fx2spv, effects, includes, cache })
    }

    /// O `.fx` de um efeito, procurando nos diretórios configurados.
    pub fn source(&self, name: &str) -> Result<PathBuf, CompileError> {
        if !valid_name(name) {
            return Err(CompileError::BadName(name.to_string()));
        }
        self.effects
            .iter()
            .map(|d| d.join(format!("{name}.fx")))
            .find(|p| p.is_file())
            .ok_or_else(|| CompileError::UnknownEffect { name: name.to_string(), searched: self.effects.clone() })
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
        match (modified(&self.bundle_dir(name, width, height).join("manifest.json")), modified(&source)) {
            (Some(compiled), Some(edited)) => compiled >= edited,
            _ => false,
        }
    }

    /// Quando o `.fx` do efeito foi editado pela última vez.
    pub fn source_modified(&self, name: &str) -> Option<SystemTime> {
        self.source(name).ok().and_then(|p| modified(&p))
    }

    /// Garante um bundle atual de `name` para `width`x`height`: usa o do cache se o `.fx` não mudou
    /// desde a compilação, senão compila. (Mudanças só num `#include` não são notadas.)
    pub fn ensure(&self, name: &str, width: u32, height: u32) -> Result<Ensured, CompileError> {
        let source = self.source(name)?;
        let dir = self.bundle_dir(name, width, height);
        if self.is_current(name, width, height) {
            return Ok(Ensured::Cached(dir));
        }
        std::fs::create_dir_all(&self.cache).map_err(|e| CompileError::Io(format!("{}: {e}", self.cache.display())))?;
        let mut command = Command::new(&self.fx2spv);
        command.arg("--width").arg(width.to_string()).arg("--height").arg(height.to_string());
        for include in &self.includes {
            command.arg("-I").arg(include);
        }
        command.arg("-o").arg(&dir).arg(&source);
        let output = run_retrying_busy(&mut command).map_err(|e| CompileError::Io(format!("{}: {e}", self.fx2spv.display())))?;
        if !output.status.success() {
            return Err(CompileError::Tool {
                status: output.status.to_string(),
                output: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        Ok(Ensured::Compiled(dir))
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
            effects: vec![root.join("effects")],
            includes: vec![root.join("inc")],
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

    #[test]
    fn names_cannot_escape_the_cache() {
        for ok in ["Sepia", "Film_Grain-2", "a.b"] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in ["", "../x", "a/b", "/etc/passwd", ".oculto", "a b", "a;b", "é"] {
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
        assert!(call.contains("/cache/Sepia-800x600"), "{call}");
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
    fn an_unknown_effect_lists_where_it_looked() {
        let f = fake("unknown", OK_TOOL);
        match f.compiler.ensure("Nada", 10, 10) {
            Err(CompileError::UnknownEffect { searched, .. }) => assert_eq!(searched, f.compiler.effects),
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
        let compiler = Compiler { fx2spv: tool, effects: vec![repo.join("effects")], includes: vec![], cache };
        let ensured = compiler.ensure("Sepia", 64, 48).expect("compilar o Sépia de verdade");
        let bundle = fx_bundle::Bundle::load(ensured.dir()).expect("o bundle compilado carrega");
        assert_eq!((bundle.manifest.target.width, bundle.manifest.target.height), (64, 48));
    }
}
