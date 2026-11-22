//! Do disco e das camadas para o que a tela mostra: reler o preset, a análise dos efeitos, o aviso de profundidade e a lista montada.

use super::*;

impl Home {
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

    pub(super) fn update_depth_note(&mut self) {
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
    pub(super) fn refresh_analysis(&mut self) -> bool {
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
    pub(super) fn load_preset(&mut self) -> bool {
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
    pub(super) fn build_view(&mut self) {
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
    pub(super) fn ensure_schema(&mut self, name: &str, compile: bool) -> Result<(), String> {
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

    /// Compila (se preciso) os efeitos do preset mostrado, para a tela ter rótulos e controles.
    pub(super) fn prepare_effects(&mut self) {
        let Some(loaded) = &self.loaded else { return };
        let files: Vec<String> = presets::rows(&loaded.preset).into_iter().filter_map(|r| r.id.file).collect();
        for file in files {
            let _ = self.ensure_schema(effect_name(&file), true);
        }
        let notice = self.snapshot.notice.clone();
        self.done(notice);
    }
}
