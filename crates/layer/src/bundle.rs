//! Qual bundle de efeito cada swapchain executa.
//!
//! A escolha é um *modelo* de caminho: o que o app de controle publicou para este jogo (canal próprio)
//! ou, na falta, para todos (canal global, `lrctl use`) e, por último, a variável `LINUX_RESHADE_BUNDLE`. O modelo pode conter `{W}` e `{H}`, trocados pela
//! resolução do swapchain, e assim um mesmo efeito escolhido serve a jogos de resoluções diferentes
//! (cada resolução tem o seu bundle compilado).
//!
//! Com um *plano* publicado no canal do jogo (ver `ipc::plan`) vale o plano: uma lista de etapas, cada uma com
//! o seu modelo de caminho, a técnica e o bloco de uniforms. Sem plano, vale a escolha única de antes, que
//! é uma cadeia de uma etapa.
//!
//! O custo por frame é uma leitura atômica (a geração da seleção). O trabalho de verdade (ler o
//! disco, validar) só acontece quando a escolha é publicada de novo, e isso sobe a *época*: cada
//! swapchain compara a época que viu com a atual e, se mudou, resolve seu bundle outra vez.

use crate::log::log;
use fx_bundle::Bundle;
use ipc::{ChannelSelection, Plan, PlanRead, SelectionRead};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

/// Valor de `LAST_GENERATION` antes de a seleção ter sido lida.
const NEVER: u64 = u64::MAX;
/// A geração de um canal que ainda não existe. Gerações de verdade são `seq / 2` e nunca chegam perto.
const ABSENT: u32 = u32::MAX - 1;

/// Última geração da seleção que já foi processada.
static LAST_GENERATION: AtomicU64 = AtomicU64::new(NEVER);
/// Última geração do plano do jogo que já foi processada (`ABSENT` se não há canal).
static LAST_PLAN_GENERATION: AtomicU32 = AtomicU32::new(ABSENT);
/// Sobe a cada vez que a escolha é publicada de novo. 0 = nunca foi lida.
static EPOCH: AtomicU64 = AtomicU64::new(0);

struct State {
    /// O modelo de caminho vigente, se houver (sem plano).
    template: Option<String>,
    /// O plano do jogo, se o app de controle publicou um: manda sobre `template`.
    plan: Option<Plan>,
    /// Bundles já carregados nesta época, por diretório. É esvaziado a cada nova publicação, para um
    /// efeito recompilado (mesmo diretório, conteúdo novo) ser lido de novo.
    cache: HashMap<PathBuf, Arc<Bundle>>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(|| Mutex::new(State { template: None, plan: None, cache: HashMap::new() }));

/// De onde vêm os uniforms de uma etapa.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepParams {
    /// Os blocos do app de controle (canal do jogo e global), como sempre foi com a escolha única.
    Legacy,
    /// Os do plano: a imagem do buffer de uniforms, ou os padrões do manifesto se vazia ou de outro efeito.
    Plan(Option<Vec<u8>>),
}

/// Uma etapa pronta para construir: o bundle da resolução, a técnica e de onde vêm os uniforms.
#[derive(Clone)]
pub struct ResolvedStep {
    pub bundle: Arc<Bundle>,
    pub technique: usize,
    pub params: StepParams,
}

/// O que a escolha (ou o plano) dá para um swapchain de certo tamanho.
pub enum Resolved {
    /// Nenhum efeito escolhido (ou plano sem etapas).
    None,
    /// Há efeito escolhido, mas ainda faltam bundles para esta resolução (o app de controle os compila).
    Waiting(Vec<PathBuf>),
    /// A cadeia, na ordem de execução.
    Ready(Vec<ResolvedStep>),
    /// Um diretório existe, mas o bundle não carrega, ou a técnica pedida não existe.
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
    // As gerações dos canais viram uma chave: qualquer publicação em qualquer um a muda.
    let generation = u64::from(global.map_or(ABSENT, |r| r.selection_generation())) << 32 | u64::from(own.map_or(ABSENT, |c| c.selection_generation()));
    let plan_generation = own.map_or(ABSENT, |c| c.plan_generation());
    if generation == LAST_GENERATION.load(Ordering::Acquire) && plan_generation == LAST_PLAN_GENERATION.load(Ordering::Acquire) {
        return EPOCH.load(Ordering::Acquire);
    }
    let plan = match own.map(|c| c.read_plan()) {
        Some(PlanRead::Unstable) => return EPOCH.load(Ordering::Acquire),
        Some(PlanRead::Plan(plan)) => Some(plan),
        Some(PlanRead::None) | None => None,
    };
    let template = if plan.is_some() {
        None
    } else {
        match choose(own.map(|c| c.read_selection()), || global.map_or(SelectionRead::Cleared, |r| r.read_selection()), from_environment) {
            Choice::Unstable => return EPOCH.load(Ordering::Acquire),
            Choice::Template(template) => template,
        }
    };
    let mut state = STATE.lock().unwrap();
    if state.plan != plan {
        match &plan {
            Some(p) => log!("plano do jogo: {} etapa(s)", p.steps.len()),
            None => log!("sem plano do jogo"),
        }
        state.plan = plan;
    }
    if state.template != template {
        match &template {
            Some(t) => log!("efeito escolhido: {t}"),
            None if state.plan.is_none() => log!("nenhum efeito escolhido"),
            None => {}
        }
        state.template = template;
    }
    state.cache.clear();
    LAST_GENERATION.store(generation, Ordering::Release);
    LAST_PLAN_GENERATION.store(plan_generation, Ordering::Release);
    EPOCH.fetch_add(1, Ordering::AcqRel) + 1
}

enum Loaded {
    Ready(Arc<Bundle>),
    Missing(PathBuf),
    Invalid(String),
}

/// O bundle de um diretório, do cache desta época ou do disco.
fn load(state: &mut State, dir: PathBuf) -> Loaded {
    if let Some(bundle) = state.cache.get(&dir) {
        return Loaded::Ready(bundle.clone());
    }
    if !dir.join("manifest.json").is_file() {
        return Loaded::Missing(dir);
    }
    match Bundle::load(&dir) {
        Ok(bundle) => {
            let t = bundle.manifest.target;
            log!("bundle carregado: {} ({}x{}, {} técnica(s))", dir.display(), t.width, t.height, bundle.manifest.techniques.len());
            let bundle = Arc::new(bundle);
            state.cache.insert(dir, bundle.clone());
            Loaded::Ready(bundle)
        }
        Err(e) => {
            log!("bundle {} recusado: {e}", dir.display());
            Loaded::Invalid(e.to_string())
        }
    }
}

/// A cadeia vigente para um swapchain de `width` x `height`.
pub fn resolve(width: u32, height: u32) -> Resolved {
    let mut state = STATE.lock().unwrap();
    if let Some(plan) = state.plan.clone() {
        return resolve_plan(&mut state, &plan, width, height);
    }
    let Some(template) = state.template.clone() else { return Resolved::None };
    match load(&mut state, expand(&template, width, height)) {
        Loaded::Ready(bundle) => Resolved::Ready(vec![ResolvedStep { bundle, technique: 0, params: StepParams::Legacy }]),
        Loaded::Missing(dir) => Resolved::Waiting(vec![dir]),
        Loaded::Invalid(why) => Resolved::Invalid(why),
    }
}

fn resolve_plan(state: &mut State, plan: &Plan, width: u32, height: u32) -> Resolved {
    let (mut steps, mut missing) = (Vec::new(), Vec::new());
    for (n, step) in plan.steps.iter().enumerate() {
        let bundle = match load(state, expand(&step.template, width, height)) {
            Loaded::Ready(bundle) => bundle,
            Loaded::Missing(dir) => {
                missing.push(dir);
                continue;
            }
            Loaded::Invalid(why) => return Resolved::Invalid(format!("etapa {n}: {why}")),
        };
        let technique = if step.technique.is_empty() {
            0
        } else {
            match postfx::chain::technique_index(&bundle, &step.technique) {
                Some(i) => i,
                None => return Resolved::Invalid(format!("etapa {n}: o efeito não tem a técnica {:?}", step.technique)),
            }
        };
        // Os uniforms do plano só valem para o efeito que o controlador tinha em mente.
        let uniforms = (!step.uniforms.is_empty() && step.params_id == bundle.params_id()).then(|| step.uniforms.clone());
        if !step.uniforms.is_empty() && uniforms.is_none() {
            log!("etapa {n}: uniforms do plano são de outro efeito; valem os padrões");
        }
        steps.push(ResolvedStep { bundle, technique, params: StepParams::Plan(uniforms) });
    }
    match (missing.is_empty(), steps.is_empty()) {
        (false, _) => Resolved::Waiting(missing),
        (true, true) => Resolved::None,
        (true, false) => Resolved::Ready(steps),
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
