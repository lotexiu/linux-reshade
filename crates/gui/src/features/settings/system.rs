//! Aba Sistema: instalação da layer, daemon e início automático.

use gtk::prelude::*;
use gtk4 as gtk;
use std::rc::Rc;
use control_core::i18n::{fmt, tr};
use super::widgets::*;

/// A aba Sistema: a camada do Vulkan (jogos que não são Flatpak), o daemon e o início dele com a sessão. É o que, antes, se fazia por
/// comandos no terminal.
pub(super) fn system_page() -> gtk::Box {
    use control_core::install;
    let page = page_box();
    page.append(&hint(tr("For games that are not Flatpak, linux-reshade needs two things: the Vulkan layer installed for your user and the daemon running (it compiles the effects of open games and registers the shortcuts). After installing the layer, turn it on in a game with LINUX_RESHADE=1 in its launch options.")));

    let layer_status = gtk::Label::new(None);
    layer_status.set_xalign(0.0);
    layer_status.set_wrap(true);
    layer_status.set_selectable(true);
    no_autoselect(&layer_status);
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
