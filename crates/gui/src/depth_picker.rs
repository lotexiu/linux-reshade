//! O botão "Profundidade…": lista as imagens de profundidade que a layer achou no jogo aberto (e a opção automática) para a
//! pessoa escolher qual os efeitos usam. Sem `DropDown` dentro do popover (ele trava os cliques ao fechar): botões de opção.

use gtk::prelude::*;
use gtk4 as gtk;
use ipc::DepthPick;
use panel::DepthChoice;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

pub struct DepthPicker {
    pub button: gtk::MenuButton,
    list: gtk::Box,
    on_change: RefCell<Option<Rc<dyn Fn(DepthPick)>>>,
    /// Verdadeiro enquanto a lista é refeita (para não tratar isso como escolha da pessoa).
    building: Cell<bool>,
    signature: RefCell<String>,
}

impl DepthPicker {
    pub fn new() -> Rc<Self> {
        let list = gtk::Box::new(gtk::Orientation::Vertical, 8);
        list.set_margin_top(12);
        list.set_margin_bottom(12);
        list.set_margin_start(12);
        list.set_margin_end(12);
        list.set_size_request(440, -1);
        let popover = gtk::Popover::new();
        popover.set_child(Some(&list));
        let button = gtk::MenuButton::new();
        button.set_label("Profundidade…");
        button.set_tooltip_text(Some("Escolhe qual imagem de profundidade os efeitos usam neste jogo (vale até fechá-lo)"));
        button.set_popover(Some(&popover));
        button.set_visible(false);
        Rc::new(Self { button, list, on_change: RefCell::new(None), building: Cell::new(false), signature: RefCell::new(String::new()) })
    }

    pub fn connect_changed(&self, f: impl Fn(DepthPick) + 'static) {
        *self.on_change.borrow_mut() = Some(Rc::new(f));
    }

    /// Mostra as candidatas e o pedido atual. Sem candidatas, o botão some. Só refaz a lista se algo mudou.
    pub fn set(self: &Rc<Self>, choices: &[DepthChoice], pick: DepthPick) {
        self.button.set_visible(!choices.is_empty());
        let signature = format!("{choices:?}|{pick:?}");
        if *self.signature.borrow() == signature || self.button.is_active() {
            return;
        }
        *self.signature.borrow_mut() = signature;
        self.building.set(true);
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let weak = Rc::downgrade(self);
        let first = option("Automática (a layer escolhe)", pick == DepthPick::Auto, None, DepthPick::Auto, &weak);
        self.list.append(&first);
        for c in choices {
            let mut text = c.label.clone();
            if c.automatic {
                text.push_str(" · a automática");
            }
            if let Some(why) = &c.unusable {
                text.push_str(&format!(" · não serve: {why}"));
            }
            self.list.append(&option(&text, pick == DepthPick::Candidate(c.number), Some(&first), DepthPick::Candidate(c.number), &weak));
        }

        // A profundidade depois de um passe específico (avançado).
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let current_pass = if let DepthPick::Pass(n) = pick { n } else { 1 };
        let pass_option = gtk::CheckButton::with_label("Depois do passe");
        pass_option.set_group(Some(&first));
        pass_option.set_active(matches!(pick, DepthPick::Pass(_)));
        let spin = gtk::SpinButton::with_range(1.0, 200.0, 1.0);
        spin.set_value(f64::from(current_pass));
        let apply = {
            let (weak, spin) = (weak.clone(), spin.clone());
            move || {
                if let Some(me) = weak.upgrade() {
                    me.emit(DepthPick::Pass(spin.value() as u32));
                }
            }
        };
        pass_option.connect_toggled({
            let apply = apply.clone();
            move |c| {
                if c.is_active() {
                    apply();
                }
            }
        });
        let use_button = gtk::Button::with_label("Usar");
        use_button.connect_clicked(move |_| apply());
        line.append(&pass_option);
        line.append(&spin);
        line.append(&use_button);
        self.list.append(&line);
        let hint = gtk::Label::new(Some("A escolha vale enquanto o jogo estiver aberto. Se o resultado piorar, volte para Automática."));
        hint.set_wrap(true);
        hint.set_xalign(0.0);
        hint.add_css_class("dim-label");
        hint.add_css_class("caption");
        self.list.append(&hint);
        self.building.set(false);
    }

    fn emit(&self, pick: DepthPick) {
        if self.building.get() {
            return;
        }
        let f = self.on_change.borrow().clone();
        if let Some(f) = f {
            f(pick);
        }
    }
}

fn option(text: &str, active: bool, group: Option<&gtk::CheckButton>, pick: DepthPick, weak: &Weak<DepthPicker>) -> gtk::CheckButton {
    let b = gtk::CheckButton::with_label(text);
    if let Some(g) = group {
        b.set_group(Some(g));
    }
    b.set_active(active);
    b.connect_toggled({
        let weak = weak.clone();
        move |c| {
            if c.is_active()
                && let Some(me) = weak.upgrade()
            {
                me.emit(pick);
            }
        }
    });
    b
}
