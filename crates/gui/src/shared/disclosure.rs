//! Uma seção que abre e fecha no próprio lugar, para usar DENTRO de um popover.
//!
//! Regra da interface: nunca um popover (nem um `DropDown`, que abre um) dentro de outro. O GTK4 só deixa o popover de
//! dentro tratar o clique que o fecha; o de fora fica aberto até outro clique, e a janela parece travada. Dentro de um
//! popover, o que precisa de "mais opções" usa esta seção: um botão que revela o conteúdo logo abaixo.

use gtk::prelude::*;
use gtk4 as gtk;

pub struct Disclosure {
    /// O botão que abre e fecha; vai onde a seção deve começar.
    pub button: gtk::ToggleButton,
    /// O conteúdo; vai logo depois do botão.
    pub body: gtk::Revealer,
}

impl Disclosure {
    pub fn new(label: &str, content: &impl IsA<gtk::Widget>) -> Self {
        let button = gtk::ToggleButton::with_label(label);
        let body = gtk::Revealer::new();
        body.set_child(Some(content));
        body.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        let revealer = body.clone();
        button.connect_toggled(move |b| revealer.set_reveal_child(b.is_active()));
        Self { button, body }
    }
}
