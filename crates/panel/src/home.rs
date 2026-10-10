//! A tela principal, no estilo da aba Home do ReShade: o jogo, o preset dele, e a lista de técnicas em ordem
//! de execução, cada uma com os seus controles. Nada aqui sabe desenhar; a interface só mostra o
//! [`HomeSnapshot`] e repassa os cliques.
//!
//! Cada ação grava o arquivo do preset na hora (como o "Auto Save" do ReShade); o vigia (`lrctl daemon`) relê o
//! arquivo e a layer aplica. Editar um jogo que ainda usa o preset global cria antes o preset dele (`Padrão`, uma
//! cópia do global), para uma mudança num jogo nunca alterar os outros sem querer.

use crate::control::{Control, controls_of};
use control_core::compiler::{Compiler, DEFAULT_SIZE, valid_name};
use control_core::analysis::{self, EffectInfo};
use control_core::{game_plan, presets};
use fx_bundle::Bundle;
use fx_bundle::manifest::{Scalar, Value};
use ipc::{LiveLayer, TechniqueState};
use preset::game::{self, Source};
use preset::{Preset, TechId};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

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
    /// Efeitos que podem ser adicionados (nomes de arquivo sem `.fx`).
    pub effects: Vec<String>,
    /// O que se sabe de cada efeito (compatibilidade e características), conforme a análise em segundo plano avança.
    pub infos: HashMap<String, EffectInfo>,
    /// Andamento da análise: efeitos analisados e total.
    pub analysis: (usize, usize),
    pub layers: Vec<LiveLayer>,
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
    snapshot: HomeSnapshot,
}

fn stamp(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
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
        let mut home = Self { compiler, presets_dir, state, layers, game: None, loaded: None, schemas: HashMap::new(), analysis_seen: None, snapshot: HomeSnapshot::default() };
        if let Err(why) = &home.compiler {
            home.snapshot.notice = Some(format!("sem compilador de efeitos: {why}"));
        } else if home.presets_dir.is_none() {
            home.snapshot.notice = Some("nenhuma pasta de dados escolhida: rode 'lrctl config init'".into());
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
        self.presets_dir.as_deref().ok_or_else(|| "nenhuma pasta de dados escolhida: rode 'lrctl config init'".to_string())
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
        self.snapshot.enabled = self.state.effects_enabled();
        self.snapshot.layers = (self.layers)();
        self.snapshot.effects = self
            .compiler
            .as_ref()
            .map(|c| c.available().into_iter().map(|(n, _)| n).collect())
            .unwrap_or_default();

        self.refresh_analysis();

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
        self.snapshot.games = games;
        self.snapshot.game = self.game.clone();

        self.load_preset();
        self.build_view();
    }

    /// Relê as análises dos efeitos quando o worker salvou mais alguma.
    fn refresh_analysis(&mut self) {
        let Ok(compiler) = &self.compiler else { return };
        let saved = analysis::count_saved(compiler);
        if self.analysis_seen != Some(saved) || self.snapshot.infos.len() != saved.min(self.snapshot.effects.len()) {
            self.snapshot.infos = self.snapshot.effects.iter().filter_map(|n| analysis::load(compiler, n).map(|i| (n.clone(), i))).collect();
            self.analysis_seen = Some(saved);
        }
        self.snapshot.analysis = (self.snapshot.infos.len(), self.snapshot.effects.len());
    }

    /// Carrega o arquivo do preset que vale agora, se mudou (ou se é outro).
    fn load_preset(&mut self) {
        let Some((path, inherited)) = self.resolve() else {
            self.loaded = None;
            self.snapshot.preset = None;
            return;
        };
        let current = stamp(&path);
        let fresh = self.loaded.as_ref().is_some_and(|l| l.path == path && l.stamp == current);
        if !fresh {
            match presets::load(&path) {
                Ok(preset) => self.loaded = Some(Loaded { path: path.clone(), stamp: current, preset }),
                Err(why) => {
                    self.loaded = None;
                    self.snapshot.notice = Some(why);
                }
            }
        }
        self.snapshot.preset = self.loaded.as_ref().map(|l| PresetInfo { name: display_name(&l.path), path: l.path.clone(), inherited });
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
        let Some(loaded) = &self.loaded else { return };
        let preset = loaded.preset.clone();
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
                    (label_of(b, &row.id.name).unwrap_or_else(|| row.id.name.clone()), (!known).then(|| "o efeito não declara esta técnica".to_string()))
                }
                None => (row.id.name.clone(), Some("efeito ainda não compilado (abra um jogo com o daemon rodando, ou adicione-o de novo)".to_string())),
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
            self.snapshot.techniques.push(TechniqueView { id: row.id, label, file, enabled: row.enabled, controls, runtime, problem });
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
            return Err(format!("{name:?} não é um nome de efeito válido"));
        }
        let compiler = self.compiler.as_ref().map_err(|why| format!("sem compilador de efeitos: {why}"))?;
        let dir = match compiler.any_bundle(name) {
            Some(dir) => dir,
            None if compile => compiler.ensure(name, DEFAULT_SIZE.0, DEFAULT_SIZE.1).map_err(|e| e.to_string())?.dir_owned(),
            None => return Err("efeito ainda não compilado".into()),
        };
        let bundle = Bundle::load(&dir).map_err(|e| e.to_string())?;
        self.schemas.insert(name.to_string(), bundle);
        Ok(())
    }

    fn done(&mut self, notice: Option<String>) {
        self.refresh();
        self.snapshot.notice = notice;
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
            _ => Err("escolha um jogo primeiro".to_string()),
        };
        self.loaded = None;
        self.done(result.err());
        self.prepare_effects();
    }

    /// Cria um preset do jogo (cópia do que vale agora) e passa a usá-lo.
    pub fn new_preset(&mut self, name: &str) {
        let result = match (&self.game, self.presets_dir.clone()) {
            (Some(game), Some(dir)) => presets::new_for_game(&dir, game, name.trim()).map(drop),
            _ => Err("escolha um jogo primeiro".to_string()),
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
            let at = rows.iter().position(|r| r.id.matches(&id)).ok_or("o preset não conhece esta técnica")?;
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
            let at = order.iter().position(|f| *f == file).ok_or("o preset não conhece este efeito")?;
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
                return Err("o preset não conhece esta técnica".into());
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
            _ => Err("escolha um jogo primeiro".to_string()),
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
            self.done(Some(format!("{name}: o efeito não tem técnicas")));
            return;
        }
        self.edit(move |p, _| {
            for id in &ids {
                p.set_enabled(id, true);
            }
            Ok(None)
        });
    }

    /// Altera um parâmetro de um efeito do preset.
    pub fn set_control(&mut self, file: &str, name: &str, values: &[f64]) {
        let (file, name, values) = (file.to_string(), name.to_string(), values.to_vec());
        self.edit(move |p, compiler| {
            let compiler = compiler.ok_or("sem compilador de efeitos")?;
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
            status: LayerStatus {
                depth: Vec::new(),
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
        assert!(h.snapshot().notice.as_deref().is_some_and(|n| n.contains("fora da faixa")));

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
        assert!(notice.starts_with("Inexistente:") && notice.contains("não achei"), "{notice}");
    }

    #[test]
    fn an_effect_that_was_never_compiled_shows_a_problem_not_a_crash() {
        let e = env("problem");
        fs::write(e.root.join("Presets/preset.ini"), "Techniques=Fantasma@Fantasma.fx\n").unwrap();
        let (h, _) = home(e, vec![]);
        let t = &h.snapshot().techniques[0];
        assert_eq!(t.label, "Fantasma");
        assert!(t.problem.as_deref().is_some_and(|p| p.contains("não compilado")), "{:?}", t.problem);
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
                for name in analysis::pending(&compiler) {
                    analysis::ensure(&compiler, &name);
                }
                // Efeitos novos nas pastas aparecem depois: olha de novo de vez em quando.
                std::thread::sleep(std::time::Duration::from_secs(10));
            }
        })
        .ok()
}
