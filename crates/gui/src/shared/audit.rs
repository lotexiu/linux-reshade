//! `--dump-texts`: lista todo texto que a janela mostra (rótulos, botões, dicas, campos de busca, abas, listas), um por linha.
//! Serve para conferir um idioma por inteiro: o que sobra em outra língua aparece na lista.

use gtk4 as gtk;
use gtk::prelude::*;
use std::collections::BTreeSet;

fn collect(widget: &gtk::Widget, out: &mut BTreeSet<String>, in_row: bool) {
    let mut add = |s: Option<gtk::glib::GString>| {
        if let Some(s) = s.filter(|s| !s.trim().is_empty()) {
            out.insert(s.to_string());
        }
    };
    // As listas das linhas dos efeitos (opções de `ui_items` do próprio efeito) são dados do efeito, não texto da interface: nem os itens nem o
    // rótulo do item escolhido (que fica dentro do botão da lista) entram.
    if in_row && widget.is::<gtk::DropDown>() {
        return;
    }
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
    // As listas das linhas dos efeitos (opções de `ui_items` do próprio efeito) são dados do efeito, não texto da interface.
    if let Some(d) = widget.downcast_ref::<gtk::DropDown>()
        && !in_row
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
                collect(&tab, out, in_row);
            }
        }
    }
    let in_row = in_row || widget.is::<gtk::ListBoxRow>();
    let mut child = widget.first_child();
    while let Some(c) = child {
        collect(&c, out, in_row);
        child = c.next_sibling();
    }
}

/// A primeira lista suspensa de uma janela (para os testes que agem como a pessoa).
pub fn first_dropdown(widget: &gtk::Widget) -> Option<gtk::DropDown> {
    if let Some(d) = widget.downcast_ref::<gtk::DropDown>() {
        return Some(d.clone());
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        if let Some(d) = first_dropdown(&c) {
            return Some(d);
        }
        child = c.next_sibling();
    }
    None
}

/// Os textos visíveis de uma janela e de tudo o que há dentro dela.
pub fn texts_of(window: &impl IsA<gtk::Widget>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    collect(window.upcast_ref(), &mut out, false);
    out
}
