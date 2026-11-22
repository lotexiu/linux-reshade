//! O cache dos bundles compilados: onde cada um mora, o que está em dia, o que se pode apagar.

use super::*;

/// `Nome-1920x1080` -> `("Nome", 1920, 1080)`.
pub(super) fn parse_bundle_name(file_name: &str) -> Option<(String, u32, u32)> {
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

impl Compiler {
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
}
