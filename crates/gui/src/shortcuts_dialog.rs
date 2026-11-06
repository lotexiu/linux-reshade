//! A janela de atalhos: uma linha por tecla do `config.ini` (seção `[INPUT]`), com a tecla atual, um botão que captura a próxima
//! combinação digitada, "Limpar" e "Padrão". O daemon reinicia sozinho ao ver a mudança e passa a usar a tecla nova.

use gtk::prelude::*;
use gtk4 as gtk;
use shortcuts::trigger;
use std::cell::RefCell;
use std::rc::Rc;

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
        row.button.set_label("Sem atalho");
        row.note.set_text("");
        return;
    }
    match trigger::normalize(stored) {
        Ok(t) => {
            row.button.set_label(&trigger::display(&t));
            let warn = "Sem modificador: essa tecla deixa de funcionar em todos os programas enquanto o daemon estiver rodando.";
            row.note.set_text(if trigger::is_bare_key(&t) { warn } else { "" });
        }
        Err(why) => {
            row.button.set_label(stored);
            row.note.set_text(&format!("Não entendi esta tecla ({why}); ela é ignorada."));
        }
    }
}

fn save(key: &str, value: &str) -> Result<(), String> {
    let path = config::Config::load().config_path().ok_or("sem pasta de dados: rode `lrctl config init`")?;
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
    let intro = gtk::Label::new(Some("Atalhos globais: valem com o jogo em foco. Clique na tecla e digite a nova combinação (Esc cancela). Quem aplica o atalho é o daemon (`lrctl daemon`), que se reinicia sozinho ao ver a mudança."));
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
        let label = gtk::Label::new(Some(description));
        label.set_xalign(0.0);
        label.set_hexpand(true);
        row.button.set_size_request(190, -1);
        let clear = gtk::Button::with_label("Limpar");
        let factory = gtk::Button::with_label("Padrão");
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
            button.set_label("Pressione a tecla…");
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
                        status_key.set_text("Salvo. O daemon aplica em até 2 s.");
                    }
                    Err(why) => status_key.set_text(&why),
                }
            }
            Some(Err(why)) => {
                status_key.set_text(&format!("Essa tecla não serve: {why}"));
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
    let choose = gtk::Button::with_label(if list { "Adicionar pasta…" } else { "Escolher…" });
    line.append(&entry);
    line.append(&choose);
    page.append(&line);
    let save = Rc::new(save);
    let commit = {
        let (entry, status, save) = (entry.clone(), status.clone(), save.clone());
        move || match save(entry.text().trim()) {
            Ok(()) => status.set_text("Salvo. O daemon reinicia sozinho; feche e abra o painel para ele também usar o novo valor."),
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
    page.append(&hint("As pastas ficam no config.ini. Uma entrada com /** inclui as subpastas. O que é relativo (./) conta a partir da pasta do config.ini."));
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_wrap(true);
    let list = |section: &'static str, key: &'static str| {
        move |text: &str| -> Result<(), String> {
            let path = config_file().ok_or("sem pasta de dados: rode `lrctl config init`")?;
            let items: Vec<String> = text.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            config::set_list(&path, section, key, &items).map_err(|e| format!("{}: {e}", path.display()))
        }
    };
    let single = |section: &'static str, key: &'static str| {
        move |text: &str| -> Result<(), String> {
            let path = config_file().ok_or("sem pasta de dados: rode `lrctl config init`")?;
            config::set_value(&path, section, key, text).map_err(|e| format!("{}: {e}", path.display()))
        }
    };
    folder_row(&page, window, "Efeitos (.fx)", "Pastas onde os efeitos são procurados (a primeira com o nome vence).", ini_list("GENERAL", "EffectSearchPaths", "./Shaders/**"), true, list("GENERAL", "EffectSearchPaths"), &status);
    folder_row(&page, window, "Texturas dos efeitos", "Pastas das imagens que os efeitos carregam (LUTs, ruído...).", ini_list("GENERAL", "TextureSearchPaths", "./Textures/**"), true, list("GENERAL", "TextureSearchPaths"), &status);
    folder_row(&page, window, "Presets", "Onde ficam o preset global e a pasta de cada jogo.", ini_text("GENERAL", "PresetsPath", "./Presets"), false, single("GENERAL", "PresetsPath"), &status);
    folder_row(&page, window, "Efeitos compilados (cache)", "Onde os efeitos compilados ficam guardados; pode ser apagada à vontade.", ini_text("GENERAL", "IntermediateCachePath", &config::Config::load().cache_dir.to_string_lossy()), false, single("GENERAL", "IntermediateCachePath"), &status);
    page.append(&status);
    page
}

fn captures_page(window: &gtk::Window) -> gtk::Box {
    let page = page_box();
    page.append(&hint("A tecla de captura fica na aba Atalhos. Cada jogo com a layer aberta grava o quadro dele."));
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_wrap(true);
    let default = config::Config::load().screenshot_dir.to_string_lossy().into_owned();
    folder_row(&page, window, "Pasta das capturas", "Cada jogo ganha uma subpasta; os arquivos levam a data e a hora.", ini_text("SCREENSHOT", "SavePath", &default), false, |text| {
        let path = config_file().ok_or("sem pasta de dados: rode `lrctl config init`")?;
        config::set_value(&path, "SCREENSHOT", "SavePath", text).map_err(|e| format!("{}: {e}", path.display()))
    }, &status);
    let both = gtk::CheckButton::with_label("Guardar também o quadro sem efeitos (para comparar), com o final \"_sem-efeitos\"");
    both.set_active(config::Config::load().screenshot_before);
    {
        let status = status.clone();
        both.connect_toggled(move |b| {
            let Some(path) = config_file() else { return };
            match config::set_value(&path, "SCREENSHOT", "SaveBeforeShot", if b.is_active() { "1" } else { "0" }) {
                Ok(()) => status.set_text("Salvo."),
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
    let Ok(mut file) = std::fs::File::open(path) else { return format!("(não consegui abrir {})", path.display()) };
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
        files.push(("Daemon".to_string(), dir.join("daemon.log")));
        files.push(("Layer (jogos nativos, vkcube)".to_string(), dir.join("layer.log")));
    }
    files.push(("Layer (Flatpak, Sober)".to_string(), std::path::PathBuf::from("/tmp/linux-reshade-flatpak.log")));
    files.into_iter().filter(|(_, p)| p.exists()).collect()
}

fn log_page() -> gtk::Box {
    let page = page_box();
    let files = log_files();
    if files.is_empty() {
        page.append(&hint("Nenhum log encontrado: o daemon grava em ~/.local/state/linux-reshade/daemon.log, e a layer em LINUX_RESHADE_LOG."));
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
    let button = gtk::Button::with_label("Atualizar");
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
    page.append(&hint("Mede quanto a layer custa por quadro no processador e quanto a GPU gasta em cada efeito. Medir custa um pouco; ligue só para ver."));
    let toggle = gtk::CheckButton::with_label("Medir o desempenho");
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
                    None => out.push_str(&format!("{name}\n  (sem medição{})\n\n", if ipc::stats_on() { ": aguardando o próximo segundo" } else { ": ligue a medição acima" })),
                }
            }
            if layers.is_empty() {
                out = "Nenhum jogo com a layer aberto.".to_string();
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
    format!("{}\n{} efeitos compilados, {} análises, {:.1} MB", compiler.cache.display(), u.bundles, u.analyses, u.bytes as f64 / 1e6)
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
    page.append(&hint("Os efeitos compilados e as análises de compatibilidade ficam guardados. Limpar à força serve para testar a compilação do zero; o que um jogo aberto usa é recompilado em seguida."));
    let compiler = Rc::new(control_core::compiler::Compiler::from_env());
    let usage = gtk::Label::new(None);
    usage.set_xalign(0.0);
    usage.set_selectable(true);
    let refresh_usage = {
        let (usage, compiler) = (usage.clone(), compiler.clone());
        move || usage.set_text(&match &*compiler {
            Ok(c) => cache_usage_text(c),
            Err(e) => format!("Sem compilador: {e}"),
        })
    };
    refresh_usage();
    page.append(&usage);

    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_wrap(true);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    for (label, what, tip) in [
        ("Limpar compilados", Clear::Bundles, "Apaga os efeitos compilados (as análises ficam)"),
        ("Limpar análises", Clear::Analyses, "Apaga as análises de compatibilidade; o painel as refaz em segundo plano"),
        ("Limpar tudo", Clear::All, "Apaga os compilados e as análises"),
    ] {
        let button = gtk::Button::with_label(label);
        button.set_tooltip_text(Some(tip));
        let (compiler, status, refresh_usage) = (compiler.clone(), status.clone(), refresh_usage.clone());
        button.connect_clicked(move |_| {
            if let Ok(c) = &*compiler {
                let n = maintenance::clear(c, what);
                status.set_text(&format!("{n} itens apagados."));
                refresh_usage();
            }
        });
        buttons.append(&button);
    }
    page.append(&buttons);

    page.append(&hint("Medir a compilação apaga os compilados dos efeitos do preset global e os compila de novo, um por vez (1920x1080, sem macros), contando o tempo de cada um."));
    let measure = gtk::Button::with_label("Recompilar o preset global e medir");
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
                status.set_text("O preset global não tem efeitos.");
                return;
            }
            measure_button.set_sensitive(false);
            status.set_text(&format!("Compilando {} efeitos…", effects.len()));
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
                    status.set_text("Pronto.");
                    measure_button.set_sensitive(true);
                    refresh_usage();
                    return gtk::glib::ControlFlow::Break;
                }
                status.set_text(&format!("Compilando… {n}/{total}"));
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
    page.append(&hint("Liga o linux-reshade em todos os apps Flatpak de uma vez, como a configuração global do Flatseal: instala a extensão da layer para cada versão de runtime em uso e libera as permissões na configuração global. Para tirar um app só, use \"Desligar neste app\" na página Jogos Flatpak, ou, no Flatseal, a variável de ambiente LINUX_RESHADE=0 nele."));
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let label = gtk::Label::new(Some("Aplicar a todos os apps Flatpak"));
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
        status.set_text("O comando 'flatpak' não está instalado nesta máquina.");
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
            status.set_text(if on { "Instalando as extensões e ajustando as permissões…" } else { "Desfazendo…" });
            let (t, status) = (t.clone(), status.clone());
            gtk::glib::spawn_future_local(async move {
                let result = gtk::gio::spawn_blocking(move || if on { flatpak::enable_global() } else { flatpak::disable_global() }).await.unwrap_or_else(|_| Err("a operação falhou".to_string()));
                match result {
                    Ok(()) => status.set_text(if on { "Ligado para todos os apps Flatpak (vale na próxima vez que cada um abrir)." } else { "Desfeito: nenhum app Flatpak carrega mais o linux-reshade pela configuração global." }),
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

fn about_page() -> gtk::Box {
    let page = page_box();
    let config = config::Config::load();
    let show = |p: Option<&std::path::Path>| p.map_or("(não configurado)".to_string(), |p| p.display().to_string());
    let text = format!(
        "linux-reshade {}\nCamada Vulkan compatível com o ReShade, para Linux.\n\nPasta de dados: {}\nconfig.ini: {}\nPresets: {}\nCache dos efeitos compilados: {}\nCapturas de tela: {}\nEstado compartilhado (memória): {}\n\nOpções do ReShade e o que falta: docs/reshade-opcoes.md",
        env!("CARGO_PKG_VERSION"),
        show(config.data_dir.as_deref()),
        show(config.config_path().as_deref()),
        show(config.presets_dir.as_deref()),
        config.cache_dir.display(),
        config.screenshot_dir.display(),
        ipc::state_dir().display(),
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
    let window = gtk::Window::builder().title("Configurações").transient_for(parent).modal(true).default_width(700).default_height(600).build();
    let notebook = gtk::Notebook::new();
    notebook.append_page(&shortcuts_page(&window), Some(&gtk::Label::new(Some("Atalhos"))));
    notebook.append_page(&captures_page(&window), Some(&gtk::Label::new(Some("Capturas de tela"))));
    notebook.append_page(&folders_page(&window), Some(&gtk::Label::new(Some("Pastas"))));
    notebook.append_page(&performance_page(&window), Some(&gtk::Label::new(Some("Desempenho"))));
    notebook.append_page(&cache_page(), Some(&gtk::Label::new(Some("Cache"))));
    notebook.append_page(&flatpak_page(), Some(&gtk::Label::new(Some("Flatpak"))));
    notebook.append_page(&log_page(), Some(&gtk::Label::new(Some("Registro"))));
    notebook.append_page(&about_page(), Some(&gtk::Label::new(Some("Sobre"))));
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
    if let Some(tab) = std::env::var("LINUX_RESHADE_SETTINGS_TAB").ok().and_then(|v| v.parse().ok()) {
        notebook.set_current_page(Some(tab));
    }
    window
}

/// Uma janelinha que espera uma combinação de teclas: devolve o atalho (formato XDG; texto vazio para "sem atalho") ou nada se
/// a pessoa cancelou.
pub fn capture_key(parent: &impl IsA<gtk::Window>, title: &str, on_done: impl Fn(String) + 'static) {
    let window = gtk::Window::builder().title(title).transient_for(parent).modal(true).default_width(360).build();
    let page = page_box();
    let message = gtk::Label::new(Some("Pressione a tecla (ou combinação) desejada.\nEsc cancela."));
    message.set_wrap(true);
    let status = gtk::Label::new(None);
    status.set_wrap(true);
    status.add_css_class("error");
    let clear = gtk::Button::with_label("Tirar o atalho");
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
            Some(Err(why)) => status_key.set_text(&format!("Essa tecla não serve: {why}")),
        }
        gtk::glib::Propagation::Stop
    });
    window.add_controller(controller);
    window.set_child(Some(&page));
    window.present();
}
