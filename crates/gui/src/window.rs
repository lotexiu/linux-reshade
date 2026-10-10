//! A janela, no estilo da aba Home do ReShade: jogo e preset no alto, busca e "adicionar efeito", e a lista de
//! técnicas em ordem de execução, cada uma com caixa de seleção, setas, remover e os parâmetros do efeito.
//! Nenhuma regra de negócio mora aqui; tudo o que se decide está em `panel::Home`.

use crate::controls::ControlsView;
use crate::games::GamesView;
use gtk4 as gtk;
use crate::macros::MacroEditor;
use gtk::glib;
use gtk::prelude::*;
use control_core::analysis::{EffectInfo, Feature};
use panel::{EffectGroup, Home};
use preset::TechId;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::time::Duration;

/// Intervalo entre duas leituras do estado (jogos, flag, mudanças feitas pelo `lrctl` ou editando o arquivo).
const REFRESH: Duration = Duration::from_millis(500);
const GLOBAL_GAME: &str = "Preset global";
const INHERIT_PRESET: &str = "(usar o preset global)";

pub struct Ui {
    pub window: gtk::ApplicationWindow,
    home: Rc<RefCell<Home>>,
    enabled: gtk::Switch,
    game: gtk::DropDown,
    /// O jogo de cada item da lista (`None`: o preset global).
    game_ids: RefCell<Vec<Option<String>>>,
    preset: gtk::DropDown,
    preset_names: RefCell<Vec<String>>,
    preset_note: gtk::Label,
    new_preset: gtk::MenuButton,
    macros: Rc<MacroEditor>,
    new_name: gtk::Entry,
    new_create: gtk::Button,
    search: gtk::SearchEntry,
    list: gtk::ListBox,
    scroller: gtk::ScrolledWindow,
    delete_preset: gtk::Button,
    empty: gtk::Label,
    rows: RefCell<Vec<TechRow>>,
    /// Descreve o que a lista mostra (ordem, rótulos, estados): se mudar, a lista é refeita.
    signature: RefCell<String>,
    /// Efeitos (arquivos) com os parâmetros abertos: sobrevive às refeituras da lista.
    expanded: RefCell<HashSet<String>>,
    add_list: gtk::ListBox,
    add_rows: RefCell<Vec<AddRow>>,
    add_filter: Rc<RefCell<AddFilter>>,
    add_progress: gtk::Label,
    /// Quantas análises de efeitos a lista já mostra (para só reescrevê-la quando mudam).
    add_infos_shown: Cell<usize>,
    add_effects: RefCell<Vec<String>>,
    add_popover: gtk::Popover,
    games_view: RefCell<GamesView>,
    notice: gtk::Label,
    /// Verdadeiro enquanto a janela atualiza os widgets a partir do modelo.
    updating: Cell<bool>,
}

/// Os widgets de um efeito da lista (com as técnicas dele).
struct TechRow {
    /// Arquivo do efeito (`cGaussianBlur.fx`).
    file: String,
    /// O que a busca compara: nome do efeito, arquivo e nomes das técnicas.
    haystack: String,
    check: gtk::CheckButton,
    controls: Option<RefCell<ControlsView>>,
    row: gtk::ListBoxRow,
}

/// O que a lista "Adicionar efeito" mostra: a busca, as características marcadas e o que se sabe de cada efeito.
#[derive(Default)]
struct AddFilter {
    query: String,
    /// O efeito precisa ter TODAS as características marcadas.
    features: HashSet<Feature>,
    only_compatible: bool,
    infos: HashMap<String, EffectInfo>,
}

impl AddFilter {
    fn accepts(&self, name: &str) -> bool {
        if !self.query.is_empty() && !name.to_lowercase().contains(&self.query) {
            return false;
        }
        let info = self.infos.get(name);
        if self.only_compatible && !info.is_some_and(EffectInfo::compatible) {
            return false;
        }
        // Sem análise ainda, um efeito só aparece enquanto nenhuma característica está marcada.
        self.features.iter().all(|f| info.is_some_and(|i| i.has(*f)))
    }
}

/// Uma linha da lista "Adicionar efeito".
struct AddRow {
    name: String,
    title: gtk::Label,
    detail: gtk::Label,
}

/// Quem refaz a tela depois de uma ação; preenchido quando a `Ui` existe (as ações nascem antes dela).
type Hook = Rc<RefCell<Option<Box<dyn Fn()>>>>;

fn margins(w: &impl IsA<gtk::Widget>, all: i32) {
    w.set_margin_top(all);
    w.set_margin_bottom(all);
    w.set_margin_start(all);
    w.set_margin_end(all);
}

fn caption(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_halign(gtk::Align::Start);
    l.add_css_class("dim-label");
    l.set_width_chars(7);
    l.set_xalign(0.0);
    l
}

impl Ui {
    pub fn new(app: &gtk::Application, home: Home) -> Rc<Ui> {
        let home = Rc::new(RefCell::new(home));
        let hook: Hook = Rc::new(RefCell::new(None));
        let redraw = {
            let hook = hook.clone();
            move || {
                if let Some(f) = &*hook.borrow() {
                    f();
                }
            }
        };

        let enabled = gtk::Switch::new();
        enabled.set_valign(gtk::Align::Center);
        let game = gtk::DropDown::from_strings(&[GLOBAL_GAME]);
        game.set_hexpand(true);
        let preset = gtk::DropDown::from_strings(&[INHERIT_PRESET]);
        preset.set_hexpand(true);
        let new_preset = gtk::MenuButton::new();
        new_preset.set_label("Novo");
        new_preset.set_tooltip_text(Some("Novo preset para este jogo (cópia do atual)"));
        let macros = MacroEditor::new("Definições…", "Definições de pré-processador do preset, para todos os efeitos (como no ReShade). Mudar recompila os efeitos afetados.");
        let delete_preset = gtk::Button::from_icon_name("user-trash-symbolic");
        delete_preset.set_tooltip_text(Some("Apagar este preset"));
        let preset_note = gtk::Label::new(None);
        preset_note.set_halign(gtk::Align::Start);
        preset_note.set_wrap(true);
        preset_note.add_css_class("dim-label");
        preset_note.add_css_class("caption");

        let search = gtk::SearchEntry::new();
        search.set_hexpand(true);
        search.set_placeholder_text(Some("Procurar nos efeitos do preset"));
        let add_button = gtk::MenuButton::new();
        add_button.set_label("Adicionar efeito");

        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        list.add_css_class("boxed-list");
        let empty = gtk::Label::new(None);
        empty.set_wrap(true);
        empty.set_margin_top(24);
        empty.set_margin_bottom(24);
        empty.add_css_class("dim-label");
        let notice = gtk::Label::new(None);
        notice.set_halign(gtk::Align::Start);
        notice.set_wrap(true);
        notice.add_css_class("warning");
        notice.add_css_class("heading");

        // "Adicionar efeito": busca e lista de todos os efeitos que as pastas de shaders têm.
        let add_search = gtk::SearchEntry::new();
        add_search.set_placeholder_text(Some("Procurar efeito"));
        let add_filter = Rc::new(RefCell::new(AddFilter::default()));
        let add_progress = gtk::Label::new(None);
        add_progress.set_halign(gtk::Align::Start);
        add_progress.add_css_class("dim-label");
        add_progress.add_css_class("caption");
        let add_list = gtk::ListBox::new();
        add_list.set_selection_mode(gtk::SelectionMode::None);
        let add_scroller = gtk::ScrolledWindow::new();
        add_scroller.set_child(Some(&add_list));
        add_scroller.set_min_content_height(380);
        add_scroller.set_min_content_width(420);
        let add_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        margins(&add_box, 8);
        add_box.append(&add_search);
        // Filtro por característica (várias ao mesmo tempo) e só os compatíveis.
        let chips = gtk::FlowBox::new();
        chips.set_selection_mode(gtk::SelectionMode::None);
        chips.set_max_children_per_line(3);
        chips.set_row_spacing(4);
        chips.set_column_spacing(4);
        let only_ok = gtk::ToggleButton::with_label("Só compatíveis");
        chips.append(&only_ok);
        let chip_buttons: Vec<(Feature, gtk::ToggleButton)> = [Feature::Depth, Feature::Color, Feature::Textures, Feature::Mipmaps, Feature::Srgb, Feature::Compute]
            .into_iter()
            .map(|f| (f, gtk::ToggleButton::with_label(f.label())))
            .collect();
        for (_, b) in &chip_buttons {
            chips.append(b);
        }
        add_box.append(&chips);
        add_box.append(&add_progress);
        add_box.append(&add_scroller);
        let add_popover = gtk::Popover::new();
        add_popover.set_child(Some(&add_box));
        add_button.set_popover(Some(&add_popover));
        {
            let filter = add_filter.clone();
            add_list.set_filter_func(move |row| filter.borrow().accepts(&row.widget_name()));
            let (list, filter) = (add_list.clone(), add_filter.clone());
            add_search.connect_search_changed(move |e| {
                filter.borrow_mut().query = e.text().to_lowercase();
                list.invalidate_filter();
            });
            let (list, filter) = (add_list.clone(), add_filter.clone());
            only_ok.connect_toggled(move |b| {
                filter.borrow_mut().only_compatible = b.is_active();
                list.invalidate_filter();
            });
            for (feature, button) in chip_buttons {
                let (list, filter) = (add_list.clone(), add_filter.clone());
                button.connect_toggled(move |b| {
                    let mut f = filter.borrow_mut();
                    if b.is_active() {
                        f.features.insert(feature);
                    } else {
                        f.features.remove(&feature);
                    }
                    drop(f);
                    list.invalidate_filter();
                });
            }
        }

        // "Novo preset": um nome e criar.
        let name_entry = gtk::Entry::new();
        name_entry.set_placeholder_text(Some("Nome do preset"));
        let create = gtk::Button::with_label("Criar");
        create.add_css_class("suggested-action");
        let new_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        margins(&new_box, 8);
        new_box.append(&name_entry);
        new_box.append(&create);
        let new_popover = gtk::Popover::new();
        new_popover.set_child(Some(&new_box));
        new_preset.set_popover(Some(&new_popover));

        let games_view = GamesView::new();
        let games_title = gtk::Label::new(Some("Jogos abertos"));
        games_title.set_halign(gtk::Align::Start);
        games_title.add_css_class("heading");
        let games_expander = gtk::Expander::new(None);
        games_expander.set_label_widget(Some(&games_title));
        games_expander.set_child(Some(&games_view.root));

        let line = |caption_text: &str, widgets: &[&gtk::Widget]| {
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            b.append(&caption(caption_text));
            for w in widgets {
                b.append(*w);
            }
            b
        };
        let page = gtk::Box::new(gtk::Orientation::Vertical, 10);
        margins(&page, 14);
        page.append(&line("Jogo", &[game.upcast_ref()]));
        page.append(&line("Preset", &[preset.upcast_ref(), new_preset.upcast_ref(), delete_preset.upcast_ref()]));
        page.append(&preset_note);
        macros.button.set_halign(gtk::Align::Start);
        page.append(&line("Macros", &[macros.button.upcast_ref()]));
        let tools = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        tools.append(&search);
        tools.append(&add_button);
        page.append(&tools);

        let list_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list_box.append(&list);
        list_box.append(&empty);
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_child(Some(&list_box));
        scroller.set_vexpand(true);
        scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
        page.append(&notice);
        page.append(&scroller);
        page.append(&games_expander);

        let header = gtk::HeaderBar::new();
        let toggle = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        toggle.append(&gtk::Label::new(Some("Efeitos ligados")));
        toggle.append(&enabled);
        header.pack_end(&toggle);

        let window = gtk::ApplicationWindow::builder().application(app).title("linux-reshade").default_width(600).default_height(780).child(&page).build();
        window.set_titlebar(Some(&header));

        let ui = Rc::new(Ui {
            window,
            home,
            enabled,
            game,
            game_ids: RefCell::new(vec![None]),
            preset,
            preset_names: RefCell::new(Vec::new()),
            preset_note,
            new_preset: new_preset.clone(),
            macros: macros.clone(),
            new_name: name_entry.clone(),
            new_create: create.clone(),
            search,
            list,
            scroller: scroller.clone(),
            delete_preset: delete_preset.clone(),
            empty,
            rows: RefCell::new(Vec::new()),
            signature: RefCell::new(String::new()),
            expanded: RefCell::new(HashSet::new()),
            add_list,
            add_rows: RefCell::new(Vec::new()),
            add_filter,
            add_progress,
            add_infos_shown: Cell::new(usize::MAX),
            add_effects: RefCell::new(Vec::new()),
            add_popover,
            games_view: RefCell::new(games_view),
            notice,
            updating: Cell::new(false),
        });

        let weak: Weak<Ui> = Rc::downgrade(&ui);
        *hook.borrow_mut() = Some(Box::new(move || {
            if let Some(ui) = weak.upgrade() {
                ui.apply();
            }
        }));

        ui.connect(redraw, name_entry, create, new_popover);
        ui.apply();
        ui
    }

    /// Liga os widgets fixos da janela às ações do modelo.
    fn connect(self: &Rc<Self>, redraw: impl Fn() + Clone + 'static, name_entry: gtk::Entry, create: gtk::Button, new_popover: gtk::Popover) {
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

        // Escolher um efeito na lista "Adicionar efeito".
        self.add_list.connect_row_activated({
            let weak = weak.clone();
            move |_, row| {
                if let Some(ui) = weak.upgrade() {
                    let name = row.widget_name().to_string();
                    ui.add_popover.popdown();
                    ui.act(|h| h.add_effect(&name));
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
                ui.home.borrow_mut().refresh();
                ui.apply();
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });
    }

    /// Apaga o preset em uso, depois de perguntar (a menos que a pessoa tenha pedido para não perguntar mais).
    fn ask_delete_preset(self: &Rc<Self>) {
        let Some(name) = self.home.borrow().snapshot().active_preset.clone() else { return };
        if !crate::settings::confirm_delete_preset() {
            self.act(|h| h.delete_preset(&name));
            return;
        }
        let dialog = gtk::Window::builder().transient_for(&self.window).modal(true).title("Apagar preset").default_width(380).resizable(false).build();
        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        margins(&body, 16);
        let text = gtk::Label::new(Some(&format!("Apagar o preset \"{name}\"?\nO arquivo é removido e as configurações dele se perdem.")));
        text.set_wrap(true);
        text.set_halign(gtk::Align::Start);
        text.set_xalign(0.0);
        let never = gtk::CheckButton::with_label("Não perguntar novamente (dá para voltar a perguntar nas configurações)");
        let cancel = gtk::Button::with_label("Cancelar");
        let delete = gtk::Button::with_label("Apagar");
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
                    crate::settings::set_confirm_delete_preset(false);
                }
                dialog.close();
                if let Some(ui) = weak.upgrade() {
                    ui.act(|h| h.delete_preset(&name));
                }
            }
        });
        dialog.present();
    }

    /// Uma ação do modelo seguida de refazer a tela (as linhas da lista usam isto).
    fn act(self: &Rc<Self>, action: impl FnOnce(&mut Home)) {
        action(&mut self.home.borrow_mut());
        self.apply();
    }

    /// Mostra/oculta as linhas conforme a busca.
    fn apply_search(&self) {
        let q = self.search.text().to_lowercase();
        for row in self.rows.borrow().iter() {
            let hit = q.is_empty() || row.haystack.contains(&q);
            row.row.set_visible(hit);
        }
    }

    /// Empurra o estado do modelo para os widgets.
    pub fn apply(self: &Rc<Self>) {
        self.updating.set(true);
        {
            let home = self.home.borrow();
            let s = home.snapshot();

            if s.enabled != self.enabled.is_active() {
                self.enabled.set_active(s.enabled);
            }

            // Jogos.
            let ids: Vec<Option<String>> = std::iter::once(None).chain(s.games.iter().map(|g| Some(g.id.clone()))).collect();
            if *self.game_ids.borrow() != ids {
                let mut items = vec![GLOBAL_GAME.to_string()];
                items.extend(s.games.iter().map(|g| if g.open { format!("● {}", g.id) } else { g.id.clone() }));
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
            self.new_preset.set_tooltip_text(Some(if has_game { "Novo preset para este jogo (cópia do atual)" } else { "Escolha um jogo para criar um preset dele" }));
            if *self.preset_names.borrow() != s.presets {
                let mut items = vec![INHERIT_PRESET.to_string()];
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
                (Some(p), true) if p.inherited => format!("Este jogo usa o preset global ({}). A primeira alteração cria o preset dele.", p.name),
                (Some(p), _) => format!("{}", p.path.display()),
                (None, true) => "Este jogo não tem preset. Adicione um efeito para criar o dele.".to_string(),
                (None, false) => "Ainda não há preset global. Adicione um efeito para criá-lo (vale para os jogos sem preset próprio).".to_string(),
            });

            // Efeitos que podem ser adicionados.
            if *self.add_effects.borrow() != s.effects {
                while let Some(child) = self.add_list.first_child() {
                    self.add_list.remove(&child);
                }
                let mut rows = Vec::new();
                for name in &s.effects {
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
                    self.add_list.append(&row);
                    rows.push(AddRow { name: name.clone(), title, detail });
                }
                *self.add_rows.borrow_mut() = rows;
                *self.add_effects.borrow_mut() = s.effects.clone();
                self.add_infos_shown.set(usize::MAX);
            }
            // O que a análise já sabe: vermelho para os incompatíveis, e as características embaixo.
            if self.add_infos_shown.get() != s.infos.len() {
                for row in self.add_rows.borrow().iter() {
                    match s.infos.get(&row.name) {
                        Some(info) => {
                            let tags: Vec<&str> = info.features.iter().map(|f| f.label().trim_start_matches("Usa ")).collect();
                            row.detail.set_text(&match info.reason() {
                                Some(why) => format!("Incompatível: {why}"),
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
                self.add_filter.borrow_mut().infos = s.infos.clone();
                self.add_list.invalidate_filter();
                self.add_infos_shown.set(s.infos.len());
            }
            let (done, total) = s.analysis;
            self.add_progress.set_visible(done < total);
            self.add_progress.set_text(&format!("Analisando os efeitos para saber quais servem… {done}/{total}"));

            self.show_groups(&s.groups);

            self.empty.set_visible(s.groups.is_empty());
            self.empty.set_text(if s.groups.is_empty() {
                "Nenhum efeito no preset.\nUse \"Adicionar efeito\" para começar."
            } else {
                ""
            });
            self.games_view.borrow_mut().show(&s.layers);
            self.notice.set_visible(s.notice.is_some());
            self.notice.set_text(s.notice.as_deref().unwrap_or(""));
        }
        self.updating.set(false);
    }

    fn signature_of(groups: &[EffectGroup]) -> String {
        groups
            .iter()
            .map(|g| {
                let techniques: Vec<String> = g.techniques.iter().map(|t| format!("{}:{}:{}:{:?}:{:?}", t.id, t.label, t.enabled, t.problem, t.runtime)).collect();
                format!("{}|{}|{}\n", g.file, g.macros, techniques.join(";"))
            })
            .collect()
    }

    /// A lista de efeitos: refeita quando a estrutura muda; senão só atualiza os valores dos controles.
    fn show_groups(self: &Rc<Self>, groups: &[EffectGroup]) {
        let signature = Self::signature_of(groups);
        if signature != *self.signature.borrow() {
            self.rebuild(groups);
            *self.signature.borrow_mut() = signature;
        }
        for (row, g) in self.rows.borrow().iter().zip(groups) {
            if let Some(view) = &row.controls {
                view.borrow_mut().show(&g.controls);
            }
        }
        self.apply_search();
    }

    fn rebuild(self: &Rc<Self>, groups: &[EffectGroup]) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let mut rows = Vec::new();
        let last = groups.len().saturating_sub(1);
        for (index, g) in groups.iter().enumerate() {
            rows.push(self.build_row(index, last, g));
        }
        for r in &rows {
            self.list.append(&r.row);
        }
        *self.rows.borrow_mut() = rows;
    }

    /// O texto e a cor do que a layer fez com uma técnica no jogo aberto.
    fn runtime_label(state: &ipc::TechniqueState) -> gtk::Label {
        let (text, class) = match state {
            ipc::TechniqueState::Applied => ("● aplicada no jogo".to_string(), "success"),
            ipc::TechniqueState::Skipped(why) => (format!("pulada no jogo: {why}"), "error"),
        };
        let l = gtk::Label::new(Some(&text));
        l.set_halign(gtk::Align::Start);
        l.set_xalign(0.0);
        l.set_wrap(true);
        l.add_css_class(class);
        l.add_css_class("caption");
        l
    }

    fn build_row(self: &Rc<Self>, index: usize, last: usize, g: &EffectGroup) -> TechRow {
        let weak = Rc::downgrade(self);
        let file = g.file.clone();
        let multi = g.techniques.len() > 1;

        // A caixa do efeito liga e desliga todas as técnicas dele (marcada pela metade se só algumas estão ligadas).
        let check = gtk::CheckButton::new();
        check.set_active(g.any_enabled());
        check.set_inconsistent(g.any_enabled() && !g.all_enabled());
        check.set_valign(gtk::Align::Center);
        check.set_tooltip_text(Some("Ligar ou desligar o efeito"));
        check.connect_toggled({
            let (weak, file) = (weak.clone(), file.clone());
            move |c| {
                if let Some(ui) = weak.upgrade()
                    && !ui.updating.get()
                {
                    let on = c.is_active();
                    ui.act(|h| h.set_effect_enabled(&file, on));
                }
            }
        });

        let title = gtk::Label::new(Some(&g.name));
        title.set_halign(gtk::Align::Start);
        title.set_xalign(0.0);
        title.add_css_class("heading");
        let subtitle = gtk::Label::new(Some(&g.file));
        subtitle.set_halign(gtk::Align::Start);
        subtitle.add_css_class("dim-label");
        subtitle.add_css_class("caption");
        let names = gtk::Box::new(gtk::Orientation::Vertical, 0);
        names.set_hexpand(true);
        names.append(&title);
        names.append(&subtitle);

        // Embaixo do nome fica só um resumo do que a layer fez no jogo aberto e de problemas; as técnicas de um efeito
        // com várias ficam dentro do painel que abre (⋮), como os parâmetros.
        let skipped = g.techniques.iter().find_map(|t| match &t.runtime {
            Some(ipc::TechniqueState::Skipped(why)) => Some(why.clone()),
            _ => None,
        });
        let applied = g.techniques.iter().filter(|t| matches!(t.runtime, Some(ipc::TechniqueState::Applied))).count();
        if let Some(why) = &skipped {
            names.append(&Self::runtime_label(&ipc::TechniqueState::Skipped(why.clone())));
        } else if applied > 0 {
            names.append(&Self::runtime_label(&ipc::TechniqueState::Applied));
        }
        if let Some(problem) = g.techniques.iter().find_map(|t| t.problem.as_ref()) {
            let p = gtk::Label::new(Some(problem));
            p.set_halign(gtk::Align::Start);
            p.set_xalign(0.0);
            p.set_wrap(true);
            p.add_css_class("warning");
            p.add_css_class("caption");
            names.append(&p);
        }

        let icon_button = |icon: &str, tip: &str| {
            let b = gtk::Button::from_icon_name(icon);
            b.set_tooltip_text(Some(tip));
            b.add_css_class("flat");
            b.set_valign(gtk::Align::Center);
            b
        };
        let up = icon_button("go-up-symbolic", "Rodar antes");
        up.set_sensitive(index > 0);
        up.connect_clicked({
            let (weak, file) = (weak.clone(), file.clone());
            move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.act(|h| h.move_effect(&file, -1));
                }
            }
        });
        let down = icon_button("go-down-symbolic", "Rodar depois");
        down.set_sensitive(index < last);
        down.connect_clicked({
            let (weak, file) = (weak.clone(), file.clone());
            move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.act(|h| h.move_effect(&file, 1));
                }
            }
        });
        let remove = icon_button("user-trash-symbolic", "Tirar o efeito do preset (apaga as configurações dele)");
        remove.connect_clicked({
            let (weak, file) = (weak.clone(), file.clone());
            move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.act(|h| h.remove_effect(&file));
                }
            }
        });

        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        head.append(&check);
        head.append(&names);

        // O painel que abre e fecha (⋮): as técnicas do efeito (se são várias) e os parâmetros dele.
        let controls = (!g.controls.is_empty()).then(|| {
            let view = ControlsView::new(
                {
                    let (weak, file) = (weak.clone(), file.clone());
                    Rc::new(move |name: &str, values: &[f64]| {
                        if let Some(ui) = weak.upgrade() {
                            ui.act(|h| h.set_control(&file, name, values));
                        }
                    })
                },
                self.scroller.clone(),
            );
            RefCell::new(view)
        });
        let outer = gtk::Box::new(gtk::Orientation::Vertical, 4);
        margins(&outer, 8);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        top.append(&head);
        let has_panel = true; // as definições de pré-processador do efeito estão sempre no painel
        let revealer = gtk::Revealer::new();
        if has_panel {
            let panel = gtk::Box::new(gtk::Orientation::Vertical, 8);
            panel.set_margin_start(32);
            panel.set_margin_top(6);
            if multi {
                let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
                let heading = gtk::Label::new(Some("Técnicas"));
                heading.set_halign(gtk::Align::Start);
                heading.add_css_class("dim-label");
                heading.add_css_class("caption");
                list.append(&heading);
                for t in &g.techniques {
                    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                    let sub = gtk::CheckButton::with_label(&t.label);
                    sub.set_active(t.enabled);
                    sub.set_tooltip_text(Some("Ligar ou desligar só esta técnica"));
                    sub.connect_toggled({
                        let (weak, id) = (weak.clone(), t.id.clone());
                        move |c| {
                            if let Some(ui) = weak.upgrade()
                                && !ui.updating.get()
                            {
                                let on = c.is_active();
                                ui.act(|h| h.set_technique_enabled(&id, on));
                            }
                        }
                    });
                    line.append(&sub);
                    if let Some(state) = &t.runtime {
                        line.append(&Self::runtime_label(state));
                    }
                    list.append(&line);
                }
                panel.append(&list);
            }
            // Definições de pré-processador só deste efeito (mudar recompila o efeito).
            let macros = MacroEditor::new("Definições do efeito…", "Definições de pré-processador só deste efeito; valem mais que as do preset. Mudar recompila o efeito.");
            macros.set_text(&g.macros);
            macros.button.set_halign(gtk::Align::Start);
            macros.connect_changed({
                let (weak, file, original) = (weak.clone(), file.clone(), g.macros.clone());
                move |text| {
                    if text != original && let Some(ui) = weak.upgrade() && !ui.updating.get() {
                        ui.act(|h| h.set_effect_macros(&file, &text));
                    }
                }
            });
            panel.append(&macros.button);
            if let Some(view) = &controls {
                panel.append(&view.borrow().root);
                let reset = gtk::Button::with_label("Restaurar os padrões do efeito");
                reset.set_halign(gtk::Align::Start);
                reset.connect_clicked({
                    let (weak, file) = (weak.clone(), file.clone());
                    move |_| {
                        if let Some(ui) = weak.upgrade() {
                            ui.act(|h| h.reset_effect(&file));
                        }
                    }
                });
                panel.append(&reset);
            }
            revealer.set_child(Some(&panel));
            let open = self.expanded.borrow().contains(&file);
            revealer.set_reveal_child(open);
            let toggle = gtk::ToggleButton::new();
            toggle.set_icon_name("view-more-symbolic");
            toggle.set_tooltip_text(Some(if multi { "Técnicas e parâmetros" } else { "Parâmetros" }));
            toggle.add_css_class("flat");
            toggle.set_valign(gtk::Align::Center);
            toggle.set_active(open);
            toggle.connect_toggled({
                let (weak, file, revealer) = (weak.clone(), file.clone(), revealer.clone());
                move |b| {
                    revealer.set_reveal_child(b.is_active());
                    if let Some(ui) = weak.upgrade() {
                        if b.is_active() {
                            ui.expanded.borrow_mut().insert(file.clone());
                        } else {
                            ui.expanded.borrow_mut().remove(&file);
                        }
                    }
                }
            });
            top.append(&toggle);
        }
        top.append(&up);
        top.append(&down);
        top.append(&remove);
        outer.append(&top);
        if has_panel {
            outer.append(&revealer);
        }

        let row = gtk::ListBoxRow::new();
        row.set_activatable(false);
        row.set_child(Some(&outer));
        let haystack = std::iter::once(g.name.clone()).chain(std::iter::once(g.file.clone())).chain(g.techniques.iter().flat_map(|t| [t.label.clone(), t.id.name.clone()])).collect::<Vec<_>>().join(" ").to_lowercase();
        TechRow { file: g.file.clone(), haystack, check, controls, row }
    }

    /// O que a tela sabe dos jogos, em texto (para os testes automáticos conferirem de fora).
    pub fn macros_popup(&self) {
        self.macros.button.popup();
    }

    pub fn describe_games(&self) -> String {
        let home = self.home.borrow();
        let s = home.snapshot();
        let games: Vec<String> = s.games.iter().map(|g| format!("{}{}", g.id, if g.open { "*" } else { "" })).collect();
        // O que o seletor de jogos mostra de verdade (o widget, não o modelo).
        let shown: Vec<String> = self
            .game
            .model()
            .map(|m| (0..m.n_items()).filter_map(|i| m.item(i).and_downcast::<gtk::StringObject>().map(|o| o.string().to_string())).collect())
            .unwrap_or_default();
        format!("jogos: [{}] camadas: {} seletor: {:?}", games.join(", "), s.layers.len(), shown)
    }

    // ---- Apoio aos testes automáticos: aciona os widgets de verdade e confere o que o modelo viu. ----

    /// Adiciona um efeito ao preset do jogo escolhido, como pelo botão. `false` se o efeito não existe.
    pub fn choose_effect(self: &Rc<Self>, name: &str) -> bool {
        // Ativa a linha da lista, o mesmo caminho do clique do usuário.
        let mut child = self.add_list.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if widget.widget_name() == name
                && let Ok(row) = widget.downcast::<gtk::ListBoxRow>()
            {
                row.emit_by_name::<()>("activate", &[]);
                return true;
            }
        }
        false
    }

    /// Tira do preset as técnicas do efeito (desfaz `choose_effect`).
    pub fn remove_effect(self: &Rc<Self>, name: &str) {
        let file = format!("{name}.fx");
        let ids: Vec<TechId> = self.home.borrow().snapshot().techniques.iter().filter(|t| t.file == file).map(|t| t.id.clone()).collect();
        for id in ids {
            self.act(|h| h.remove_technique(&id));
        }
    }

    /// Percorre a interface como um usuário e devolve o que deu errado (vazio = tudo certo). Deixa o preset como estava.
    pub fn smoke(self: &Rc<Self>, effect_name: &str) -> Vec<String> {
        let mut problems = Vec::new();
        let mut check = |ok: bool, what: &str| {
            if !ok {
                problems.push(what.to_string());
            }
        };
        let file = format!("{effect_name}.fx");
        let find = |ui: &Ui| ui.home.borrow().snapshot().techniques.iter().find(|t| t.file == file).cloned();

        // 1. adicionar o efeito
        check(self.choose_effect(effect_name), &format!("o efeito {effect_name:?} não está na lista"));
        let Some(t) = find(self) else {
            check(false, "o efeito não apareceu na lista de técnicas");
            return problems;
        };
        check(t.enabled && self.home.borrow().snapshot().notice.is_none(), &format!("estado ao adicionar: {:?}", self.home.borrow().snapshot().notice));
        check(self.rows.borrow().iter().any(|r| r.file == file), "a linha do efeito não foi desenhada");

        // 2. desligar e ligar pela caixa de seleção
        let check_button = self.rows.borrow().iter().find(|r| r.file == file).map(|r| r.check.clone());
        if let Some(c) = check_button {
            c.set_active(false);
            check(find(self).is_some_and(|t| !t.enabled), "a caixa não desligou a técnica");
        }
        let check_button = self.rows.borrow().iter().find(|r| r.file == file).map(|r| r.check.clone());
        if let Some(c) = check_button {
            c.set_active(true);
            check(find(self).is_some_and(|t| t.enabled), "a caixa não religou a técnica");
        }

        // 3. mexer num slider e ver o modelo receber; depois restaurar
        let scales = self.rows.borrow().iter().find(|r| r.file == file).and_then(|r| r.controls.as_ref().map(|c| c.borrow().first_scales())).unwrap_or_default();
        if let Some(scale) = scales.first() {
            let adj = scale.adjustment();
            let target = adj.lower() + (adj.upper() - adj.lower()) * 0.25;
            scale.set_value(target);
            let seen = find(self).and_then(|t| t.controls.first().map(|c| c.value[0]));
            check(seen.is_some_and(|v| (v - target).abs() < 0.02), &format!("o modelo viu {seen:?}, esperava ~{target}"));
            self.act(|h| h.reset_effect(&file));
            check(find(self).is_some_and(|t| t.controls.iter().all(|c| c.is_default())), "restaurar não voltou aos padrões");
        } else {
            check(false, "o efeito não gerou nenhum controle");
        }

        // 3b. criar um preset pelo botão "Criar" do popover (precisa de um jogo escolhido)
        if self.home.borrow().snapshot().game.is_some() {
            self.new_name.set_text("smoke-novo");
            self.new_create.emit_clicked();
            let active = self.home.borrow().snapshot().active_preset.clone();
            check(active.as_deref() == Some("smoke-novo"), &format!("o botão Criar não criou o preset (ativo: {active:?}, aviso: {:?})", self.home.borrow().snapshot().notice));
            check(self.new_name.text().is_empty(), "o campo do nome não foi limpo");
            check(self.delete_preset.is_sensitive(), "a lixeira do preset não ficou ativa");
            self.act(|h| h.delete_preset("smoke-novo"));
            check(!self.home.borrow().snapshot().presets.iter().any(|p| p == "smoke-novo"), "o preset não foi apagado");
        }

        // 3c. as definições do preset, pelo editor (liga/desliga e texto livre)
        self.macros.set_text("");
        self.act(|h| h.set_preset_macros("RESHADE_DEPTH_INPUT_IS_UPSIDE_DOWN=1,SMOKE_Q=2"));
        self.apply();
        check(self.macros.text() == "RESHADE_DEPTH_INPUT_IS_UPSIDE_DOWN=1,SMOKE_Q=2", &format!("o editor não mostra as definições do modelo: {:?}", self.macros.text()));
        self.act(|h| h.set_preset_macros(""));
        self.apply();
        check(self.macros.text().is_empty(), "o editor não limpou depois de apagar as definições");

        // 4. o interruptor de efeitos ligados
        self.enabled.set_active(true);
        check(self.home.borrow().snapshot().enabled, "o interruptor não ligou os efeitos");
        self.enabled.set_active(false);
        check(!self.home.borrow().snapshot().enabled, "o interruptor não desligou os efeitos");

        // 5. deixar o preset como estava
        self.remove_effect(effect_name);
        check(find(self).is_none(), "o efeito não saiu do preset");
        problems
    }
}
