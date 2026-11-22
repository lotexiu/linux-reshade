//! A janela de configurações: uma aba por arquivo (ver `docs/projeto/arquitetura-proposta.md`).

use gtk::prelude::*;
use gtk4 as gtk;
use control_core::i18n::tr;

mod about;
mod cache;
mod captures;
mod flatpak;
mod folders;
mod general;
mod log;
mod overlay;
mod performance;
pub mod prefs;
mod keys;
mod system;
mod widgets;

use about::about_page;
use cache::cache_page;
use captures::captures_page;
use flatpak::flatpak_page;
use folders::folders_page;
use general::general_page;
use log::log_page;
use overlay::overlay_page;
use performance::performance_page;
use keys::shortcuts_page;
use system::system_page;

/// A janela de configurações: atalhos, capturas de tela e pastas.
pub fn open(parent: &gtk::ApplicationWindow) -> gtk::Window {
    open_at(parent, 0)
}

/// Como [`open`], já na aba `tab`.
pub fn open_at(parent: &gtk::ApplicationWindow, tab: u32) -> gtk::Window {
    let window = gtk::Window::builder().title(tr("Settings")).transient_for(parent).modal(true).default_width(700).default_height(600).build();
    // Sem isto a janela não sabe de que programa é (`window.application()` vazio) e a troca de idioma que ela pede não chega à janela principal.
    window.set_application(parent.application().as_ref());
    let notebook = gtk::Notebook::new();
    notebook.append_page(&general_page(&window), Some(&gtk::Label::new(Some(tr("General")))));
    notebook.append_page(&shortcuts_page(&window), Some(&gtk::Label::new(Some(tr("Shortcuts")))));
    notebook.append_page(&captures_page(&window), Some(&gtk::Label::new(Some(tr("Screenshots")))));
    notebook.append_page(&folders_page(&window), Some(&gtk::Label::new(Some(tr("Folders")))));
    notebook.append_page(&performance_page(&window), Some(&gtk::Label::new(Some(tr("Performance")))));
    notebook.append_page(&overlay_page(), Some(&gtk::Label::new(Some(tr("Overlay")))));
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
