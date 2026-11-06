//! A página "Jogos Flatpak": lista os apps Flatpak instalados (como o Flatseal) e liga o linux-reshade neles com um clique: o painel instala
//! a extensão da layer e ajusta as permissões do app. Para desfazer há o botão "Remover alterações…", que avisa o que será desfeito.
//!
//! É uma página da janela (um `gtk::Stack`), como `add_effect.rs`, e não um menu. O trabalho (rodar o `flatpak`, que leva segundos) roda
//! fora da thread da tela; a linha do app fica desabilitada e mostra o andamento até ele terminar.

use control_core::flatpak::{self, App, State};
use gtk::glib;
use gtk::prelude::*;
use gtk4 as gtk;
use std::cell::RefCell;
use std::rc::Rc;

struct Row {
    app: App,
    status: gtk::Label,
    button: gtk::Button,
    state: State,
}

pub struct FlatpakView {
    pub root: gtk::Box,
    list: gtk::ListBox,
    search: gtk::SearchEntry,
    note: gtk::Label,
    rows: RefCell<Vec<Row>>,
}

fn margins(w: &impl IsA<gtk::Widget>, all: i32) {
    w.set_margin_top(all);
    w.set_margin_bottom(all);
    w.set_margin_start(all);
    w.set_margin_end(all);
}

fn state_text(state: State) -> &'static str {
    match state {
        State::Off => "Desligado",
        State::On => "Ligado",
        State::Global => "Ligado (configuração global)",
        State::DisabledHere => "Desligado só neste app",
    }
}

fn button_text(state: State) -> &'static str {
    match state {
        State::Off => "Ativar",
        State::On => "Remover alterações…",
        State::Global => "Desligar neste app",
        State::DisabledHere => "Seguir a configuração global",
    }
}

impl FlatpakView {
    pub fn new(on_back: impl Fn() + 'static) -> Rc<Self> {
        let back = gtk::Button::from_icon_name("go-previous-symbolic");
        back.set_tooltip_text(Some("Voltar"));
        back.connect_clicked(move |_| on_back());
        let title = gtk::Label::new(Some("Jogos Flatpak"));
        title.add_css_class("title-3");
        title.set_hexpand(true);
        title.set_xalign(0.0);
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Procurar app"));
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        top.append(&back);
        top.append(&title);
        let hint = gtk::Label::new(Some("Apps Flatpak rodam numa caixa que não enxerga a layer do sistema. Ativar instala a extensão da layer para o runtime do app e libera o que ela precisa (como no Flatseal). Vale na próxima vez que o app abrir."));
        hint.set_wrap(true);
        hint.set_xalign(0.0);
        hint.add_css_class("dim-label");
        let note = gtk::Label::new(None);
        note.set_wrap(true);
        note.set_xalign(0.0);
        note.set_visible(false);
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        list.add_css_class("boxed-list");
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_child(Some(&list));
        scroller.set_vexpand(true);
        scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        margins(&root, 14);
        root.append(&top);
        root.append(&hint);
        root.append(&search);
        root.append(&note);
        root.append(&scroller);

        let view = Rc::new(Self { root, list, search, note, rows: RefCell::new(Vec::new()) });
        {
            let weak = Rc::downgrade(&view);
            view.list.set_filter_func(move |row| {
                let Some(view) = weak.upgrade() else { return true };
                let q = view.search.text().to_lowercase();
                q.is_empty() || row.widget_name().to_lowercase().contains(&q)
            });
            let list = view.list.clone();
            view.search.connect_search_changed(move |_| list.invalidate_filter());
        }
        view
    }

    /// Quantos apps a lista mostra (para o roteiro de teste).
    pub fn app_count(&self) -> usize {
        self.rows.borrow().len()
    }

    pub fn focus_search(&self) {
        self.search.grab_focus();
    }

    fn message(&self, text: &str, error: bool) {
        self.note.set_text(text);
        self.note.set_visible(!text.is_empty());
        if error {
            self.note.add_css_class("error");
        } else {
            self.note.remove_css_class("error");
        }
    }

    /// Lê os apps e o estado de cada um (fora da thread da tela) e refaz a lista.
    pub fn reload(self: &Rc<Self>) {
        if !flatpak::available() {
            self.message("O comando 'flatpak' não está instalado nesta máquina.", true);
            return;
        }
        self.message("Lendo os apps Flatpak…", false);
        let me = self.clone();
        glib::spawn_future_local(async move {
            let loaded = gtk::gio::spawn_blocking(|| flatpak::installed_apps().map(|apps| apps.into_iter().map(|a| (flatpak::state(&a.id), a)).collect::<Vec<_>>())).await;
            match loaded {
                Ok(Ok(apps)) => {
                    me.message("", false);
                    me.show(apps);
                }
                Ok(Err(why)) => me.message(&why, true),
                Err(_) => me.message("A leitura dos apps falhou.", true),
            }
        });
    }

    fn show(self: &Rc<Self>, apps: Vec<(State, App)>) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let mut rows = Vec::new();
        for (state, app) in apps {
            let row = gtk::ListBoxRow::new();
            row.set_activatable(false);
            row.set_widget_name(&format!("{} {}", app.name, app.id));
            let name = gtk::Label::new(Some(&app.name));
            name.set_xalign(0.0);
            name.add_css_class("heading");
            let id = gtk::Label::new(Some(&app.id));
            id.set_xalign(0.0);
            id.add_css_class("dim-label");
            id.add_css_class("caption");
            let status = gtk::Label::new(Some(state_text(state)));
            status.set_xalign(0.0);
            status.add_css_class("caption");
            let names = gtk::Box::new(gtk::Orientation::Vertical, 0);
            names.set_hexpand(true);
            names.append(&name);
            names.append(&id);
            names.append(&status);
            let button = gtk::Button::with_label(button_text(state));
            button.set_valign(gtk::Align::Center);
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            margins(&line, 8);
            line.append(&names);
            line.append(&button);
            row.set_child(Some(&line));
            self.list.append(&row);
            let index = rows.len();
            rows.push(Row { app, status, button: button.clone(), state });
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |b| {
                if let Some(view) = weak.upgrade() {
                    view.clicked(index, b);
                }
            });
        }
        *self.rows.borrow_mut() = rows;
        self.list.invalidate_filter();
    }

    fn clicked(self: &Rc<Self>, index: usize, button: &gtk::Button) {
        let (app, state) = {
            let rows = self.rows.borrow();
            let Some(row) = rows.get(index) else { return };
            (row.app.clone(), row.state)
        };
        match state {
            // Remover mexe nas permissões do app: avisa antes.
            State::On => {
                let dialog = gtk::AlertDialog::builder()
                    .message(format!("Remover as alterações em {}?", app.name))
                    .detail("Isto desfaz o que o linux-reshade colocou neste app Flatpak: o acesso à memória compartilhada e à pasta dos efeitos compilados, e as variáveis LINUX_RESHADE. O jogo volta a abrir sem efeitos. A extensão da layer continua instalada para outros apps.")
                    .buttons(["Cancelar", "Remover alterações"])
                    .cancel_button(0)
                    .default_button(0)
                    .modal(true)
                    .build();
                let window = button.root().and_downcast::<gtk::Window>();
                let (weak, app) = (Rc::downgrade(self), app.clone());
                glib::spawn_future_local(async move {
                    if dialog.choose_future(window.as_ref()).await == Ok(1)
                        && let Some(view) = weak.upgrade()
                    {
                        view.run(index, app, State::Off);
                    }
                });
            }
            State::Off => self.run(index, app, State::On),
            State::Global => self.run(index, app, State::DisabledHere),
            State::DisabledHere => self.run(index, app, State::Global),
        }
    }

    /// Leva o app ao estado `target` (fora da thread da tela).
    fn run(self: &Rc<Self>, index: usize, app: App, target: State) {
        {
            let rows = self.rows.borrow();
            let Some(row) = rows.get(index) else { return };
            row.button.set_sensitive(false);
            row.status.set_text(match target {
                State::On => "Instalando a extensão e ajustando as permissões…",
                State::Off => "Removendo as alterações…",
                _ => "Ajustando…",
            });
        }
        let me = self.clone();
        glib::spawn_future_local(async move {
            let id = app.id.clone();
            let work = gtk::gio::spawn_blocking(move || match target {
                State::On => flatpak::enable(&app),
                State::Off => flatpak::disable(&app.id),
                State::DisabledHere => flatpak::disable_here(&app.id),
                State::Global => flatpak::follow_global(&app.id),
            })
            .await;
            let result = work.unwrap_or_else(|_| Err("a operação falhou".to_string()));
            let now = gtk::gio::spawn_blocking(move || flatpak::state(&id)).await.unwrap_or(State::Off);
            let mut rows = me.rows.borrow_mut();
            if let Some(row) = rows.get_mut(index) {
                row.state = now;
                row.status.set_text(state_text(now));
                row.button.set_label(button_text(now));
                row.button.set_sensitive(true);
            }
            drop(rows);
            match result {
                Ok(()) => me.message("", false),
                Err(why) => me.message(&why, true),
            }
        });
    }
}
