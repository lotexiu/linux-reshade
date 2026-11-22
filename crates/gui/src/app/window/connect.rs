//! Os widgets fixos da janela ligados às ações do modelo, e a pergunta antes de apagar um preset.

use super::*;

impl Ui {
    /// Liga os widgets fixos da janela às ações do modelo.
    pub(super) fn connect(self: &Rc<Self>, redraw: impl Fn() + Clone + 'static, name_entry: gtk::Entry, create: gtk::Button, new_popover: gtk::Popover) {
        let weak = Rc::downgrade(self);

        self.enabled.connect_active_notify({
            let (weak, redraw) = (weak.clone(), redraw.clone());
            move |sw| {
                if let Some(ui) = weak.upgrade()
                    && !ui.updating.get()
                {
                    ui.home.borrow_mut().set_enabled(sw.is_active());
                    redraw();
                }
            }
        });

        self.game.connect_selected_notify({
            let (weak, redraw) = (weak.clone(), redraw.clone());
            move |dd| {
                let Some(ui) = weak.upgrade() else { return };
                if ui.updating.get() {
                    return;
                }
                // O último item não é um jogo: abre a página dos apps Flatpak e a escolha volta ao jogo que estava.
                if dd.selected() as usize >= ui.game_ids.borrow().len() {
                    ui.show_page("flatpak");
                    ui.apply_game_selection();
                    return;
                }
                let id = ui.game_ids.borrow().get(dd.selected() as usize).cloned().flatten();
                ui.home.borrow_mut().select_game(id.as_deref());
                redraw();
            }
        });

        self.preset.connect_selected_notify({
            let (weak, redraw) = (weak.clone(), redraw.clone());
            move |dd| {
                let Some(ui) = weak.upgrade() else { return };
                if ui.updating.get() {
                    return;
                }
                let name = (dd.selected() as usize).checked_sub(1).and_then(|i| ui.preset_names.borrow().get(i).cloned());
                ui.home.borrow_mut().select_preset(name.as_deref());
                redraw();
            }
        });

        create.connect_clicked({
            let (weak, redraw, entry) = (weak.clone(), redraw.clone(), name_entry.clone());
            move |_| {
                let Some(ui) = weak.upgrade() else { return };
                let name = entry.text().to_string();
                if !name.trim().is_empty() {
                    ui.home.borrow_mut().new_preset(&name);
                    entry.set_text("");
                    new_popover.popdown();
                    redraw();
                }
            }
        });
        name_entry.connect_activate({
            let create = create.clone();
            move |_| create.emit_clicked()
        });

        self.depth_button.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.show_page("depth");
                }
            }
        });
        // A profundidade escolhida pela pessoa: grava o pedido que a layer lê.
        self.depth_view.connect_changed({
            let weak = weak.clone();
            move |pick| {
                if let Some(ui) = weak.upgrade()
                    && !ui.updating.get()
                {
                    ui.act(|h| h.set_depth_pick(pick));
                }
            }
        });

        // As definições do preset: cada mudança no popover aplica (os números esperam um instante, pois recompilam).
        self.macros.connect_changed({
            let weak = weak.clone();
            move |text| {
                if let Some(ui) = weak.upgrade()
                    && !ui.updating.get()
                    && text != ui.home.borrow().snapshot().preset_macros
                {
                    ui.act(|h| h.set_preset_macros(&text));
                }
            }
        });

        self.delete_preset.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.ask_delete_preset();
                }
            }
        });

        self.search.connect_search_changed({
            let weak = weak.clone();
            move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.apply_search();
                }
            }
        });

        // Lê o estado de tempos em tempos: jogos que abrem e fecham, o atalho global, o `lrctl`, o arquivo editado à mão.
        glib::timeout_add_local(REFRESH, move || match weak.upgrade() {
            Some(ui) => {
                ui.activity.set(&crate::shared::activity::compiling(&ui.cache_dir));
                let t0 = std::time::Instant::now();
                ui.home.borrow_mut().refresh();
                let t1 = std::time::Instant::now();
                let changed = ui.home.borrow().version() != ui.applied_version.get();
                if changed {
                    ui.apply();
                }
                if std::env::var_os("LRDBG").is_some() {
                    eprintln!("DBG tick: refresh {:?}  {} {:?}", t1 - t0, if changed { "apply" } else { "(nada mudou)" }, t1.elapsed());
                }
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });
    }

    /// Apaga o preset em uso, depois de perguntar (a menos que a pessoa tenha pedido para não perguntar mais).
    pub(super) fn ask_delete_preset(self: &Rc<Self>) {
        let Some(name) = self.home.borrow().snapshot().active_preset.clone() else { return };
        if !crate::features::settings::prefs::confirm_delete_preset() {
            self.act(|h| h.delete_preset(&name));
            return;
        }
        let dialog = gtk::Window::builder().transient_for(&self.window).modal(true).title(tr("Delete preset")).default_width(380).resizable(false).build();
        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        margins(&body, 16);
        let text = gtk::Label::new(Some(&fmt("Delete the preset \"{}\"?\nThe file is removed and its settings are lost.", &[&name])));
        text.set_wrap(true);
        text.set_halign(gtk::Align::Start);
        text.set_xalign(0.0);
        let never = gtk::CheckButton::with_label(tr("Don't ask again (you can turn this back on in the settings)"));
        let cancel = gtk::Button::with_label(tr("Cancel"));
        let delete = gtk::Button::with_label(tr("Delete"));
        delete.add_css_class("destructive-action");
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        buttons.set_halign(gtk::Align::End);
        buttons.append(&cancel);
        buttons.append(&delete);
        body.append(&text);
        body.append(&never);
        body.append(&buttons);
        dialog.set_child(Some(&body));
        cancel.connect_clicked({
            let dialog = dialog.clone();
            move |_| dialog.close()
        });
        delete.connect_clicked({
            let (weak, dialog, never) = (Rc::downgrade(self), dialog.clone(), never.clone());
            move |_| {
                if never.is_active() {
                    crate::features::settings::prefs::set_confirm_delete_preset(false);
                }
                dialog.close();
                if let Some(ui) = weak.upgrade() {
                    ui.act(|h| h.delete_preset(&name));
                }
            }
        });
        dialog.present();
    }
}
