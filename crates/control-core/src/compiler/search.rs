//! A busca dos efeitos nas pastas configuradas (com subpastas) e das imagens das texturas.

use super::*;

pub(super) fn find_texture(dirs: &[PathBuf], source: &str) -> Option<PathBuf> {
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

    pub(super) fn scan_available(&self, dirs: &[PathBuf]) -> Vec<(String, PathBuf)> {
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
}
