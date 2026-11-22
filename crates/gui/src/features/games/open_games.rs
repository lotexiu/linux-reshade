//! A lista de jogos com a layer ativa.

use control_core::layers::{effect_text, format_name};
use gtk4 as gtk;
use gtk::prelude::*;
use ipc::{EffectState, LiveLayer};
use control_core::i18n::tr;

pub struct GamesView {
    pub root: gtk::Box,
    /// O texto que a lista mostra hoje, para só refazê-la quando muda.
    shown: String,
}

impl GamesView {
    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let view = Self { root, shown: String::from("\u{0}") };
        view
    }

    pub fn show(&mut self, layers: &[LiveLayer]) {
        let key = format!("{layers:?}");
        if key == self.shown {
            return;
        }
        self.shown = key;
        while let Some(child) = self.root.first_child() {
            self.root.remove(&child);
        }
        let rows: Vec<_> = layers.iter().flat_map(|l| l.status.swapchains.iter().map(move |s| (l, s))).collect();
        if rows.is_empty() {
            let empty = gtk::Label::new(Some(tr("No game with the layer active.\nOpen a game with LINUX_RESHADE=1 in its launch options.")));
            empty.set_halign(gtk::Align::Start);
            empty.add_css_class("dim-label");
            self.root.append(&empty);
            return;
        }
        for (layer, swapchain) in rows {
            let app = if layer.status.app.is_empty() { layer.id.clone() } else { layer.status.app.clone() };
            let row = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let title = gtk::Label::new(Some(&app));
            title.set_halign(gtk::Align::Start);
            title.add_css_class("heading");
            let detail = gtk::Label::new(Some(&format!(
                "{}x{} · {} · {}",
                swapchain.width,
                swapchain.height,
                format_name(swapchain.format),
                effect_text(&swapchain.effect)
            )));
            detail.set_halign(gtk::Align::Start);
            detail.set_wrap(true);
            match swapchain.effect {
                EffectState::Ready => detail.add_css_class("success"),
                EffectState::Failed(_) => detail.add_css_class("error"),
                EffectState::Unavailable(_) | EffectState::Waiting => detail.add_css_class("warning"),
                EffectState::Idle => detail.add_css_class("dim-label"),
            }
            row.append(&title);
            row.append(&detail);
            self.root.append(&row);
        }
    }
}
