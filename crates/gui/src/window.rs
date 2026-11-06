//! A janela, no estilo da aba Home do ReShade: jogo e preset no alto, busca e "adicionar efeito", e a lista de
//! técnicas em ordem de execução, cada uma com caixa de seleção, setas, remover e os parâmetros do efeito.
//! Nenhuma regra de negócio mora aqui; tudo o que se decide está em `panel::Home`.

use crate::add_effect::AddEffectView;
use crate::effect_row::{EffectRow, RowCtx};
use crate::flatpak_view::FlatpakView;
use crate::games::GamesView;
use gtk4 as gtk;
use crate::depth_picker::DepthPicker;
use control_core::i18n::{fmt, tr};
use crate::macros::MacroEditor;
use gtk::glib;
use gtk::prelude::*;
use panel::{EffectGroup, Home};
use preset::TechId;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::{Rc, Weak};
use std::time::Duration;

/// Intervalo entre duas leituras do estado (jogos, flag, mudanças feitas pelo `lrctl` ou editando o arquivo).
const REFRESH: Duration = Duration::from_millis(500);
fn global_game() -> &'static str {
    tr("Global preset")
}
/// O último item da lista de jogos: abre a página que liga o linux-reshade em apps Flatpak.
fn add_flatpak() -> &'static str {
    tr("＋ Add Flatpak game…")
}
fn inherit_preset() -> &'static str {
    tr("(use the global preset)")
}

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
    depth_note: gtk::Label,
    depth_picker: Rc<DepthPicker>,
    new_preset: gtk::MenuButton,
    macros: Rc<MacroEditor>,
    new_name: gtk::Entry,
    new_create: gtk::Button,
    search: gtk::SearchEntry,
    list: gtk::ListBox,
    scroller: gtk::ScrolledWindow,
    delete_preset: gtk::Button,
    empty: gtk::Label,
    rows: RefCell<Vec<EffectRow>>,
    /// Descreve o que a lista mostra (ordem, rótulos, estados): se mudar, a lista é refeita.
    signature: RefCell<String>,
    /// Efeitos (arquivos) com os parâmetros abertos: sobrevive às refeituras da lista.
    expanded: Rc<RefCell<HashSet<String>>>,
    /// A página "Adicionar efeito" e a pilha que a alterna com a lista do preset.
    add_view: Rc<AddEffectView>,
    stack: gtk::Stack,
    flatpak_view: Rc<FlatpakView>,
    /// Aparece quando o jogo escolhido é um app Flatpak configurado: abre a página onde se desfaz a configuração.
    flatpak_button: gtk::Button,
    /// A versão do modelo que a tela já mostra: se a leitura periódica não a mudou, não há o que redesenhar.
    applied_version: Cell<u64>,
    games_view: RefCell<GamesView>,
    notice: gtk::Label,
    /// Verdadeiro enquanto a janela atualiza os widgets a partir do modelo.
    updating: Cell<bool>,
}

/// Marca "a janela está mudando widgets" enquanto vive; ao sair (por qualquer caminho) desmarca. Aninhado, o de fora é que desmarca.
struct Programmatic<'a> {
    flag: &'a Cell<bool>,
    was: bool,
}

impl<'a> Programmatic<'a> {
    fn enter(flag: &'a Cell<bool>) -> Self {
        let was = flag.replace(true);
        Self { flag, was }
    }
}

impl Drop for Programmatic<'_> {
    fn drop(&mut self) {
        self.flag.set(self.was);
    }
}

/// Quem refaz a tela depois de uma ação; preenchido quando a `Ui` existe (as ações nascem antes dela).
type Hook = Rc<RefCell<Option<Box<dyn Fn()>>>>;

pub(crate) fn margins(w: &impl IsA<gtk::Widget>, all: i32) {
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
        Self::with_home(app, Rc::new(RefCell::new(home)))
    }

    /// O modelo da janela: quem refaz a janela (a troca de idioma) o passa à nova, que continua de onde a outra estava.
    pub fn home(&self) -> Rc<RefCell<Home>> {
        self.home.clone()
    }

    pub fn with_home(app: &gtk::Application, home: Rc<RefCell<Home>>) -> Rc<Ui> {
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
        let game = gtk::DropDown::from_strings(&[global_game(), add_flatpak()]);
        game.set_hexpand(true);
        let preset = gtk::DropDown::from_strings(&[inherit_preset()]);
        preset.set_hexpand(true);
        let new_preset = gtk::MenuButton::new();
        new_preset.set_label(tr("New"));
        new_preset.set_tooltip_text(Some(tr("New preset for this game (copy of the current one)")));
        let macros = MacroEditor::new(tr("Definitions…"), tr("Preprocessor definitions of the preset, for all effects (as in ReShade). Changing them recompiles the affected effects."));
        let delete_preset = gtk::Button::from_icon_name("user-trash-symbolic");
        delete_preset.set_tooltip_text(Some(tr("Delete this preset")));
        let depth_picker = DepthPicker::new();
        let depth_note = gtk::Label::new(None);
        depth_note.set_halign(gtk::Align::Start);
        depth_note.set_xalign(0.0);
        depth_note.set_wrap(true);
        depth_note.add_css_class("caption");
        depth_note.set_visible(false);
        depth_note.set_hexpand(true);
        let preset_note = gtk::Label::new(None);
        preset_note.set_halign(gtk::Align::Start);
        preset_note.set_wrap(true);
        preset_note.add_css_class("dim-label");
        preset_note.add_css_class("caption");

        let search = gtk::SearchEntry::new();
        search.set_hexpand(true);
        search.set_placeholder_text(Some(tr("Search the preset's effects")));
        let add_button = gtk::Button::with_label(tr("Add effect"));

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

        // "Novo preset": um nome e criar.
        let name_entry = gtk::Entry::new();
        name_entry.set_placeholder_text(Some(tr("Preset name")));
        let create = gtk::Button::with_label(tr("Create"));
        create.add_css_class("suggested-action");
        let new_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        margins(&new_box, 8);
        new_box.append(&name_entry);
        new_box.append(&create);
        let new_popover = gtk::Popover::new();
        new_popover.set_child(Some(&new_box));
        new_preset.set_popover(Some(&new_popover));

        let games_view = GamesView::new();
        let games_title = gtk::Label::new(Some(tr("Open games")));
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
        let flatpak_button = gtk::Button::with_label("Flatpak…");
        flatpak_button.set_tooltip_text(Some(tr("This game runs as a Flatpak app: opens the page where its setup is shown and undone")));
        flatpak_button.set_visible(false);
        game.set_hexpand(true);
        page.append(&line(tr("Game"), &[game.upcast_ref(), flatpak_button.upcast_ref()]));
        page.append(&line(tr("Preset"), &[preset.upcast_ref(), new_preset.upcast_ref(), delete_preset.upcast_ref()]));
        page.append(&preset_note);
        let depth_line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        depth_line.append(&depth_note);
        depth_line.append(&depth_picker.button);
        page.append(&depth_line);
        macros.button.set_halign(gtk::Align::Start);
        page.append(&line(tr("Macros"), &[macros.button.upcast_ref()]));
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
        toggle.append(&gtk::Label::new(Some(tr("Effects on"))));
        toggle.append(&enabled);
        header.pack_end(&toggle);
        let shortcuts_button = gtk::Button::from_icon_name("emblem-system-symbolic");
        shortcuts_button.set_tooltip_text(Some(tr("Settings: shortcuts, screenshots, folders, performance and cache")));
        header.pack_start(&shortcuts_button);

        // A página "Adicionar efeito" e a lista do preset se alternam numa pilha (sem popover: ver `add_effect.rs`). Os callbacks da
        // página só guardam uma referência fraca da interface, que ainda não existe aqui.
        let ui_ref: Rc<RefCell<Weak<Ui>>> = Rc::new(RefCell::new(Weak::new()));
        let add_view = AddEffectView::new(
            {
                let ui_ref = ui_ref.clone();
                move |name| {
                    let ui = ui_ref.borrow().upgrade();
                    if let Some(ui) = ui {
                        ui.show_page("main");
                        ui.act(|h| h.add_effect(name));
                    }
                }
            },
            {
                let ui_ref = ui_ref.clone();
                move || {
                    let ui = ui_ref.borrow().upgrade();
                    if let Some(ui) = ui {
                        ui.show_page("main");
                    }
                }
            },
        );
        let flatpak_view = FlatpakView::new({
            let ui_ref = ui_ref.clone();
            move || {
                let ui = ui_ref.borrow().upgrade();
                if let Some(ui) = ui {
                    ui.show_page("main");
                }
            }
        });
        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
        stack.add_named(&page, Some("main"));
        stack.add_named(&add_view.root, Some("add"));
        stack.add_named(&flatpak_view.root, Some("flatpak"));

        let window = gtk::ApplicationWindow::builder().application(app).title("linux-reshade").default_width(600).default_height(780).child(&stack).build();
        window.set_titlebar(Some(&header));
        // Esc na página "Adicionar efeito" volta à lista do preset (um campo de busca com texto usa o primeiro Esc para limpá-lo).
        let escape = gtk::EventControllerKey::new();
        escape.connect_key_pressed({
            let ui_ref = ui_ref.clone();
            move |_, keyval, _, _| {
                let ui = ui_ref.borrow().upgrade();
                if keyval == gtk::gdk::Key::Escape
                    && let Some(ui) = ui
                    && matches!(ui.stack.visible_child_name().as_deref(), Some("add" | "flatpak"))
                {
                    ui.show_page("main");
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            }
        });
        window.add_controller(escape);
        {
            let window = window.clone();
            shortcuts_button.connect_clicked(move |_| {
                crate::shortcuts_dialog::open(&window);
            });
        }

        let ui = Rc::new(Ui {
            window,
            home,
            enabled,
            game,
            game_ids: RefCell::new(vec![None]),
            preset,
            preset_names: RefCell::new(Vec::new()),
            preset_note,
            depth_note: depth_note.clone(),
            depth_picker: depth_picker.clone(),
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
            expanded: Rc::new(RefCell::new(HashSet::new())),
            add_view: add_view.clone(),
            stack: stack.clone(),
            flatpak_view: flatpak_view.clone(),
            flatpak_button: flatpak_button.clone(),
            applied_version: Cell::new(u64::MAX),
            games_view: RefCell::new(games_view),
            notice,
            updating: Cell::new(false),
        });

        let weak: Weak<Ui> = Rc::downgrade(&ui);
        *ui_ref.borrow_mut() = weak.clone();
        flatpak_button.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.show_page("flatpak");
                }
            }
        });
        add_button.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.show_page("add");
                }
            }
        });
        *hook.borrow_mut() = Some(Box::new(move || {
            if let Some(ui) = weak.upgrade() {
                ui.apply();
            }
        }));

        ui.connect(redraw, name_entry, create, new_popover);
        ui.apply();
        ui
    }

    /// Põe no seletor o jogo que o modelo tem escolhido (depois de o último item, "adicionar Flatpak", ter sido clicado).
    fn apply_game_selection(self: &Rc<Self>) {
        let _programmatic = Programmatic::enter(&self.updating);
        let selected = {
            let home = self.home.borrow();
            let game = home.snapshot().game.clone();
            self.game_ids.borrow().iter().position(|g| *g == game).unwrap_or(0) as u32
        };
        self.game.set_selected(selected);
    }

    /// Mostra uma das páginas da janela (`"main"`: a lista do preset; `"add"`: adicionar efeito).
    pub fn show_page(self: &Rc<Self>, name: &str) {
        self.stack.set_visible_child_name(name);
        if name == "flatpak" {
            self.flatpak_view.reload();
            self.flatpak_view.focus_search();
        }
        if name == "add" {
            self.add_view.focus_search();
            let ui = self.clone();
            glib::idle_add_local_once(move || ui.apply()); // começa a montar a lista já, sem esperar a próxima atualização
        }
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

        // A profundidade escolhida pela pessoa: grava o pedido que a layer lê.
        self.depth_picker.connect_changed({
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
    fn ask_delete_preset(self: &Rc<Self>) {
        let Some(name) = self.home.borrow().snapshot().active_preset.clone() else { return };
        if !crate::settings::confirm_delete_preset() {
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

            self.depth_picker.set(&s.depth_choices, s.depth_pick);

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
    fn signature_of(groups: &[EffectGroup]) -> String {
        groups.iter().map(|g| format!("{}\n", EffectRow::signature_of(g))).collect()
    }

    /// A lista de efeitos: refeita quando a estrutura muda; senão só atualiza os valores dos controles.
    fn show_groups(self: &Rc<Self>, groups: &[EffectGroup]) {
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
    fn row_ctx(self: &Rc<Self>) -> Rc<RowCtx> {
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
    fn rebuild(self: &Rc<Self>, groups: &[EffectGroup]) {
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


    /// O que a tela sabe dos jogos, em texto (para os testes automáticos conferirem de fora).
    /// Mexe na página "Adicionar efeito" como uma pessoa (abrir, abrir os filtros, escolher uma pasta, digitar, voltar, reabrir) e
    /// devolve o que deu errado: uma página que some, um campo que não aparece. Os avisos e erros do GTK durante o roteiro também
    /// contam (é assim que um widget com dois pais aparece).
    pub async fn smoke_popovers(self: &Rc<Self>) -> Vec<String> {
        use std::time::Duration;
        let mut problems = Vec::new();
        let step = |ms: u64| glib::timeout_future(Duration::from_millis(ms));
        let warnings_before = crate::gtk_warnings();
        let on_add = |ui: &Ui| ui.stack.visible_child_name().as_deref() == Some("add");

        self.show_page("add");
        step(800).await;
        if !on_add(self) {
            problems.push("a página de adicionar efeito não abriu".to_string());
            return problems;
        }
        if !self.add_view.is_search_shown() {
            problems.push("o campo de busca dos efeitos não aparece".to_string());
        }
        if !self.add_view.is_filters_button_shown() {
            problems.push("o botão Filtros não aparece".to_string());
        }
        if self.add_view.rows_built() == 0 && !self.home.borrow().snapshot().effects.is_empty() {
            problems.push("a lista de efeitos não foi montada".to_string());
        }

        self.add_view.open_filters(true);
        step(500).await;
        if !on_add(self) {
            problems.push("abrir os filtros tirou a página".to_string());
            return problems;
        }
        if !self.add_view.is_folder_list_shown() {
            problems.push("a lista de pastas não apareceu ao abrir os filtros".to_string());
        }
        self.add_view.select_folder(1);
        step(400).await;
        self.add_view.type_query("a");
        step(400).await;
        if !on_add(self) {
            problems.push("escolher uma pasta ou digitar fechou a página".to_string());
            return problems;
        }
        self.add_view.open_filters(false);
        step(300).await;
        self.add_view.type_query("");
        self.add_view.select_folder(0);
        self.show_page("main");
        step(500).await;
        if on_add(self) {
            problems.push("voltar não mostrou a lista do preset".to_string());
        }
        self.show_page("add");
        step(500).await;
        if !on_add(self) {
            problems.push("a página não reabriu depois de fechada".to_string());
        }
        self.show_page("main");
        step(300).await;

        // A página dos apps Flatpak, pelo último item da lista de jogos (a escolha volta ao jogo que estava).
        let before = self.game.selected();
        self.game.set_selected(self.game_ids.borrow().len() as u32);
        step(2500).await;
        if self.stack.visible_child_name().as_deref() != Some("flatpak") {
            problems.push("o último item da lista de jogos não abriu a página dos apps Flatpak".to_string());
        }
        if self.game.selected() != before {
            problems.push("a lista de jogos não voltou ao jogo anterior".to_string());
        }
        if control_core::flatpak::available() && self.flatpak_view.app_count() == 0 {
            problems.push("a página dos apps Flatpak não listou nenhum app".to_string());
        }
        self.show_page("main");
        step(300).await;

        let new = crate::gtk_warnings().saturating_sub(warnings_before);
        if new > 0 {
            problems.push(format!("o GTK reclamou {new} vez(es) durante o roteiro (veja os avisos acima)"));
        }
        problems
    }

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

    /// Escolhe um jogo, como pela lista.
    pub fn select_game(self: &Rc<Self>, id: &str) {
        self.act(|h| h.select_game(Some(id)));
    }

    /// Adiciona um efeito ao preset do jogo escolhido, como pelo botão. `false` se o efeito não existe.
    pub fn choose_effect(self: &Rc<Self>, name: &str) -> bool {
        // A lista da página "Adicionar efeito" só é montada quando ela está na tela: vai direto ao modelo (o mesmo que o clique faz).
        if self.home.borrow().snapshot().effects.iter().any(|e| e == name) {
            self.act(|h| h.add_effect(name));
            return true;
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
        let row_before = self.rows.borrow().iter().find(|r| r.file == file).map(|r| r.row.clone());
        let check_button = self.rows.borrow().iter().find(|r| r.file == file).map(|r| r.check.clone());
        if let Some(c) = check_button {
            // Ligar e desligar várias vezes seguidas: o estado acompanha cada clique e a linha NÃO é refeita (refazer troca a caixa
            // sob o ponteiro e o clique seguinte se perde).
            for n in 0..6 {
                c.set_active(n % 2 == 1);
                check(find(self).is_some_and(|t| t.enabled == (n % 2 == 1)), &format!("a caixa não acompanhou o clique {n}"));
            }
            check(find(self).is_some_and(|t| t.enabled), "a caixa não religou a técnica");
            let row_after = self.rows.borrow().iter().find(|r| r.file == file).map(|r| r.row.clone());
            check(row_before.is_some() && row_before == row_after, "ligar e desligar refez a linha do efeito");
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

        // 4b. acrescentar e tirar OUTRO efeito não refaz a linha do primeiro (a lista reaproveita as linhas, como o `key=` do React)
        let first = self.rows.borrow().iter().find(|r| r.file == file).map(|r| r.row.clone());
        if effect_name != "Tonemap" && self.choose_effect("Tonemap") {
            check(self.rows.borrow().iter().any(|r| r.file == "Tonemap.fx"), "o segundo efeito não apareceu na lista");
            let same = self.rows.borrow().iter().find(|r| r.file == file).map(|r| r.row.clone());
            check(first.is_some() && first == same, "acrescentar outro efeito refez a linha do primeiro");
            let order: Vec<String> = self.rows.borrow().iter().map(|r| r.file.clone()).collect();
            let shown: Vec<String> = std::iter::successors(self.list.first_child(), |w| w.next_sibling()).map(|w| w.widget_name().to_string()).collect();
            check(shown.len() == order.len(), &format!("a lista mostra {} linhas, o modelo tem {}", shown.len(), order.len()));
            // Mover um efeito para cima: a lista segue a ordem do modelo e as linhas continuam as mesmas.
            let tonemap_row = self.rows.borrow().iter().find(|r| r.file == "Tonemap.fx").map(|r| r.row.clone());
            self.act(|h| h.move_effect("Tonemap.fx", -1));
            let model: Vec<String> = self.home.borrow().snapshot().groups.iter().map(|g| g.file.clone()).collect();
            let rows: Vec<String> = self.rows.borrow().iter().map(|r| r.file.clone()).collect();
            let shown: Vec<gtk::Widget> = std::iter::successors(self.list.first_child(), |w| w.next_sibling()).collect();
            let shown_rows: Vec<gtk::Widget> = self.rows.borrow().iter().map(|r| r.row.clone().upcast()).collect();
            check(model == rows, &format!("a ordem das linhas {rows:?} não segue o modelo {model:?}"));
            check(shown == shown_rows, "a ordem na tela não segue a das linhas");
            check(tonemap_row.is_some() && tonemap_row == self.rows.borrow().iter().find(|r| r.file == "Tonemap.fx").map(|r| r.row.clone()), "mover refez a linha");
            self.remove_effect("Tonemap");
            let same = self.rows.borrow().iter().find(|r| r.file == file).map(|r| r.row.clone());
            check(first.is_some() && first == same, "tirar outro efeito refez a linha do primeiro");
            check(!self.rows.borrow().iter().any(|r| r.file == "Tonemap.fx"), "o segundo efeito não saiu da lista");
        }

        // 5. deixar o preset como estava
        self.remove_effect(effect_name);
        check(find(self).is_none(), "o efeito não saiu do preset");
        problems
    }
}
