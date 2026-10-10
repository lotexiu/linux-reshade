//! O vigia: mantém compilados os bundles que os jogos em execução precisam.
//!
//! Cada layer segue a escolha de efeito do próprio jogo, se o usuário fez uma, e senão a global
//! ([`effective`]). O que precisa ser compilado sai de uma função pura ([`plan`]): dadas essas escolhas e as
//! layers vivas, quais pares (efeito, resolução) faltam ou estão desatualizados. [`tick`] executa o plano
//! e, se compilou algo, republica a escolha de quem precisava dele, o que faz aquelas layers resolverem
//! seus bundles de novo.

use crate::compiler::{CompileError, Compiler, Ensured};
use ipc::{ChannelSelection, EffectState, LiveLayer, SelectionRead};
use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, SystemTime};

/// Intervalo entre duas olhadas nas layers. Olhar é barato (listar um diretório e ler alguns bytes), e
/// quanto menor, mais cedo o efeito aparece depois de uma mudança de tamanho de janela.
pub const INTERVAL: Duration = Duration::from_millis(200);
/// A poda do cache roda no máximo com esta frequência.
const PRUNE_EVERY: Duration = Duration::from_secs(30);
/// Quantos bundles recentes de cada efeito ficam no cache (fora os que um jogo aberto usa).
const KEEP_PER_EFFECT: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Job {
    pub name: String,
    pub width: u32,
    pub height: u32,
}

/// Falhas já vistas: não se insiste enquanto o `.fx` não for editado.
pub type Failures = HashMap<Job, Option<SystemTime>>;

/// A escolha de efeito que vale para uma layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted<'a> {
    pub layer: &'a LiveLayer,
    /// O modelo de caminho vigente (`None`: nenhum efeito).
    pub template: Option<String>,
    /// A escolha é do próprio jogo, e não herdada da global.
    pub own: bool,
}

/// A escolha de cada layer: a própria, se tem; senão a global. Uma leitura instável da própria vale como
/// "sem escolha própria" neste instante (a próxima olhada acerta).
pub fn effective<'a>(state: &ipc::Writer, layers: &'a [LiveLayer]) -> Vec<Wanted<'a>> {
    let global = match state.selection() {
        SelectionRead::Path(template) => Some(template),
        SelectionRead::Cleared | SelectionRead::Unstable => None,
    };
    layers
        .iter()
        .map(|layer| match state.layer_selection(&layer.id) {
            ChannelSelection::Path(template) => Wanted { layer, template: Some(template), own: true },
            ChannelSelection::NoEffect => Wanted { layer, template: None, own: true },
            ChannelSelection::Inherit | ChannelSelection::Unstable => Wanted { layer, template: global.clone(), own: false },
        })
        .collect()
}

/// O nome do efeito de uma escolha, quando foi feita *por nome* (um diretório literal não é nosso para
/// recompilar).
fn named(compiler: &Compiler, wanted: &Wanted) -> Option<String> {
    compiler.name_from_template(wanted.template.as_deref()?)
}

/// As resoluções em que a layer pode receber um efeito.
fn resolutions(layer: &LiveLayer) -> impl Iterator<Item = (u32, u32)> + '_ {
    layer.status.swapchains.iter().filter(|s| !matches!(s.effect, EffectState::Unavailable(_)) && s.width > 0 && s.height > 0).map(|s| (s.width, s.height))
}

/// O que compilar agora: cada efeito escolhido por nome, em cada resolução de cada layer que o usa.
pub fn plan(compiler: &Compiler, wanted: &[Wanted], failures: &Failures) -> Vec<Job> {
    let mut jobs: BTreeSet<Job> = BTreeSet::new();
    for w in wanted {
        let Some(name) = named(compiler, w) else { continue };
        for (width, height) in resolutions(w.layer) {
            jobs.insert(Job { name: name.clone(), width, height });
        }
    }
    jobs.into_iter()
        .filter(|job| !compiler.is_current(&job.name, job.width, job.height))
        // Uma falha anterior só é repetida se o `.fx` mudou desde então.
        .filter(|job| failures.get(job).is_none_or(|at_failure| *at_failure != compiler.source_modified(&job.name)))
        .collect()
}

/// As resoluções que jogos abertos usam dos efeitos escolhidos: o que a poda do cache não pode apagar.
pub fn in_use(compiler: &Compiler, wanted: &[Wanted]) -> std::collections::HashSet<(String, u32, u32)> {
    let mut used = std::collections::HashSet::new();
    for w in wanted {
        let Some(name) = named(compiler, w) else { continue };
        used.extend(w.layer.status.swapchains.iter().map(|s| (name.clone(), s.width, s.height)));
    }
    used
}

/// O resultado de um `tick`, para o chamador mostrar ao usuário.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub compiled: Vec<Job>,
    pub failed: Vec<(Job, String)>,
}

/// Executa o plano uma vez. Se algo foi compilado, republica a escolha de quem precisava dele (o canal
/// global para as layers que o seguem, o canal próprio para as que têm escolha própria), e só dessas.
pub fn tick(compiler: &Compiler, state: &ipc::Writer, layers: &[LiveLayer], failures: &mut Failures) -> Report {
    let wanted = effective(state, layers);
    let mut report = Report::default();
    for job in plan(compiler, &wanted, failures) {
        match compiler.ensure(&job.name, job.width, job.height) {
            Ok(Ensured::Compiled(_)) => {
                failures.remove(&job);
                report.compiled.push(job);
            }
            Ok(Ensured::Cached(_)) => {}
            Err(e) => {
                let edited = compiler.source_modified(&job.name);
                failures.insert(job.clone(), edited);
                report.failed.push((job, describe(&e)));
            }
        }
    }
    if !report.compiled.is_empty() {
        republish(compiler, state, &wanted, &report.compiled);
    }
    report
}

/// Mesmo caminho, nova publicação: a geração muda e cada layer relê o que lhe cabe.
fn republish(compiler: &Compiler, state: &ipc::Writer, wanted: &[Wanted], compiled: &[Job]) {
    let mut global = false;
    for w in wanted {
        let (Some(template), Some(name)) = (&w.template, named(compiler, w)) else { continue };
        let touched = w.layer.status.swapchains.iter().any(|s| compiled.iter().any(|j| j.name == name && j.width == s.width && j.height == s.height));
        if !touched {
            continue;
        }
        if w.own {
            if let Ok(channel) = state.channel(&w.layer.id) {
                let _ = ipc::SelectionStore::publish_selection(&channel, template);
            }
        } else {
            global = true;
        }
    }
    if global && let SelectionRead::Path(template) = state.selection() {
        let _ = state.publish_selection(&template);
    }
}

fn describe(e: &CompileError) -> String {
    e.to_string()
}

/// Vigia para sempre: a cada [`INTERVAL`] olha as layers vivas e compila o que falta, contando o que fez.
pub fn run(compiler: &Compiler, state: &ipc::Writer, presets_dir: Option<std::path::PathBuf>, global_macros: &str) -> ! {
    let mut failures = Failures::new();
    let mut games = presets_dir.map(|dir| crate::games::Games::new(dir).with_global_macros(global_macros));
    let mut last_prune = std::time::Instant::now();
    loop {
        let layers = ipc::live_layers().unwrap_or_default();
        let report = tick(compiler, state, &layers, &mut failures);
        let games_report = games.as_mut().map(|g| g.tick(compiler, state, &layers, &mut failures));
        if last_prune.elapsed() >= PRUNE_EVERY {
            last_prune = std::time::Instant::now();
            state.prune_channels();
            let mut used = in_use(compiler, &effective(state, &layers));
            if let Some(g) = &games {
                used.extend(g.in_use(&layers));
            }
            let removed = compiler.prune(&used, KEEP_PER_EFFECT);
            if removed > 0 {
                println!("cache: {removed} bundle(s) antigo(s) removido(s)");
            }
        }
        for job in &report.compiled {
            println!("compilado {} para {}x{}", job.name, job.width, job.height);
        }
        for (job, why) in &report.failed {
            eprintln!("falhou ao compilar {} para {}x{}: {why}", job.name, job.width, job.height);
        }
        if let Some(r) = games_report {
            for (game, steps) in &r.published {
                println!("plano de {game} publicado: {steps} etapa(s)");
            }
            for job in &r.compiled {
                println!("compilado {} para {}x{}", job.name, job.width, job.height);
            }
            for (who, why) in &r.warnings {
                eprintln!("aviso ({who}): {why}");
            }
        }
        std::thread::sleep(INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::testing::{OK_TOOL, calls, fake};
    use ipc::{LayerStatus, SelectionStore, SwapchainStatus};
    use std::fs;
    use std::path::Path;

    fn layer(app: &str, swapchains: &[(u32, u32, EffectState)]) -> LiveLayer {
        LiveLayer {
            id: format!("layer-{app}"),
            status: LayerStatus {
                depth: Vec::new(),
                techniques: Vec::new(),
                exe: String::new(),
                app: app.into(),
                swapchains: swapchains
                    .iter()
                    .map(|(w, h, e)| SwapchainStatus { width: *w, height: *h, format: 44, images: 3, effect: e.clone() })
                    .collect(),
            },
        }
    }

    fn by_name(compiler: &Compiler, name: &str) -> SelectionRead {
        SelectionRead::Path(compiler.template(name))
    }

    /// Todas as layers seguindo a mesma escolha (como era antes dos canais por jogo).
    fn all<'a>(selection: &SelectionRead, layers: &'a [LiveLayer]) -> Vec<Wanted<'a>> {
        let template = match selection {
            SelectionRead::Path(t) => Some(t.clone()),
            _ => None,
        };
        layers.iter().map(|layer| Wanted { layer, template: template.clone(), own: false }).collect()
    }

    fn state(root: &Path) -> ipc::Writer {
        ipc::Writer::create_at(&root.join("state-dir/state")).unwrap()
    }

    fn job(w: u32, h: u32) -> Job {
        Job { name: "Sepia".into(), width: w, height: h }
    }

    #[test]
    fn plans_one_job_per_distinct_resolution() {
        let f = fake("plan-basic", OK_TOOL);
        let layers = [
            layer("A", &[(1920, 1080, EffectState::Waiting)]),
            layer("B", &[(1920, 1080, EffectState::Idle), (1914, 1048, EffectState::Waiting)]),
        ];
        let jobs = plan(&f.compiler, &all(&by_name(&f.compiler, "Sepia"), &layers), &Failures::new());
        assert_eq!(jobs, vec![job(1914, 1048), job(1920, 1080)], "duas resoluções, sem repetir a que dois jogos têm");
    }

    #[test]
    fn nothing_to_do_without_a_named_selection() {
        let f = fake("plan-none", OK_TOOL);
        let layers = [layer("A", &[(100, 100, EffectState::Waiting)])];
        let no_failures = Failures::new();
        assert!(plan(&f.compiler, &all(&SelectionRead::Cleared, &layers), &no_failures).is_empty());
        assert!(plan(&f.compiler, &all(&SelectionRead::Unstable, &layers), &no_failures).is_empty());
        let literal = SelectionRead::Path(f.compiler.bundle_dir("Sepia", 100, 100).to_string_lossy().into_owned());
        assert!(plan(&f.compiler, &all(&literal, &layers), &no_failures).is_empty(), "um diretório literal não é nosso para recompilar");
        let outside = SelectionRead::Path("/outro/Sepia-{W}x{H}".into());
        assert!(plan(&f.compiler, &all(&outside, &layers), &no_failures).is_empty());
    }

    #[test]
    fn swapchains_that_cannot_take_an_effect_are_not_compiled_for() {
        let f = fake("plan-unavailable", OK_TOOL);
        let layers = [layer("A", &[(640, 480, EffectState::Unavailable("sRGB".into())), (800, 600, EffectState::Idle)])];
        assert_eq!(plan(&f.compiler, &all(&by_name(&f.compiler, "Sepia"), &layers), &Failures::new()), vec![job(800, 600)]);
    }

    #[test]
    fn an_up_to_date_bundle_needs_no_job() {
        let f = fake("plan-current", OK_TOOL);
        f.compiler.ensure("Sepia", 800, 600).unwrap();
        let layers = [layer("A", &[(800, 600, EffectState::Ready)])];
        assert!(plan(&f.compiler, &all(&by_name(&f.compiler, "Sepia"), &layers), &Failures::new()).is_empty());
    }

    #[test]
    fn a_failure_is_not_retried_until_the_source_changes() {
        let f = fake("plan-failure", OK_TOOL);
        let layers = [layer("A", &[(800, 600, EffectState::Waiting)])];
        let selection = by_name(&f.compiler, "Sepia");
        let mut failures = Failures::new();
        failures.insert(job(800, 600), f.compiler.source_modified("Sepia"));
        assert!(plan(&f.compiler, &all(&selection, &layers), &failures).is_empty(), "mesma versão do .fx que já falhou");

        let later = SystemTime::now() + Duration::from_secs(60);
        fs::File::options().write(true).open(f.root.join("effects/Sepia.fx")).unwrap().set_modified(later).unwrap();
        assert_eq!(plan(&f.compiler, &all(&selection, &layers), &failures), vec![job(800, 600)], "o .fx foi editado: tenta de novo");
    }

    #[test]
    fn in_use_is_every_resolution_of_the_selected_effect_on_open_games() {
        let f = fake("in-use", OK_TOOL);
        let layers = [layer("A", &[(100, 100, EffectState::Ready), (200, 200, EffectState::Idle)]), layer("B", &[(100, 100, EffectState::Ready)])];
        let used = in_use(&f.compiler, &all(&by_name(&f.compiler, "Sepia"), &layers));
        assert_eq!(used, std::collections::HashSet::from([("Sepia".to_string(), 100, 100), ("Sepia".to_string(), 200, 200)]));
        assert!(in_use(&f.compiler, &all(&SelectionRead::Cleared, &layers)).is_empty());
    }

    #[test]
    fn tick_compiles_what_is_missing_and_republishes_the_selection() {
        let f = fake("tick", OK_TOOL);
        let control = state(&f.root);
        let template = f.compiler.template("Sepia");
        control.publish_selection(&template).unwrap();
        let reader = ipc::Reader::open_at(&f.root.join("state-dir/state")).unwrap().unwrap();
        let generation = reader.selection_generation();

        let layers = [layer("A", &[(1920, 1080, EffectState::Waiting)]), layer("B", &[(1914, 1048, EffectState::Waiting)])];
        let mut failures = Failures::new();
        let report = tick(&f.compiler, &control, &layers, &mut failures);
        assert_eq!(report.compiled, vec![job(1914, 1048), job(1920, 1080)]);
        assert!(report.failed.is_empty());
        assert_eq!(calls(&f).len(), 2);
        assert!(f.compiler.bundle_dir("Sepia", 1920, 1080).join("manifest.json").is_file());
        assert_ne!(reader.selection_generation(), generation, "a escolha foi republicada para as layers relerem");
        assert_eq!(reader.read_selection(), SelectionRead::Path(template), "e é a mesma escolha");

        // Segundo tick: nada mais a fazer, e a escolha não é republicada à toa.
        let generation = reader.selection_generation();
        assert_eq!(tick(&f.compiler, &control, &layers, &mut failures), Report::default());
        assert_eq!(reader.selection_generation(), generation);
    }

    #[test]
    fn tick_remembers_failures_and_does_not_hammer_the_tool() {
        let f = fake("tick-fail", "#!/bin/sh\necho 'erro' >&2\nexit 1\n");
        let control = state(&f.root);
        control.publish_selection(&f.compiler.template("Sepia")).unwrap();
        let layers = [layer("A", &[(800, 600, EffectState::Waiting)])];
        let mut failures = Failures::new();

        let first = tick(&f.compiler, &control, &layers, &mut failures);
        assert_eq!(first.failed.len(), 1);
        assert!(first.failed[0].1.contains("erro"), "{:?}", first.failed);
        let again = tick(&f.compiler, &control, &layers, &mut failures);
        assert_eq!(again, Report::default(), "não tenta de novo a mesma versão do .fx");
    }

    fn add_effect(f: &crate::compiler::testing::Fake, name: &str) {
        fs::write(f.root.join(format!("effects/{name}.fx")), "// de mentira").unwrap();
    }

    #[test]
    fn each_layer_follows_its_own_choice_or_the_global_one() {
        let f = fake("effective", OK_TOOL);
        add_effect(&f, "Vibrance");
        let control = state(&f.root);
        control.publish_selection(&f.compiler.template("Sepia")).unwrap();
        let layers = [layer("A", &[(800, 600, EffectState::Idle)]), layer("B", &[(800, 600, EffectState::Idle)]), layer("C", &[(800, 600, EffectState::Idle)])];
        control.channel("layer-B").unwrap().publish_selection(&f.compiler.template("Vibrance")).unwrap();
        control.channel("layer-C").unwrap().publish_no_effect();

        let wanted = effective(&control, &layers);
        let summary: Vec<_> = wanted.iter().map(|w| (w.layer.status.app.as_str(), w.template.as_deref().and_then(|t| f.compiler.name_from_template(t)), w.own)).collect();
        assert_eq!(summary, vec![("A", Some("Sepia".to_string()), false), ("B", Some("Vibrance".to_string()), true), ("C", None, true)]);

        // O plano compila cada efeito só onde alguém o quer; a layer sem efeito não pede nada.
        let jobs = plan(&f.compiler, &wanted, &Failures::new());
        assert_eq!(jobs.iter().map(|j| j.name.as_str()).collect::<Vec<_>>(), ["Sepia", "Vibrance"]);
        let used = in_use(&f.compiler, &wanted);
        assert_eq!(used, std::collections::HashSet::from([("Sepia".to_string(), 800, 600), ("Vibrance".to_string(), 800, 600)]));
    }

    #[test]
    fn an_own_choice_works_even_when_nothing_is_chosen_globally() {
        let f = fake("effective-own-only", OK_TOOL);
        let control = state(&f.root);
        let layers = [layer("A", &[(640, 480, EffectState::Idle)]), layer("B", &[(640, 480, EffectState::Idle)])];
        control.channel("layer-A").unwrap().publish_selection(&f.compiler.template("Sepia")).unwrap();
        let wanted = effective(&control, &layers);
        assert_eq!(plan(&f.compiler, &wanted, &Failures::new()), vec![job(640, 480)]);
        assert_eq!(wanted[1].template, None, "o outro jogo continua sem efeito");
    }

    #[test]
    fn tick_republishes_only_the_channels_that_needed_the_new_bundle() {
        let f = fake("tick-own", OK_TOOL);
        add_effect(&f, "Vibrance");
        let control = state(&f.root);
        control.publish_selection(&f.compiler.template("Sepia")).unwrap();
        let global = ipc::Reader::open_at(&f.root.join("state-dir/state")).unwrap().unwrap();
        // A tem 800x600 e segue a global; B tem 640x480 com escolha própria.
        let layers = [layer("A", &[(800, 600, EffectState::Waiting)]), layer("B", &[(640, 480, EffectState::Waiting)])];
        control.channel("layer-B").unwrap().publish_selection(&f.compiler.template("Vibrance")).unwrap();
        let own = ipc::ChannelReader::open_in(&f.root.join("state-dir"), "layer-B").unwrap().unwrap();
        let (g0, o0) = (global.selection_generation(), own.selection_generation());

        let mut failures = Failures::new();
        let report = tick(&f.compiler, &control, &layers, &mut failures);
        assert_eq!(report.compiled.len(), 2, "{report:?}");
        assert!(f.compiler.bundle_dir("Vibrance", 640, 480).join("manifest.json").is_file());
        assert!(!f.compiler.bundle_dir("Vibrance", 800, 600).exists(), "Vibrance só para o jogo que a escolheu");
        assert_ne!(global.selection_generation(), g0, "A segue a global: ela foi republicada");
        assert_ne!(own.selection_generation(), o0, "B precisava do seu: o canal dele foi republicado");
        assert_eq!(own.read_selection(), ipc::ChannelSelection::Path(f.compiler.template("Vibrance")));

        // Nada mais a fazer: nada é republicado.
        let (g1, o1) = (global.selection_generation(), own.selection_generation());
        assert_eq!(tick(&f.compiler, &control, &layers, &mut failures), Report::default());
        assert_eq!((global.selection_generation(), own.selection_generation()), (g1, o1));
    }

    #[test]
    fn a_game_with_its_own_choice_does_not_republish_the_global_channel() {
        let f = fake("tick-own-only", OK_TOOL);
        let control = state(&f.root);
        let global = ipc::Reader::open_at(&f.root.join("state-dir/state")).unwrap().unwrap();
        let layers = [layer("A", &[(320, 240, EffectState::Waiting)])];
        control.channel("layer-A").unwrap().publish_selection(&f.compiler.template("Sepia")).unwrap();
        let g0 = global.selection_generation();
        let report = tick(&f.compiler, &control, &layers, &mut Failures::new());
        assert_eq!(report.compiled, vec![job(320, 240)]);
        assert_eq!(global.selection_generation(), g0);
    }
}
