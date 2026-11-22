//! Editar o preset do jogo escolhido: técnicas, efeitos, macros, atalhos, controles e a escolha de profundidade.

use super::*;

impl Home {
    /// O arquivo a editar: o do jogo; se ele ainda usa o global, cria antes o preset `Padrão` dele.
    pub(super) fn writable(&mut self) -> Result<PathBuf, String> {
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
    pub(super) fn edit(&mut self, edit: impl FnOnce(&mut Preset, Option<&Compiler>) -> Result<Option<String>, String>) {
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
