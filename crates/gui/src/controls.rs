//! Um widget para cada tipo de controle do efeito, e a atualização dos valores sem disparar as ações.

use gtk4 as gtk;
use gtk::prelude::*;
use panel::{Control, Kind};
use std::cell::Cell;
use std::rc::Rc;

/// Chamado quando o usuário muda um controle: `(nome, valores)`.
pub type OnChange = Rc<dyn Fn(&str, &[f64])>;

/// Os widgets de um controle.
enum Inputs {
    Scales(Vec<gtk::Scale>),
    Switch(gtk::Switch),
    Combo(gtk::DropDown),
    Color { button: gtk::ColorDialogButton, alpha: bool },
    Spins(Vec<gtk::SpinButton>),
}

struct Row {
    inputs: Inputs,
    reset: gtk::Button,
}

/// A roda do mouse sobre um controle rola a tela, em vez de mexer no controle (um slider que pula quando se tenta
/// rolar a lista é irritante). O evento é capturado antes do controle e repassado à lista que rola.
pub fn forward_scroll(widget: &impl IsA<gtk::Widget>, scroller: &gtk::ScrolledWindow) {
    let controller = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    let scroller = scroller.clone();
    controller.connect_scroll(move |_, _dx, dy| {
        let adj = scroller.vadjustment();
        let step = adj.page_size().powf(2.0 / 3.0).max(24.0);
        let top = (adj.upper() - adj.page_size()).max(adj.lower());
        adj.set_value((adj.value() + dy * step).clamp(adj.lower(), top));
        gtk::glib::Propagation::Stop
    });
    widget.add_controller(controller);
}

/// A coluna de controles do efeito escolhido.
pub struct ControlsView {
    pub root: gtk::Box,
    /// A lista que rola; a roda do mouse sobre um controle é repassada a ela.
    scroller: gtk::ScrolledWindow,
    rows: Vec<Row>,
    /// Descreve os controles que a coluna mostra (nomes e tipos): se mudar, a coluna é refeita.
    signature: Vec<(String, Kind)>,
    on_change: OnChange,
    /// Verdadeiro enquanto a interface atualiza os widgets a partir do modelo (para não tratar isso como ação do usuário).
    updating: Rc<Cell<bool>>,
}

const EPSILON: f64 = 1e-5;

impl ControlsView {
    pub fn new(on_change: OnChange, scroller: gtk::ScrolledWindow) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
        Self { root, scroller, rows: Vec::new(), signature: Vec::new(), on_change, updating: Rc::new(Cell::new(false)) }
    }

    /// Mostra `controls`: refaz a coluna se os controles mudaram, senão só atualiza os valores.
    pub fn show(&mut self, controls: &[Control]) {
        let signature: Vec<_> = controls.iter().map(|c| (c.name.clone(), c.kind.clone())).collect();
        if signature != self.signature {
            self.rebuild(controls);
            self.signature = signature;
        }
        self.updating.set(true);
        for (row, control) in self.rows.iter().zip(controls) {
            set_values(&row.inputs, &control.value);
            row.reset.set_sensitive(!control.is_default());
        }
        self.updating.set(false);
    }

    /// Para o teste de fumaça: as escalas da primeira linha (se a primeira linha tem escalas).
    pub fn first_scales(&self) -> Vec<gtk::Scale> {
        match self.rows.first().map(|r| &r.inputs) {
            Some(Inputs::Scales(s)) => s.clone(),
            _ => Vec::new(),
        }
    }

    fn rebuild(&mut self, controls: &[Control]) {
        while let Some(child) = self.root.first_child() {
            self.root.remove(&child);
        }
        self.rows.clear();
        let mut category: Option<String> = None;
        for control in controls {
            if control.category != category {
                category = control.category.clone();
                if let Some(title) = &category {
                    let heading = gtk::Label::new(Some(title));
                    heading.set_halign(gtk::Align::Start);
                    heading.add_css_class("heading");
                    self.root.append(&heading);
                }
            }
            let row = self.build_row(control);
            self.root.append(&row.0);
            self.rows.push(row.1);
        }
    }

    /// Uma linha: o rótulo em cima, o controle e o botão de restaurar embaixo.
    fn build_row(&self, control: &Control) -> (gtk::Box, Row) {
        let outer = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let label = gtk::Label::new(Some(&control.label));
        label.set_halign(gtk::Align::Start);
        if let Some(tip) = &control.tooltip {
            outer.set_tooltip_text(Some(tip));
        }
        outer.append(&label);

        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let (inputs, widgets) = self.build_inputs(control);
        for w in &widgets {
            line.append(w);
        }
        let reset = gtk::Button::from_icon_name("edit-undo-symbolic");
        reset.set_tooltip_text(Some("Restaurar o valor padrão"));
        reset.set_valign(gtk::Align::Center);
        let (name, default, on_change) = (control.name.clone(), control.default.clone(), self.on_change.clone());
        reset.connect_clicked(move |_| on_change(&name, &default));
        line.append(&reset);
        outer.append(&line);
        (outer, Row { inputs, reset })
    }

    /// A ação de um widget do controle: coleta os valores da linha e avisa a quem usa a interface.
    fn notifier<W>(&self, control: &Control, collect: Box<dyn Fn() -> Vec<f64>>) -> impl Fn(&W) + 'static {
        notify_change(&control.name, &self.updating, &self.on_change, collect)
    }

    /// Cria o(s) widget(s) do controle e liga cada um à ação de mudar o valor.
    fn build_inputs(&self, control: &Control) -> (Inputs, Vec<gtk::Widget>) {
        let n = control.components;
        match &control.kind {
            Kind::Switch => {
                let sw = gtk::Switch::new();
                sw.set_valign(gtk::Align::Center);
                let this = sw.clone();
                sw.connect_active_notify(self.notifier(control, Box::new(move || vec![if this.is_active() { 1.0 } else { 0.0 }])));
                let widget = sw.clone().upcast();
                (Inputs::Switch(sw), vec![widget])
            }
            Kind::Slider { min, max, step } => {
                // O valor fica numa caixa de número ao lado (digitável) ligada ao mesmo ajuste da barra, com folga para o botão
                // da barra não passar por cima do número.
                let digits = digits(*step);
                let pairs: Vec<(gtk::Scale, gtk::SpinButton)> = (0..n)
                    .map(|_| {
                        let adjustment = gtk::Adjustment::new(*min, *min, *max, *step, *step * 10.0, 0.0);
                        let scale = gtk::Scale::new(gtk::Orientation::Horizontal, Some(&adjustment));
                        scale.set_hexpand(true);
                        scale.set_draw_value(false);
                        let spin = gtk::SpinButton::new(Some(&adjustment), *step, digits);
                        spin.set_numeric(true);
                        spin.set_width_chars(8);
                        spin.set_valign(gtk::Align::Center);
                        (scale, spin)
                    })
                    .collect();
                let scales: Vec<gtk::Scale> = pairs.iter().map(|(s, _)| s.clone()).collect();
                for (scale, spin) in &pairs {
                    forward_scroll(scale, &self.scroller);
                    forward_scroll(spin, &self.scroller);
                    let all = scales.clone();
                    scale.connect_value_changed(self.notifier(control, Box::new(move || all.iter().map(|s| s.value()).collect())));
                }
                let widgets = pairs
                    .iter()
                    .map(|(scale, spin)| {
                        let pair = gtk::Box::new(gtk::Orientation::Horizontal, 14);
                        pair.set_hexpand(true);
                        pair.append(scale);
                        pair.append(spin);
                        pair.upcast()
                    })
                    .collect();
                (Inputs::Scales(scales), widgets)
            }
            Kind::Combo { items } => {
                let refs: Vec<&str> = items.iter().map(String::as_str).collect();
                let dd = gtk::DropDown::from_strings(&refs);
                dd.set_hexpand(true);
                forward_scroll(&dd, &self.scroller);
                let this = dd.clone();
                dd.connect_selected_notify(self.notifier(control, Box::new(move || vec![f64::from(this.selected())])));
                let widget = dd.clone().upcast();
                (Inputs::Combo(dd), vec![widget])
            }
            Kind::Color { alpha } => {
                let dialog = gtk::ColorDialog::new();
                dialog.set_with_alpha(*alpha);
                let button = gtk::ColorDialogButton::new(Some(dialog));
                button.set_hexpand(true);
                let (this, alpha) = (button.clone(), *alpha);
                button.connect_rgba_notify(self.notifier(control, Box::new(move || {
                    let c = this.rgba();
                    let mut v = vec![f64::from(c.red()), f64::from(c.green()), f64::from(c.blue())];
                    if alpha {
                        v.push(f64::from(c.alpha()));
                    }
                    v
                })));
                let widget = button.clone().upcast();
                (Inputs::Color { button, alpha }, vec![widget])
            }
            Kind::Number { min, max, step } => {
                let spins: Vec<gtk::SpinButton> = (0..n)
                    .map(|_| {
                        let s = gtk::SpinButton::with_range(min.unwrap_or(-1.0e9), max.unwrap_or(1.0e9), *step);
                        s.set_digits(digits(*step));
                        s.set_hexpand(true);
                        s
                    })
                    .collect();
                for spin in &spins {
                    forward_scroll(spin, &self.scroller);
                    let all = spins.clone();
                    spin.connect_value_changed(self.notifier(control, Box::new(move || all.iter().map(|s| s.value()).collect())));
                }
                let widgets = spins.iter().map(|s| s.clone().upcast()).collect();
                (Inputs::Spins(spins), widgets)
            }
        }
    }
}

/// A ação de um widget: quando o usuário o muda, coleta os valores da linha e avisa. Ignora as mudanças feitas
/// pela própria interface ao atualizar os widgets a partir do modelo (`updating`).
fn notify_change<W>(
    name: &str,
    updating: &Rc<Cell<bool>>,
    on_change: &OnChange,
    collect: Box<dyn Fn() -> Vec<f64>>,
) -> impl Fn(&W) + 'static {
    let (name, updating, on_change) = (name.to_string(), updating.clone(), on_change.clone());
    move |_| {
        if !updating.get() {
            on_change(&name, &collect());
        }
    }
}

/// Casas decimais para mostrar o passo (0,01 -> 2; 1 -> 0).
fn digits(step: f64) -> u32 {
    (0..=6).find(|d| (step * 10f64.powi(*d)).fract().abs() < 1e-9).unwrap_or(6) as u32
}

/// Põe os valores do modelo nos widgets, pulando os que já estão certos (assim não se briga com um arrasto em curso).
fn set_values(inputs: &Inputs, value: &[f64]) {
    match inputs {
        Inputs::Scales(scales) => {
            for (scale, v) in scales.iter().zip(value) {
                if (scale.value() - v).abs() > EPSILON {
                    scale.set_value(*v);
                }
            }
        }
        Inputs::Switch(sw) => {
            let on = value.first().is_some_and(|v| *v != 0.0);
            if sw.is_active() != on {
                sw.set_active(on);
            }
        }
        Inputs::Combo(dd) => {
            let index = value.first().copied().unwrap_or(0.0).max(0.0) as u32;
            if dd.selected() != index {
                dd.set_selected(index);
            }
        }
        Inputs::Color { button, alpha } => {
            let c = |i: usize| value.get(i).copied().unwrap_or(0.0) as f32;
            let a = if *alpha { c(3) } else { 1.0 };
            let current = button.rgba();
            if (current.red() - c(0)).abs() > 1e-4 || (current.green() - c(1)).abs() > 1e-4 || (current.blue() - c(2)).abs() > 1e-4 || (current.alpha() - a).abs() > 1e-4 {
                button.set_rgba(&gtk::gdk::RGBA::new(c(0), c(1), c(2), a));
            }
        }
        Inputs::Spins(spins) => {
            for (spin, v) in spins.iter().zip(value) {
                if (spin.value() - v).abs() > EPSILON {
                    spin.set_value(*v);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::digits;

    #[test]
    fn the_number_of_decimals_follows_the_step() {
        assert_eq!(digits(1.0), 0);
        assert_eq!(digits(0.1), 1);
        assert_eq!(digits(0.01), 2);
        assert_eq!(digits(0.25), 2);
        assert_eq!(digits(0.0001), 4);
    }
}
