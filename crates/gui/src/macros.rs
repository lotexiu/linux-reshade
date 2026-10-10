//! O editor de definições de pré-processador: um botão que abre a lista "nome à esquerda, controle à direita"
//! (liga/desliga, automático/sim/não, número), no lugar de um campo de texto `NOME=VALOR,OUTRO=2`.

use gtk::prelude::*;
use gtk4 as gtk;
use panel::macro_defs::{self, KnownMacro, MacroKind};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

/// Quanto esperar depois do último clique num número antes de aplicar (cada mudança recompila o efeito).
const NUMBER_DEBOUNCE_MS: u64 = 700;

pub struct MacroEditor {
    pub button: gtk::MenuButton,
    list: gtk::Box,
    /// O texto que a lista mostra agora (a forma guardada no preset).
    text: RefCell<String>,
    on_change: RefCell<Option<Rc<dyn Fn(String)>>>,
    /// Verdadeiro enquanto a lista é refeita a partir do texto (para não tratar isso como ação da pessoa).
    building: Cell<bool>,
    generation: Cell<u64>,
}

impl MacroEditor {
    pub fn new(label: &str, tooltip: &str) -> Rc<Self> {
        let list = gtk::Box::new(gtk::Orientation::Vertical, 8);
        list.set_size_request(460, -1);
        list.set_margin_top(10);
        list.set_margin_bottom(10);
        list.set_margin_start(12);
        list.set_margin_end(12);
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_child(Some(&list));
        scroller.set_min_content_width(480);
        scroller.set_min_content_height(200);
        scroller.set_max_content_height(460);
        scroller.set_propagate_natural_height(true);
        scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
        let popover = gtk::Popover::new();
        popover.set_child(Some(&scroller));
        let button = gtk::MenuButton::new();
        button.set_label(label);
        button.set_tooltip_text(Some(tooltip));
        button.set_popover(Some(&popover));
        let editor = Rc::new(Self { button, list, text: RefCell::new(String::new()), on_change: RefCell::new(None), building: Cell::new(false), generation: Cell::new(0) });
        editor.rebuild();
        editor
    }

    pub fn text(&self) -> String {
        self.text.borrow().clone()
    }

    /// O que fazer quando a pessoa muda uma definição: recebe o texto novo.
    pub fn connect_changed(&self, f: impl Fn(String) + 'static) {
        *self.on_change.borrow_mut() = Some(Rc::new(f));
    }

    /// Mostra o texto vindo do modelo. Refaz a lista só se mudou, e não enquanto a pessoa mexe nela.
    pub fn set_text(self: &Rc<Self>, text: &str) {
        if *self.text.borrow() == text {
            return;
        }
        *self.text.borrow_mut() = text.to_string();
        if !self.button.is_active() {
            self.rebuild();
        }
        self.update_label();
    }

    fn update_label(&self) {
        let n = preset::parse_macros(&self.text.borrow()).len();
        let base = self.button.label().map(|l| l.split(" (").next().unwrap_or("").to_string()).unwrap_or_default();
        self.button.set_label(&if n == 0 { base } else { format!("{base} ({n})") });
    }

    /// A pessoa mudou uma definição: guarda o novo texto e avisa.
    fn change(&self, name: &str, value: Option<&str>) {
        if self.building.get() {
            return;
        }
        let new = macro_defs::set(&self.text.borrow(), name, value);
        *self.text.borrow_mut() = new.clone();
        self.update_label();
        let f = self.on_change.borrow().clone();
        if let Some(f) = f {
            f(new);
        }
    }

    pub fn rebuild(self: &Rc<Self>) {
        self.building.set(true);
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let text = self.text.borrow().clone();
        let weak = Rc::downgrade(self);
        for k in macro_defs::KNOWN {
            self.list.append(&known_row(k, macro_defs::get(&text, k.name), &weak));
        }
        let custom = macro_defs::custom(&text);
        let heading = gtk::Label::new(Some("Outras definições"));
        heading.set_halign(gtk::Align::Start);
        heading.add_css_class("heading");
        heading.set_margin_top(8);
        self.list.append(&heading);
        for (name, value) in custom {
            self.list.append(&custom_row(&name, &value, &weak));
        }
        self.list.append(&add_row(&weak));
        self.building.set(false);
    }
}

fn row(label: &str, hint: &str) -> (gtk::Box, gtk::Label) {
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let caption = gtk::Label::new(Some(label));
    caption.set_halign(gtk::Align::Start);
    caption.set_hexpand(true);
    caption.set_xalign(0.0);
    caption.set_wrap(true);
    if !hint.is_empty() {
        line.set_tooltip_text(Some(hint));
    }
    line.append(&caption);
    (line, caption)
}

fn right(w: &impl IsA<gtk::Widget>) {
    w.set_halign(gtk::Align::End);
    w.set_valign(gtk::Align::Center);
}

fn known_row(k: &'static KnownMacro, value: Option<String>, weak: &Weak<MacroEditor>) -> gtk::Box {
    let (line, _) = row(k.label, &format!("{}\n({})", k.hint, k.name));
    match k.kind {
        MacroKind::Switch { default } => {
            let on = value.as_deref().map_or(default, |v| v != "0");
            let sw = gtk::Switch::new();
            sw.set_active(on);
            right(&sw);
            sw.connect_active_notify({
                let weak = weak.clone();
                move |s| {
                    if let Some(ed) = weak.upgrade() {
                        let now = s.is_active();
                        // Igual ao padrão do ReShade: tira a definição, em vez de gravar uma que não muda nada.
                        ed.change(k.name, if now == default { None } else { Some(if now { "1" } else { "0" }) });
                    }
                }
            });
            line.append(&sw);
        }
        MacroKind::Auto => {
            // Três botões ligados em vez de um DropDown: o DropDown abre um popover dentro do popover e, ao fechar,
            // deixa a janela anterior sem receber cliques.
            let group = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            group.add_css_class("linked");
            right(&group);
            let selected = match value.as_deref() {
                None => 0,
                Some("0") => 2,
                Some(_) => 1,
            };
            let mut first: Option<gtk::ToggleButton> = None;
            for (i, (caption, written)) in [("Automático", None), ("Sim", Some("1")), ("Não", Some("0"))].into_iter().enumerate() {
                let b = gtk::ToggleButton::with_label(caption);
                match &first {
                    Some(f) => b.set_group(Some(f)),
                    None => first = Some(b.clone()),
                }
                b.set_active(i == selected);
                b.connect_toggled({
                    let weak = weak.clone();
                    move |b| {
                        if b.is_active()
                            && let Some(ed) = weak.upgrade()
                        {
                            ed.change(k.name, written);
                        }
                    }
                });
                group.append(&b);
            }
            line.append(&group);
        }
        MacroKind::Number { default, min, max, step, digits } => {
            let current = value.as_deref().and_then(|v| v.parse::<f64>().ok()).unwrap_or(default);
            let spin = gtk::SpinButton::with_range(min, max, step);
            spin.set_digits(digits);
            spin.set_value(current);
            spin.set_width_chars(9);
            right(&spin);
            spin.connect_value_changed({
                let weak = weak.clone();
                move |s| {
                    let Some(ed) = weak.upgrade() else { return };
                    if ed.building.get() {
                        return;
                    }
                    let generation = ed.generation.get() + 1;
                    ed.generation.set(generation);
                    let (weak, value) = (Rc::downgrade(&ed), s.value());
                    gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(NUMBER_DEBOUNCE_MS), move || {
                        let Some(ed) = weak.upgrade() else { return };
                        if ed.generation.get() != generation {
                            return;
                        }
                        let same = (value - default).abs() < 1e-9;
                        let written = macro_defs::number_text(value, digits);
                        ed.change(k.name, if same { None } else { Some(&written) });
                    });
                }
            });
            line.append(&spin);
        }
    }
    line
}

fn custom_row(name: &str, value: &str, weak: &Weak<MacroEditor>) -> gtk::Box {
    let (line, _) = row(name, "Definição sua, fora da lista conhecida");
    let entry = gtk::Entry::new();
    entry.set_text(value);
    entry.set_width_chars(10);
    right(&entry);
    let apply = {
        let (weak, name) = (weak.clone(), name.to_string());
        move |e: &gtk::Entry| {
            if let Some(ed) = weak.upgrade() {
                let v = e.text().trim().to_string();
                if !v.is_empty() {
                    ed.change(&name, Some(&v));
                }
            }
        }
    };
    entry.connect_activate({
        let apply = apply.clone();
        move |e| apply(e)
    });
    let focus = gtk::EventControllerFocus::new();
    focus.connect_leave({
        let (apply, entry) = (apply, entry.clone());
        move |_| apply(&entry)
    });
    entry.add_controller(focus);
    let remove = gtk::Button::from_icon_name("user-trash-symbolic");
    remove.set_tooltip_text(Some("Remover esta definição"));
    remove.connect_clicked({
        let (weak, name) = (weak.clone(), name.to_string());
        move |_| {
            if let Some(ed) = weak.upgrade() {
                ed.change(&name, None);
                ed.rebuild();
            }
        }
    });
    line.append(&entry);
    line.append(&remove);
    line
}

fn add_row(weak: &Weak<MacroEditor>) -> gtk::Box {
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let name = gtk::Entry::new();
    name.set_placeholder_text(Some("NOME"));
    name.set_hexpand(true);
    let value = gtk::Entry::new();
    value.set_placeholder_text(Some("valor"));
    value.set_width_chars(8);
    let add = gtk::Button::from_icon_name("list-add-symbolic");
    add.set_tooltip_text(Some("Adicionar a definição"));
    let go = {
        let (weak, name, value) = (weak.clone(), name.clone(), value.clone());
        move || {
            let Some(ed) = weak.upgrade() else { return };
            let (n, v) = (name.text().trim().to_string(), value.text().trim().to_string());
            if n.is_empty() || !control_core::compiler::valid_define(&n, if v.is_empty() { "1" } else { &v }) {
                name.add_css_class("error");
                return;
            }
            ed.change(&n, Some(if v.is_empty() { "1" } else { &v }));
            ed.rebuild();
        }
    };
    add.connect_clicked({
        let go = go.clone();
        move |_| go()
    });
    value.connect_activate(move |_| go());
    line.append(&name);
    line.append(&value);
    line.append(&add);
    line
}
