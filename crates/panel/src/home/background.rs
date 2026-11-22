//! A análise dos efeitos em segundo plano e o nome da pasta de um efeito.

use super::*;

/// Analisa em segundo plano, um por vez, os efeitos que ainda não têm análise (compila cada um uma vez e guarda o
/// resultado no cache). Usa o próprio compilador do processo e termina com ele. Devolve `None` sem compilador.
pub fn start_analysis() -> Option<std::thread::JoinHandle<()>> {
    let compiler = Compiler::from_env().ok()?;
    std::thread::Builder::new()
        .name("analise-de-efeitos".into())
        .spawn(move || {
            loop {
                for name in analysis::pending_for(&compiler, &config::Config::load()) {
                    analysis::ensure(&compiler, &name);
                }
                // Efeitos novos nas pastas aparecem depois: olha de novo de vez em quando.
                std::thread::sleep(std::time::Duration::from_secs(10));
            }
        })
        .ok()
}

/// A subpasta de `dir` relativa à pasta de busca que a contém (a mais específica, se há mais de uma).
pub(super) fn folder_of(roots: &[PathBuf], dir: &Path) -> String {
    roots
        .iter()
        .filter_map(|r| dir.strip_prefix(r).ok())
        .min_by_key(|rel| rel.components().count())
        .map(|rel| rel.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default()
}
