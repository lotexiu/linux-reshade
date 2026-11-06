//! A página "Adicionar efeito": busca, filtros e a lista de todos os efeitos das pastas de shaders.
//!
//! É uma página da janela (um `gtk::Stack`), não um menu: a lista tem mais de mil linhas, filtros que abrem e fecham e um campo de
//! busca, e um popover que muda de tamanho enquanto está aberto é fechado pelo compositor (no Wayland o menu é um "popup" com
//! grab, e o GTK4 ainda perde o grab em popovers que se redimensionam ou se aninham). Uma página não tem nada disso.
//!
//! O componente não conhece a janela: recebe os dados por [`AddEffectView::set`] e avisa o que a pessoa fez pelos dois callbacks
//! que ganha em [`AddEffectView::new`] (escolheu um efeito; pediu para voltar).

use crate::disclosure::Disclosure;
use control_core::analysis::{EffectInfo, Feature};
use gtk::glib;
use gtk::prelude::*;
use gtk4 as gtk;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use control_core::i18n::{fmt, tr};

/// O que a lista mostra: a busca, as características marcadas e o que se sabe de cada efeito.
#[derive(Default)]
struct Filter {
    query: String,
    /// O efeito precisa ter TODAS as características marcadas.
    features: HashSet<Feature>,
    only_compatible: bool,
    /// Só os efeitos desta subpasta (e das de dentro dela); vazio: todas.
    folder: String,
    folders: HashMap<String, String>,
    infos: HashMap<String, EffectInfo>,
}

impl Filter {
    fn accepts(&self, name: &str) -> bool {
        let folder = self.folders.get(name).map(String::as_str).unwrap_or("");
        if !self.query.is_empty() && !name.to_lowercase().contains(&self.query) && !folder.to_lowercase().contains(&self.query) {
            return false;
        }
        if !self.folder.is_empty() && folder != self.folder && !folder.strip_prefix(&self.folder).is_some_and(|rest| rest.starts_with('/')) {
            return false;
        }
        let info = self.infos.get(name);
        if self.only_compatible && !info.is_some_and(EffectInfo::compatible) {
            return false;
        }
        // Sem análise ainda, um efeito só aparece enquanto nenhuma característica está marcada.
        self.features.iter().all(|f| info.is_some_and(|i| i.has(*f)))
    }

    /// O texto do botão de filtros: quantos estão ligados.
    fn label(&self) -> String {
        let n = self.features.len() + usize::from(self.only_compatible) + usize::from(!self.folder.is_empty());
        if n == 0 { tr("Filters").to_string() } else { format!("{} ({n})", tr("Filters")) }
    }
}

/// Uma linha da lista.
struct Row {
    name: String,
    title: gtk::Label,
    detail: gtk::Label,
}

pub struct AddEffectView {
    pub root: gtk::Box,
    search: gtk::SearchEntry,
    filters_toggle: gtk::ToggleButton,
    folder_list: gtk::ListBox,
    list: gtk::ListBox,
    progress: gtk::Label,
    filter: Rc<RefCell<Filter>>,
    rows: RefCell<Vec<Row>>,
    effects: RefCell<Vec<String>>,
    /// A lista tem mais de mil linhas: monta aos poucos, e há uma montagem em andamento.
    building: Cell<bool>,
    infos_shown: Cell<usize>,
}

fn margins(w: &impl IsA<gtk::Widget>, all: i32) {
    w.set_margin_top(all);
    w.set_margin_bottom(all);
    w.set_margin_start(all);
    w.set_margin_end(all);
}

impl AddEffectView {
    pub fn new(on_pick: impl Fn(&str) + 'static, on_back: impl Fn() + 'static) -> Rc<Self> {
        let filter = Rc::new(RefCell::new(Filter::default()));

        // Linha de cima: voltar, busca e o botão dos filtros.
        let back = gtk::Button::from_icon_name("go-previous-symbolic");
        back.set_tooltip_text(Some(tr("Back to the preset's effect list")));
        back.connect_clicked(move |_| on_back());
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some(tr("Search effect or folder")));
        search.set_hexpand(true);

        // Os filtros: seção que abre no lugar (nada de menu dentro de menu).
        let only_ok = gtk::CheckButton::with_label(tr("Compatible only"));
        let chips: Vec<(Feature, gtk::CheckButton)> = [Feature::Depth, Feature::Color, Feature::Textures, Feature::Mipmaps, Feature::Srgb, Feature::Compute]
            .into_iter()
            .map(|f| (f, gtk::CheckButton::with_label(f.label())))
            .collect();
        let features_box = gtk::FlowBox::new();
        features_box.set_selection_mode(gtk::SelectionMode::None);
        features_box.set_max_children_per_line(3);
        features_box.insert(&only_ok, -1);
        for (_, b) in &chips {
            features_box.insert(b, -1);
        }
        let folder_search = gtk::SearchEntry::new();
        folder_search.set_placeholder_text(Some(tr("Search folder")));
        let folder_list = gtk::ListBox::new();
        folder_list.set_selection_mode(gtk::SelectionMode::Single);
        let folder_query = Rc::new(RefCell::new(String::new()));
        {
            let query = folder_query.clone();
            // A linha tr("All folders") (nome vazio) sempre aparece.
            folder_list.set_filter_func(move |row| {
                let q = query.borrow();
                q.is_empty() || row.widget_name().is_empty() || row.widget_name().to_lowercase().contains(q.as_str())
            });
            let list = folder_list.clone();
            folder_search.connect_search_changed(move |e| {
                *folder_query.borrow_mut() = e.text().to_lowercase();
                list.invalidate_filter();
            });
        }
        let folder_scroller = gtk::ScrolledWindow::new();
        folder_scroller.set_child(Some(&folder_list));
        folder_scroller.set_min_content_height(160);
        folder_scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
        let clear = gtk::Button::with_label(tr("Clear filters"));
        clear.set_halign(gtk::Align::Start);
        let filters_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        filters_box.append(&features_box);
        filters_box.append(&folder_search);
        filters_box.append(&folder_scroller);
        filters_box.append(&clear);
        let filters = Disclosure::new(tr("Filters"), &filters_box);

        let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        top.append(&back);
        top.append(&search);
        top.append(&filters.button);

        let progress = gtk::Label::new(None);
        progress.set_halign(gtk::Align::Start);
        progress.add_css_class("dim-label");
        progress.add_css_class("caption");
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
        root.append(&filters.body);
        root.append(&progress);
        root.append(&scroller);

        // Ligações: cada controle muda o filtro, refiltra a lista e atualiza o texto do botão.
        let refresh = {
            let (list, filter, button) = (list.clone(), filter.clone(), filters.button.clone());
            Rc::new(move || {
                list.invalidate_filter();
                button.set_label(&filter.borrow().label());
            })
        };
        {
            let filter = filter.clone();
            list.set_filter_func(move |row| filter.borrow().accepts(&row.widget_name()));
        }
        {
            let (f, r) = (filter.clone(), refresh.clone());
            search.connect_search_changed(move |e| {
                f.borrow_mut().query = e.text().to_lowercase();
                r();
            });
            let (f, r) = (filter.clone(), refresh.clone());
            only_ok.connect_toggled(move |b| {
                f.borrow_mut().only_compatible = b.is_active();
                r();
            });
            for (feature, button) in chips.iter().cloned() {
                let (f, r) = (filter.clone(), refresh.clone());
                button.connect_toggled(move |b| {
                    let mut f = f.borrow_mut();
                    if b.is_active() {
                        f.features.insert(feature);
                    } else {
                        f.features.remove(&feature);
                    }
                    drop(f);
                    r();
                });
            }
            let (f, r) = (filter.clone(), refresh.clone());
            folder_list.connect_row_selected(move |_, row| {
                let Some(row) = row else { return };
                f.borrow_mut().folder = row.widget_name().to_string();
                r();
            });
            let (only_ok, folders, chips) = (only_ok.clone(), folder_list.clone(), chips.iter().map(|(_, b)| b.clone()).collect::<Vec<_>>());
            clear.connect_clicked(move |_| {
                only_ok.set_active(false);
                folders.select_row(folders.row_at_index(0).as_ref());
                for b in &chips {
                    b.set_active(false);
                }
            });
        }
        list.connect_row_activated(move |_, row| on_pick(&row.widget_name()));

        Rc::new(Self {
            root,
            search,
            filters_toggle: filters.button.clone(),
            folder_list,
            list,
            progress,
            filter,
            rows: RefCell::new(Vec::new()),
            effects: RefCell::new(Vec::new()),
            building: Cell::new(false),
            infos_shown: Cell::new(usize::MAX),
        })
    }

    /// Leva o foco ao campo de busca (ao abrir a página).
    pub fn focus_search(&self) {
        self.search.grab_focus();
    }

    /// Atualiza os dados mostrados. `visible`: a página está na tela; a lista só é montada (aos poucos) enquanto estiver.
    pub fn set(self: &Rc<Self>, effects: &[String], folders: &HashMap<String, String>, infos: &HashMap<String, EffectInfo>, progress: (usize, usize), visible: bool) {
        if self.filter.borrow().folders != *folders {
            self.show_folders(folders);
        }
        let (done, total) = progress;
        self.progress.set_visible(done < total);
        self.progress.set_text(&fmt("Analyzing the effects to see which ones work… {}/{}", &[&done, &total]));
        if !visible {
            return;
        }
        if *self.effects.borrow() != effects {
            while let Some(child) = self.list.first_child() {
                self.list.remove(&child);
            }
            self.rows.borrow_mut().clear();
            *self.effects.borrow_mut() = effects.to_vec();
            self.infos_shown.set(usize::MAX);
        }
        if self.rows.borrow().len() < self.effects.borrow().len() && !self.building.replace(true) {
            self.build_rows();
        }
        if self.infos_shown.get() != infos.len() && self.rows.borrow().len() == self.effects.borrow().len() {
            self.show_infos(infos);
        }
    }

    fn show_folders(&self, folders: &HashMap<String, String>) {
        let mut names: Vec<&String> = folders.values().filter(|f| !f.is_empty()).collect();
        names.sort();
        names.dedup();
        let keep = self.filter.borrow().folder.clone();
        while let Some(child) = self.folder_list.first_child() {
            self.folder_list.remove(&child);
        }
        let mut selected = None;
        for (i, name) in std::iter::once("").chain(names.iter().map(|n| n.as_str())).enumerate() {
            let row = gtk::ListBoxRow::new();
            row.set_widget_name(name);
            let label = gtk::Label::new(Some(if name.is_empty() { tr("All folders") } else { name }));
            label.set_halign(gtk::Align::Start);
            margins(&label, 4);
            row.set_child(Some(&label));
            self.folder_list.append(&row);
            if name == keep {
                selected = Some(i);
            }
        }
        self.folder_list.select_row(self.folder_list.row_at_index(selected.unwrap_or(0) as i32).as_ref());
        self.filter.borrow_mut().folders = folders.clone();
        self.list.invalidate_filter();
    }

    /// Monta as linhas em pedaços, nos intervalos do laço do GTK (mais de mil de uma vez travam a janela).
    fn build_rows(self: &Rc<Self>) {
        let me = self.clone();
        glib::idle_add_local(move || {
            const CHUNK: usize = 60;
            let effects = me.effects.borrow();
            let mut rows = me.rows.borrow_mut();
            for name in effects.iter().skip(rows.len()).take(CHUNK) {
                let row = gtk::ListBoxRow::new();
                row.set_widget_name(name);
                let title = gtk::Label::new(Some(name));
                title.set_halign(gtk::Align::Start);
                title.set_xalign(0.0);
                let detail = gtk::Label::new(None);
                detail.set_halign(gtk::Align::Start);
                detail.set_xalign(0.0);
                detail.set_wrap(true);
                detail.add_css_class("caption");
                let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
                margins(&b, 4);
                b.append(&title);
                b.append(&detail);
                row.set_child(Some(&b));
                me.list.append(&row);
                rows.push(Row { name: name.clone(), title, detail });
            }
            if rows.len() < effects.len() {
                return glib::ControlFlow::Continue;
            }
            me.infos_shown.set(usize::MAX); // as linhas novas ainda não têm o que a análise sabe
            me.building.set(false);
            glib::ControlFlow::Break
        });
    }

    /// O que a análise já sabe: vermelho para os incompatíveis, e as características embaixo.
    fn show_infos(&self, infos: &HashMap<String, EffectInfo>) {
        for row in self.rows.borrow().iter() {
            match infos.get(&row.name) {
                Some(info) => {
                    let tags: Vec<&str> = info.features.iter().map(|f| f.label()).collect();
                    row.detail.set_text(&match info.reason() {
                        Some(why) => fmt("Incompatible: {}", &[&why]),
                        None => tags.join(" · "),
                    });
                    if info.compatible() {
                        row.title.remove_css_class("error");
                        row.detail.remove_css_class("error");
                        row.detail.add_css_class("dim-label");
                    } else {
                        row.title.add_css_class("error");
                        row.detail.add_css_class("error");
                        row.detail.remove_css_class("dim-label");
                    }
                }
                None => {
                    row.detail.set_text("");
                    row.title.remove_css_class("error");
                }
            }
        }
        self.filter.borrow_mut().infos = infos.clone();
        self.list.invalidate_filter();
        self.infos_shown.set(infos.len());
    }

    // ---- para o roteiro de teste (`--smoke-ui`) ----

    pub fn is_search_shown(&self) -> bool {
        self.search.is_mapped()
    }

    pub fn is_filters_button_shown(&self) -> bool {
        self.filters_toggle.is_mapped()
    }

    pub fn open_filters(&self, on: bool) {
        self.filters_toggle.set_active(on);
    }

    pub fn is_folder_list_shown(&self) -> bool {
        self.folder_list.is_mapped()
    }

    pub fn select_folder(&self, index: i32) {
        if let Some(row) = self.folder_list.row_at_index(index) {
            self.folder_list.select_row(Some(&row));
        }
    }

    pub fn type_query(&self, text: &str) {
        self.search.set_text(text);
    }

    pub fn rows_built(&self) -> usize {
        self.rows.borrow().len()
    }
}
