//! `--dump-texts`: lista todo texto que a janela mostra (rótulos, botões, dicas, campos de busca, abas, listas), um por linha.
//! Serve para conferir um idioma por inteiro: o que sobra em outra língua aparece na lista.

use gtk4 as gtk;
use gtk::prelude::*;
use std::collections::BTreeSet;

fn collect(widget: &gtk::Widget, out: &mut BTreeSet<String>) {
    let mut add = |s: Option<gtk::glib::GString>| {
        if let Some(s) = s.filter(|s| !s.trim().is_empty()) {
            out.insert(s.to_string());
        }
    };
    add(widget.tooltip_text());
    if let Some(l) = widget.downcast_ref::<gtk::Label>() {
        add(Some(l.text()));
    }
    if let Some(b) = widget.downcast_ref::<gtk::Button>() {
        add(b.label());
    }
    if let Some(b) = widget.downcast_ref::<gtk::CheckButton>() {
        add(b.label());
    }
    if let Some(b) = widget.downcast_ref::<gtk::ToggleButton>() {
        add(b.label());
    }
    if let Some(b) = widget.downcast_ref::<gtk::MenuButton>() {
        add(b.label());
    }
    if let Some(e) = widget.downcast_ref::<gtk::Entry>() {
        add(e.placeholder_text());
    }
    if let Some(e) = widget.downcast_ref::<gtk::SearchEntry>() {
        add(e.placeholder_text());
    }
    if let Some(w) = widget.downcast_ref::<gtk::Window>() {
        add(w.title());
    }
    if let Some(d) = widget.downcast_ref::<gtk::DropDown>()
        && let Some(model) = d.model()
    {
        for i in 0..model.n_items() {
            if let Some(s) = model.item(i).and_downcast::<gtk::StringObject>() {
                add(Some(s.string()));
            }
        }
    }
    if let Some(n) = widget.downcast_ref::<gtk::Notebook>() {
        for i in 0..n.n_pages() {
            if let Some(tab) = n.nth_page(Some(i)).and_then(|p| n.tab_label(&p)) {
                collect(&tab, out);
            }
        }
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        collect(&c, out);
        child = c.next_sibling();
    }
}

/// Os textos visíveis de uma janela e de tudo o que há dentro dela.
pub fn texts_of(window: &impl IsA<gtk::Widget>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    collect(window.upcast_ref(), &mut out);
    out
}
