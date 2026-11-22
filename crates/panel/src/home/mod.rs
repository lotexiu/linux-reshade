//! A tela principal, no estilo da aba Home do ReShade: o jogo, o preset dele, e a lista de técnicas em ordem
//! de execução, cada uma com os seus controles. Nada aqui sabe desenhar; a interface só mostra o
//! [`HomeSnapshot`] e repassa os cliques.
//!
//! Cada ação grava o arquivo do preset na hora (como o "Auto Save" do ReShade); o vigia (`lrctl daemon`) relê o
//! arquivo e a layer aplica. Editar um jogo que ainda usa o preset global cria antes o preset dele (`Padrão`, uma
//! cópia do global), para uma mudança num jogo nunca alterar os outros sem querer.

use crate::control::{Control, controls_of};
use control_core::compiler::{Compiler, DEFAULT_SIZE, valid_name};
use control_core::analysis::{self, EffectInfo, Feature};
use control_core::{game_plan, presets};
use fx_bundle::Bundle;
use fx_bundle::manifest::{Scalar, Value};
use ipc::{LiveLayer, TechniqueState};
use preset::game::{self, Source};
use preset::{Preset, TechId};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use control_core::i18n::{fmt, tr};

mod background;
mod editing;
mod refresh;
mod selection;
mod snapshot;
#[cfg(test)]
mod tests;

use background::folder_of;
pub use background::start_analysis;
pub use snapshot::*;

/// O nome do preset que nasce da primeira edição de um jogo que usava o global.
pub const DEFAULT_PRESET: &str = "Padrão";

type LayersSource = Box<dyn Fn() -> Vec<LiveLayer>>;

struct Loaded {
    path: PathBuf,
    stamp: Option<SystemTime>,
    preset: Preset,
}

pub struct Home {
    compiler: Result<Compiler, String>,
    presets_dir: Option<PathBuf>,
    state: ipc::Writer,
    layers: LayersSource,
    game: Option<String>,
    loaded: Option<Loaded>,
    /// O bundle de referência de cada efeito (nome sem `.fx`): dele saem rótulos e controles.
    schemas: HashMap<String, Bundle>,
    /// Quantas análises havia da última vez que se leram todas (para só reler quando mudam).
    analysis_seen: Option<usize>,
    /// Quando se procurou por última vez análises que faltavam (uma refeita no lugar da antiga não muda a contagem).
    analysis_checked: Option<std::time::Instant>,
    /// Sobe a cada mudança do que a tela mostra: quem desenha compara com a que já desenhou e pula o trabalho se é a mesma.
    version: u64,
    /// Sobem quando o preset é (re)lido do disco e quando a lista de jogos abertos (as camadas) muda: com o que mais entra na tela
    /// montada (`build_view`), formam a chave que decide se ela precisa ser refeita.
    loaded_rev: u64,
    layers_rev: u64,
    infos_rev: u64,
    view_key: Option<(u64, u64, u64, Option<String>, usize, Option<SystemTime>)>,
    snapshot: HomeSnapshot,
}

/// Troca o valor se mudou e diz se mudou (para só avisar a tela quando há o que mostrar).
fn replace_if_changed<T: PartialEq>(slot: &mut T, new: T) -> bool {
    if *slot == new {
        false
    } else {
        *slot = new;
        true
    }
}

fn stamp(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// `NOME=VALOR,OUTRO=2`, como se mostra e se digita.
pub fn macros_text(macros: &[(String, String)]) -> String {
    macros.iter().map(|(n, v)| format!("{n}={v}")).collect::<Vec<_>>().join(",")
}

fn effect_name(file: &str) -> &str {
    file.strip_suffix(".fx").unwrap_or(file)
}

fn display_name(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn label_of(bundle: &Bundle, technique: &str) -> Option<String> {
    let t = bundle.manifest.techniques.iter().find(|t| t.name == technique)?;
    match t.annotations.get("ui_label") {
        Some(Value::Scalar(Scalar::Text(text))) if !text.is_empty() => Some(text.clone()),
        _ => None,
    }
}

impl Home {
    pub fn new(compiler: Result<Compiler, String>, presets_dir: Option<PathBuf>, state: ipc::Writer) -> Self {
        Self::with_layers(compiler, presets_dir, state, Box::new(|| ipc::live_layers().unwrap_or_default()))
    }

    pub fn with_layers(compiler: Result<Compiler, String>, presets_dir: Option<PathBuf>, state: ipc::Writer, layers: LayersSource) -> Self {
        let mut home = Self { compiler, presets_dir, state, layers, game: None, loaded: None, schemas: HashMap::new(), analysis_seen: None, analysis_checked: None, version: 0, loaded_rev: 0, layers_rev: 0, infos_rev: 0, view_key: None, snapshot: HomeSnapshot::default() };
        if let Err(why) = &home.compiler {
            home.snapshot.notice = Some(fmt("no effect compiler: {}", &[why]));
        } else if home.presets_dir.is_none() {
            home.snapshot.notice = Some(tr("no data folder chosen: run 'lrctl config init'").into());
        }
        // Começa no primeiro jogo aberto, se houver (é o que a pessoa quer ajustar).
        home.game = (home.layers)().first().map(|l| l.status.game_id().to_string()).filter(|id| game::valid_game_id(id));
        home.refresh();
        home.prepare_effects();
        home
    }

    pub fn snapshot(&self) -> &HomeSnapshot {
        &self.snapshot
    }

    fn dir(&self) -> Result<&Path, String> {
        self.presets_dir.as_deref().ok_or_else(|| tr("no data folder chosen: run 'lrctl config init'").to_string())
    }

    /// O preset que vale para o alvo atual: o do jogo, o global que ele herda, ou o global.
    fn resolve(&self) -> Option<(PathBuf, bool)> {
        let dir = self.presets_dir.as_deref()?;
        match &self.game {
            Some(id) => game::find(dir, id).map(|f| (f.path, f.source == Source::Global)),
            None => {
                let path = dir.join(game::GLOBAL_PRESET);
                path.is_file().then_some((path, false))
            }
        }
    }

    /// Depois de trocar o idioma: refaz o que o modelo guarda já traduzido (os avisos das linhas do preset, o aviso da profundidade). Os
    /// avisos pontuais de uma ação (`done`) ficam como estavam. Sem isto a janela nova mostrava esses textos no idioma antigo.
    pub fn relocalize(&mut self) {
        self.view_key = None;
        self.version += 1;
        self.refresh();
    }

    /// Sobe a cada mudança do que a tela mostra (ver `refresh`): a interface pula o desenho se a versão é a que ela já desenhou.
    pub fn version(&self) -> u64 {
        self.version
    }

    fn done(&mut self, notice: Option<String>) {
        self.refresh();
        self.snapshot.notice = notice;
        self.version += 1;
    }

    // ---- ações ----
}
