//! Qual bundle de efeito cada swapchain executa.
//!
//! A escolha é um *modelo* de caminho: o que o app de controle publicou com `lrctl use` e, na falta
//! dele, a variável `LINUX_RESHADE_BUNDLE`. O modelo pode conter `{W}` e `{H}`, trocados pela
//! resolução do swapchain, e assim um mesmo efeito escolhido serve a jogos de resoluções diferentes
//! (cada resolução tem o seu bundle compilado).
//!
//! O custo por frame é uma leitura atômica (a geração da seleção). O trabalho de verdade (ler o
//! disco, validar) só acontece quando a escolha é publicada de novo, e isso sobe a *época*: cada
//! swapchain compara a época que viu com a atual e, se mudou, resolve seu bundle outra vez.

use crate::log::log;
use fx_bundle::Bundle;
use ipc::SelectionRead;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

/// Valor de `LAST_GENERATION` antes de a seleção ter sido lida.
const NEVER: u64 = u64::MAX;
/// Valor de `LAST_GENERATION` enquanto o app de controle ainda não criou o canal.
const NO_CHANNEL: u64 = u64::MAX - 1;

/// Última geração da seleção que já foi processada.
static LAST_GENERATION: AtomicU64 = AtomicU64::new(NEVER);
/// Sobe a cada vez que a escolha é publicada de novo. 0 = nunca foi lida.
static EPOCH: AtomicU64 = AtomicU64::new(0);

struct State {
    /// O modelo de caminho vigente, se houver.
    template: Option<String>,
    /// Bundles já carregados nesta época, por diretório. É esvaziado a cada nova publicação, para um
    /// efeito recompilado (mesmo diretório, conteúdo novo) ser lido de novo.
    cache: HashMap<PathBuf, Arc<Bundle>>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(|| Mutex::new(State { template: None, cache: HashMap::new() }));

/// O que a escolha dá para um swapchain de certo tamanho.
pub enum Resolved {
    /// Nenhum efeito escolhido.
    None,
    /// Há efeito escolhido, mas ainda não existe um bundle para esta resolução (o app de controle o compila).
    Waiting(PathBuf),
    Ready(Arc<Bundle>),
    /// O diretório existe, mas o bundle não carrega.
    Invalid(String),
}

/// O modelo de caminho com `{W}` e `{H}` trocados pela resolução.
pub fn expand(template: &str, width: u32, height: u32) -> PathBuf {
    PathBuf::from(template.replace("{W}", &width.to_string()).replace("{H}", &height.to_string()))
}

fn from_environment() -> Option<String> {
    std::env::var_os("LINUX_RESHADE_BUNDLE").map(|p| p.to_string_lossy().into_owned())
}

/// A época atual da escolha. Barata quando nada mudou; relê a escolha quando ela foi republicada.
pub fn epoch() -> u64 {
    let reader = crate::control::reader();
    let generation = reader.map_or(NO_CHANNEL, |r| u64::from(r.selection_generation()));
    if generation == LAST_GENERATION.load(Ordering::Acquire) {
        return EPOCH.load(Ordering::Acquire);
    }
    let template = match reader.map_or(SelectionRead::Cleared, |r| r.read_selection()) {
        // A leitura caiu no meio de uma escrita: tenta no próximo frame.
        SelectionRead::Unstable => return EPOCH.load(Ordering::Acquire),
        SelectionRead::Path(path) => Some(path),
        SelectionRead::Cleared => from_environment(),
    };
    let mut state = STATE.lock().unwrap();
    if state.template != template {
        match &template {
            Some(t) => log!("efeito escolhido: {t}"),
            None => log!("nenhum efeito escolhido"),
        }
        state.template = template;
    }
    state.cache.clear();
    LAST_GENERATION.store(generation, Ordering::Release);
    EPOCH.fetch_add(1, Ordering::AcqRel) + 1
}

/// O bundle da escolha vigente para um swapchain de `width` x `height`.
pub fn resolve(width: u32, height: u32) -> Resolved {
    let mut state = STATE.lock().unwrap();
    let Some(template) = state.template.clone() else { return Resolved::None };
    let dir = expand(&template, width, height);
    if let Some(bundle) = state.cache.get(&dir) {
        return Resolved::Ready(bundle.clone());
    }
    if !dir.join("manifest.json").is_file() {
        return Resolved::Waiting(dir);
    }
    match Bundle::load(&dir) {
        Ok(bundle) => {
            let t = bundle.manifest.target;
            log!("bundle carregado: {} ({}x{}, {} técnica(s))", dir.display(), t.width, t.height, bundle.manifest.techniques.len());
            let bundle = Arc::new(bundle);
            state.cache.insert(dir, bundle.clone());
            Resolved::Ready(bundle)
        }
        Err(e) => {
            log!("bundle {} recusado: {e}", dir.display());
            Resolved::Invalid(e.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_template_is_filled_with_the_swapchain_size() {
        assert_eq!(expand("/c/Sepia-{W}x{H}", 1920, 1080), PathBuf::from("/c/Sepia-1920x1080"));
        assert_eq!(expand("/c/{W}/{H}/Sepia", 1914, 1048), PathBuf::from("/c/1914/1048/Sepia"));
    }

    #[test]
    fn a_template_without_placeholders_is_a_literal_path() {
        assert_eq!(expand("/c/Sepia-500x500", 1920, 1080), PathBuf::from("/c/Sepia-500x500"));
    }
}
