//! A janela de atalhos: uma linha por tecla do `config.ini` (seção `[INPUT]`), com a tecla atual, um botão que captura a próxima
//! combinação digitada, "Limpar" e "Padrão". O daemon reinicia sozinho ao ver a mudança e passa a usar a tecla nova.

use gtk::prelude::*;
use gtk4 as gtk;
use shortcuts::trigger;
use std::cell::RefCell;
use std::rc::Rc;
use control_core::i18n::{fmt, tr};

/// Uma linha da janela: qual tecla do config.ini, o botão que mostra o atalho e o aviso embaixo.
struct Row {
    key: &'static str,
    button: gtk::Button,
    note: gtk::Label,
}

/// O texto guardado de uma tecla, no formato XDG ("" = sem atalho).
fn current(config: &config::Config, key: &str) -> String {
    config.key(key)
}

fn show(row: &Row, stored: &str) {
    if stored.trim().is_empty() {
        row.button.set_label(tr("No shortcut"));
        row.note.set_text("");
        return;
    }
    match trigger::normalize(stored) {
        Ok(t) => {
            row.button.set_label(&trigger::display(&t));
            let warn = tr("No modifier: this key stops working in every program while the daemon is running.");
            row.note.set_text(if trigger::is_bare_key(&t) { warn } else { "" });
        }
        Err(why) => {
            row.button.set_label(stored);
            row.note.set_text(&fmt("This key was not understood ({}); it is ignored.", &[&why]));
        }
    }
}

fn save(key: &str, value: &str) -> Result<(), String> {
    let path = config::Config::load().config_path().ok_or(tr("no data folder: run `lrctl config init`"))?;
    config::set_key(&path, key, value).map_err(|e| format!("{}: {e}", path.display()))
}

/// A combinação digitada, no texto da biblioteca de atalhos (`CTRL+SHIFT+F10`). `None` para um modificador sozinho.
pub(crate) fn captured(window: &gtk::Window, keyval: gtk::gdk::Key, keycode: u32, state: gtk::gdk::ModifierType) -> Option<Result<String, String>> {
    use gtk::gdk::{Key, ModifierType};
    let modifier_key = matches!(
        keyval,
        Key::Shift_L | Key::Shift_R | Key::Control_L | Key::Control_R | Key::Alt_L | Key::Alt_R | Key::Super_L | Key::Super_R | Key::Meta_L | Key::Meta_R | Key::ISO_Level3_Shift | Key::Caps_Lock
    );
    if modifier_key {
        return None;
    }
    // A tecla sem shift (a do ' e do " é a mesma, e o Shift+1 é a tecla 1), pelo código físico da tecla.
    let base = WidgetExt::display(window)
        .map_keycode(keycode)
        .and_then(|entries| entries.into_iter().find(|(keymap, _)| keymap.level() == 0 && keymap.group() == 0).map(|(_, key)| key))
        .unwrap_or(keyval);
    let name = base.name()?;
    let mut parts: Vec<&str> = Vec::new();
    if state.contains(ModifierType::CONTROL_MASK) {
        parts.push("Ctrl");
    }
    if state.contains(ModifierType::ALT_MASK) {
        parts.push("Alt");
    }
    if state.contains(ModifierType::SHIFT_MASK) {
        parts.push("Shift");
    }
    if state.contains(ModifierType::SUPER_MASK) {
        parts.push("Super");
    }
    parts.push(name.as_str());
    Some(trigger::normalize(&parts.join("+")))
}

fn shortcuts_page(window: &gtk::Window) -> gtk::Box {
    let window = window.clone();
    let page = gtk::Box::new(gtk::Orientation::Vertical, 12);
    page.set_margin_top(16);
    page.set_margin_bottom(16);
    page.set_margin_start(16);
    page.set_margin_end(16);
    let intro = gtk::Label::new(Some(tr("Global shortcuts: they work with the game in focus. Click the key and type the new combination (Esc cancels). The daemon (`lrctl daemon`) applies the shortcut, and restarts by itself when it sees the change.")));
    intro.set_wrap(true);
    intro.set_xalign(0.0);
    intro.add_css_class("dim-label");
    page.append(&intro);
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_wrap(true);

    let config = config::Config::load();
    let rows: Rc<Vec<Row>> = Rc::new(
        config::KEY_NAMES
            .iter()
            .map(|(key, _)| Row { key, button: gtk::Button::new(), note: gtk::Label::new(None) })
            .collect(),
    );
    // A linha que espera a próxima tecla, se alguma.
    let waiting: Rc<RefCell<Option<usize>>> = Rc::new(RefCell::new(None));

    for (index, (key, description)) in config::KEY_NAMES.iter().enumerate() {
        let row = &rows[index];
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let label = gtk::Label::new(Some(tr(description)));
        label.set_xalign(0.0);
        label.set_hexpand(true);
        row.button.set_size_request(190, -1);
        let clear = gtk::Button::with_label(tr("Clear"));
        let factory = gtk::Button::with_label(tr("Default"));
        line.append(&label);
        line.append(&row.button);
        line.append(&clear);
        line.append(&factory);
        row.note.set_xalign(0.0);
        row.note.set_wrap(true);
        row.note.add_css_class("warning");
        let block = gtk::Box::new(gtk::Orientation::Vertical, 2);
        block.append(&line);
        block.append(&row.note);
        page.append(&block);
        show(row, &current(&config, key));

        let (rows_click, waiting_click) = (rows.clone(), waiting.clone());
        row.button.connect_clicked(move |button| {
            if let Some(previous) = waiting_click.borrow_mut().replace(index) {
                show(&rows_click[previous], &current(&config::Config::load(), rows_click[previous].key));
            }
            button.set_label(tr("Press the key…"));
        });
        let (rows_clear, status_clear) = (rows.clone(), status.clone());
        let key = *key;
        clear.connect_clicked(move |_| match save(key, "") {
            Ok(()) => {
                show(&rows_clear[index], "");
                status_clear.set_text("");
            }
            Err(why) => status_clear.set_text(&why),
        });
        let (rows_default, status_default) = (rows.clone(), status.clone());
        factory.connect_clicked(move |_| {
            let value = config::default_key(key);
            match save(key, value) {
                Ok(()) => {
                    show(&rows_default[index], value);
                    status_default.set_text("");
                }
                Err(why) => status_default.set_text(&why),
            }
        });
    }
    page.append(&status);

    let controller = gtk::EventControllerKey::new();
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    let (rows_key, waiting_key, status_key, window_key) = (rows.clone(), waiting.clone(), status.clone(), window.clone());
    controller.connect_key_pressed(move |_, keyval, keycode, state| {
        let Some(index) = *waiting_key.borrow() else { return gtk::glib::Propagation::Proceed };
        if keyval == gtk::gdk::Key::Escape {
            *waiting_key.borrow_mut() = None;
            show(&rows_key[index], &current(&config::Config::load(), rows_key[index].key));
            return gtk::glib::Propagation::Stop;
        }
        match captured(&window_key, keyval, keycode, state) {
            None => {} // um modificador sozinho: espera o resto da combinação
            Some(Ok(text)) => {
                *waiting_key.borrow_mut() = None;
                match save(rows_key[index].key, &text) {
                    Ok(()) => {
                        show(&rows_key[index], &text);
                        status_key.set_text(tr("Saved. The daemon applies it within 2 s."));
                    }
                    Err(why) => status_key.set_text(&why),
                }
            }
            Some(Err(why)) => {
                status_key.set_text(&fmt("That key does not work: {}", &[&why]));
            }
        }
        gtk::glib::Propagation::Stop
    });
    window.add_controller(controller);
    page
}

fn page_box() -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 12);
    page.set_margin_top(16);
    page.set_margin_bottom(16);
    page.set_margin_start(16);
    page.set_margin_end(16);
    page
}

/// A aba "Geral": o idioma da interface (vale na hora: a janela é refeita) e a pergunta antes de apagar um preset.
fn general_page(window: &gtk::Window) -> gtk::Box {
    let page = page_box();
    let languages = control_core::i18n::languages();
    let current = languages.iter().position(|l| l.code == control_core::i18n::code()).unwrap_or(0) as u32;

    let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let label = gtk::Label::new(Some(tr("Language")));
    label.set_hexpand(true);
    label.set_xalign(0.0);
    let names: Vec<&str> = languages.iter().map(|l| l.name.as_str()).collect();
    let dropdown = gtk::DropDown::from_strings(&names);
    dropdown.set_selected(current);
    line.append(&label);
    line.append(&dropdown);
    dropdown.connect_selected_notify({
        let window = window.clone();
        move |dd| {
            let Some(language) = languages.get(dd.selected() as usize) else { return };
            if language.code == control_core::i18n::code() {
                return;
            }
            control_core::i18n::set(&language.code);
            control_core::i18n::save(&language.code);
            // Refazer a janela destrói esta (e o seletor que acabou de avisar): fica para depois do aviso.
            if let Some(app) = window.application() {
                gtk::glib::idle_add_local_once(move || app.activate_action("relocalize", None));
            }
        }
    });

    let ask = gtk::CheckButton::with_label(tr("Ask before deleting a preset"));
    ask.set_active(crate::settings::confirm_delete_preset());
    ask.connect_toggled(|b| crate::settings::set_confirm_delete_preset(b.is_active()));

    page.append(&line);
    page.append(&hint(tr("The whole interface changes language right away.")));
    page.append(&ask);
    page
}

fn hint(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_wrap(true);
    label.set_xalign(0.0);
    label.add_css_class("dim-label");
    label
}

/// O `config.ini` que a janela edita (existindo ou não).
fn config_file() -> Option<std::path::PathBuf> {
    config::Config::load().config_path()
}

fn ini_text(section: &str, key: &str, default: &str) -> String {
    config_file().and_then(|f| ini::Ini::load(&f).ok()).and_then(|i| i.get(section, key).map(String::from)).unwrap_or_else(|| default.to_string())
}

fn ini_list(section: &str, key: &str, default: &str) -> String {
    config_file().and_then(|f| ini::Ini::load(&f).ok()).map(|i| i.get_list(section, key)).filter(|l| !l.is_empty()).map_or_else(|| default.to_string(), |l| l.join(", "))
}

/// Uma linha de pasta: o texto editável, "Escolher…" (ou "Adicionar pasta…" numa lista) e o salvamento.
fn folder_row(page: &gtk::Box, window: &gtk::Window, label: &str, tip: &str, current: String, list: bool, save: impl Fn(&str) -> Result<(), String> + 'static, status: &gtk::Label) {
    let title = gtk::Label::new(Some(label));
    title.set_xalign(0.0);
    title.add_css_class("heading");
    page.append(&title);
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let entry = gtk::Entry::new();
    entry.set_hexpand(true);
    entry.set_text(&current);
    entry.set_tooltip_text(Some(tip));
    let choose = gtk::Button::with_label(if list { tr("Add folder…") } else { tr("Choose…") });
    line.append(&entry);
    line.append(&choose);
    page.append(&line);
    let save = Rc::new(save);
    let commit = {
        let (entry, status, save) = (entry.clone(), status.clone(), save.clone());
        move || match save(entry.text().trim()) {
            Ok(()) => status.set_text(tr("Saved. The daemon restarts by itself; close and reopen the panel so it also uses the new value.")),
            Err(why) => status.set_text(&why),
        }
    };
    {
        let commit = commit.clone();
        entry.connect_activate(move |_| commit());
    }
    let focus = gtk::EventControllerFocus::new();
    {
        let commit = commit.clone();
        focus.connect_leave(move |_| commit());
    }
    entry.add_controller(focus);
    let window = window.clone();
    choose.connect_clicked(move |_| {
        let (entry, commit, window) = (entry.clone(), commit.clone(), window.clone());
        gtk::FileDialog::new().select_folder(Some(&window), gtk::gio::Cancellable::NONE, move |result| {
            let Ok(file) = result else { return };
            let Some(path) = file.path() else { return };
            let chosen = path.to_string_lossy().into_owned();
            if list {
                let mut text = entry.text().trim().to_string();
                if !text.is_empty() {
                    text.push_str(", ");
                }
                text.push_str(&format!("{chosen}/**"));
                entry.set_text(&text);
            } else {
                entry.set_text(&chosen);
            }
            commit();
        });
    });
}

fn folders_page(window: &gtk::Window) -> gtk::Box {
    let page = page_box();
    page.append(&hint(tr("The folders are kept in config.ini. An entry ending in /** includes the subfolders. Relative paths (./) count from the config.ini folder.")));
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_wrap(true);
    let list = |section: &'static str, key: &'static str| {
        move |text: &str| -> Result<(), String> {
            let path = config_file().ok_or(tr("no data folder: run `lrctl config init`"))?;
            let items: Vec<String> = text.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            config::set_list(&path, section, key, &items).map_err(|e| format!("{}: {e}", path.display()))
        }
    };
    let single = |section: &'static str, key: &'static str| {
        move |text: &str| -> Result<(), String> {
            let path = config_file().ok_or(tr("no data folder: run `lrctl config init`"))?;
            config::set_value(&path, section, key, text).map_err(|e| format!("{}: {e}", path.display()))
        }
    };
    folder_row(&page, window, tr("Effects (.fx)"), tr("Folders where effects are searched for (the first with the name wins)."), ini_list("GENERAL", "EffectSearchPaths", "./Shaders/**"), true, list("GENERAL", "EffectSearchPaths"), &status);
    folder_row(&page, window, tr("Effect textures"), tr("Folders of the images the effects load (LUTs, noise...)."), ini_list("GENERAL", "TextureSearchPaths", "./Textures/**"), true, list("GENERAL", "TextureSearchPaths"), &status);
    folder_row(&page, window, tr("Presets"), tr("Where the global preset and each game's folder live."), ini_text("GENERAL", "PresetsPath", "./Presets"), false, single("GENERAL", "PresetsPath"), &status);
    folder_row(&page, window, tr("Compiled effects (cache)"), tr("Where the compiled effects are kept; it can be deleted freely."), ini_text("GENERAL", "IntermediateCachePath", &config::Config::load().cache_dir.to_string_lossy()), false, single("GENERAL", "IntermediateCachePath"), &status);
    let all = gtk::CheckButton::with_label(tr("Analyze every effect in the background (otherwise only those used by a preset)"));
    all.set_tooltip_text(Some(tr("The analysis compiles each effect once to tell which ones are compatible. With hundreds of effects that takes minutes of CPU; the ones outside presets are analyzed when you pick them.")));
    all.set_active(config::Config::load().analyze_all);
    {
        let status = status.clone();
        all.connect_toggled(move |b| {
            let Some(path) = config_file() else { return };
            match config::set_value(&path, "GENERAL", "AnalyzeAllEffects", if b.is_active() { "1" } else { "0" }) {
                Ok(()) => status.set_text(tr("Saved.")),
                Err(e) => status.set_text(&format!("{}: {e}", path.display())),
            }
        });
    }
    page.append(&all);
    folder_row(&page, window, tr("Skip in the background analysis"), tr("Effects in these folders are not analyzed ahead of time (a preset that uses them still compiles them)."), ini_list("GENERAL", "AnalysisExclude", ""), true, list("GENERAL", "AnalysisExclude"), &status);
    page.append(&status);
    page
}

fn captures_page(window: &gtk::Window) -> gtk::Box {
    let page = page_box();
    page.append(&hint(tr("The capture key is in the Shortcuts tab. Each game with the layer open saves its own frame.")));
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_wrap(true);
    let default = config::Config::load().screenshot_dir.to_string_lossy().into_owned();
    folder_row(&page, window, tr("Captures folder"), tr("Each game gets a subfolder; files carry the date and time."), ini_text("SCREENSHOT", "SavePath", &default), false, |text| {
        let path = config_file().ok_or(tr("no data folder: run `lrctl config init`"))?;
        config::set_value(&path, "SCREENSHOT", "SavePath", text).map_err(|e| format!("{}: {e}", path.display()))
    }, &status);
    let both = gtk::CheckButton::with_label(tr("Also save the frame without effects (to compare), ending in \"_no-effects\""));
    both.set_active(config::Config::load().screenshot_before);
    {
        let status = status.clone();
        both.connect_toggled(move |b| {
            let Some(path) = config_file() else { return };
            match config::set_value(&path, "SCREENSHOT", "SaveBeforeShot", if b.is_active() { "1" } else { "0" }) {
                Ok(()) => status.set_text(tr("Saved.")),
                Err(e) => status.set_text(&format!("{}: {e}", path.display())),
            }
        });
    }
    page.append(&both);
    page.append(&status);
    page
}

/// Os últimos `max` bytes de um arquivo de texto, a partir de uma linha inteira.
fn tail_of(path: &std::path::Path, max: u64) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut file) = std::fs::File::open(path) else { return fmt("(could not open {})", &[&path.display()]) };
    let len = file.metadata().map_or(0, |m| m.len());
    let start = len.saturating_sub(max);
    let _ = file.seek(SeekFrom::Start(start));
    let mut bytes = Vec::new();
    let _ = file.read_to_end(&mut bytes);
    let text = String::from_utf8_lossy(&bytes).into_owned();
    if start > 0 { text.split_once('\n').map_or(text.clone(), |(_, rest)| rest.to_string()) } else { text }
}

/// Os arquivos de log que existem, com um nome para mostrar.
fn log_files() -> Vec<(String, std::path::PathBuf)> {
    let state = std::env::var_os("XDG_STATE_HOME").map(std::path::PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/state"))).map(|d| d.join(config::LOCATION_DIR));
    let mut files = Vec::new();
    if let Some(dir) = state {
        files.push((tr("Daemon").to_string(), dir.join("daemon.log")));
        files.push((tr("Layer (native games, vkcube)").to_string(), dir.join("layer.log")));
    }
    files.push((tr("Layer (Flatpak, Sober)").to_string(), std::path::PathBuf::from("/tmp/linux-reshade-flatpak.log")));
    files.into_iter().filter(|(_, p)| p.exists()).collect()
}

fn log_page() -> gtk::Box {
    let page = page_box();
    let files = log_files();
    if files.is_empty() {
        page.append(&hint(tr("No log found: the daemon writes to ~/.local/state/linux-reshade/daemon.log, and the layer to LINUX_RESHADE_LOG.")));
        return page;
    }
    let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
    let chooser = gtk::DropDown::from_strings(&names);
    let view = gtk::TextView::new();
    view.set_editable(false);
    view.set_monospace(true);
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_child(Some(&view));
    scroller.set_vexpand(true);
    let refresh = {
        let (chooser, view, scroller, files) = (chooser.clone(), view.clone(), scroller.clone(), files.clone());
        Rc::new(move || {
            let Some((_, path)) = files.get(chooser.selected() as usize) else { return };
            view.buffer().set_text(&tail_of(path, 64 * 1024));
            // Rola para o fim (o que acabou de acontecer).
            let scroller = scroller.clone();
            gtk::glib::idle_add_local_once(move || {
                let adjustment = scroller.vadjustment();
                adjustment.set_value(adjustment.upper() - adjustment.page_size());
            });
        })
    };
    {
        let refresh = refresh.clone();
        chooser.connect_selected_notify(move |_| refresh());
    }
    let button = gtk::Button::with_label(tr("Refresh"));
    {
        let refresh = refresh.clone();
        button.connect_clicked(move |_| refresh());
    }
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    chooser.set_hexpand(true);
    top.append(&chooser);
    top.append(&button);
    page.append(&top);
    page.append(&scroller);
    refresh();
    page
}

fn performance_page(window: &gtk::Window) -> gtk::Box {
    let page = page_box();
    page.append(&hint(tr("Measures how much the layer costs per frame on the CPU and how much the GPU spends on each effect. Measuring costs a little; turn it on only to look.")));
    let toggle = gtk::CheckButton::with_label(tr("Measure performance"));
    toggle.set_active(ipc::stats_on());
    toggle.connect_toggled(|b| {
        let _ = ipc::set_stats_on(b.is_active());
    });
    let text = gtk::Label::new(None);
    text.set_xalign(0.0);
    text.set_yalign(0.0);
    text.set_selectable(true);
    text.add_css_class("monospace");
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_child(Some(&text));
    scroller.set_vexpand(true);
    page.append(&toggle);
    page.append(&scroller);
    let refresh = {
        let text = text.clone();
        move || {
            let layers = ipc::live_layers().unwrap_or_default();
            let mut out = String::new();
            for l in &layers {
                let name = if l.status.app.is_empty() { l.id.clone() } else { l.status.app.clone() };
                match &l.status.perf {
                    Some(perf) => out.push_str(&format!("{name}\n{}\n", control_core::layers::perf_text(perf))),
                    None => out.push_str(&format!("{name}\n  ({}{})\n\n", tr("no measurement"), if ipc::stats_on() { tr(": waiting for the next second") } else { tr(": turn the measurement on above") })),
                }
            }
            if layers.is_empty() {
                out = tr("No game with the layer open.").to_string();
            }
            text.set_text(&out);
        }
    };
    refresh();
    let window = window.clone();
    gtk::glib::timeout_add_seconds_local(1, move || {
        if !window.is_visible() {
            return gtk::glib::ControlFlow::Break;
        }
        refresh();
        gtk::glib::ControlFlow::Continue
    });
    page
}

/// Quanto ocupa o cache dos efeitos compilados, em texto.
fn cache_usage_text(compiler: &control_core::compiler::Compiler) -> String {
    let u = control_core::maintenance::usage(compiler);
    format!("{}\n{}", compiler.cache.display(), fmt("{} compiled effects, {} analyses, {} MB", &[&u.bundles, &u.analyses, &format!("{:.1}", u.bytes as f64 / 1e6)]))
}

/// Os nomes dos efeitos do preset global (sem repetir), para medir a compilação deles.
fn global_preset_effects() -> Vec<String> {
    let Some(dir) = config::Config::load().presets_dir else { return Vec::new() };
    let Ok(preset) = control_core::presets::load(&dir.join("preset.ini")) else { return Vec::new() };
    let mut names: Vec<String> = Vec::new();
    for row in control_core::presets::rows(&preset) {
        if let Some(file) = row.id.file {
            let name = file.trim_end_matches(".fx").to_string();
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

fn cache_page() -> gtk::Box {
    use control_core::maintenance::{self, Clear};
    let page = page_box();
    page.append(&hint(tr("Compiled effects and compatibility analyses are kept. Forced clearing is for testing compilation from scratch; whatever an open game uses is recompiled right after.")));
    let compiler = Rc::new(control_core::compiler::Compiler::from_env());
    let usage = gtk::Label::new(None);
    usage.set_xalign(0.0);
    usage.set_selectable(true);
    let refresh_usage = {
        let (usage, compiler) = (usage.clone(), compiler.clone());
        move || usage.set_text(&match &*compiler {
            Ok(c) => cache_usage_text(c),
            Err(e) => fmt("No compiler: {}", &[&e]),
        })
    };
    refresh_usage();
    page.append(&usage);

    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_wrap(true);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    for (label, what, tip) in [
        (tr("Clear compiled"), Clear::Bundles, tr("Deletes the compiled effects (the analyses stay)")),
        (tr("Clear analyses"), Clear::Analyses, tr("Deletes the compatibility analyses; the panel redoes them in the background")),
        (tr("Clear everything"), Clear::All, tr("Deletes the compiled effects and the analyses")),
    ] {
        let button = gtk::Button::with_label(label);
        button.set_tooltip_text(Some(tip));
        let (compiler, status, refresh_usage) = (compiler.clone(), status.clone(), refresh_usage.clone());
        button.connect_clicked(move |_| {
            if let Ok(c) = &*compiler {
                let n = maintenance::clear(c, what);
                status.set_text(&fmt("{} items deleted.", &[&n]));
                refresh_usage();
            }
        });
        buttons.append(&button);
    }
    page.append(&buttons);

    page.append(&hint(tr("Measuring compilation deletes the compiled versions of the global preset's effects and compiles them again, one at a time (1920x1080, no macros), timing each one.")));
    let measure = gtk::Button::with_label(tr("Recompile the global preset and measure"));
    measure.set_halign(gtk::Align::Start);
    let report = gtk::Label::new(None);
    report.set_xalign(0.0);
    report.set_yalign(0.0);
    report.set_selectable(true);
    report.add_css_class("monospace");
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_child(Some(&report));
    scroller.set_vexpand(true);
    {
        let (compiler, status, report, refresh_usage, measure_button) = (compiler.clone(), status.clone(), report.clone(), refresh_usage.clone(), measure.clone());
        measure.connect_clicked(move |_| {
            if compiler.is_err() {
                return;
            }
            let effects = global_preset_effects();
            if effects.is_empty() {
                status.set_text(tr("The global preset has no effects."));
                return;
            }
            measure_button.set_sensitive(false);
            status.set_text(&fmt("Compiling {} effects…", &[&effects.len()]));
            report.set_text("");
            let total = effects.len();
            let (tx, rx) = std::sync::mpsc::channel::<maintenance::Timing>();
            let worker = control_core::compiler::Compiler::from_env();
            std::thread::spawn(move || {
                let Ok(worker) = worker else { return };
                maintenance::time_compiles(&worker, &effects, |t| {
                    let _ = tx.send(t.clone());
                });
            });
            let done: Rc<RefCell<Vec<maintenance::Timing>>> = Rc::new(RefCell::new(Vec::new()));
            let (status, report, refresh_usage, measure_button) = (status.clone(), report.clone(), refresh_usage.clone(), measure_button.clone());
            gtk::glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
                let mut finished = false;
                loop {
                    match rx.try_recv() {
                        Ok(t) => done.borrow_mut().push(t),
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            finished = true;
                            break;
                        }
                    }
                }
                let n = done.borrow().len();
                if finished || n >= total {
                    report.set_text(&maintenance::report(&done.borrow()));
                    status.set_text(tr("Done."));
                    measure_button.set_sensitive(true);
                    refresh_usage();
                    return gtk::glib::ControlFlow::Break;
                }
                status.set_text(&fmt("Compiling… {}/{}", &[&n, &total]));
                gtk::glib::ControlFlow::Continue
            });
        });
    }
    page.append(&measure);
    page.append(&status);
    page.append(&scroller);
    page
}

/// A aba Flatpak: ligar o linux-reshade em TODOS os apps Flatpak de uma vez (como a configuração global do Flatseal).
fn flatpak_page() -> gtk::Box {
    use control_core::flatpak;
    let page = page_box();
    page.append(&hint(tr("Turns linux-reshade on for all Flatpak apps at once, like Flatseal's global setting: it installs the layer extension for every runtime version in use and grants the permissions in the global configuration. To leave out a single app, use \"Turn off for this app\" on the Flatpak games page, or set the environment variable LINUX_RESHADE=0 on it in Flatseal.")));
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let label = gtk::Label::new(Some(tr("Apply to all Flatpak apps")));
    label.set_hexpand(true);
    label.set_xalign(0.0);
    let toggle = gtk::Switch::new();
    toggle.set_valign(gtk::Align::Center);
    line.append(&label);
    line.append(&toggle);
    page.append(&line);
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_wrap(true);
    page.append(&status);
    if !flatpak::available() {
        toggle.set_sensitive(false);
        status.set_text(tr("The 'flatpak' command is not installed on this machine."));
        return page;
    }
    // O estado atual, lido fora da thread da tela.
    let guard = Rc::new(std::cell::Cell::new(true)); // enquanto o interruptor é posto pelo programa, o aviso dele não conta
    {
        let (toggle, guard) = (toggle.clone(), guard.clone());
        gtk::glib::spawn_future_local(async move {
            if let Ok(on) = gtk::gio::spawn_blocking(flatpak::global_enabled).await {
                toggle.set_active(on);
            }
            guard.set(false);
        });
    }
    toggle.connect_active_notify({
        let (status, guard) = (status.clone(), guard.clone());
        move |t| {
            if guard.get() {
                return;
            }
            let guard = guard.clone();
            let on = t.is_active();
            t.set_sensitive(false);
            status.remove_css_class("error");
            status.set_text(if on { tr("Installing the extensions and adjusting the permissions…") } else { tr("Undoing…") });
            let (t, status) = (t.clone(), status.clone());
            gtk::glib::spawn_future_local(async move {
                let result = gtk::gio::spawn_blocking(move || if on { flatpak::enable_global() } else { flatpak::disable_global() }).await.unwrap_or_else(|_| Err(tr("the operation failed").to_string()));
                match result {
                    Ok(()) => status.set_text(if on { tr("On for all Flatpak apps (takes effect the next time each one opens).") } else { tr("Undone: no Flatpak app loads linux-reshade through the global setting any more.") }),
                    Err(why) => {
                        status.add_css_class("error");
                        status.set_text(&why);
                    }
                }
                let actual = gtk::gio::spawn_blocking(flatpak::global_enabled).await.unwrap_or(false);
                // O interruptor mostra o que ficou valendo (se falhou, volta).
                if t.is_active() != actual {
                    // O guarda evita que a própria correção dispare a ação de novo.
                    guard.set(true);
                    t.set_active(actual);
                    guard.set(false);
                }
                t.set_sensitive(true);
            });
        }
    });
    page
}

/// A aba Sistema: a camada do Vulkan (jogos que não são Flatpak), o daemon e o início dele com a sessão. É o que, antes, se fazia por
/// comandos no terminal.
fn system_page() -> gtk::Box {
    use control_core::install;
    let page = page_box();
    page.append(&hint(tr("For games that are not Flatpak, linux-reshade needs two things: the Vulkan layer installed for your user and the daemon running (it compiles the effects of open games and registers the shortcuts). After installing the layer, turn it on in a game with LINUX_RESHADE=1 in its launch options.")));

    let layer_status = gtk::Label::new(None);
    layer_status.set_xalign(0.0);
    layer_status.set_wrap(true);
    layer_status.set_selectable(true);
    let layer_button = gtk::Button::with_label(tr("Install or update the layer"));
    layer_button.set_halign(gtk::Align::Start);
    let show_layer = {
        let layer_status = layer_status.clone();
        Rc::new(move || {
            layer_status.set_text(&if install::layer_installed() {
                fmt("Vulkan layer: installed and up to date\n{}", &[&install::manifest_path().display()])
            } else {
                tr("Vulkan layer: not installed (or out of date)").to_string()
            });
        })
    };
    show_layer();
    layer_button.connect_clicked({
        let (layer_status, show_layer) = (layer_status.clone(), show_layer.clone());
        move |_| match install::install_layer() {
            Ok(_) => show_layer(),
            Err(why) => layer_status.set_text(&fmt("Could not install: {}", &[&why])),
        }
    });

    let daemon_status = gtk::Label::new(None);
    daemon_status.set_xalign(0.0);
    let daemon_button = gtk::Button::with_label(tr("Start the daemon now"));
    daemon_button.set_halign(gtk::Align::Start);
    let show_daemon = {
        let (daemon_status, daemon_button) = (daemon_status.clone(), daemon_button.clone());
        Rc::new(move || {
            let running = ipc::daemon_lock::is_running();
            let outdated = install::daemon_outdated();
            daemon_status.set_text(if outdated {
                tr("Daemon: running, but from an older build")
            } else if running {
                tr("Daemon: running")
            } else {
                tr("Daemon: stopped")
            });
            daemon_button.set_label(if outdated {
                tr("Replace the daemon with this build")
            } else if running {
                tr("Restart the daemon")
            } else {
                tr("Start the daemon now")
            });
        })
    };
    show_daemon();
    daemon_button.connect_clicked({
        let show_daemon = show_daemon.clone();
        move |_| {
            // Parado: inicia. Rodando: reinicia (um de outra build é trocado pelo desta).
            if ipc::daemon_lock::is_running() {
                install::stop_daemon();
            }
            install::start_daemon_if_needed();
            // Ele leva um instante para pegar a trava.
            let show_daemon = show_daemon.clone();
            gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(1200), move || show_daemon());
        }
    });

    let autostart_line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let autostart_label = gtk::Label::new(Some(tr("Start the daemon with the session")));
    autostart_label.set_hexpand(true);
    autostart_label.set_xalign(0.0);
    let autostart = gtk::Switch::new();
    autostart.set_valign(gtk::Align::Center);
    autostart.set_active(install::autostart_enabled());
    let autostart_note = gtk::Label::new(None);
    autostart_note.set_xalign(0.0);
    autostart_note.set_wrap(true);
    autostart.connect_active_notify({
        let autostart_note = autostart_note.clone();
        move |sw| match install::set_autostart(sw.is_active()) {
            Ok(()) => autostart_note.set_text(""),
            Err(why) => {
                autostart_note.add_css_class("error");
                autostart_note.set_text(&why);
            }
        }
    });
    autostart_line.append(&autostart_label);
    autostart_line.append(&autostart);

    page.append(&layer_status);
    page.append(&layer_button);
    page.append(&daemon_status);
    page.append(&daemon_button);
    page.append(&autostart_line);
    page.append(&autostart_note);
    page
}

fn about_page() -> gtk::Box {
    let page = page_box();
    let config = config::Config::load();
    let show = |p: Option<&std::path::Path>| p.map_or(tr("(not configured)").to_string(), |p| p.display().to_string());
    let text = fmt(
        "linux-reshade {}\nReShade-compatible Vulkan layer for Linux.\n\nData folder: {}\nconfig.ini: {}\nPresets: {}\nCompiled effects cache: {}\nScreenshots: {}\nShared state (memory): {}\n\nReShade options and what is missing: docs/reshade-opcoes.md",
        &[
            &env!("CARGO_PKG_VERSION"),
            &show(config.data_dir.as_deref()),
            &show(config.config_path().as_deref()),
            &show(config.presets_dir.as_deref()),
            &config.cache_dir.display(),
            &config.screenshot_dir.display(),
            &ipc::state_dir().display(),
        ],
    );
    let label = gtk::Label::new(Some(&text));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_selectable(true);
    page.append(&label);
    page
}

/// A janela de configurações: atalhos, capturas de tela e pastas.
pub fn open(parent: &gtk::ApplicationWindow) -> gtk::Window {
    open_at(parent, 0)
}

/// Como [`open`], já na aba `tab`.
pub fn open_at(parent: &gtk::ApplicationWindow, tab: u32) -> gtk::Window {
    let window = gtk::Window::builder().title(tr("Settings")).transient_for(parent).modal(true).default_width(700).default_height(600).build();
    let notebook = gtk::Notebook::new();
    notebook.append_page(&general_page(&window), Some(&gtk::Label::new(Some(tr("General")))));
    notebook.append_page(&shortcuts_page(&window), Some(&gtk::Label::new(Some(tr("Shortcuts")))));
    notebook.append_page(&captures_page(&window), Some(&gtk::Label::new(Some(tr("Screenshots")))));
    notebook.append_page(&folders_page(&window), Some(&gtk::Label::new(Some(tr("Folders")))));
    notebook.append_page(&performance_page(&window), Some(&gtk::Label::new(Some(tr("Performance")))));
    notebook.append_page(&cache_page(), Some(&gtk::Label::new(Some(tr("Cache")))));
    notebook.append_page(&system_page(), Some(&gtk::Label::new(Some(tr("System")))));
    notebook.append_page(&flatpak_page(), Some(&gtk::Label::new(Some(tr("Flatpak")))));
    notebook.append_page(&log_page(), Some(&gtk::Label::new(Some(tr("Log")))));
    notebook.append_page(&about_page(), Some(&gtk::Label::new(Some(tr("About")))));
    window.set_child(Some(&notebook));
    // Esc fecha a janela. Enquanto a aba de atalhos espera uma tecla, o Esc cancela só a captura (o controle dela, na fase de captura,
    // consome a tecla antes deste).
    let escape = gtk::EventControllerKey::new();
    escape.connect_key_pressed({
        let window = window.clone();
        move |_, keyval, _, _| {
            if keyval == gtk::gdk::Key::Escape {
                window.close();
                return gtk::glib::Propagation::Stop;
            }
            gtk::glib::Propagation::Proceed
        }
    });
    window.add_controller(escape);
    window.present();
    // Só para os testes de captura da tela: abrir direto numa aba.
    let tab = std::env::var("LINUX_RESHADE_SETTINGS_TAB").ok().and_then(|v| v.parse().ok()).unwrap_or(tab);
    notebook.set_current_page(Some(tab));
    window
}

/// Uma janelinha que espera uma combinação de teclas: devolve o atalho (formato XDG; texto vazio para "sem atalho") ou nada se
/// a pessoa cancelou.
pub fn capture_key(parent: &impl IsA<gtk::Window>, title: &str, on_done: impl Fn(String) + 'static) {
    let window = gtk::Window::builder().title(title).transient_for(parent).modal(true).default_width(360).build();
    let page = page_box();
    let message = gtk::Label::new(Some(tr("Press the desired key (or combination).\nEsc cancels.")));
    message.set_wrap(true);
    let status = gtk::Label::new(None);
    status.set_wrap(true);
    status.add_css_class("error");
    let clear = gtk::Button::with_label(tr("Remove the shortcut"));
    page.append(&message);
    page.append(&status);
    page.append(&clear);
    let on_done = Rc::new(on_done);
    {
        let (window, on_done) = (window.clone(), on_done.clone());
        clear.connect_clicked(move |_| {
            on_done(String::new());
            window.close();
        });
    }
    let controller = gtk::EventControllerKey::new();
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    let (window_key, status_key) = (window.clone(), status.clone());
    controller.connect_key_pressed(move |_, keyval, keycode, state| {
        if keyval == gtk::gdk::Key::Escape {
            window_key.close();
            return gtk::glib::Propagation::Stop;
        }
        match captured(&window_key, keyval, keycode, state) {
            None => {}
            Some(Ok(text)) => {
                on_done(text);
                window_key.close();
            }
            Some(Err(why)) => status_key.set_text(&fmt("That key does not work: {}", &[&why])),
        }
        gtk::glib::Propagation::Stop
    });
    window.add_controller(controller);
    window.set_child(Some(&page));
    window.present();
}
