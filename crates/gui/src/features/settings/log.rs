//! Aba Log: os registros do daemon e da layer.

use gtk::prelude::*;
use gtk4 as gtk;
use std::rc::Rc;
use control_core::i18n::{fmt, tr};
use super::widgets::*;

/// Os últimos `max` bytes de um arquivo de texto, a partir de uma linha inteira.
pub(super) fn tail_of(path: &std::path::Path, max: u64) -> String {
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
pub(super) fn log_files() -> Vec<(String, std::path::PathBuf)> {
    let state = paths::state_home().map(|d| d.join(config::LOCATION_DIR));
    let mut files = Vec::new();
    if let Some(dir) = state {
        files.push((tr("Daemon").to_string(), dir.join("daemon.log")));
        files.push((tr("Layer (native games, vkcube)").to_string(), dir.join("layer.log")));
    }
    files.push((tr("Layer (Flatpak, Sober)").to_string(), std::path::PathBuf::from("/tmp/linux-reshade-flatpak.log")));
    files.into_iter().filter(|(_, p)| p.exists()).collect()
}

pub(super) fn log_page() -> gtk::Box {
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
