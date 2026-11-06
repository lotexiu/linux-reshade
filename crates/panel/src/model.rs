//! O estado que a interface mostra e as ações que ela dispara.
//!
//! Tudo o que a interface precisa saber sai de [`Model::snapshot`]; cada clique vira uma chamada em
//! [`Model`], que escreve no canal com a layer (o mesmo do `lrctl`) e relê o estado. Uma mudança feita
//! de fora, pelo `lrctl` ou pelo atalho global, aparece no próximo [`Model::refresh`].

use crate::control::{Control, controls_of};
use control_core::compiler::Compiler;
use control_core::{params, select};
use fx_bundle::Bundle;
use ipc::{LiveLayer, SelectionRead};
use std::path::PathBuf;
use control_core::i18n::{fmt, tr};

/// Qual efeito está escolhido.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    None,
    /// Escolhido por nome (um bundle por resolução, compilado sob demanda).
    Effect(String),
    /// Um bundle já compilado, fixo numa resolução (escolhido com `lrctl use <diretório>`).
    Fixed(PathBuf),
}

/// O que a interface mostra, num instante.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// Efeitos que podem ser escolhidos por nome.
    pub effects: Vec<String>,
    pub selection: Selection,
    pub layers: Vec<LiveLayer>,
    /// Os efeitos estão ligados (a flag que o atalho global alterna).
    pub enabled: bool,
    /// Os controles do efeito escolhido (vazio se não há efeito, ou ele não tem parâmetros).
    pub controls: Vec<Control>,
    /// A última mensagem para o usuário (um erro, um aviso); some na próxima ação.
    pub notice: Option<String>,
}

impl Default for Selection {
    fn default() -> Self {
        Selection::None
    }
}

/// De onde vem a lista de jogos abertos.
type LayersSource = Box<dyn Fn() -> Vec<LiveLayer>>;

pub struct Model {
    compiler: Result<Compiler, String>,
    state: ipc::Writer,
    layers: LayersSource,
    /// O bundle de onde vêm os controles, com o nome do efeito (ou o caminho) a que corresponde.
    schema: Option<(String, Bundle)>,
    snapshot: Snapshot,
}

impl Model {
    pub fn new(compiler: Result<Compiler, String>, state: ipc::Writer) -> Self {
        Self::with_layers(compiler, state, Box::new(|| ipc::live_layers().unwrap_or_default()))
    }

    /// Como [`new`](Self::new), com a lista de jogos abertos dada (para testes).
    pub fn with_layers(compiler: Result<Compiler, String>, state: ipc::Writer, layers: LayersSource) -> Self {
        let mut model = Self { compiler, state, layers, schema: None, snapshot: Snapshot::default() };
        if let Err(why) = &model.compiler {
            model.snapshot.notice = Some(fmt("no effect compiler: {}", &[why]));
        }
        model.refresh();
        model
    }

    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    /// Relê tudo: jogos, escolha, flag e valores dos controles. Barato; a interface pode chamar a cada meio segundo.
    pub fn refresh(&mut self) {
        let snap = &mut self.snapshot;
        snap.layers = (self.layers)();
        snap.enabled = self.state.effects_enabled();
        snap.effects = self.compiler.as_ref().map(|c| c.available().into_iter().map(|(n, _)| n).collect()).unwrap_or_default();
        snap.selection = match self.state.selection() {
            SelectionRead::Path(path) => match self.compiler.as_ref().ok().and_then(|c| c.name_from_template(&path)) {
                Some(name) => Selection::Effect(name),
                None => Selection::Fixed(PathBuf::from(path)),
            },
            // Se a leitura caiu no meio de uma escrita, mantém o que já se sabia.
            SelectionRead::Unstable => snap.selection.clone(),
            SelectionRead::Cleared => Selection::None,
        };
        self.refresh_controls();
    }

    /// Carrega (se preciso) o bundle de referência do efeito escolhido e lê os valores atuais dos controles.
    fn refresh_controls(&mut self) {
        let key = match &self.snapshot.selection {
            Selection::None => None,
            Selection::Effect(name) => Some(name.clone()),
            Selection::Fixed(dir) => Some(dir.display().to_string()),
        };
        let Some(key) = key else {
            self.schema = None;
            self.snapshot.controls.clear();
            return;
        };
        if self.schema.as_ref().map(|(k, _)| k) != Some(&key) {
            let dir = match &self.snapshot.selection {
                Selection::Effect(name) => self.compiler.as_ref().ok().and_then(|c| c.any_bundle(name)),
                Selection::Fixed(dir) => Some(dir.clone()),
                Selection::None => None,
            };
            self.schema = dir.and_then(|d| Bundle::load(&d).ok()).map(|bundle| (key, bundle));
        }
        self.snapshot.controls = match &self.schema {
            Some((_, bundle)) => controls_of(bundle, &params::current_block(bundle, &self.state)),
            None => Vec::new(),
        };
    }

    fn done(&mut self, notice: Option<String>) {
        self.refresh();
        self.snapshot.notice = notice;
    }

    /// Escolhe um efeito por nome: compila para as resoluções dos jogos abertos e publica a escolha.
    pub fn select_effect(&mut self, name: &str) {
        let outcome = match &self.compiler {
            Err(why) => Err(fmt("no effect compiler: {}", &[why])),
            Ok(compiler) => {
                let mut resolutions: Vec<(u32, u32)> = (self.layers)()
                    .iter()
                    .flat_map(|l| &l.status.swapchains)
                    .filter(|s| !matches!(s.effect, ipc::EffectState::Unavailable(_)))
                    .map(|s| (s.width, s.height))
                    .collect();
                resolutions.sort_unstable();
                resolutions.dedup();
                select::prepare(compiler, &self.state, name, &resolutions)
                    // Sem jogo aberto nada foi compilado, e os controles precisam de um bundle de onde ler os parâmetros do efeito.
                    .and_then(|_| compiler.schema_bundle(&compiler.template(name), &(self.layers)()).map(drop))
                    .map_err(|e| e.to_string())
            }
        };
        self.schema = None; // os controles são de outro efeito
        self.done(outcome.err());
    }

    /// Desfaz a escolha de efeito.
    pub fn clear_selection(&mut self) {
        self.state.clear_selection();
        self.done(None);
    }

    pub fn set_enabled(&mut self, on: bool) {
        self.state.set_effects_enabled(on);
        self.done(None);
    }

    /// Altera um controle: `values` tem um número por componente.
    pub fn set_control(&mut self, name: &str, values: &[f64]) {
        let result = match &self.schema {
            Some((_, bundle)) => params::set(bundle, &self.state, name, values).map(|c| c.warnings.into_iter().next()).map_err(|e| e.to_string()),
            None => Err(tr("no effect chosen").to_string()),
        };
        match result {
            Ok(warning) => self.done(warning),
            Err(why) => self.done(Some(why)),
        }
    }

    /// Volta todos os controles aos valores padrão do efeito.
    pub fn reset_controls(&mut self) {
        let result = match &self.schema {
            Some((_, bundle)) => params::reset(bundle, &self.state).map_err(|e| e.to_string()),
            None => Err(tr("no effect chosen").to_string()),
        };
        self.done(result.err());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::Kind;
    use ipc::{EffectState, LayerStatus, SwapchainStatus};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    /// O bundle do Sépia compilado de verdade (golden do fx-bundle), que a "ferramenta" de teste copia.
    fn golden() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../fx-bundle/tests/data/Sepia")
    }

    struct Env {
        root: PathBuf,
        compiler: Compiler,
    }

    /// Um ambiente com um efeito `Sepia.fx` e um "fx2spv" que apenas copia o bundle de referência.
    fn env(name: &str) -> Env {
        let root = std::env::temp_dir().join(format!("lr-panel-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("effects")).unwrap();
        fs::write(root.join("effects/Sepia.fx"), "// de mentira").unwrap();
        let tool = root.join("fake-fx2spv");
        let script = format!(
            "#!/bin/sh\nout=\"\"; prev=\"\"\nfor a in \"$@\"; do [ \"$prev\" = \"-o\" ] && out=\"$a\"; prev=\"$a\"; done\nrm -rf \"$out\"; cp -r '{}' \"$out\"\n",
            golden().display()
        );
        fs::write(&tool, script).unwrap();
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
        let compiler = Compiler { fx2spv: tool, effects: vec![config::SearchPath::flat(root.join("effects"))], includes: vec![], textures: vec![], cache: root.join("cache") };
        Env { root, compiler }
    }

    fn state(env: &Env) -> ipc::Writer {
        ipc::Writer::create_at(&env.root.join("state-dir/state")).unwrap()
    }

    fn game(w: u32, h: u32) -> Vec<LiveLayer> {
        vec![LiveLayer {
            id: "layer-1".into(),
            status: LayerStatus { perf: None,
                depth: Vec::new(), depth_passes: String::new(),
                techniques: Vec::new(),
                exe: String::new(),
                app: "vkcube".into(),
                swapchains: vec![SwapchainStatus { width: w, height: h, format: 44, images: 3, effect: EffectState::Idle }],
            },
        }]
    }

    fn model(env: Env, layers: Vec<LiveLayer>) -> (Model, PathBuf) {
        let s = state(&env);
        let root = env.root.clone();
        (Model::with_layers(Ok(env.compiler), s, Box::new(move || layers.clone())), root)
    }

    #[test]
    fn it_starts_empty_and_lists_the_effects() {
        let (m, _) = model(env("start"), vec![]);
        let s = m.snapshot();
        assert_eq!(s.effects, ["Sepia"]);
        assert_eq!(s.selection, Selection::None);
        assert!(s.controls.is_empty() && !s.enabled && s.notice.is_none());
    }

    #[test]
    fn selecting_an_effect_prepares_the_open_games_resolution_and_shows_its_controls() {
        let e = env("select");
        let bundle_dir = e.compiler.bundle_dir("Sepia", 1280, 720);
        let (mut m, _) = model(e, game(1280, 720));
        m.select_effect("Sepia");
        assert!(bundle_dir.join("manifest.json").is_file(), "compilou para a resolução do jogo");
        assert_eq!(m.snapshot().selection, Selection::Effect("Sepia".into()));
        assert!(m.snapshot().notice.is_none(), "{:?}", m.snapshot().notice);
        let c = &m.snapshot().controls;
        assert_eq!(c.len(), 1);
        assert_eq!((c[0].label.as_str(), c[0].value.clone()), ("Intensidade", vec![1.0]));
        assert!(matches!(c[0].kind, Kind::Slider { .. }));
    }

    #[test]
    fn selecting_with_no_game_open_still_shows_the_controls() {
        let e = env("nogame");
        let reference = e.compiler.bundle_dir("Sepia", 1920, 1080);
        let (mut m, _) = model(e, vec![]);
        m.select_effect("Sepia");
        assert!(reference.join("manifest.json").is_file(), "compilou um bundle de referência (1920x1080)");
        assert!(m.snapshot().notice.is_none(), "{:?}", m.snapshot().notice);
        assert_eq!(m.snapshot().controls.len(), 1, "os controles aparecem mesmo sem jogo aberto");
    }

    #[test]
    fn changing_and_resetting_a_control_updates_the_snapshot_and_the_shared_channel() {
        let (mut m, root) = model(env("control"), game(1280, 720));
        m.select_effect("Sepia");
        m.set_control("Strength", &[0.3]);
        assert!(m.snapshot().notice.is_none());
        assert!((m.snapshot().controls[0].value[0] - 0.3).abs() < 1e-6);
        assert!(!m.snapshot().controls[0].is_default());

        // O que a layer leria do canal compartilhado.
        let reader = ipc::Reader::open_at(&root.join("state-dir/state")).unwrap().unwrap();
        let bundle = Bundle::load(&golden()).unwrap();
        let mut seen = vec![0u8; bundle.manifest.total_uniform_size as usize];
        assert_eq!(reader.read_params(bundle.params_id(), &mut seen), Some(seen.len()));
        assert!((f32::from_le_bytes(seen[..4].try_into().unwrap()) - 0.3).abs() < 1e-6);

        m.reset_controls();
        assert_eq!(m.snapshot().controls[0].value, vec![1.0]);
        assert!(m.snapshot().controls[0].is_default());
    }

    #[test]
    fn out_of_range_values_are_applied_with_a_notice() {
        let (mut m, _) = model(env("range"), game(1280, 720));
        m.select_effect("Sepia");
        m.set_control("Strength", &[5.0]);
        let notice = m.snapshot().notice.clone().unwrap();
        assert!(notice.contains("outside the range"), "{notice}");
        assert_eq!(m.snapshot().controls[0].value, vec![5.0]);
    }

    #[test]
    fn a_change_made_from_outside_shows_up_on_the_next_refresh() {
        let (mut m, root) = model(env("outside"), game(1280, 720));
        m.select_effect("Sepia");
        // Outro processo (o lrctl) altera o parâmetro e liga os efeitos.
        let other = ipc::Writer::create_at(&root.join("state-dir/state")).unwrap();
        let bundle = Bundle::load(&golden()).unwrap();
        params::set(&bundle, &other, "Strength", &[0.6]).unwrap();
        other.set_effects_enabled(true);
        assert!(!m.snapshot().enabled, "ainda não olhou");
        m.refresh();
        assert!(m.snapshot().enabled);
        assert!((m.snapshot().controls[0].value[0] - 0.6).abs() < 1e-6);
    }

    #[test]
    fn clearing_the_selection_removes_the_controls() {
        let (mut m, _) = model(env("clear"), game(1280, 720));
        m.select_effect("Sepia");
        assert!(!m.snapshot().controls.is_empty());
        m.clear_selection();
        assert_eq!(m.snapshot().selection, Selection::None);
        assert!(m.snapshot().controls.is_empty());
    }

    #[test]
    fn the_enabled_switch_drives_the_shared_flag() {
        let (mut m, root) = model(env("enabled"), vec![]);
        m.set_enabled(true);
        assert!(m.snapshot().enabled);
        let reader = ipc::Reader::open_at(&root.join("state-dir/state")).unwrap().unwrap();
        assert!(reader.effects_enabled());
        m.set_enabled(false);
        assert!(!reader.effects_enabled());
    }

    #[test]
    fn errors_become_a_notice_and_change_nothing() {
        let (mut m, _) = model(env("errors"), game(1280, 720));
        m.select_effect("NaoExiste");
        assert!(m.snapshot().notice.as_deref().unwrap().contains("NaoExiste"));
        assert_eq!(m.snapshot().selection, Selection::None);
        m.set_control("Strength", &[0.5]);
        assert_eq!(m.snapshot().notice.as_deref(), Some(tr("no effect chosen")));
        m.select_effect("Sepia");
        m.set_control("Forca", &[0.5]);
        assert!(m.snapshot().notice.as_deref().unwrap().contains("Forca"));
        m.set_control("Strength", &[0.1, 0.2]);
        assert!(m.snapshot().notice.is_some(), "número de componentes errado");
    }

    #[test]
    fn a_fixed_bundle_chosen_with_lrctl_shows_its_controls_too() {
        let e = env("fixed");
        let s = state(&e);
        s.publish_selection(golden().to_str().unwrap()).unwrap();
        let m = Model::with_layers(Ok(e.compiler), s, Box::new(Vec::new));
        assert_eq!(m.snapshot().selection, Selection::Fixed(golden()));
        assert_eq!(m.snapshot().controls.len(), 1);
    }

    #[test]
    fn without_a_compiler_the_model_explains_instead_of_failing() {
        let e = env("nocompiler");
        let mut m = Model::with_layers(Err("não achei o fx2spv".into()), state(&e), Box::new(Vec::new));
        assert!(m.snapshot().effects.is_empty());
        assert!(m.snapshot().notice.as_deref().unwrap().contains("fx2spv"));
        m.select_effect("Sepia");
        assert!(m.snapshot().notice.as_deref().unwrap().contains("fx2spv"));
    }
}
