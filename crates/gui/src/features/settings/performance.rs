//! Aba Desempenho.

use gtk::prelude::*;
use gtk4 as gtk;
use control_core::i18n::tr;
use super::widgets::*;

pub(super) fn performance_page(window: &gtk::Window) -> gtk::Box {
    let page = page_box();
    page.append(&hint(tr("Measures how much the layer costs per frame on the CPU and how much the GPU spends on each effect. Measuring costs a little; turn it on only to look.")));
    let toggle = gtk::CheckButton::with_label(tr("Measure performance"));
    toggle.set_active(ipc::stats_on());
    toggle.connect_toggled(|b| {
        let _ = ipc::set_stats_on(b.is_active());
    });
    let text = gtk::Label::new(None);
    text.set_xalign(0.0);
    text.set_yalign(0.0);
    text.set_selectable(true);
    no_autoselect(&text);
    text.add_css_class("monospace");
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_child(Some(&text));
    scroller.set_vexpand(true);
    page.append(&toggle);
    page.append(&scroller);
    let refresh = {
        let text = text.clone();
        move || {
            let layers = ipc::live_layers().unwrap_or_default();
            let mut out = String::new();
            for l in &layers {
                let name = if l.status.app.is_empty() { l.id.clone() } else { l.status.app.clone() };
                match &l.status.perf {
                    Some(perf) => out.push_str(&format!("{name}\n{}\n", control_core::layers::perf_text(perf))),
                    None => out.push_str(&format!("{name}\n  ({}{})\n\n", tr("no measurement"), if ipc::stats_on() { tr(": waiting for the next second") } else { tr(": turn the measurement on above") })),
                }
            }
            if layers.is_empty() {
                out = tr("No game with the layer open.").to_string();
            }
            text.set_text(&out);
        }
    };
    refresh();
    let window = window.clone();
    gtk::glib::timeout_add_seconds_local(1, move || {
        if !window.is_visible() {
            return gtk::glib::ControlFlow::Break;
        }
        refresh();
        gtk::glib::ControlFlow::Continue
    });
    page
}
