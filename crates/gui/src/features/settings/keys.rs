//! Aba Atalhos: uma linha por tecla do `config.ini` (seção `[INPUT]`), com a tecla atual, um botão que captura a próxima combinação digitada,
//! "Limpar" e "Padrão". O daemon reinicia sozinho ao ver a mudança e passa a usar a tecla nova. Também o diálogo que captura uma tecla.

use gtk::prelude::*;
use gtk4 as gtk;
use shortcuts::trigger;
use std::cell::RefCell;
use std::rc::Rc;
use control_core::i18n::{fmt, tr};
use crate::shared::capture_key::captured;

/// Uma linha da janela: qual tecla do config.ini, o botão que mostra o atalho e o aviso embaixo.
pub(super) struct Row {
    key: &'static str,
    button: gtk::Button,
    note: gtk::Label,
}

/// O texto guardado de uma tecla, no formato XDG ("" = sem atalho).
pub(super) fn current(config: &config::Config, key: &str) -> String {
    config.key(key)
}

pub(super) fn show(row: &Row, stored: &str) {
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

pub(super) fn save(key: &str, value: &str) -> Result<(), String> {
    let path = config::Config::load().config_path().ok_or(tr("no data folder: run `lrctl config init`"))?;
    config::set_key(&path, key, value).map_err(|e| format!("{}: {e}", path.display()))
}

pub(super) fn shortcuts_page(window: &gtk::Window) -> gtk::Box {
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
