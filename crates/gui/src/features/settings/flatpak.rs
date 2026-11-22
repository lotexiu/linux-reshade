//! Aba Flatpak.

use gtk::prelude::*;
use gtk4 as gtk;
use std::rc::Rc;
use control_core::i18n::tr;
use super::widgets::*;

/// A aba Flatpak: ligar o linux-reshade em TODOS os apps Flatpak de uma vez (como a configuração global do Flatseal).
pub(super) fn flatpak_page() -> gtk::Box {
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
