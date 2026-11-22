//! A janelinha que espera uma combinação de teclas (e a leitura da combinação digitada): usada pela aba de atalhos das Configurações e pelo atalho
//! de cada técnica na lista de efeitos.

use gtk::prelude::*;
use gtk4 as gtk;
use control_core::i18n::{fmt, tr};
use shortcuts::trigger;
use std::rc::Rc;
use crate::shared::widgets::page_box;

/// A combinação digitada, no texto da biblioteca de atalhos (`CTRL+SHIFT+F10`). `None` para um modificador sozinho.
pub fn captured(window: &gtk::Window, keyval: gtk::gdk::Key, keycode: u32, state: gtk::gdk::ModifierType) -> Option<Result<String, String>> {
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
