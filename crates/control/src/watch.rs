//! O vigia: mantém compilados os bundles que os jogos em execução precisam.
//!
//! O que precisa ser compilado sai de uma função pura ([`plan`]): dada a escolha de efeito e as layers
//! vivas, quais pares (efeito, resolução) faltam ou estão desatualizados. [`tick`] executa esse plano e,
//! se compilou algo, republica a escolha, o que faz as layers resolverem seus bundles de novo.

use crate::compiler::{CompileError, Compiler, Ensured};
use ipc::{EffectState, LiveLayer, SelectionRead};
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

/// O que compilar agora. Só há trabalho quando o efeito foi escolhido *por nome* (um diretório
/// literal não é nosso para recompilar).
pub fn plan(compiler: &Compiler, selection: &SelectionRead, layers: &[LiveLayer], failures: &Failures) -> Vec<Job> {
    let SelectionRead::Path(template) = selection else { return Vec::new() };
    let Some(name) = compiler.name_from_template(template) else { return Vec::new() };
    let resolutions: BTreeSet<(u32, u32)> = layers
        .iter()
        .flat_map(|l| &l.status.swapchains)
        .filter(|s| !matches!(s.effect, EffectState::Unavailable(_)) && s.width > 0 && s.height > 0)
        .map(|s| (s.width, s.height))
        .collect();
    let edited = compiler.source_modified(&name);
    resolutions
        .into_iter()
        .map(|(width, height)| Job { name: name.clone(), width, height })
        .filter(|job| !compiler.is_current(&job.name, job.width, job.height))
        // Uma falha anterior só é repetida se o `.fx` mudou desde então.
        .filter(|job| failures.get(job).is_none_or(|at_failure| *at_failure != edited))
        .collect()
}

/// As resoluções que jogos abertos usam do efeito escolhido: o que a poda do cache não pode apagar.
pub fn in_use(compiler: &Compiler, selection: &SelectionRead, layers: &[LiveLayer]) -> std::collections::HashSet<(String, u32, u32)> {
    let SelectionRead::Path(template) = selection else { return Default::default() };
    let Some(name) = compiler.name_from_template(template) else { return Default::default() };
    layers.iter().flat_map(|l| &l.status.swapchains).map(|s| (name.clone(), s.width, s.height)).collect()
}

/// O resultado de um `tick`, para o chamador mostrar ao usuário.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub compiled: Vec<Job>,
    pub failed: Vec<(Job, String)>,
}

/// Executa o plano uma vez. Se algo foi compilado, republica a escolha para as layers a relerem.
pub fn tick(compiler: &Compiler, state: &ipc::Writer, layers: &[LiveLayer], failures: &mut Failures) -> Report {
    let selection = state.selection();
    let mut report = Report::default();
    for job in plan(compiler, &selection, layers, failures) {
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
        if let SelectionRead::Path(template) = &selection {
            // Mesmo caminho, nova publicação: a geração muda e cada layer relê o que lhe cabe.
            let _ = state.publish_selection(template);
        }
    }
    report
}

fn describe(e: &CompileError) -> String {
    e.to_string()
}

/// Vigia para sempre: a cada [`INTERVAL`] olha as layers vivas e compila o que falta, contando o que fez.
pub fn run(compiler: &Compiler, state: &ipc::Writer) -> ! {
    let mut failures = Failures::new();
    let mut last_prune = std::time::Instant::now();
    loop {
        let layers = ipc::live_layers().unwrap_or_default();
        let report = tick(compiler, state, &layers, &mut failures);
        if last_prune.elapsed() >= PRUNE_EVERY {
            last_prune = std::time::Instant::now();
            let removed = compiler.prune(&in_use(compiler, &state.selection(), &layers), KEEP_PER_EFFECT);
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
        std::thread::sleep(INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::testing::{OK_TOOL, calls, fake};
    use ipc::{LayerStatus, SwapchainStatus};
    use std::fs;
    use std::path::Path;

    fn layer(app: &str, swapchains: &[(u32, u32, EffectState)]) -> LiveLayer {
        LiveLayer {
            id: format!("layer-{app}"),
            status: LayerStatus {
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
        let jobs = plan(&f.compiler, &by_name(&f.compiler, "Sepia"), &layers, &Failures::new());
        assert_eq!(jobs, vec![job(1914, 1048), job(1920, 1080)], "duas resoluções, sem repetir a que dois jogos têm");
    }

    #[test]
    fn nothing_to_do_without_a_named_selection() {
        let f = fake("plan-none", OK_TOOL);
        let layers = [layer("A", &[(100, 100, EffectState::Waiting)])];
        let no_failures = Failures::new();
        assert!(plan(&f.compiler, &SelectionRead::Cleared, &layers, &no_failures).is_empty());
        assert!(plan(&f.compiler, &SelectionRead::Unstable, &layers, &no_failures).is_empty());
        let literal = SelectionRead::Path(f.compiler.bundle_dir("Sepia", 100, 100).to_string_lossy().into_owned());
        assert!(plan(&f.compiler, &literal, &layers, &no_failures).is_empty(), "um diretório literal não é nosso para recompilar");
        let outside = SelectionRead::Path("/outro/Sepia-{W}x{H}".into());
        assert!(plan(&f.compiler, &outside, &layers, &no_failures).is_empty());
    }

    #[test]
    fn swapchains_that_cannot_take_an_effect_are_not_compiled_for() {
        let f = fake("plan-unavailable", OK_TOOL);
        let layers = [layer("A", &[(640, 480, EffectState::Unavailable("sRGB".into())), (800, 600, EffectState::Idle)])];
        assert_eq!(plan(&f.compiler, &by_name(&f.compiler, "Sepia"), &layers, &Failures::new()), vec![job(800, 600)]);
    }

    #[test]
    fn an_up_to_date_bundle_needs_no_job() {
        let f = fake("plan-current", OK_TOOL);
        f.compiler.ensure("Sepia", 800, 600).unwrap();
        let layers = [layer("A", &[(800, 600, EffectState::Ready)])];
        assert!(plan(&f.compiler, &by_name(&f.compiler, "Sepia"), &layers, &Failures::new()).is_empty());
    }

    #[test]
    fn a_failure_is_not_retried_until_the_source_changes() {
        let f = fake("plan-failure", OK_TOOL);
        let layers = [layer("A", &[(800, 600, EffectState::Waiting)])];
        let selection = by_name(&f.compiler, "Sepia");
        let mut failures = Failures::new();
        failures.insert(job(800, 600), f.compiler.source_modified("Sepia"));
        assert!(plan(&f.compiler, &selection, &layers, &failures).is_empty(), "mesma versão do .fx que já falhou");

        let later = SystemTime::now() + Duration::from_secs(60);
        fs::File::options().write(true).open(f.root.join("effects/Sepia.fx")).unwrap().set_modified(later).unwrap();
        assert_eq!(plan(&f.compiler, &selection, &layers, &failures), vec![job(800, 600)], "o .fx foi editado: tenta de novo");
    }

    #[test]
    fn in_use_is_every_resolution_of_the_selected_effect_on_open_games() {
        let f = fake("in-use", OK_TOOL);
        let layers = [layer("A", &[(100, 100, EffectState::Ready), (200, 200, EffectState::Idle)]), layer("B", &[(100, 100, EffectState::Ready)])];
        let used = in_use(&f.compiler, &by_name(&f.compiler, "Sepia"), &layers);
        assert_eq!(used, std::collections::HashSet::from([("Sepia".to_string(), 100, 100), ("Sepia".to_string(), 200, 200)]));
        assert!(in_use(&f.compiler, &SelectionRead::Cleared, &layers).is_empty());
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
}
