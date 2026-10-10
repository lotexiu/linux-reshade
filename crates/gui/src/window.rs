//! A janela, no estilo da aba Home do ReShade: jogo e preset no alto, busca e "adicionar efeito", e a lista de
//! técnicas em ordem de execução, cada uma com caixa de seleção, setas, remover e os parâmetros do efeito.
//! Nenhuma regra de negócio mora aqui; tudo o que se decide está em `panel::Home`.

use crate::controls::ControlsView;
use crate::games::GamesView;
use gtk4 as gtk;
use gtk::glib;
use gtk::prelude::*;
use panel::{Home, TechniqueView};
use preset::TechId;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
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
    new_name: gtk::Entry,
    new_create: gtk::Button,
    search: gtk::SearchEntry,
    list: gtk::ListBox,
    empty: gtk::Label,
    rows: RefCell<Vec<TechRow>>,
    /// Descreve o que a lista mostra (ordem, rótulos, estados): se mudar, a lista é refeita.
    signature: RefCell<String>,
    /// Efeitos (arquivos) com os parâmetros abertos: sobrevive às refeituras da lista.
    expanded: RefCell<HashSet<String>>,
    add_list: gtk::ListBox,
    add_effects: RefCell<Vec<String>>,
    add_popover: gtk::Popover,
    games_view: RefCell<GamesView>,
    notice: gtk::Label,
    /// Verdadeiro enquanto a janela atualiza os widgets a partir do modelo.
    updating: Cell<bool>,
}

/// Os widgets de uma técnica da lista.
struct TechRow {
    id: TechId,
    file: String,
    label: String,
    check: gtk::CheckButton,
    controls: Option<RefCell<ControlsView>>,
    row: gtk::ListBoxRow,
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
        let add_list = gtk::ListBox::new();
        add_list.set_selection_mode(gtk::SelectionMode::None);
        let add_scroller = gtk::ScrolledWindow::new();
        add_scroller.set_child(Some(&add_list));
        add_scroller.set_min_content_height(320);
        add_scroller.set_min_content_width(300);
        let add_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        margins(&add_box, 8);
        add_box.append(&add_search);
        add_box.append(&add_scroller);
        let add_popover = gtk::Popover::new();
        add_popover.set_child(Some(&add_box));
        add_button.set_popover(Some(&add_popover));
        {
            let list = add_list.clone();
            let search = add_search.clone();
            list.set_filter_func(move |row| {
                let q = search.text().to_lowercase();
                q.is_empty() || row.widget_name().to_lowercase().contains(&q)
            });
            let list = add_list.clone();
            add_search.connect_search_changed(move |_| list.invalidate_filter());
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
        page.append(&line("Preset", &[preset.upcast_ref(), new_preset.upcast_ref()]));
        page.append(&preset_note);
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
            new_name: name_entry.clone(),
            new_create: create.clone(),
            search,
            list,
            empty,
            rows: RefCell::new(Vec::new()),
            signature: RefCell::new(String::new()),
            expanded: RefCell::new(HashSet::new()),
            add_list,
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

    /// Uma ação do modelo seguida de refazer a tela (as linhas da lista usam isto).
    fn act(self: &Rc<Self>, action: impl FnOnce(&mut Home)) {
        action(&mut self.home.borrow_mut());
        self.apply();
    }

    /// Mostra/oculta as linhas conforme a busca.
    fn apply_search(&self) {
        let q = self.search.text().to_lowercase();
        for row in self.rows.borrow().iter() {
            let hit = q.is_empty() || row.label.to_lowercase().contains(&q) || row.file.to_lowercase().contains(&q) || row.id.name.to_lowercase().contains(&q);
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
                for name in &s.effects {
                    let row = gtk::ListBoxRow::new();
                    row.set_widget_name(name);
                    let label = gtk::Label::new(Some(name));
                    label.set_halign(gtk::Align::Start);
                    margins(&label, 4);
                    row.set_child(Some(&label));
                    self.add_list.append(&row);
                }
                *self.add_effects.borrow_mut() = s.effects.clone();
            }

            self.show_techniques(&s.techniques);

            self.empty.set_visible(s.techniques.is_empty());
            self.empty.set_text(if s.techniques.is_empty() {
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

    fn signature_of(techniques: &[TechniqueView]) -> String {
        techniques.iter().map(|t| format!("{}|{}|{}|{:?}\n", t.id, t.label, t.enabled, t.problem)).collect()
    }

    /// A lista de técnicas: refeita quando a estrutura muda; senão só atualiza os valores dos controles.
    fn show_techniques(self: &Rc<Self>, techniques: &[TechniqueView]) {
        let signature = Self::signature_of(techniques);
        if signature != *self.signature.borrow() {
            self.rebuild(techniques);
            *self.signature.borrow_mut() = signature;
        }
        for (row, t) in self.rows.borrow().iter().zip(techniques) {
            if let Some(view) = &row.controls {
                view.borrow_mut().show(&t.controls);
            }
        }
        self.apply_search();
    }

    fn rebuild(self: &Rc<Self>, techniques: &[TechniqueView]) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let mut rows = Vec::new();
        let last = techniques.len().saturating_sub(1);
        for (index, t) in techniques.iter().enumerate() {
            rows.push(self.build_row(index, last, t));
        }
        for r in &rows {
            self.list.append(&r.row);
        }
        *self.rows.borrow_mut() = rows;
    }

    fn build_row(self: &Rc<Self>, index: usize, last: usize, t: &TechniqueView) -> TechRow {
        let weak = Rc::downgrade(self);
        let id = t.id.clone();

        let check = gtk::CheckButton::new();
        check.set_active(t.enabled);
        check.set_valign(gtk::Align::Center);
        check.set_tooltip_text(Some("Ligar ou desligar a técnica"));
        check.connect_toggled({
            let (weak, id) = (weak.clone(), id.clone());
            move |c| {
                if let Some(ui) = weak.upgrade()
                    && !ui.updating.get()
                {
                    let on = c.is_active();
                    ui.act(|h| h.set_technique_enabled(&id, on));
                }
            }
        });

        let title = gtk::Label::new(Some(&t.label));
        title.set_halign(gtk::Align::Start);
        title.set_xalign(0.0);
        title.add_css_class("heading");
        let subtitle = gtk::Label::new(Some(&t.file));
        subtitle.set_halign(gtk::Align::Start);
        subtitle.add_css_class("dim-label");
        subtitle.add_css_class("caption");
        let names = gtk::Box::new(gtk::Orientation::Vertical, 0);
        names.set_hexpand(true);
        names.append(&title);
        names.append(&subtitle);
        if let Some(problem) = &t.problem {
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
            let (weak, id) = (weak.clone(), id.clone());
            move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.act(|h| h.move_technique(&id, -1));
                }
            }
        });
        let down = icon_button("go-down-symbolic", "Rodar depois");
        down.set_sensitive(index < last);
        down.connect_clicked({
            let (weak, id) = (weak.clone(), id.clone());
            move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.act(|h| h.move_technique(&id, 1));
                }
            }
        });
        let remove = icon_button("user-trash-symbolic", "Tirar do preset");
        remove.connect_clicked({
            let (weak, id) = (weak.clone(), id.clone());
            move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.act(|h| h.remove_technique(&id));
                }
            }
        });

        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        head.append(&check);
        head.append(&names);

        // Os parâmetros do efeito, num painel que abre e fecha (só na primeira técnica de cada arquivo).
        let controls = (!t.controls.is_empty()).then(|| {
            let view = ControlsView::new({
                let (weak, file) = (weak.clone(), t.file.clone());
                Rc::new(move |name: &str, values: &[f64]| {
                    if let Some(ui) = weak.upgrade() {
                        ui.act(|h| h.set_control(&file, name, values));
                    }
                })
            });
            RefCell::new(view)
        });
        let outer = gtk::Box::new(gtk::Orientation::Vertical, 4);
        margins(&outer, 8);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        top.append(&head);
        if let Some(view) = &controls {
            let revealer = gtk::Revealer::new();
            let panel = gtk::Box::new(gtk::Orientation::Vertical, 8);
            panel.set_margin_start(32);
            panel.set_margin_top(6);
            panel.append(&view.borrow().root);
            let reset = gtk::Button::with_label("Restaurar os padrões do efeito");
            reset.set_halign(gtk::Align::Start);
            reset.connect_clicked({
                let (weak, file) = (weak.clone(), t.file.clone());
                move |_| {
                    if let Some(ui) = weak.upgrade() {
                        ui.act(|h| h.reset_effect(&file));
                    }
                }
            });
            panel.append(&reset);
            revealer.set_child(Some(&panel));
            let open = self.expanded.borrow().contains(&t.file);
            revealer.set_reveal_child(open);
            let toggle = gtk::ToggleButton::new();
            toggle.set_icon_name("view-more-symbolic");
            toggle.set_tooltip_text(Some("Parâmetros"));
            toggle.add_css_class("flat");
            toggle.set_valign(gtk::Align::Center);
            toggle.set_active(open);
            toggle.connect_toggled({
                let (weak, file, revealer) = (weak.clone(), t.file.clone(), revealer.clone());
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
            top.append(&up);
            top.append(&down);
            top.append(&remove);
            outer.append(&top);
            outer.append(&revealer);
        } else {
            top.append(&up);
            top.append(&down);
            top.append(&remove);
            outer.append(&top);
        }

        let row = gtk::ListBoxRow::new();
        row.set_activatable(false);
        row.set_child(Some(&outer));
        TechRow { id: t.id.clone(), file: t.file.clone(), label: t.label.clone(), check, controls, row }
    }

    /// O que a tela sabe dos jogos, em texto (para os testes automáticos conferirem de fora).
    pub fn describe_games(&self) -> String {
        let home = self.home.borrow();
        let s = home.snapshot();
        let games: Vec<String> = s.games.iter().map(|g| format!("{}{}", g.id, if g.open { "*" } else { "" })).collect();
        format!("jogos: [{}] camadas: {}", games.join(", "), s.layers.len())
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
        }

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
