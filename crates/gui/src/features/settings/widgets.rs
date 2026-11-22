//! O que as abas das Configurações compartilham: a caixa de uma página, o texto de ajuda, a leitura e a escrita do `config.ini`, a linha de pasta.

pub(super) use crate::shared::widgets::{hint, page_box};
use gtk::prelude::*;
use gtk4 as gtk;
use std::rc::Rc;
use control_core::i18n::tr;

/// Um texto selecionável não deve aparecer todo selecionado: o foco que a página recebe ao abrir (a primeira coisa que aceita foco) seleciona tudo.
/// Tira a seleção logo depois de a página aparecer; a pessoa ainda pode selecionar e copiar com o mouse.
pub(super) fn no_autoselect(label: &gtk::Label) {
    label.connect_map(|l| {
        // Duas vezes: o foco da página pode chegar depois do primeiro instante.
        for delay in [0u64, 250] {
            let l = l.clone();
            gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(delay), move || {
                if l.selection_bounds().is_some() && !l.has_focus() {
                    l.select_region(0, 0);
                } else if l.selection_bounds() == Some((0, l.text().chars().count() as i32)) {
                    l.select_region(0, 0);
                }
            });
        }
    });
}



/// O `config.ini` que a janela edita (existindo ou não).
pub(super) fn config_file() -> Option<std::path::PathBuf> {
    config::Config::load().config_path()
}

pub(super) fn ini_text(section: &str, key: &str, default: &str) -> String {
    config_file().and_then(|f| ini::Ini::load(&f).ok()).and_then(|i| i.get(section, key).map(String::from)).unwrap_or_else(|| default.to_string())
}

pub(super) fn ini_list(section: &str, key: &str, default: &str) -> String {
    config_file().and_then(|f| ini::Ini::load(&f).ok()).map(|i| i.get_list(section, key)).filter(|l| !l.is_empty()).map_or_else(|| default.to_string(), |l| l.join(", "))
}

/// Uma linha de pasta: o texto editável, "Escolher…" (ou "Adicionar pasta…" numa lista) e o salvamento.
pub(super) fn folder_row(page: &gtk::Box, window: &gtk::Window, label: &str, tip: &str, current: String, list: bool, save: impl Fn(&str) -> Result<(), String> + 'static, status: &gtk::Label) {
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
