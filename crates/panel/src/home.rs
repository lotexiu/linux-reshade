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

/// O nome do preset que nasce da primeira edição de um jogo que usava o global.
pub const DEFAULT_PRESET: &str = "Padrão";

/// Um jogo da lista: tem pasta de presets, está aberto, ou os dois.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameEntry {
    pub id: String,
    pub open: bool,
}

/// O preset que está sendo mostrado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetInfo {
    pub path: PathBuf,
    /// Nome para mostrar (`Noturno`, sem a extensão).
    pub name: String,
    /// Veio do `game.ini` do jogo, ou é o global que o jogo herda.
    pub inherited: bool,
}

/// Uma técnica do preset.
#[derive(Debug, Clone, PartialEq)]
pub struct TechniqueView {
    pub id: TechId,
    /// `ui_label` da técnica, ou o nome.
    pub label: String,
    /// Arquivo do efeito (`Sepia.fx`).
    pub file: String,
    pub enabled: bool,
    /// Os parâmetros do efeito, na primeira técnica de cada arquivo (as outras do mesmo arquivo não os repetem).
    pub controls: Vec<Control>,
    /// O que a layer do jogo aberto fez com esta técnica (aplicada ou pulada e por quê); `None` se o jogo não está
    /// aberto com este preset, ou a técnica está desligada.
    pub runtime: Option<TechniqueState>,
    /// Por que não dá para mostrar os parâmetros ou rodar (efeito não achado, ainda não compilado...).
    pub problem: Option<String>,
    /// O atalho que liga e desliga a técnica (formato XDG, `CTRL+F11`); vazio: sem atalho. Vem do `Key<técnica>` do preset.
    pub key: String,
}

/// Um efeito (um arquivo `.fx`) com as técnicas dele: é a unidade que a pessoa vê, move e remove. As técnicas de um
/// mesmo efeito (por exemplo o desfoque horizontal e o vertical) andam juntas e partilham os parâmetros do efeito;
/// dá para ligar e desligar cada uma, mas não remover só uma.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectGroup {
    /// Arquivo do efeito (`cGaussianBlur.fx`).
    pub file: String,
    /// Nome para mostrar: o do arquivo sem `.fx`.
    pub name: String,
    pub techniques: Vec<TechniqueView>,
    /// Os parâmetros do efeito (de um só lugar, não repetidos por técnica).
    pub controls: Vec<Control>,
    /// As definições de pré-processador só deste efeito (`NOME=VALOR,OUTRO=2`). Mudar recompila o efeito.
    pub macros: String,
}

impl EffectGroup {
    /// Alguma técnica do efeito está ligada.
    pub fn any_enabled(&self) -> bool {
        self.techniques.iter().any(|t| t.enabled)
    }

    pub fn all_enabled(&self) -> bool {
        self.techniques.iter().all(|t| t.enabled)
    }
}

#[derive(Debug, Clone, Default)]
pub struct HomeSnapshot {
    /// Os efeitos estão ligados (a flag global que o atalho alterna).
    pub enabled: bool,
    pub games: Vec<GameEntry>,
    /// O jogo escolhido; `None` é o preset global.
    pub game: Option<String>,
    /// Presets do jogo escolhido (nomes sem extensão), e qual está em uso (`None`: usa o global).
    pub presets: Vec<String>,
    pub active_preset: Option<String>,
    pub preset: Option<PresetInfo>,
    pub techniques: Vec<TechniqueView>,
    /// As mesmas técnicas agrupadas por efeito, na ordem em que cada efeito aparece pela primeira vez.
    pub groups: Vec<EffectGroup>,
    /// As definições de pré-processador do preset inteiro (`NOME=VALOR,OUTRO=2`); valem para todos os efeitos dele.
    pub preset_macros: String,
    /// Efeitos que podem ser adicionados (nomes de arquivo sem `.fx`).
    pub effects: Vec<String>,
    /// A subpasta de cada efeito, relativa à pasta de busca que o contém (`Cor/PD80`; vazio: direto na pasta de busca).
    pub folders: HashMap<String, String>,
    /// O que se sabe de cada efeito (compatibilidade e características), conforme a análise em segundo plano avança.
    pub infos: HashMap<String, EffectInfo>,
    /// Andamento da análise: efeitos analisados e total.
    pub analysis: (usize, usize),
    pub layers: Vec<LiveLayer>,
    /// O que dizer sobre a profundidade do jogo escolhido (`None`: nada a dizer).
    pub depth_note: Option<crate::DepthNote>,
    /// As candidatas de profundidade do jogo aberto e o que a pessoa pediu (vazio: nada a escolher).
    pub depth_choices: Vec<crate::DepthChoice>,
    /// As limpezas de profundidade do último quadro do jogo aberto (para o painel listar os pontos de captura possíveis).
    pub depth_clears: Vec<crate::DepthClear>,
    pub depth_passes: Vec<crate::DepthPass>,
    pub depth_pick: ipc::DepthPick,
    /// A última mensagem para o usuário; some na próxima ação.
    pub notice: Option<String>,
}

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

    /// Relê tudo: jogos, flag, e o preset em disco se mudou. Barato; a interface pode chamar a cada meio segundo.
    pub fn refresh(&mut self) {
        let mut changed = replace_if_changed(&mut self.snapshot.enabled, self.state.effects_enabled());
        if replace_if_changed(&mut self.snapshot.layers, (self.layers)()) {
            self.layers_rev += 1;
            changed = true;
        }
        let available = self.compiler.as_ref().map(|c| c.available()).unwrap_or_default();
        let folders: HashMap<String, String> = match &self.compiler {
            Ok(c) => {
                let roots: Vec<PathBuf> = c.effects.iter().map(|r| r.dir.clone()).collect();
                available.iter().map(|(n, dir)| (n.clone(), folder_of(&roots, dir))).collect()
            }
            Err(_) => HashMap::new(),
        };
        changed |= replace_if_changed(&mut self.snapshot.folders, folders);
        changed |= replace_if_changed(&mut self.snapshot.effects, available.into_iter().map(|(n, _)| n).collect());

        if self.refresh_analysis() {
            self.infos_rev += 1;
            changed = true;
        }

        // Jogos: as pastas de presets e os abertos.
        let mut games: Vec<GameEntry> = Vec::new();
        if let Some(dir) = &self.presets_dir
            && let Ok(entries) = std::fs::read_dir(dir)
        {
            for e in entries.flatten().filter(|e| e.path().is_dir()) {
                let id = e.file_name().to_string_lossy().into_owned();
                if game::valid_game_id(&id) {
                    games.push(GameEntry { id, open: false });
                }
            }
        }
        for layer in &self.snapshot.layers {
            let id = layer.status.game_id().to_string();
            if !game::valid_game_id(&id) {
                continue;
            }
            match games.iter_mut().find(|g| g.id == id) {
                Some(g) => g.open = true,
                None => games.push(GameEntry { id, open: true }),
            }
        }
        if let Some(chosen) = &self.game
            && !games.iter().any(|g| &g.id == chosen)
        {
            games.push(GameEntry { id: chosen.clone(), open: false });
        }
        games.sort_by(|a, b| b.open.cmp(&a.open).then_with(|| a.id.to_lowercase().cmp(&b.id.to_lowercase())));
        changed |= replace_if_changed(&mut self.snapshot.games, games);
        changed |= replace_if_changed(&mut self.snapshot.game, self.game.clone());

        changed |= self.load_preset();
        // A tela montada (presets do jogo, técnicas, grupos, controles) só é refeita quando algo de que ela depende mudou: o preset
        // lido de novo, os jogos abertos, as análises, o jogo escolhido ou a pasta de presets dele. Antes era refeita a cada
        // atualização (de 0,5 em 0,5 s), inclusive o cálculo dos controles de cada efeito.
        let game_dir_stamp = match (&self.presets_dir, &self.game) {
            (Some(dir), Some(id)) => game::game_dir(dir, id).and_then(|d| stamp(&d)),
            _ => None,
        };
        let key = (self.loaded_rev, self.layers_rev, self.infos_rev, self.game.clone(), self.schemas.len(), game_dir_stamp);
        if self.view_key.as_ref() != Some(&key) {
            self.build_view();
            self.update_depth_note();
            self.view_key = Some(key);
            changed = true;
        }
        if changed {
            self.version += 1;
        }
    }

    /// Sobe a cada mudança do que a tela mostra (ver `refresh`): a interface pula o desenho se a versão é a que ela já desenhou.
    pub fn version(&self) -> u64 {
        self.version
    }

    fn update_depth_note(&mut self) {
        self.snapshot.depth_note = None;
        self.snapshot.depth_choices.clear();
        self.snapshot.depth_clears.clear();
        self.snapshot.depth_passes.clear();
        self.snapshot.depth_pick = ipc::DepthPick::Auto;
        if let Some(id) = &self.game
            && let Some(layer) = self.snapshot.layers.iter().find(|l| l.status.game_id() == id)
        {
            self.snapshot.depth_choices = crate::depth_note::choices(layer);
            self.snapshot.depth_clears = crate::depth_note::clears(layer);
            self.snapshot.depth_passes = crate::depth_note::passes(layer);
            self.snapshot.depth_pick = ipc::depth_pick(&layer.id);
        }
        let Some(id) = self.game.clone() else { return };
        let uses_depth = self.snapshot.techniques.iter().filter(|t| t.enabled).any(|t| self.snapshot.infos.get(effect_name(&t.file)).is_some_and(|i| i.has(Feature::Depth)));
        let layer = self.snapshot.layers.iter().find(|l| l.status.game_id() == id);
        self.snapshot.depth_note = crate::depth_note::note(uses_depth, layer);
    }

    /// Relê as análises dos efeitos quando o worker salvou mais alguma.
    fn refresh_analysis(&mut self) -> bool {
        let Ok(compiler) = &self.compiler else { return false };
        let before = (self.snapshot.infos.len(), self.snapshot.effects.len());
        let saved = analysis::count_saved(compiler);
        // Só lê as análises que faltam (cada leitura é um arquivo e uma checagem do `.fx`; são mais de mil).
        let incomplete = self.snapshot.infos.len() < self.snapshot.effects.len() && self.analysis_checked.is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(1));
        if self.analysis_seen != Some(saved) || incomplete {
            self.analysis_checked = Some(std::time::Instant::now());
            let effects = &self.snapshot.effects;
            self.snapshot.infos.retain(|n, _| effects.binary_search(n).is_ok());
            for n in effects {
                if !self.snapshot.infos.contains_key(n)
                    && let Some(info) = analysis::load(compiler, n)
                {
                    self.snapshot.infos.insert(n.clone(), info);
                }
            }
            self.analysis_seen = Some(saved);
        }
        self.snapshot.analysis = (self.snapshot.infos.len(), self.snapshot.effects.len());
        before != self.snapshot.analysis
    }

    /// Carrega o arquivo do preset que vale agora, se mudou (ou se é outro).
    /// Carrega o arquivo do preset que vale agora, se mudou (ou se é outro). Devolve se o que a tela mostra mudou.
    fn load_preset(&mut self) -> bool {
        let Some((path, inherited)) = self.resolve() else {
            let had = self.loaded.take().is_some();
            if had {
                self.loaded_rev += 1;
            }
            return replace_if_changed(&mut self.snapshot.preset, None) || had;
        };
        let current = stamp(&path);
        let fresh = self.loaded.as_ref().is_some_and(|l| l.path == path && l.stamp == current);
        let mut changed = false;
        if !fresh {
            match presets::load(&path) {
                Ok(preset) => self.loaded = Some(Loaded { path: path.clone(), stamp: current, preset }),
                Err(why) => {
                    self.loaded = None;
                    self.snapshot.notice = Some(why);
                }
            }
            self.loaded_rev += 1;
            changed = true;
        }
        let info = self.loaded.as_ref().map(|l| PresetInfo { name: display_name(&l.path), path: l.path.clone(), inherited });
        changed | replace_if_changed(&mut self.snapshot.preset, info)
    }

    /// Os presets do jogo escolhido e as técnicas do que está em uso.
    fn build_view(&mut self) {
        self.snapshot.presets.clear();
        self.snapshot.active_preset = None;
        if let (Some(dir), Some(id)) = (&self.presets_dir, &self.game)
            && let Some(game_dir) = game::game_dir(dir, id)
        {
            let mut names: Vec<String> = std::fs::read_dir(&game_dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| e.path().is_file())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| game::valid_preset_file(n))
                .map(|n| n.trim_end_matches(".ini").trim_end_matches(".INI").to_string())
                .collect();
            names.sort_by_key(|n| n.to_lowercase());
            self.snapshot.presets = names;
            if let Some(info) = &self.snapshot.preset
                && !info.inherited
            {
                self.snapshot.active_preset = Some(info.name.clone());
            }
        }

        self.snapshot.techniques.clear();
        self.snapshot.groups.clear();
        self.snapshot.preset_macros.clear();
        let Some(loaded) = &self.loaded else { return };
        let preset = loaded.preset.clone();
        self.snapshot.preset_macros = macros_text(&preset.macros());
        let mut shown_files: Vec<String> = Vec::new();
        // O que a layer do jogo aberto informou da cadeia dela: casa por nome, na ordem, com as técnicas ligadas.
        let mut reported: Vec<ipc::TechniqueStatus> = match &self.game {
            Some(id) => self.snapshot.layers.iter().find(|l| l.status.game_id() == id).map(|l| l.status.techniques.clone()).unwrap_or_default(),
            None => Vec::new(),
        };
        for row in presets::rows(&preset) {
            let Some(file) = row.id.file.clone().or_else(|| Some(format!("{}.fx", row.id.name))) else { continue };
            let name = effect_name(&file).to_string();
            let _ = self.ensure_schema(&name, false);
            let bundle = self.schemas.get(&name);
            let (label, mut problem) = match bundle {
                Some(b) => {
                    let known = b.manifest.techniques.iter().any(|t| t.name == row.id.name);
                    (label_of(b, &row.id.name).unwrap_or_else(|| row.id.name.clone()), (!known).then(|| tr("the effect does not declare this technique").to_string()))
                }
                None => (row.id.name.clone(), Some(tr("effect not compiled yet (open a game with the daemon running, or add it again)").to_string())),
            };
            let first_of_file = !shown_files.contains(&file);
            let mut controls = Vec::new();
            if first_of_file {
                shown_files.push(file.clone());
                if let Some(b) = bundle {
                    let mut warnings = Vec::new();
                    let block = game_plan::uniform_block(b, &preset, &file, &mut warnings);
                    controls = controls_of(b, &block);
                    if let Some(w) = warnings.into_iter().next() {
                        problem.get_or_insert(w);
                    }
                }
            }
            let runtime = if row.enabled {
                reported.iter().position(|t| t.name == row.id.name).map(|i| reported.remove(i).state)
            } else {
                None
            };
            let key = preset.technique_key(&row.id).and_then(|k| shortcuts::trigger::from_vk(k.code, k.ctrl, k.shift, k.alt)).unwrap_or_default();
            self.snapshot.techniques.push(TechniqueView { id: row.id, label, file, enabled: row.enabled, controls, runtime, problem, key });
        }
        // Agrupa por efeito: os parâmetros vêm da primeira técnica de cada arquivo.
        for t in &self.snapshot.techniques {
            match self.snapshot.groups.iter_mut().find(|g| g.file == t.file) {
                Some(g) => {
                    if g.controls.is_empty() {
                        g.controls = t.controls.clone();
                    }
                    g.techniques.push(TechniqueView { controls: Vec::new(), ..t.clone() });
                }
                None => self.snapshot.groups.push(EffectGroup {
                    file: t.file.clone(),
                    name: effect_name(&t.file).to_string(),
                    controls: t.controls.clone(),
                    macros: macros_text(&preset.effect_macros(&t.file)),
                    techniques: vec![TechniqueView { controls: Vec::new(), ..t.clone() }],
                }),
            }
        }
    }

    /// Guarda o bundle de referência do efeito. Com `compile`, compila se ainda não há nenhum no cache.
    fn ensure_schema(&mut self, name: &str, compile: bool) -> Result<(), String> {
        if self.schemas.contains_key(name) {
            return Ok(());
        }
        if !valid_name(name) {
            return Err(fmt("{} is not a valid effect name", &[&format!("{name:?}")]));
        }
        let compiler = self.compiler.as_ref().map_err(|why| fmt("no effect compiler: {}", &[why]))?;
        let dir = match compiler.any_bundle(name) {
            Some(dir) => dir,
            None if compile => compiler.ensure(name, DEFAULT_SIZE.0, DEFAULT_SIZE.1).map_err(|e| e.to_string())?.dir_owned(),
            None => return Err(tr("effect not compiled yet").into()),
        };
        let bundle = Bundle::load(&dir).map_err(|e| e.to_string())?;
        self.schemas.insert(name.to_string(), bundle);
        Ok(())
    }

    fn done(&mut self, notice: Option<String>) {
        self.refresh();
        self.snapshot.notice = notice;
        self.version += 1;
    }

    // ---- ações ----

    pub fn set_enabled(&mut self, on: bool) {
        self.state.set_effects_enabled(on);
        self.done(None);
    }

    /// Escolhe o jogo mostrado (`None`: o preset global).
    pub fn select_game(&mut self, game: Option<&str>) {
        self.game = game.filter(|g| game::valid_game_id(g)).map(str::to_string);
        self.loaded = None;
        self.done(None);
        self.prepare_effects();
    }

    /// Compila (se preciso) os efeitos do preset mostrado, para a tela ter rótulos e controles.
    fn prepare_effects(&mut self) {
        let Some(loaded) = &self.loaded else { return };
        let files: Vec<String> = presets::rows(&loaded.preset).into_iter().filter_map(|r| r.id.file).collect();
        for file in files {
            let _ = self.ensure_schema(effect_name(&file), true);
        }
        let notice = self.snapshot.notice.clone();
        self.done(notice);
    }

    /// Usa um preset do jogo escolhido (`None`: volta a seguir o global).
    pub fn select_preset(&mut self, name: Option<&str>) {
        let result = match (&self.game, self.presets_dir.clone()) {
            (Some(game), Some(dir)) => presets::use_preset(&dir, game, name),
            _ => Err(tr("pick a game first").to_string()),
        };
        self.loaded = None;
        self.done(result.err());
        self.prepare_effects();
    }

    /// Cria um preset do jogo (cópia do que vale agora) e passa a usá-lo.
    pub fn new_preset(&mut self, name: &str) {
        let result = match (&self.game, self.presets_dir.clone()) {
            (Some(game), Some(dir)) => presets::new_for_game(&dir, game, name.trim()).map(drop),
            _ => Err(tr("pick a game first").to_string()),
        };
        self.loaded = None;
        self.done(result.err());
    }

    /// O arquivo a editar: o do jogo; se ele ainda usa o global, cria antes o preset `Padrão` dele.
    fn writable(&mut self) -> Result<PathBuf, String> {
        let dir = self.dir()?.to_path_buf();
        match self.game.clone() {
            None => Ok(dir.join(game::GLOBAL_PRESET)),
            Some(id) => match game::find(&dir, &id) {
                Some(found) if found.source == Source::Game => Ok(found.path),
                _ => {
                    let path = dir.join(&id).join(format!("{DEFAULT_PRESET}.ini"));
                    if path.exists() {
                        presets::use_preset(&dir, &id, Some(DEFAULT_PRESET))?;
                    } else {
                        presets::new_for_game(&dir, &id, DEFAULT_PRESET)?;
                    }
                    Ok(path)
                }
            },
        }
    }

    /// Aplica `edit` ao preset em uso e o grava.
    fn edit(&mut self, edit: impl FnOnce(&mut Preset, Option<&Compiler>) -> Result<Option<String>, String>) {
        let result = (|| {
            let path = self.writable()?;
            let mut preset = presets::load(&path)?;
            let notice = edit(&mut preset, self.compiler.as_ref().ok())?;
            preset.save(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok::<_, String>(notice)
        })();
        self.loaded = None;
        match result {
            Ok(notice) => self.done(notice),
            Err(why) => self.done(Some(why)),
        }
    }

    pub fn set_technique_enabled(&mut self, id: &TechId, on: bool) {
        let id = id.clone();
        self.edit(move |p, _| {
            p.set_enabled(&id, on);
            Ok(None)
        });
    }

    /// Move a técnica `delta` posições (negativo: para cima, que roda antes).
    pub fn move_technique(&mut self, id: &TechId, delta: i32) {
        let id = id.clone();
        self.edit(move |p, _| {
            let rows = presets::rows(p);
            let at = rows.iter().position(|r| r.id.matches(&id)).ok_or(tr("the preset does not know this technique"))?;
            let to = (at as i32 + delta).clamp(0, rows.len() as i32 - 1) as usize;
            p.move_technique(&id, to);
            Ok(None)
        });
    }

    /// Liga ou desliga todas as técnicas de um efeito.
    pub fn set_effect_enabled(&mut self, file: &str, on: bool) {
        let ids: Vec<TechId> = self.snapshot.groups.iter().filter(|g| g.file == file).flat_map(|g| g.techniques.iter().map(|t| t.id.clone())).collect();
        self.edit(move |p, _| {
            for id in &ids {
                p.set_enabled(id, on);
            }
            Ok(None)
        });
    }

    /// Move um efeito inteiro `delta` posições entre os efeitos (negativo: para cima, que roda antes). As técnicas dele
    /// ficam juntas e na mesma ordem entre si.
    pub fn move_effect(&mut self, file: &str, delta: i32) {
        let file = file.to_string();
        self.edit(move |p, _| {
            let rows = presets::rows(p);
            let mut order: Vec<String> = Vec::new(); // arquivos na ordem em que aparecem
            for r in &rows {
                let f = r.id.file.clone().unwrap_or_else(|| format!("{}.fx", r.id.name));
                if !order.contains(&f) {
                    order.push(f);
                }
            }
            let at = order.iter().position(|f| *f == file).ok_or(tr("the preset does not know this effect"))?;
            let to = (at as i32 + delta).clamp(0, order.len() as i32 - 1) as usize;
            let item = order.remove(at);
            order.insert(to, item);
            let new: Vec<TechId> = order
                .iter()
                .flat_map(|f| rows.iter().filter(move |r| r.id.file.clone().unwrap_or_else(|| format!("{}.fx", r.id.name)) == *f).map(|r| r.id.clone()))
                .collect();
            p.set_order(&new);
            Ok(None)
        });
    }

    /// Define as definições de pré-processador do preset inteiro (`NOME=VALOR,OUTRO`; vazio apaga). O vigia recompila os
    /// efeitos afetados quando o arquivo muda. Nomes que o compilador não aceita são recusados, com o motivo.
    pub fn set_preset_macros(&mut self, text: &str) {
        let parsed = preset::parse_macros(text);
        if let Some(bad) = parsed.iter().find(|(n, v)| !control_core::compiler::valid_define(n, v)) {
            self.done(Some(fmt("invalid definition: {}={} (the name must be an identifier)", &[&bad.0, &format!("{:?}", bad.1)])));
            return;
        }
        self.edit(move |p, _| {
            p.set_macros(&parsed);
            Ok(None)
        });
    }

    /// Escolhe a profundidade do jogo aberto (a layer lê o pedido em poucos quadros). Vale até o jogo fechar.
    pub fn set_depth_pick(&mut self, pick: ipc::DepthPick) {
        let Some(id) = self.game.clone() else { return };
        let Some(layer) = self.snapshot.layers.iter().find(|l| l.status.game_id() == id) else {
            self.done(Some(tr("the game is not open with the layer").into()));
            return;
        };
        let layer_id = layer.id.clone();
        match ipc::set_depth_pick(&layer_id, pick) {
            Ok(()) => {
                // Lembra para as próximas vezes que o jogo abrir (o `game.ini` do jogo).
                if let Some(dir) = &self.presets_dir
                    && let Err(e) = game::set_depth_choice(dir, &id, pick.text().as_deref())
                {
                    self.done(Some(fmt("the choice applies now, but it could not be saved to game.ini: {}", &[&e])));
                }
                self.update_depth_note();
            }
            Err(e) => self.done(Some(fmt("could not save the depth choice: {}", &[&e]))),
        }
    }

    /// Define as definições de pré-processador só de um efeito (as dele valem mais que as do preset).
    pub fn set_effect_macros(&mut self, file: &str, text: &str) {
        let parsed = preset::parse_macros(text);
        if let Some(bad) = parsed.iter().find(|(n, v)| !control_core::compiler::valid_define(n, v)) {
            self.done(Some(fmt("invalid definition: {}={} (the name must be an identifier)", &[&bad.0, &format!("{:?}", bad.1)])));
            return;
        }
        let file = file.to_string();
        self.edit(move |p, _| {
            p.set_effect_macros(&file, &parsed);
            Ok(None)
        });
    }

    /// Tira um efeito inteiro do preset, com as configurações dele.
    pub fn remove_effect(&mut self, file: &str) {
        let file = file.to_string();
        let ids: Vec<TechId> = self.snapshot.groups.iter().filter(|g| g.file == file).flat_map(|g| g.techniques.iter().map(|t| t.id.clone())).collect();
        self.edit(move |p, _| {
            for id in &ids {
                p.remove_technique(id);
            }
            p.remove_effect(&file);
            Ok(None)
        });
    }

    /// Move a técnica para a posição `to` (de 0).
    pub fn move_technique_to(&mut self, id: &TechId, to: usize) {
        let id = id.clone();
        self.edit(move |p, _| {
            if !p.move_technique(&id, to) {
                return Err(tr("the preset does not know this technique").into());
            }
            Ok(None)
        });
    }

    /// Tira a técnica do preset. Se era a última do arquivo, as configurações do efeito (valores e macros) vão junto:
    /// desligar já basta para manter o efeito sem aparecer, então remover é para esquecê-lo.
    pub fn remove_technique(&mut self, id: &TechId) {
        let id = id.clone();
        self.edit(move |p, _| {
            p.remove_technique(&id);
            if let Some(file) = &id.file
                && !presets::rows(p).iter().any(|r| r.id.file.as_deref() == Some(file))
            {
                p.remove_effect(file);
            }
            Ok(None)
        });
    }

    /// Apaga um preset do jogo escolhido. Se era o que o jogo usava, ele volta a seguir o global.
    pub fn delete_preset(&mut self, name: &str) {
        let result = match (&self.game, self.presets_dir.clone()) {
            (Some(game), Some(dir)) => presets::delete_for_game(&dir, game, name),
            _ => Err(tr("pick a game first").to_string()),
        };
        self.loaded = None;
        self.done(result.err());
    }

    /// Adiciona as técnicas de um efeito ao fim do preset, ligadas. Compila o efeito (para saber as técnicas).
    pub fn add_effect(&mut self, name: &str) {
        if let Err(why) = self.ensure_schema(name, true) {
            self.done(Some(format!("{name}: {why}")));
            return;
        }
        let Some(bundle) = self.schemas.get(name) else { return };
        let file = format!("{name}.fx");
        let ids: Vec<TechId> = bundle.manifest.techniques.iter().map(|t| TechId { name: t.name.clone(), file: Some(file.clone()) }).collect();
        if ids.is_empty() {
            self.done(Some(fmt("{}: the effect has no techniques", &[&name])));
            return;
        }
        self.edit(move |p, _| {
            for id in &ids {
                p.set_enabled(id, true);
            }
            Ok(None)
        });
    }

    /// Dá à técnica o atalho (formato XDG, `CTRL+F11`) que a liga e desliga, no `Key<técnica>` do preset (o formato do ReShade); vazio
    /// tira o atalho. O daemon se reinicia sozinho ao ver a mudança.
    pub fn set_technique_key(&mut self, id: &TechId, trigger: &str) {
        let combo = if trigger.trim().is_empty() {
            None
        } else {
            match shortcuts::trigger::normalize(trigger).ok().and_then(|t| shortcuts::trigger::to_vk(&t)) {
                Some((code, ctrl, shift, alt)) => Some(preset::KeyCombo { code, ctrl, shift, alt }),
                None => {
                    self.done(Some(fmt("{}: the preset (ReShade format) does not store this key; avoid Super and special keys", &[&trigger])));
                    return;
                }
            }
        };
        let id = id.clone();
        self.edit(move |p, _| {
            p.set_technique_key(&id, combo);
            Ok(None)
        });
    }

    /// Altera um parâmetro de um efeito do preset.
    pub fn set_control(&mut self, file: &str, name: &str, values: &[f64]) {
        // Os efeitos com auxílio visual de edição (pontos do autofoco...) o mostram enquanto isto se repete.
        if let Some(game) = &self.game {
            let _ = ipc::set_overlay(game, file, name);
        }
        let (file, name, values) = (file.to_string(), name.to_string(), values.to_vec());
        self.edit(move |p, compiler| {
            let compiler = compiler.ok_or(tr("no effect compiler"))?;
            let warnings = presets::set_uniform(p, compiler, &file, &name, &values)?;
            Ok(warnings.into_iter().next())
        });
    }

    /// Volta os parâmetros de um efeito aos padrões (apaga os valores dele do preset).
    pub fn reset_effect(&mut self, file: &str) {
        let file = file.to_string();
        self.edit(move |p, _| {
            let names: Vec<String> = p.values(&file).iter().map(|(n, _)| n.to_string()).collect();
            for n in names {
                p.remove_value(&file, &n);
            }
            Ok(None)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipc::{EffectState, LayerStatus, SwapchainStatus};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn golden() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../fx-bundle/tests/data/Sepia")
    }

    struct Env {
        root: PathBuf,
        compiler: Compiler,
    }

    /// Um ambiente com um efeito `Sepia.fx` e um "fx2spv" que apenas copia o bundle de referência.
    fn env(name: &str) -> Env {
        let root = std::env::temp_dir().join(format!("lr-home-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("effects")).unwrap();
        fs::create_dir_all(root.join("Presets")).unwrap();
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

    fn open_game(id: &str) -> Vec<LiveLayer> {
        vec![LiveLayer {
            id: "layer-1-1".into(),
            status: LayerStatus { perf: None,
                depth: Vec::new(), depth_passes: String::new(),
                techniques: Vec::new(),
                exe: id.into(),
                app: "App".into(),
                swapchains: vec![SwapchainStatus { width: 800, height: 600, format: 44, images: 3, effect: EffectState::Idle }],
            },
        }]
    }

    fn home(e: Env, layers: Vec<LiveLayer>) -> (Home, PathBuf) {
        let state = ipc::Writer::create_at(&e.root.join("state-dir/state")).unwrap();
        let presets = e.root.join("Presets");
        (Home::with_layers(Ok(e.compiler), Some(presets.clone()), state, Box::new(move || layers.clone())), presets)
    }

    fn sepia() -> TechId {
        TechId::new("Sepia", "Sepia.fx")
    }

    #[test]
    fn it_starts_on_the_open_game_with_nothing_to_show() {
        let (h, _) = home(env("start"), open_game("vkcube"));
        let s = h.snapshot();
        assert_eq!(s.game.as_deref(), Some("vkcube"));
        assert_eq!(s.games, [GameEntry { id: "vkcube".into(), open: true }]);
        assert!(s.preset.is_none() && s.techniques.is_empty());
        assert_eq!(s.effects, ["Sepia"]);
    }

    #[test]
    fn adding_an_effect_to_a_game_without_preset_creates_its_default_preset() {
        let (mut h, presets) = home(env("add"), open_game("vkcube"));
        h.add_effect("Sepia");
        assert!(h.snapshot().notice.is_none(), "{:?}", h.snapshot().notice);
        assert_eq!(fs::read_to_string(presets.join("vkcube/game.ini")).unwrap(), "preset=Padrão.ini\n");
        assert!(fs::read_to_string(presets.join("vkcube/Padrão.ini")).unwrap().contains("Techniques=Sepia@Sepia.fx"));
        let s = h.snapshot();
        assert_eq!(s.active_preset.as_deref(), Some("Padrão"));
        assert_eq!(s.presets, ["Padrão"]);
        assert_eq!(s.techniques.len(), 1);
        let t = &s.techniques[0];
        assert_eq!((t.label.as_str(), t.file.as_str(), t.enabled), ("Sépia", "Sepia.fx", true));
        assert_eq!(t.controls.len(), 1);
        assert_eq!(t.controls[0].value, vec![1.0]);
    }

    #[test]
    fn editing_a_game_that_uses_the_global_preset_copies_it_and_leaves_the_global_alone() {
        let e = env("inherit");
        fs::write(e.root.join("Presets/preset.ini"), "; global\nTechniques=Sepia@Sepia.fx\n").unwrap();
        let (mut h, presets) = home(e, open_game("vkcube"));
        assert!(h.snapshot().preset.as_ref().unwrap().inherited, "mostra o global que o jogo herda");
        assert!(h.snapshot().techniques[0].enabled);

        h.set_technique_enabled(&sepia(), false);
        assert!(!h.snapshot().techniques[0].enabled);
        assert!(!h.snapshot().preset.as_ref().unwrap().inherited, "agora o jogo tem o preset dele");
        assert_eq!(fs::read_to_string(presets.join("preset.ini")).unwrap(), "; global\nTechniques=Sepia@Sepia.fx\n", "o global não mudou");
    }

    #[test]
    fn controls_are_written_to_the_preset_and_reset_removes_them() {
        let (mut h, presets) = home(env("controls"), open_game("vkcube"));
        h.add_effect("Sepia");
        h.set_control("Sepia.fx", "Strength", &[0.25]);
        assert!(h.snapshot().notice.is_none(), "{:?}", h.snapshot().notice);
        assert_eq!(h.snapshot().techniques[0].controls[0].value, vec![0.25]);
        assert!(fs::read_to_string(presets.join("vkcube/Padrão.ini")).unwrap().contains("[Sepia.fx]\nStrength=0.25"));

        h.set_control("Sepia.fx", "Strength", &[9.0]);
        assert!(h.snapshot().notice.as_deref().is_some_and(|n| n.contains("outside the range")));

        h.reset_effect("Sepia.fx");
        assert_eq!(h.snapshot().techniques[0].controls[0].value, vec![1.0]);
        assert!(h.snapshot().techniques[0].controls[0].is_default());
    }

    #[test]
    fn techniques_can_be_reordered_and_removed() {
        let e = env("order");
        fs::write(e.root.join("Presets/preset.ini"), "Techniques=Sepia@Sepia.fx,Outra@Outra.fx\n").unwrap();
        let (mut h, _) = home(e, vec![]);
        assert_eq!(h.snapshot().game, None, "sem jogo aberto mostra o preset global");
        h.move_technique(&TechId::new("Outra", "Outra.fx"), -1);
        let names: Vec<_> = h.snapshot().techniques.iter().map(|t| t.id.name.clone()).collect();
        assert_eq!(names, ["Outra", "Sepia"]);
        h.move_technique(&TechId::new("Outra", "Outra.fx"), -5);
        assert_eq!(h.snapshot().techniques[0].id.name, "Outra", "limitado ao começo");
        h.remove_technique(&TechId::new("Outra", "Outra.fx"));
        assert_eq!(h.snapshot().techniques.len(), 1);
        assert_eq!(h.snapshot().techniques[0].id.name, "Sepia");
    }

    #[test]
    fn what_the_layer_did_with_each_technique_is_shown_for_the_open_game() {
        let e = env("runtime");
        fs::write(e.root.join("Presets/preset.ini"), "Techniques=Sepia@Sepia.fx,CAS@CAS.fx,Outra@Outra.fx\nTechniqueSorting=Sepia@Sepia.fx,CAS@CAS.fx,Outra@Outra.fx,Off@Off.fx\n").unwrap();
        let mut layers = open_game("vkcube");
        layers[0].status.techniques = vec![
            ipc::TechniqueStatus { name: "Sepia".into(), state: TechniqueState::Applied },
            ipc::TechniqueStatus { name: "CAS".into(), state: TechniqueState::Skipped("passe 0: escrita sRGB numa textura".into()) },
        ];
        let (h, _) = home(e, layers);
        let rt: Vec<_> = h.snapshot().techniques.iter().map(|t| (t.id.name.as_str(), t.runtime.clone())).collect();
        assert_eq!(
            rt,
            [
                ("Sepia", Some(TechniqueState::Applied)),
                ("CAS", Some(TechniqueState::Skipped("passe 0: escrita sRGB numa textura".into()))),
                ("Outra", None), // a layer não a conhece (ainda não construiu)
                ("Off", None),   // desligada
            ]
        );
    }

    #[test]
    fn the_effect_analysis_shows_up_in_the_snapshot_as_the_worker_saves_it() {
        let e = env("analysis");
        let compiler_root = e.root.clone();
        let (mut h, _) = home(e, vec![]);
        assert_eq!(h.snapshot().analysis, (0, 1));
        assert!(h.snapshot().infos.is_empty());
        // O worker (aqui, à mão) analisa o efeito e salva.
        let c = Compiler { fx2spv: compiler_root.join("fake-fx2spv"), effects: vec![config::SearchPath::flat(compiler_root.join("effects"))], includes: vec![], textures: vec![], cache: compiler_root.join("cache") };
        analysis::ensure(&c, "Sepia");
        h.refresh();
        assert_eq!(h.snapshot().analysis, (1, 1));
        let info = &h.snapshot().infos["Sepia"];
        assert!(info.compatible());
        assert_eq!(info.techniques, ["Sepia"]);
    }

    #[test]
    fn removing_the_last_technique_of_an_effect_deletes_its_settings() {
        let (mut h, presets) = home(env("forget"), open_game("vkcube"));
        h.add_effect("Sepia");
        h.set_control("Sepia.fx", "Strength", &[0.25]);
        let file = presets.join("vkcube/Padrão.ini");
        assert!(fs::read_to_string(&file).unwrap().contains("[Sepia.fx]"));
        // Desligar não apaga nada: as configurações ficam.
        h.set_technique_enabled(&sepia(), false);
        assert!(fs::read_to_string(&file).unwrap().contains("Strength=0.25"));
        // Remover esquece o efeito e as configurações; adicionar de novo começa dos padrões.
        h.remove_technique(&sepia());
        assert!(!fs::read_to_string(&file).unwrap().contains("Sepia"), "{}", fs::read_to_string(&file).unwrap());
        h.add_effect("Sepia");
        assert_eq!(h.snapshot().techniques[0].controls[0].value, vec![1.0]);
    }

    fn blur_techniques(h: &mut Home) {
        // Um efeito com duas técnicas (como o desfoque gaussiano horizontal e vertical), no mesmo arquivo.
        h.edit(|p, _| {
            p.set_enabled(&TechId::new("Horizontal", "Blur.fx"), true);
            p.set_enabled(&TechId::new("Vertical", "Blur.fx"), true);
            Ok(None)
        });
    }

    #[test]
    fn techniques_of_one_effect_are_one_group_with_one_set_of_controls() {
        let (mut h, _) = home(env("groups"), open_game("vkcube"));
        h.add_effect("Sepia");
        blur_techniques(&mut h);
        let g = &h.snapshot().groups;
        assert_eq!(g.iter().map(|g| (g.file.as_str(), g.techniques.len())).collect::<Vec<_>>(), [("Sepia.fx", 1), ("Blur.fx", 2)]);
        assert_eq!(g[0].name, "Sepia");
        assert_eq!(g[0].controls.len(), 1, "os parâmetros são do efeito");
        assert!(g[1].techniques.iter().all(|t| t.controls.is_empty()), "e não se repetem em cada técnica");
    }

    #[test]
    fn an_effect_moves_removes_and_toggles_as_a_whole() {
        let (mut h, presets) = home(env("group-actions"), open_game("vkcube"));
        h.add_effect("Sepia");
        blur_techniques(&mut h);
        h.set_control("Sepia.fx", "Strength", &[0.5]);
        let order = |h: &Home| h.snapshot().techniques.iter().map(|t| t.id.name.clone()).collect::<Vec<_>>();
        assert_eq!(order(&h), ["Sepia", "Horizontal", "Vertical"]);

        h.move_effect("Blur.fx", -1); // as duas técnicas sobem juntas, na mesma ordem
        assert_eq!(order(&h), ["Horizontal", "Vertical", "Sepia"]);
        h.move_effect("Blur.fx", 1);
        assert_eq!(order(&h), ["Sepia", "Horizontal", "Vertical"]);
        h.move_effect("Sepia.fx", -5);
        assert_eq!(order(&h), ["Sepia", "Horizontal", "Vertical"], "já é o primeiro");

        h.set_effect_enabled("Blur.fx", false);
        assert!(h.snapshot().groups[1].techniques.iter().all(|t| !t.enabled));
        assert!(h.snapshot().groups[0].all_enabled());
        h.set_technique_enabled(&TechId::new("Horizontal", "Blur.fx"), true);
        assert!(h.snapshot().groups[1].any_enabled() && !h.snapshot().groups[1].all_enabled());

        h.remove_effect("Sepia.fx");
        assert_eq!(order(&h), ["Horizontal", "Vertical"]);
        assert!(!fs::read_to_string(presets.join("vkcube/Padrão.ini")).unwrap().contains("Sepia"), "as configurações vão junto");
    }

    #[test]
    fn macros_of_the_preset_and_of_an_effect_are_edited_and_validated() {
        let (mut h, presets) = home(env("macros"), open_game("vkcube"));
        h.add_effect("Sepia");
        h.set_preset_macros("QUALITY=2, FLAG");
        assert_eq!(h.snapshot().preset_macros, "QUALITY=2,FLAG=1", "valor vazio vale 1");
        h.set_effect_macros("Sepia.fx", "FAST=3");
        assert_eq!(h.snapshot().groups[0].macros, "FAST=3");
        let text = fs::read_to_string(presets.join("vkcube/Padrão.ini")).unwrap();
        assert!(text.contains("PreprocessorDefinitions=QUALITY=2,FLAG=1") && text.contains("PreprocessorDefinitions=FAST=3"), "{text}");

        h.set_preset_macros("1ruim=2");
        assert!(h.snapshot().notice.as_deref().is_some_and(|n| n.contains("invalid definition")), "{:?}", h.snapshot().notice);
        assert_eq!(h.snapshot().preset_macros, "QUALITY=2,FLAG=1", "a inválida não foi gravada");
        h.set_preset_macros("");
        assert_eq!(h.snapshot().preset_macros, "");
    }

    #[test]
    fn deleting_a_preset_from_the_screen() {
        let (mut h, presets) = home(env("delete"), open_game("vkcube"));
        h.add_effect("Sepia");
        h.new_preset("Noturno");
        h.delete_preset("Noturno");
        assert!(!presets.join("vkcube/Noturno.ini").exists());
        assert_eq!(h.snapshot().presets, ["Padrão"]);
        assert_eq!(h.snapshot().active_preset, None, "o jogo ficou sem o preset ativo: volta ao global (que não existe)");
        h.delete_preset("Nada");
        assert!(h.snapshot().notice.is_some());
    }

    #[test]
    fn adding_an_effect_that_does_not_exist_says_why() {
        let (mut h, _) = home(env("whynot"), open_game("vkcube"));
        h.add_effect("Inexistente");
        let notice = h.snapshot().notice.clone().unwrap();
        assert!(notice.starts_with("Inexistente:") && notice.contains("could not find"), "{notice}");
    }

    #[test]
    fn an_effect_that_was_never_compiled_shows_a_problem_not_a_crash() {
        let e = env("problem");
        fs::write(e.root.join("Presets/preset.ini"), "Techniques=Fantasma@Fantasma.fx\n").unwrap();
        let (h, _) = home(e, vec![]);
        let t = &h.snapshot().techniques[0];
        assert_eq!(t.label, "Fantasma");
        assert!(t.problem.as_deref().is_some_and(|p| p.contains("not compiled")), "{:?}", t.problem);
        assert!(t.controls.is_empty());
    }

    #[test]
    fn presets_of_a_game_can_be_created_and_switched() {
        let (mut h, _) = home(env("presets"), open_game("vkcube"));
        h.add_effect("Sepia");
        h.new_preset("Noturno");
        assert_eq!(h.snapshot().active_preset.as_deref(), Some("Noturno"));
        assert_eq!(h.snapshot().presets, ["Noturno", "Padrão"]);
        h.set_technique_enabled(&sepia(), false);
        h.select_preset(Some("Padrão"));
        assert!(h.snapshot().techniques[0].enabled, "o Padrão não foi tocado");
        h.select_preset(Some("Noturno"));
        assert!(!h.snapshot().techniques[0].enabled);
        h.select_preset(None);
        assert_eq!(h.snapshot().active_preset, None, "sem preset global, o jogo fica sem nada");
        assert!(h.snapshot().techniques.is_empty());
    }

    #[test]
    fn a_file_edited_from_outside_shows_up_on_the_next_refresh() {
        let e = env("outside");
        fs::write(e.root.join("Presets/preset.ini"), "Techniques=Sepia@Sepia.fx\n").unwrap();
        let (mut h, presets) = home(e, vec![]);
        assert!(h.snapshot().techniques[0].enabled);
        fs::write(presets.join("preset.ini"), "Techniques=\nTechniqueSorting=Sepia@Sepia.fx\n").unwrap();
        fs::File::options().write(true).open(presets.join("preset.ini")).unwrap().set_modified(SystemTime::now() + std::time::Duration::from_secs(5)).unwrap();
        h.refresh();
        assert!(!h.snapshot().techniques[0].enabled);
    }

    #[test]
    fn choosing_the_game_changes_what_is_shown_and_an_unknown_id_is_ignored() {
        let e = env("games");
        fs::create_dir_all(e.root.join("Presets/Antigo")).unwrap();
        fs::write(e.root.join("Presets/Antigo/game.ini"), "preset=A.ini\n").unwrap();
        fs::write(e.root.join("Presets/Antigo/A.ini"), "Techniques=Sepia@Sepia.fx\n").unwrap();
        let (mut h, _) = home(e, open_game("vkcube"));
        let ids: Vec<_> = h.snapshot().games.iter().map(|g| (g.id.as_str(), g.open)).collect();
        assert_eq!(ids, [("vkcube", true), ("Antigo", false)], "os abertos primeiro");
        h.select_game(Some("Antigo"));
        assert_eq!(h.snapshot().techniques.len(), 1);
        h.select_game(Some("../x"));
        assert_eq!(h.snapshot().game, None, "id inválido vira o preset global");
    }

    #[test]
    fn the_global_enable_switch_goes_through_the_shared_state() {
        let (mut h, _) = home(env("switch"), vec![]);
        assert!(!h.snapshot().enabled);
        h.set_enabled(true);
        assert!(h.snapshot().enabled);
    }
}

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
fn folder_of(roots: &[PathBuf], dir: &Path) -> String {
    roots
        .iter()
        .filter_map(|r| dir.strip_prefix(r).ok())
        .min_by_key(|rel| rel.components().count())
        .map(|rel| rel.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default()
}
