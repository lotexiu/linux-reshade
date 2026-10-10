//! Os presets dos jogos abertos: o vigia olha cada layer viva, acha o preset do jogo, monta o plano
//! ([`game_plan`](crate::game_plan)) e o publica no canal dela. Relê quando o arquivo do preset muda e
//! compila as etapas para cada resolução de cada swapchain do jogo.
//!
//! Quando um jogo tem preset (o dele ou o global), o plano manda sobre a escolha única de efeito (`lrctl use`).
//! O id do jogo é o nome do executável que a layer informa (`RocketLeague` para `RocketLeague.exe`, também sob
//! Proton) e, se ela não o souber, o nome da aplicação no Vulkan.

use crate::compiler::{Compiler, Ensured};
use crate::game_plan;
use crate::watch::{Failures, Job};
use ipc::{EffectState, LiveLayer};
use preset::Preset;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::SystemTime;

/// O que já foi publicado para uma layer.
struct Published {
    file: PathBuf,
    modified: Option<SystemTime>,
    /// Efeitos do plano (nomes), para compilar em outras resoluções e para a poda do cache não os apagar.
    effects: Vec<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// `(jogo, etapas)` de cada plano publicado neste tick.
    pub published: Vec<(String, usize)>,
    pub compiled: Vec<Job>,
    /// `(jogo ou efeito, texto)`.
    pub warnings: Vec<(String, String)>,
}

pub struct Games {
    presets_dir: PathBuf,
    published: HashMap<String, Published>,
}

fn modified(path: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// As resoluções em que a layer pode receber efeitos.
fn resolutions(layer: &LiveLayer) -> Vec<(u32, u32)> {
    let mut out: Vec<(u32, u32)> = layer
        .status
        .swapchains
        .iter()
        .filter(|s| !matches!(s.effect, EffectState::Unavailable(_)) && s.width > 0 && s.height > 0)
        .map(|s| (s.width, s.height))
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

impl Games {
    pub fn new(presets_dir: PathBuf) -> Self {
        Self { presets_dir, published: HashMap::new() }
    }

    /// Os bundles que os planos publicados usam nos jogos abertos (o que a poda do cache não pode apagar).
    pub fn in_use(&self, layers: &[LiveLayer]) -> HashSet<(String, u32, u32)> {
        let mut used = HashSet::new();
        for layer in layers {
            let Some(p) = self.published.get(&layer.id) else { continue };
            for effect in &p.effects {
                used.extend(layer.status.swapchains.iter().map(|s| (effect.clone(), s.width, s.height)));
            }
        }
        used
    }

    /// Uma olhada: publica o plano de quem ainda não tem ou cujo preset mudou, e compila o que falta para as
    /// resoluções novas. `failures` é o mesmo registro do vigia (uma falha só se repete se o `.fx` mudar).
    pub fn tick(&mut self, compiler: &Compiler, state: &ipc::Writer, layers: &[LiveLayer], failures: &mut Failures) -> Report {
        let mut report = Report::default();
        self.published.retain(|id, _| layers.iter().any(|l| &l.id == id));
        for layer in layers {
            self.tick_layer(compiler, state, layer, failures, &mut report);
        }
        report
    }

    fn tick_layer(&mut self, compiler: &Compiler, state: &ipc::Writer, layer: &LiveLayer, failures: &mut Failures, report: &mut Report) {
        let game = layer.status.game_id();
        let found = preset::game::find(&self.presets_dir, game);
        let Some(found) = found else {
            // Perdeu o preset (apagado, ou game.ini mudou): a layer volta à escolha única.
            if self.published.remove(&layer.id).is_some() && let Ok(channel) = state.channel(&layer.id) {
                channel.clear_plan();
            }
            return;
        };
        let resolutions = resolutions(layer);
        let Some(&(width, height)) = resolutions.first() else { return }; // ainda sem swapchain: a próxima olhada
        let stamp = modified(&found.path);
        let changed = self.published.get(&layer.id).is_none_or(|p| p.file != found.path || p.modified != stamp);

        let mut republish = false;
        let mut plan = None;
        if changed {
            let text = match std::fs::read(&found.path) {
                Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                Err(e) => {
                    report.warnings.push((game.into(), format!("{}: {e}", found.path.display())));
                    return;
                }
            };
            let built = game_plan::build(compiler, &Preset::parse(&text), width, height);
            report.warnings.extend(built.warnings.into_iter().map(|w| (game.to_string(), w)));
            let effects = built.plan.steps.iter().filter_map(|s| compiler.name_from_template(&s.template)).collect();
            self.published.insert(layer.id.clone(), Published { file: found.path.clone(), modified: stamp, effects });
            plan = Some(built.plan);
            republish = true;
        }

        // Cada etapa em cada resolução do jogo (a primeira já foi compilada ao montar o plano).
        let effects = self.published.get(&layer.id).map(|p| p.effects.clone()).unwrap_or_default();
        for name in &effects {
            for &(w, h) in &resolutions {
                let job = Job { name: name.clone(), width: w, height: h };
                if compiler.is_current(name, w, h) || failures.get(&job).is_some_and(|at| *at == compiler.source_modified(name)) {
                    continue;
                }
                match compiler.ensure(name, w, h) {
                    Ok(Ensured::Compiled(_)) => {
                        failures.remove(&job);
                        report.compiled.push(job);
                        republish = true;
                    }
                    Ok(Ensured::Cached(_)) => {}
                    Err(e) => {
                        failures.insert(job.clone(), compiler.source_modified(name));
                        report.warnings.push((name.clone(), format!("{w}x{h}: {e}")));
                    }
                }
            }
        }

        if republish && let Ok(channel) = state.channel(&layer.id) {
            // Sem plano novo (só bundles novos), publica o mesmo de novo para a layer relê-los.
            let plan = match plan {
                Some(p) => p,
                None => match channel.plan() {
                    ipc::PlanRead::Plan(p) => p,
                    _ => return,
                },
            };
            match channel.publish_plan(&plan) {
                Ok(()) => report.published.push((game.to_string(), plan.steps.len())),
                Err(e) => report.warnings.push((game.to_string(), format!("não consegui publicar o plano: {e}"))),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::testing::{calls, fake_with_bundles};
    use ipc::{LayerStatus, SwapchainStatus};
    use std::fs;
    use std::time::Duration;

    fn layer(app: &str, sizes: &[(u32, u32)]) -> LiveLayer {
        LiveLayer {
            id: format!("layer-{app}"),
            status: LayerStatus {
                techniques: Vec::new(),
                exe: String::new(),
                app: app.into(),
                swapchains: sizes.iter().map(|&(w, h)| SwapchainStatus { width: w, height: h, format: 44, images: 3, effect: EffectState::Waiting }).collect(),
            },
        }
    }

    struct Env {
        f: crate::compiler::testing::Fake,
        state: ipc::Writer,
        presets: PathBuf,
    }

    fn env(name: &str) -> Env {
        let f = fake_with_bundles(name);
        let state = ipc::Writer::create_at(&f.root.join("state-dir/state")).unwrap();
        let presets = f.root.join("Presets");
        fs::create_dir_all(&presets).unwrap();
        Env { f, state, presets }
    }

    fn reader(e: &Env, layer: &str) -> ipc::ChannelReader {
        ipc::ChannelReader::open_in(&e.f.root.join("state-dir"), layer).unwrap().expect("canal criado")
    }

    #[test]
    fn publishes_the_game_preset_as_a_plan_and_compiles_every_resolution() {
        let e = env("games-publish");
        fs::create_dir_all(e.presets.join("Jogo")).unwrap();
        fs::write(e.presets.join("Jogo/game.ini"), "preset=Noturno.ini\n").unwrap();
        fs::write(e.presets.join("Jogo/Noturno.ini"), "Techniques=Sepia@Sepia.fx,Invert@Invert.fx\n\n[Sepia.fx]\nStrength=0.5\n").unwrap();
        fs::write(e.presets.join("preset.ini"), "Techniques=Invert@Invert.fx\n").unwrap();

        let layers = [layer("Jogo", &[(800, 600), (640, 480)])];
        let mut games = Games::new(e.presets.clone());
        let report = games.tick(&e.f.compiler, &e.state, &layers, &mut Failures::new());
        assert_eq!(report.published, vec![("Jogo".to_string(), 2)], "{report:?}");
        assert!(report.warnings.is_empty(), "{report:?}");
        for name in ["Sepia", "Invert"] {
            for (w, h) in [(800, 600), (640, 480)] {
                assert!(e.f.compiler.bundle_dir(name, w, h).join("manifest.json").is_file(), "{name} {w}x{h}");
            }
        }
        let ipc::PlanRead::Plan(plan) = reader(&e, "layer-Jogo").read_plan() else { panic!("sem plano") };
        assert_eq!(plan.steps.iter().map(|s| s.technique.as_str()).collect::<Vec<_>>(), ["Sepia", "Invert"], "o preset do jogo, não o global");
        assert_eq!(f32::from_le_bytes(plan.steps[0].uniforms[..4].try_into().unwrap()), 0.5);
        assert_eq!(games.in_use(&layers).len(), 4, "dois efeitos em duas resoluções");
    }

    #[test]
    fn nothing_changes_while_the_preset_file_is_untouched() {
        let e = env("games-stable");
        fs::write(e.presets.join("preset.ini"), "Techniques=Invert@Invert.fx\n").unwrap();
        let layers = [layer("Jogo", &[(800, 600)])];
        let mut games = Games::new(e.presets.clone());
        let mut failures = Failures::new();
        assert_eq!(games.tick(&e.f.compiler, &e.state, &layers, &mut failures).published.len(), 1, "usa o preset global");
        let channel = reader(&e, "layer-Jogo");
        let generation = channel.plan_generation();
        let compiled = calls(&e.f).len();

        let again = games.tick(&e.f.compiler, &e.state, &layers, &mut failures);
        assert_eq!(again, Report::default());
        assert_eq!(channel.plan_generation(), generation, "não republica à toa");
        assert_eq!(calls(&e.f).len(), compiled);
    }

    #[test]
    fn editing_the_preset_republishes_and_a_new_resolution_compiles() {
        let e = env("games-edit");
        let file = e.presets.join("preset.ini");
        fs::write(&file, "Techniques=Invert@Invert.fx\n").unwrap();
        let mut games = Games::new(e.presets.clone());
        let mut failures = Failures::new();
        games.tick(&e.f.compiler, &e.state, &[layer("Jogo", &[(800, 600)])], &mut failures);
        let channel = reader(&e, "layer-Jogo");
        let g0 = channel.plan_generation();

        // O jogo muda de resolução: compila para ela e republica o mesmo plano.
        let bigger = [layer("Jogo", &[(800, 600), (1024, 768)])];
        let report = games.tick(&e.f.compiler, &e.state, &bigger, &mut failures);
        assert_eq!(report.compiled, vec![Job { name: "Invert".into(), width: 1024, height: 768 }]);
        assert_ne!(channel.plan_generation(), g0);

        // O usuário edita o preset (a data do arquivo muda): o plano novo é publicado.
        fs::write(&file, "Techniques=Sepia@Sepia.fx,Invert@Invert.fx\n").unwrap();
        fs::File::options().write(true).open(&file).unwrap().set_modified(SystemTime::now() + Duration::from_secs(60)).unwrap();
        let report = games.tick(&e.f.compiler, &e.state, &bigger, &mut failures);
        assert_eq!(report.published, vec![("Jogo".to_string(), 2)]);
        let ipc::PlanRead::Plan(plan) = channel.read_plan() else { panic!() };
        assert_eq!(plan.steps.len(), 2);
    }

    #[test]
    fn losing_the_preset_clears_the_plan() {
        let e = env("games-clear");
        let file = e.presets.join("preset.ini");
        fs::write(&file, "Techniques=Invert@Invert.fx\n").unwrap();
        let layers = [layer("Jogo", &[(800, 600)])];
        let mut games = Games::new(e.presets.clone());
        let mut failures = Failures::new();
        games.tick(&e.f.compiler, &e.state, &layers, &mut failures);
        let channel = reader(&e, "layer-Jogo");
        assert!(matches!(channel.read_plan(), ipc::PlanRead::Plan(_)));

        fs::remove_file(&file).unwrap();
        games.tick(&e.f.compiler, &e.state, &layers, &mut failures);
        assert_eq!(channel.read_plan(), ipc::PlanRead::None, "volta à escolha única");
    }

    #[test]
    fn a_game_without_any_preset_is_left_alone() {
        let e = env("games-none");
        let mut games = Games::new(e.presets.clone());
        let report = games.tick(&e.f.compiler, &e.state, &[layer("Jogo", &[(800, 600)])], &mut Failures::new());
        assert_eq!(report, Report::default());
        assert!(ipc::ChannelReader::open_in(&e.f.root.join("state-dir"), "layer-Jogo").unwrap().is_none(), "nem cria canal");
    }

    #[test]
    fn a_layer_without_a_swapchain_waits() {
        let e = env("games-wait");
        fs::write(e.presets.join("preset.ini"), "Techniques=Invert@Invert.fx\n").unwrap();
        let mut games = Games::new(e.presets.clone());
        let mut failures = Failures::new();
        assert_eq!(games.tick(&e.f.compiler, &e.state, &[layer("Jogo", &[])], &mut failures), Report::default());
        assert_eq!(games.tick(&e.f.compiler, &e.state, &[layer("Jogo", &[(100, 100)])], &mut failures).published.len(), 1);
    }
}
