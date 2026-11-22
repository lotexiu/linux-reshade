//! Aba Geral: idioma e perguntar antes de apagar um preset.

use gtk::prelude::*;
use gtk4 as gtk;
use control_core::i18n::tr;
use super::widgets::*;

/// A aba "Geral": o idioma da interface (vale na hora: a janela é refeita) e a pergunta antes de apagar um preset.
pub(super) fn general_page(window: &gtk::Window) -> gtk::Box {
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
    ask.set_active(crate::features::settings::prefs::confirm_delete_preset());
    ask.connect_toggled(|b| crate::features::settings::prefs::set_confirm_delete_preset(b.is_active()));

    page.append(&line);
    page.append(&hint(tr("The whole interface changes language right away.")));
    page.append(&ask);
    page
}
