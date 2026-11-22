//! Do modelo à tela: o que mudou desde o último desenho (cabeçalho, jogo, preset, lista de efeitos).

use super::*;

impl Ui {
    /// Empurra o estado do modelo para os widgets.
    pub fn apply(self: &Rc<Self>) {
        self.applied_version.set(self.home.borrow().version());
        // Enquanto a janela põe valores do modelo nos widgets, os avisos deles não são da pessoa: o guarda desliga isto sozinho ao sair
        // (um `return` ou um pânico no meio deixaria a interface ignorando todos os cliques para sempre).
        let _programmatic = Programmatic::enter(&self.updating);
        {
            let home = self.home.borrow();
            let s = home.snapshot();

            if s.enabled != self.enabled.is_active() {
                self.enabled.set_active(s.enabled);
            }

            // Jogos.
            let ids: Vec<Option<String>> = std::iter::once(None).chain(s.games.iter().map(|g| Some(g.id.clone()))).collect();
            if *self.game_ids.borrow() != ids {
                let mut items = vec![global_game().to_string()];
                items.extend(s.games.iter().map(|g| if g.open { format!("● {}", g.id) } else { g.id.clone() }));
                items.push(add_flatpak().to_string());
                let refs: Vec<&str> = items.iter().map(String::as_str).collect();
                self.game.set_model(Some(&gtk::StringList::new(&refs)));
                *self.game_ids.borrow_mut() = ids.clone();
            }
            let selected = ids.iter().position(|g| *g == s.game).unwrap_or(0) as u32;
            if self.game.selected() != selected {
                self.game.set_selected(selected);
            }

            // Presets do jogo.
            let has_game = s.game.is_some();
            self.preset.set_sensitive(has_game);
            self.new_preset.set_sensitive(has_game);
            self.macros.set_text(&s.preset_macros);
            self.delete_preset.set_sensitive(s.active_preset.is_some());
            self.new_preset.set_tooltip_text(Some(if has_game { tr("New preset for this game (copy of the current one)") } else { tr("Pick a game to create a preset for it") }));
            if *self.preset_names.borrow() != s.presets {
                let mut items = vec![inherit_preset().to_string()];
                items.extend(s.presets.iter().cloned());
                let refs: Vec<&str> = items.iter().map(String::as_str).collect();
                self.preset.set_model(Some(&gtk::StringList::new(&refs)));
                *self.preset_names.borrow_mut() = s.presets.clone();
            }
            let active = s.active_preset.as_ref().and_then(|a| s.presets.iter().position(|p| p == a)).map_or(0, |i| i as u32 + 1);
            if self.preset.selected() != active {
                self.preset.set_selected(active);
            }
            self.preset_note.set_text(&match (&s.preset, has_game) {
                (Some(p), true) if p.inherited => fmt("This game uses the global preset ({}). The first change creates its own preset.", &[&p.name]),
                (Some(p), _) => format!("{}", p.path.display()),
                (None, true) => tr("This game has no preset. Add an effect to create one.").to_string(),
                (None, false) => tr("There is no global preset yet. Add an effect to create it (it applies to games without their own preset).").to_string(),
            });

            match &s.depth_note {
                Some(n) => {
                    self.depth_note.set_text(&n.text);
                    if n.attention {
                        self.depth_note.add_css_class("warning");
                    } else {
                        self.depth_note.remove_css_class("warning");
                    }
                    self.depth_note.set_visible(true);
                }
                None => self.depth_note.set_visible(false),
            }

            self.depth_button.set_visible(!s.depth_choices.is_empty());
            self.depth_view.set(&s.depth_choices, &s.depth_clears, &s.depth_passes, s.depth_pick, self.stack.visible_child_name().as_deref() == Some("depth"));

            // A página "Adicionar efeito" (só monta a lista enquanto está na tela).
            self.add_view.set(&s.effects, &s.folders, &s.infos, s.analysis, self.stack.visible_child_name().as_deref() == Some("add"));

            self.show_groups(&s.groups);

            // O botão "Flatpak…" só aparece se o jogo escolhido é um app Flatpak em que o linux-reshade foi ligado.
            self.flatpak_button.set_visible(s.game.as_deref().is_some_and(|g| control_core::flatpak::app_of_game(g).is_some()));

            self.empty.set_visible(s.groups.is_empty());
            self.empty.set_text(if s.groups.is_empty() {
                tr("No effects in the preset.\nUse \"Add effect\" to start.")
            } else {
                ""
            });
            self.games_view.borrow_mut().show(&s.layers);
            self.notice.set_visible(s.notice.is_some());
            self.notice.set_text(s.notice.as_deref().unwrap_or(""));
        }
    }

    /// O que exige mexer na lista: a ordem e a assinatura de cada linha (ver `EffectRow::signature_of`).
    pub(super) fn signature_of(groups: &[EffectGroup]) -> String {
        groups.iter().map(|g| format!("{}\n", EffectRow::signature_of(g))).collect()
    }

    /// A lista de efeitos: refeita quando a estrutura muda; senão só atualiza os valores dos controles.
    pub(super) fn show_groups(self: &Rc<Self>, groups: &[EffectGroup]) {
        let signature = Self::signature_of(groups);
        if signature != *self.signature.borrow() {
            self.rebuild(groups);
            *self.signature.borrow_mut() = signature;
        }
        for (row, g) in self.rows.borrow().iter().zip(groups) {
            EffectRow::update(row, g);
            if let Some(view) = &row.controls {
                view.borrow_mut().show(&g.controls);
            }
        }
        self.apply_search();
    }

    /// O que as linhas da lista podem pedir à janela (ver `effect_row.rs`).
    pub(super) fn row_ctx(self: &Rc<Self>) -> Rc<RowCtx> {
        let weak = Rc::downgrade(self);
        Rc::new(RowCtx {
            act: Rc::new({
                let weak = weak.clone();
                move |action| {
                    if let Some(ui) = weak.upgrade() {
                        ui.act(action);
                    }
                }
            }),
            programmatic: Rc::new({
                let weak = weak.clone();
                move || weak.upgrade().is_none_or(|ui| ui.updating.get())
            }),
            current_macros: Rc::new({
                let weak = weak.clone();
                move |file| weak.upgrade().and_then(|ui| ui.home.borrow().snapshot().groups.iter().find(|g| g.file == file).map(|g| g.macros.clone())).unwrap_or_default()
            }),
            expanded: self.expanded.clone(),
            scroller: self.scroller.clone(),
        })
    }

    /// Põe a lista de linhas no estado de `groups` mexendo só no que mudou, como o `key=` de uma lista do React: a linha de um efeito que
    /// continua igual é a MESMA (não perde o foco, o painel aberto nem o clique em andamento), uma que mudou é refeita, uma nova entra e
    /// uma que saiu sai; a ordem se ajusta movendo as linhas, sem recriá-las.
    pub(super) fn rebuild(self: &Rc<Self>, groups: &[EffectGroup]) {
        let adjustment = self.scroller.vadjustment();
        let scroll = adjustment.value();
        let ctx = self.row_ctx();
        let mut old: std::collections::HashMap<String, EffectRow> = self.rows.take().into_iter().map(|r| (r.file.clone(), r)).collect();
        let last = groups.len().saturating_sub(1);
        let mut rows = Vec::with_capacity(groups.len());
        let mut structural = false;
        for (index, g) in groups.iter().enumerate() {
            let signature = EffectRow::signature_of(g);
            let row = match old.remove(&g.file) {
                Some(r) if r.signature == signature => r,
                other => {
                    if let Some(gone) = other {
                        self.list.remove(&gone.row);
                    }
                    structural = true;
                    EffectRow::new(&ctx, index, last, g)
                }
            };
            row.set_position(index, last);
            rows.push(row);
        }
        for (_, gone) in old {
            self.list.remove(&gone.row);
            structural = true;
        }
        // Ordem: cada linha vai à posição dela (as que já estão no lugar não se mexem).
        for (index, r) in rows.iter().enumerate() {
            let in_place = self.list.row_at_index(index as i32).is_some_and(|current| current == r.row);
            if !in_place {
                if r.row.parent().is_some() {
                    self.list.remove(&r.row);
                }
                self.list.insert(&r.row, index as i32);
            }
        }
        *self.rows.borrow_mut() = rows;
        if structural && scroll > 0.0 {
            // Linhas novas mudam o tamanho: a rolagem é devolvida depois que ele é recalculado.
            for delay in [0u64, 40, 150] {
                let adjustment = adjustment.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(delay), move || adjustment.set_value(scroll.min((adjustment.upper() - adjustment.page_size()).max(0.0))));
            }
        }
    }
}
