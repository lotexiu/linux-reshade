//! A página "Depth Buffer": a profundidade que os efeitos usam no jogo aberto, numa página da pilha (como "Adicionar efeito"; sem popover).
//! Uma lista de opções (o automático e cada imagem de profundidade que a layer achou). Sob o automático ficam os pontos de captura (a
//! profundidade antes de cada limpeza que o jogo faz, ou depois de um passe), já com os números que o jogo tem, para a pessoa não adivinhar.
//! Componente: recebe os dados por `set` e avisa a escolha por `connect_changed`; não conhece a janela.

use control_core::i18n::{fmt, tr};
use gtk::prelude::*;
use gtk4 as gtk;
use ipc::DepthPick;
use panel::{DepthChoice, DepthClear};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

pub struct DepthView {
    pub root: gtk::Box,
    list: gtk::Box,
    on_change: RefCell<Option<Rc<dyn Fn(DepthPick)>>>,
    /// Verdadeiro enquanto a lista é refeita (para não tratar isso como escolha da pessoa).
    building: Cell<bool>,
    signature: RefCell<String>,
    rows: Cell<usize>,
    /// O que muda a todo quadro (os números dos textos): atualizado no lugar, sem refazer a lista (refazer tira o clique do botão de opção).
    live: RefCell<Vec<Box<dyn Fn(&[DepthChoice], &[DepthClear])>>>,
}

impl DepthView {
    pub fn new(on_back: impl Fn() + 'static) -> Rc<Self> {
        let back = gtk::Button::from_icon_name("go-previous-symbolic");
        back.set_tooltip_text(Some(tr("Back to the preset's effect list")));
        back.connect_clicked(move |_| on_back());
        let title = gtk::Label::new(Some(tr("Depth Buffer")));
        title.add_css_class("title-3");
        title.set_xalign(0.0);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        top.append(&back);
        top.append(&title);

        let intro = gtk::Label::new(Some(tr("Effects such as ambient occlusion, fog and depth of field read the game's depth image. Automatic usually works. Pick another image or a capture point only if those effects look wrong. The choice lasts while the game is open.")));
        intro.set_wrap(true);
        intro.set_xalign(0.0);
        intro.add_css_class("dim-label");

        let list = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_child(Some(&list));
        scroller.set_vexpand(true);
        scroller.set_hscrollbar_policy(gtk::PolicyType::Never);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
        for side in [gtk::Widget::set_margin_top, gtk::Widget::set_margin_bottom, gtk::Widget::set_margin_start, gtk::Widget::set_margin_end] {
            side(root.upcast_ref(), 14);
        }
        root.append(&top);
        root.append(&intro);
        root.append(&scroller);
        Rc::new(Self { root, list, on_change: RefCell::new(None), building: Cell::new(false), signature: RefCell::new(String::new()), rows: Cell::new(0), live: RefCell::new(Vec::new()) })
    }

    pub fn connect_changed(&self, f: impl Fn(DepthPick) + 'static) {
        *self.on_change.borrow_mut() = Some(Rc::new(f));
    }

    /// Quantas opções a lista tem agora (para o roteiro automático).
    pub fn rows_built(&self) -> usize {
        self.rows.get()
    }

    /// Mostra as candidatas e o pedido atual. Só refaz a lista se algo mudou e a página está na tela.
    pub fn set(self: &Rc<Self>, choices: &[DepthChoice], clears: &[DepthClear], pick: DepthPick, shown: bool) {
        if !shown {
            return;
        }
        // Só a estrutura decide refazer a lista (quais opções existem, quais servem, o pedido atual); os números dos textos mudam a todo
        // quadro e são atualizados no lugar.
        let structure: Vec<_> = choices.iter().map(|c| (c.number, c.automatic, c.unusable.clone(), c.blocking)).collect();
        let points: Vec<bool> = clears.iter().map(|c| c.splittable).collect();
        let signature = format!("{structure:?}|{points:?}|{pick:?}");
        if *self.signature.borrow() == signature {
            for update in self.live.borrow().iter() {
                update(choices, clears);
            }
            return;
        }
        *self.signature.borrow_mut() = signature;
        self.build(choices, clears, pick);
        for update in self.live.borrow().iter() {
            update(choices, clears);
        }
    }

    fn build(self: &Rc<Self>, choices: &[DepthChoice], clears: &[DepthClear], pick: DepthPick) {
        self.building.set(true);
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        self.live.borrow_mut().clear();
        let weak = Rc::downgrade(self);
        let mut rows = 0;

        // O automático e, sob ele, os pontos de captura.
        let automatic_active = matches!(pick, DepthPick::Auto | DepthPick::Clear(_) | DepthPick::Pass(_));
        let first = option(tr("Automatic (the layer chooses)"), Some(tr("Recommended. The layer picks the depth image and the moment of the frame to copy it.")), automatic_active, None, None, DepthPick::Auto, &weak);
        self.list.append(&first.0);
        rows += 1;
        if automatic_active {
            self.list.append(&capture_points(clears, pick, &weak, &mut rows, &self.live));
        }

        // Cada imagem de profundidade que a layer achou.
        for c in choices {
            let mut details = None;
            if c.automatic {
                details = Some(tr("The one the automatic choice would pick.").to_string());
            }
            if let Some(why) = &c.unusable {
                details = Some(fmt(if c.blocking { "Unusable: {}" } else { "May not give the expected result: {}" }, &[why]));
            }
            let row = option(&c.label, details.as_deref(), pick == DepthPick::Candidate(c.number), Some(&first.1), None, DepthPick::Candidate(c.number), &weak);
            if let Some(why) = &c.unusable {
                if c.blocking {
                    row.1.set_sensitive(false);
                    row.0.set_tooltip_text(Some(&fmt("The layer cannot use this image: {}", &[why])));
                } else {
                    row.0.add_css_class("warning");
                    row.0.set_tooltip_text(Some(&fmt("The automatic choice would not pick this image ({}). You can use it, but the result may not be the expected one.", &[why])));
                }
            }
            self.list.append(&row.0);
            rows += 1;
            let (button, number) = (row.1.clone(), c.number);
            self.live.borrow_mut().push(Box::new(move |choices, _| {
                if let Some(c) = choices.iter().find(|c| c.number == number)
                    && button.label().as_deref() != Some(c.label.as_str())
                {
                    button.set_label(Some(&c.label));
                }
            }));
        }
        self.rows.set(rows);
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

/// Uma opção: o botão de opção, com uma linha pequena embaixo. Devolve a caixa e o botão (para agrupar os seguintes).
fn option(title: &str, subtitle: Option<&str>, active: bool, group: Option<&gtk::CheckButton>, tooltip: Option<&str>, pick: DepthPick, weak: &Weak<DepthView>) -> (gtk::Box, gtk::CheckButton, Option<gtk::Label>) {
    let button = gtk::CheckButton::with_label(title);
    if let Some(g) = group {
        button.set_group(Some(g));
    }
    button.set_active(active);
    button.set_tooltip_text(tooltip);
    button.connect_toggled({
        let weak = weak.clone();
        move |c| {
            if c.is_active()
                && let Some(me) = weak.upgrade()
            {
                me.emit(pick);
            }
        }
    });
    let row = gtk::Box::new(gtk::Orientation::Vertical, 0);
    row.append(&button);
    let mut caption = None;
    if let Some(text) = subtitle {
        let label = gtk::Label::new(Some(text));
        label.set_wrap(true);
        label.set_xalign(0.0);
        label.set_margin_start(28);
        label.add_css_class("dim-label");
        label.add_css_class("caption");
        row.append(&label);
        caption = Some(label);
    }
    (row, button, caption)
}

/// Os pontos de captura sob o automático: automático, antes de cada limpeza (lista do jogo) e depois de um passe.
type Live = RefCell<Vec<Box<dyn Fn(&[DepthChoice], &[DepthClear])>>>;

fn capture_points(clears: &[DepthClear], pick: DepthPick, weak: &Weak<DepthView>, rows: &mut usize, live: &Live) -> gtk::Box {
    let section = gtk::Box::new(gtk::Orientation::Vertical, 4);
    section.set_margin_start(28);
    section.set_margin_bottom(6);
    let heading = gtk::Label::new(Some(tr("Capture point")));
    heading.add_css_class("heading");
    heading.set_xalign(0.0);
    section.append(&heading);

    // Cada ponto é um botão de opção de um grupo próprio (o do automático acima é outro).
    let (auto_point, group, _) = option(
        tr("Automatic"),
        Some(tr("The layer chooses the moment of the frame where the scene is complete (before the interface).")),
        pick == DepthPick::Auto,
        None,
        None,
        DepthPick::Auto,
        weak,
    );
    section.append(&auto_point);
    *rows += 1;

    let before = gtk::Label::new(Some(tr("Before a depth clear")));
    before.add_css_class("heading");
    before.set_xalign(0.0);
    before.set_margin_top(6);
    before.set_tooltip_text(Some(tr("Copies the depth right before the game erases it. Use it when the effects see an empty or wrong depth.")));
    section.append(&before);
    let note = gtk::Label::new(Some(tr("Copies the depth right before the game erases it. The first clear (start of the frame) is steady but one frame behind; later ones are more current.")));
    note.set_wrap(true);
    note.set_xalign(0.0);
    note.add_css_class("dim-label");
    note.add_css_class("caption");
    section.append(&note);
    if clears.is_empty() {
        section.append(&numbered(tr("Clear number"), matches!(pick, DepthPick::Clear(_)), match pick { DepthPick::Clear(n) => n, _ => 1 }, &group, weak, DepthPick::Clear));
        *rows += 1;
    }
    for (i, clear) in clears.iter().enumerate() {
        let n = i as u32 + 1;
        let title = fmt("Clear {}", &[&n]);
        let subtitle = clear_text(i, clear);
        let tip = if clear.splittable { None } else { Some(tr("The game clears here inside a pass with an open query, which the layer cannot split: it cannot copy at this point.")) };
        let row = option(&title, Some(&subtitle), pick == DepthPick::Clear(n), Some(&group), tip, DepthPick::Clear(n), weak);
        if !clear.splittable {
            row.1.set_sensitive(false);
        }
        section.append(&row.0);
        *rows += 1;
        if let Some(label) = row.2 {
            live.borrow_mut().push(Box::new(move |_, clears| {
                if let Some(clear) = clears.get(i) {
                    let text = clear_text(i, clear);
                    if label.text() != text {
                        label.set_text(&text);
                    }
                }
            }));
        }
    }

    let after = gtk::Label::new(Some(tr("After a pass")));
    after.add_css_class("heading");
    after.set_xalign(0.0);
    after.set_margin_top(6);
    after.set_tooltip_text(Some(tr("Advanced: copies the depth right after the Nth render pass that draws to it. The number of passes changes from game to game; try small numbers first.")));
    section.append(&after);
    section.append(&numbered(tr("Pass number"), matches!(pick, DepthPick::Pass(_)), match pick { DepthPick::Pass(n) => n, _ => 1 }, &group, weak, DepthPick::Pass));
    *rows += 1;
    section
}

/// O texto de uma limpeza: onde ela fica no quadro e, se a layer não consegue copiar ali, por quê (visível, não só na dica).
fn clear_text(index: usize, clear: &DepthClear) -> String {
    let place = if clear.draws_before == 0 && index == 0 { tr("Start of the frame: the depth the previous frame left.").to_string() } else { fmt("After {} more draws", &[&clear.draws_before]) };
    if clear.splittable {
        place
    } else {
        format!("{place} {}", tr("Unavailable: the game erases the depth inside a pass that has an open query, and the layer cannot split such a pass."))
    }
}

/// Uma opção com um número e o botão "Use".
fn numbered(label: &str, active: bool, current: u32, group: &gtk::CheckButton, weak: &Weak<DepthView>, make: fn(u32) -> DepthPick) -> gtk::Box {
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let option = gtk::CheckButton::with_label(label);
    option.set_group(Some(group));
    option.set_active(active);
    let spin = gtk::SpinButton::with_range(1.0, 400.0, 1.0);
    crate::icons::arrows(&spin);
    spin.set_value(f64::from(current));
    let apply = {
        let (weak, spin) = (weak.clone(), spin.clone());
        move || {
            if let Some(me) = weak.upgrade() {
                me.emit(make(spin.value() as u32));
            }
        }
    };
    option.connect_toggled({
        let apply = apply.clone();
        move |c| {
            if c.is_active() {
                apply();
            }
        }
    });
    let use_button = gtk::Button::with_label(tr("Use"));
    use_button.connect_clicked(move |_| apply());
    line.append(&option);
    line.append(&spin);
    line.append(&use_button);
    line
}
