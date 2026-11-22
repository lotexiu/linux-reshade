//! A construção da janela principal (`Ui::with_home`): monta os widgets e liga cada componente.

use super::*;

impl Ui {
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
        let macros = MacroEditor::new(tr("Preprocessor Definitions…"), tr("Preprocessor definitions of the preset, for all effects (as in ReShade). Changing them recompiles the affected effects."));
        let delete_preset = gtk::Button::from_icon_name("user-trash-symbolic");
        delete_preset.set_tooltip_text(Some(tr("Delete this preset")));
        let depth_button = gtk::Button::with_label(tr("Depth Buffer…"));
        depth_button.set_tooltip_text(Some(tr("Chooses which depth image the effects use in this game, and when in the frame it is copied (until the game is closed)")));
        depth_button.set_visible(false);
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
        // À direita de "Open games": o que está sendo compilado em segundo plano.
        let activity = crate::shared::activity::Activity::new();
        let games_head = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        games_head.set_hexpand(true);
        games_title.set_hexpand(true);
        games_title.set_xalign(0.0);
        games_head.append(&games_title);
        activity.root.set_halign(gtk::Align::End);
        games_head.append(&activity.root);
        games_expander.set_label_widget(Some(&games_head));
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
        // A linha de opções de inicialização que liga a layer num jogo da Steam (a layer só carrega com LINUX_RESHADE=1).
        let launch_button = gtk::Button::with_label(tr("Copy launch options"));
        launch_button.set_tooltip_text(Some(tr("Copies the line to paste in Steam: right-click the game, Properties, General, Launch options. It turns the layer on for that game. Games that are not on Steam use it before their command.")));
        launch_button.connect_clicked(|b| {
            b.clipboard().set_text("LINUX_RESHADE=1 %command%");
            b.set_label(tr("Copied: paste in Steam"));
            let b = b.clone();
            glib::timeout_add_local_once(std::time::Duration::from_secs(3), move || b.set_label(tr("Copy launch options")));
        });
        page.append(&line(tr("Game"), &[game.upcast_ref(), flatpak_button.upcast_ref(), launch_button.upcast_ref()]));
        page.append(&line(tr("Preset"), &[preset.upcast_ref(), new_preset.upcast_ref(), delete_preset.upcast_ref()]));
        page.append(&preset_note);
        // Definições do pré-processador e profundidade na mesma linha, com a profundidade em uso à direita dos botões.
        macros.button.set_size_request(230, -1);
        let depth_line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        depth_line.append(&macros.button);
        depth_line.append(&depth_button);
        depth_line.append(&depth_note);
        page.append(&depth_line);
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
        let depth_view = DepthView::new({
            let ui_ref = ui_ref.clone();
            move || {
                let ui = ui_ref.borrow().upgrade();
                if let Some(ui) = ui {
                    ui.show_page("main");
                }
            }
        });
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
        stack.add_named(&depth_view.root, Some("depth"));

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
                    && matches!(ui.stack.visible_child_name().as_deref(), Some("add" | "flatpak" | "depth"))
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
                crate::features::settings::open(&window);
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
            depth_view: depth_view.clone(),
            depth_button: depth_button.clone(),
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
            activity,
            cache_dir: config::Config::load().cache_dir,
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
}
