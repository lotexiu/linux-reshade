//! O que cada efeito pede e se o runtime dá conta, para a lista "Adicionar efeito" marcar os incompatíveis e
//! filtrar por característica. O `.fx` sozinho não diz isso (depende do que o compilador gera), então a análise
//! vem do bundle compilado, ou do erro da compilação, e fica guardada no cache (`<cache>/analysis/<Efeito>.ini`).

use crate::compiler::{CompileError, Compiler};
use fx_bundle::Manifest;
use fx_bundle::images::source_of;
use ini::Ini;
use std::path::PathBuf;
use crate::i18n::tr;

/// Uma característica de um efeito (o que o filtro da lista oferece).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Feature {
    /// Lê a profundidade do jogo (`DEPTH`): suportado quando o jogo é aberto com `LINUX_RESHADE_DEPTH=1`.
    Depth,
    /// Lê a imagem do jogo (o back buffer).
    Color,
    /// Usa imagens de arquivo (LUTs, ruído...).
    Textures,
    /// Usa texturas com mipmaps.
    Mipmaps,
    /// Usa as conversões sRGB de leitura ou escrita.
    Srgb,
    /// Usa compute shaders e imagens de armazenamento.
    Compute,
}

impl Feature {
    pub const ALL: [Feature; 6] = [Feature::Depth, Feature::Color, Feature::Textures, Feature::Mipmaps, Feature::Srgb, Feature::Compute];

    /// O texto que a interface mostra.
    pub fn label(self) -> &'static str {
        match self {
            Feature::Depth => tr("depth"),
            Feature::Color => tr("game image"),
            Feature::Textures => tr("image files"),
            Feature::Mipmaps => tr("mipmaps"),
            Feature::Srgb => tr("sRGB"),
            Feature::Compute => tr("compute"),
        }
    }

    fn key(self) -> &'static str {
        match self {
            Feature::Depth => "depth",
            Feature::Color => "color",
            Feature::Textures => "textures",
            Feature::Mipmaps => "mipmaps",
            Feature::Srgb => "srgb",
            Feature::Compute => "compute",
        }
    }

    fn from_key(key: &str) -> Option<Feature> {
        Feature::ALL.into_iter().find(|f| f.key() == key)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EffectInfo {
    /// Nome do efeito (arquivo sem `.fx`).
    pub name: String,
    pub techniques: Vec<String>,
    pub features: Vec<Feature>,
    /// Por que o runtime não dá conta dele (uma linha por motivo); vazio = compatível.
    pub problems: Vec<String>,
    /// A compilação falhou (ou a imagem de uma textura não foi achada): o texto do erro.
    pub error: Option<String>,
}

impl EffectInfo {
    pub fn compatible(&self) -> bool {
        self.problems.is_empty() && self.error.is_none()
    }

    /// Por que não serve, numa linha (`None` se serve).
    pub fn reason(&self) -> Option<String> {
        match (&self.error, self.problems.first()) {
            (Some(e), _) => Some(e.clone()),
            (None, Some(p)) => Some(crate::i18n::tr_dyn(p)),
            _ => None,
        }
    }

    pub fn has(&self, feature: Feature) -> bool {
        self.features.contains(&feature)
    }
}

/// Analisa o manifesto de um efeito compilado.
pub fn analyze(name: &str, manifest: &Manifest) -> EffectInfo {
    let mut features = Vec::new();
    let mut add = |f: Feature| {
        if !features.contains(&f) {
            features.push(f);
        }
    };
    let texture_of = |sampler: u32| manifest.samplers.get(sampler as usize).and_then(|s| manifest.textures.iter().find(|t| t.unique_name == s.texture));

    for technique in &manifest.techniques {
        for pass in &technique.passes {
            if pass.compute_entry.is_some() {
                add(Feature::Compute);
            }
            if pass.srgb_write {
                add(Feature::Srgb);
            }
            for binding in &pass.samplers {
                if manifest.samplers.get(binding.sampler as usize).is_some_and(|s| s.srgb) {
                    add(Feature::Srgb);
                }
                match texture_of(binding.sampler).and_then(|t| t.semantic.as_deref()) {
                    Some("DEPTH") => add(Feature::Depth),
                    Some("COLOR") => add(Feature::Color),
                    _ => {}
                }
            }
        }
    }
    for texture in &manifest.textures {
        if source_of(texture).is_some() {
            add(Feature::Textures);
        }
        if texture.levels > 1 {
            add(Feature::Mipmaps);
        }
    }
    features.sort();

    // Profundidade, compute, armazenamento e texturas 1D/3D são suportados; a profundidade só precisa do jogo aberto com
    // LINUX_RESHADE_DEPTH=1. O que o bundle não suporta já é recusado ao compilar (`fx-bundle`).
    let problems: Vec<String> = Vec::new();
    EffectInfo { name: name.to_string(), techniques: manifest.techniques.iter().map(|t| t.name.clone()).collect(), features, problems, error: None }
}

/// O que se sabe de um efeito cuja compilação falhou.
pub fn from_error(name: &str, error: &CompileError) -> EffectInfo {
    let text = error.to_string();
    let mut info = EffectInfo { name: name.to_string(), ..EffectInfo::default() };
    if text.contains("armazenamento") || text.contains("storage") {
        info.features.push(Feature::Compute);
        info.problems.push(tr("uses storage images (compute), not supported yet").to_string());
    } else {
        // A primeira linha basta para a lista; o texto inteiro está no log do compilador.
        info.error = Some(text.lines().find(|l| !l.trim().is_empty()).unwrap_or(tr("compilation failed")).trim().to_string());
    }
    info
}

fn dir(compiler: &Compiler) -> PathBuf {
    compiler.cache.join("analysis")
}

fn path(compiler: &Compiler, name: &str) -> PathBuf {
    dir(compiler).join(format!("{name}.ini"))
}

fn one_line(text: &str) -> String {
    text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

/// Versão do formato e das regras da análise: mudou, as guardadas deixam de valer.
const VERSION: &str = "4";

/// Guarda a análise no cache.
pub fn save(compiler: &Compiler, info: &EffectInfo) -> std::io::Result<()> {
    let mut ini = Ini::new();
    ini.set("", "version", VERSION);
    ini.set("", "search", &compiler.search_fingerprint());
    ini.set("", "techniques", &info.techniques.iter().map(|t| one_line(t)).collect::<Vec<_>>().join(","));
    ini.set("", "features", &info.features.iter().map(|f| f.key()).collect::<Vec<_>>().join(","));
    ini.set("", "problems", &info.problems.iter().map(|p| one_line(p)).collect::<Vec<_>>().join(" | "));
    if let Some(error) = &info.error {
        ini.set("", "error", &one_line(error));
    }
    ini.save(&path(compiler, &info.name))
}

/// Lê a análise guardada, se existe e ainda vale (o `.fx` não foi editado depois).
pub fn load(compiler: &Compiler, name: &str) -> Option<EffectInfo> {
    let file = path(compiler, name);
    let saved = std::fs::metadata(&file).and_then(|m| m.modified()).ok()?;
    if compiler.source_modified(name).is_some_and(|edited| edited > saved) {
        return None;
    }
    let ini = Ini::load(&file).ok()?;
    if ini.get("", "version") != Some(VERSION) || ini.get("", "search") != Some(compiler.search_fingerprint().as_str()) {
        return None;
    }
    let list = |key: &str| ini.get("", key).unwrap_or("").split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect::<Vec<_>>();
    Some(EffectInfo {
        name: name.to_string(),
        techniques: list("techniques"),
        features: list("features").iter().filter_map(|k| Feature::from_key(k)).collect(),
        problems: ini.get("", "problems").unwrap_or("").split(" | ").map(str::trim).filter(|s| !s.is_empty() && !s.contains("1D ou 3D")).map(String::from).collect(), // análises antigas guardavam este aviso, hoje falso
        error: ini.get("", "error").filter(|e| !e.is_empty()).map(String::from),
    })
}

/// Analisa um efeito: usa a análise guardada ou compila (para a resolução padrão) e guarda o resultado.
pub fn ensure(compiler: &Compiler, name: &str) -> EffectInfo {
    if let Some(info) = load(compiler, name) {
        return info;
    }
    let (width, height) = crate::compiler::DEFAULT_SIZE;
    let info = match compiler.ensure(name, width, height) {
        Ok(ensured) => {
            let manifest = std::fs::read_to_string(ensured.dir_owned().join("manifest.json")).ok().and_then(|t| Manifest::parse(&t).ok());
            match manifest {
                Some(m) => analyze(name, &m),
                None => EffectInfo { name: name.to_string(), error: Some(tr("the compiled bundle has no readable manifest").into()), ..EffectInfo::default() },
            }
        }
        Err(e) => from_error(name, &e),
    };
    let _ = save(compiler, &info);
    info
}

/// Os nomes de efeito que ainda não têm análise válida.
pub fn pending(compiler: &Compiler) -> Vec<String> {
    compiler.available().into_iter().map(|(name, _)| name).filter(|name| load(compiler, name).is_none()).collect()
}

/// Quantas análises existem em `dir` (para mostrar o andamento sem ler todas).
///
/// Guardada pela data da pasta (um arquivo novo a muda): o painel pergunta isto de meio em meio segundo.
pub fn count_saved(compiler: &Compiler) -> usize {
    use std::sync::Mutex;
    static CACHE: Mutex<Option<(PathBuf, Option<std::time::SystemTime>, usize)>> = Mutex::new(None);
    let dir = dir(compiler);
    let stamp = std::fs::metadata(&dir).and_then(|m| m.modified()).ok();
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((path, at, n)) = cache.as_ref()
        && *path == dir
        && *at == stamp
        && stamp.is_some()
    {
        return *n;
    }
    let n = std::fs::read_dir(&dir).map(|e| e.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "ini")).count()).unwrap_or(0);
    *cache = Some((dir, stamp, n));
    n
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::testing::fake_with_bundles;
    use std::path::Path;

    fn manifest(name: &str) -> Manifest {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../postfx/tests/data").join(name);
        Manifest::parse(&std::fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap()
    }

    #[test]
    fn a_plain_effect_is_compatible_and_its_features_are_listed() {
        let info = analyze("Sepia", &manifest("Sepia"));
        assert!(info.compatible(), "{info:?}");
        assert_eq!(info.techniques, ["Sepia"]);
        assert_eq!(info.features, [Feature::Color]);
        assert_eq!(analyze("Mip", &manifest("Mip")).features, [Feature::Color, Feature::Mipmaps]);
        assert_eq!(analyze("Tex", &manifest("Tex")).features, [Feature::Textures]);
    }

    #[test]
    fn depth_is_a_feature_not_a_problem_now_that_the_runtime_supports_it() {
        let mut m = manifest("Sepia");
        // O sampler do Sépia passa a ler uma textura de profundidade.
        m.textures[0].semantic = Some("DEPTH".into());
        let info = analyze("Sepia", &m);
        assert!(info.compatible(), "{info:?}");
        assert!(info.has(Feature::Depth) && !info.has(Feature::Color));
    }

    #[test]
    fn compute_is_a_feature_and_1d_or_3d_textures_are_supported() {
        let mut m = manifest("Sepia");
        m.techniques[0].passes[0].compute_entry = Some("CS".into());
        let info = analyze("Sepia", &m);
        assert!(info.compatible() && info.has(Feature::Compute), "{info:?}");
        m.textures[0].kind = fx_bundle::manifest::TextureType::D3;
        assert!(analyze("Sepia", &m).compatible());
    }

    #[test]
    fn a_storage_error_is_classified_as_compute_and_other_errors_keep_their_first_line() {
        let storage = from_error("X", &CompileError::Tool { status: "1".into(), output: "erro: imagens de armazenamento (storage) não são suportadas".into() });
        assert!(!storage.compatible() && storage.has(Feature::Compute) && storage.error.is_none());
        let other = from_error("Y", &CompileError::Texture("não achei a imagem \"lut.png\"".into()));
        assert_eq!(other.error.as_deref(), Some("não achei a imagem \"lut.png\""));
        assert!(!other.compatible());
    }

    #[test]
    fn the_analysis_is_saved_loaded_and_goes_stale_when_the_source_is_edited() {
        let f = fake_with_bundles("analysis-cache");
        assert!(load(&f.compiler, "Sepia").is_none());
        let info = ensure(&f.compiler, "Sepia");
        assert!(info.compatible(), "{info:?}");
        assert_eq!(load(&f.compiler, "Sepia"), Some(info.clone()));
        assert_eq!(count_saved(&f.compiler), 1);
        assert!(pending(&f.compiler).contains(&"Invert".to_string()) && !pending(&f.compiler).contains(&"Sepia".to_string()));

        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(120);
        std::fs::File::options().write(true).open(f.root.join("effects/Sepia.fx")).unwrap().set_modified(later).unwrap();
        assert!(load(&f.compiler, "Sepia").is_none(), "o .fx foi editado depois");
    }

    #[test]
    fn a_failing_effect_is_remembered_as_incompatible() {
        let f = fake_with_bundles("analysis-fail");
        let info = ensure(&f.compiler, "Quebrado"); // a ferramenta de teste falha para este
        assert!(!info.compatible());
        assert!(load(&f.compiler, "Quebrado").is_some(), "não recompila a cada vez");
    }
}
