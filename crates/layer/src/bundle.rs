//! Qual bundle de efeito cada swapchain executa.
//!
//! A escolha é um *modelo* de caminho: o que o app de controle publicou para este jogo (canal próprio)
//! ou, na falta, para todos (canal global, `lrctl use`) e, por último, a variável `LINUX_RESHADE_BUNDLE`. O modelo pode conter `{W}` e `{H}`, trocados pela
//! resolução do swapchain, e assim um mesmo efeito escolhido serve a jogos de resoluções diferentes
//! (cada resolução tem o seu bundle compilado).
//!
//! O custo por frame é uma leitura atômica (a geração da seleção). O trabalho de verdade (ler o
//! disco, validar) só acontece quando a escolha é publicada de novo, e isso sobe a *época*: cada
//! swapchain compara a época que viu com a atual e, se mudou, resolve seu bundle outra vez.

use crate::log::log;
use fx_bundle::Bundle;
use ipc::{ChannelSelection, SelectionRead};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

/// Valor de `LAST_GENERATION` antes de a seleção ter sido lida.
const NEVER: u64 = u64::MAX;
/// A geração de um canal que ainda não existe. Gerações de verdade são `seq / 2` e nunca chegam perto.
const ABSENT: u32 = u32::MAX - 1;

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

/// O que as escolhas dão juntas.
#[derive(Debug, PartialEq, Eq)]
enum Choice {
    /// Uma leitura caiu no meio de uma escrita: tenta no próximo frame.
    Unstable,
    /// O modelo vigente (`None` = nenhum efeito).
    Template(Option<String>),
}

/// Quem manda: a escolha do próprio jogo; sem ela, a global; sem ela, o ambiente. `NoEffect` do jogo vence
/// tudo (não cai no ambiente).
fn choose(own: Option<ChannelSelection>, global: impl FnOnce() -> SelectionRead, environment: impl FnOnce() -> Option<String>) -> Choice {
    match own {
        Some(ChannelSelection::Unstable) => return Choice::Unstable,
        Some(ChannelSelection::NoEffect) => return Choice::Template(None),
        Some(ChannelSelection::Path(path)) => return Choice::Template(Some(path)),
        Some(ChannelSelection::Inherit) | None => {}
    }
    match global() {
        SelectionRead::Unstable => Choice::Unstable,
        SelectionRead::Path(path) => Choice::Template(Some(path)),
        SelectionRead::Cleared => Choice::Template(environment()),
    }
}

/// A época atual da escolha. Barata quando nada mudou; relê a escolha quando ela foi republicada.
pub fn epoch() -> u64 {
    let global = crate::control::reader();
    let own = crate::control::channel();
    // As gerações dos dois canais viram uma só chave: qualquer publicação em qualquer um a muda.
    let generation = u64::from(global.map_or(ABSENT, |r| r.selection_generation())) << 32 | u64::from(own.map_or(ABSENT, |c| c.selection_generation()));
    if generation == LAST_GENERATION.load(Ordering::Acquire) {
        return EPOCH.load(Ordering::Acquire);
    }
    let template = match choose(own.map(|c| c.read_selection()), || global.map_or(SelectionRead::Cleared, |r| r.read_selection()), from_environment) {
        Choice::Unstable => return EPOCH.load(Ordering::Acquire),
        Choice::Template(template) => template,
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

    fn path(p: &str) -> SelectionRead {
        SelectionRead::Path(p.into())
    }

    fn template(c: Choice) -> Option<String> {
        match c {
            Choice::Template(t) => t,
            Choice::Unstable => panic!("instável"),
        }
    }

    #[test]
    fn the_games_own_choice_wins_over_the_global_one() {
        let own = Some(ChannelSelection::Path("/meu".into()));
        assert_eq!(template(choose(own, || path("/global"), || Some("/env".into()))), Some("/meu".into()));
    }

    #[test]
    fn without_an_own_choice_the_global_and_then_the_environment_apply() {
        for own in [None, Some(ChannelSelection::Inherit)] {
            assert_eq!(template(choose(own, || path("/global"), || Some("/env".into()))), Some("/global".into()));
        }
        assert_eq!(template(choose(None, || SelectionRead::Cleared, || Some("/env".into()))), Some("/env".into()));
        assert_eq!(template(choose(None, || SelectionRead::Cleared, || None)), None);
    }

    #[test]
    fn no_effect_for_this_game_beats_everything_and_skips_the_environment() {
        assert_eq!(template(choose(Some(ChannelSelection::NoEffect), || path("/global"), || Some("/env".into()))), None);
    }

    #[test]
    fn a_torn_read_in_either_channel_means_try_again_later() {
        assert_eq!(choose(Some(ChannelSelection::Unstable), || path("/g"), || None), Choice::Unstable);
        assert_eq!(choose(None, || SelectionRead::Unstable, || None), Choice::Unstable);
        // O próprio jogo decidido: uma leitura instável do global não importa.
        assert_eq!(template(choose(Some(ChannelSelection::NoEffect), || SelectionRead::Unstable, || None)), None);
    }

    #[test]
    fn a_template_without_placeholders_is_a_literal_path() {
        assert_eq!(expand("/c/Sepia-500x500", 1920, 1080), PathBuf::from("/c/Sepia-500x500"));
    }
}
