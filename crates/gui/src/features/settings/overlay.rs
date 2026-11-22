//! Aba Overlay: o contador de desempenho e o ícone de processando que a layer desenha nos jogos.

use gtk::prelude::*;
use gtk4 as gtk;
use control_core::i18n::tr;
use super::widgets::*;

/// O contador de desempenho (FPS, média, mínimo, máximo e 1% low) que a layer desenha num canto dos jogos. Grava em `[OVERLAY]` do `config.ini`;
/// o daemon percebe a mudança e republica os planos sozinho.
pub(super) fn overlay_page() -> gtk::Box {
    let page = page_box();
    page.append(&hint(tr("A small counter drawn in a corner of the game: FPS now, average, minimum, maximum and 1% low of the last 5 seconds, and the frame time. It is added after the preset's effects. It needs the game to have a preset (or the global one).")));
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_wrap(true);
    let current = config::Config::load().overlay;
    let save = |key: &'static str, value: String, status: gtk::Label| {
        let Some(path) = config_file() else { return };
        match config::set_value(&path, "OVERLAY", key, &value) {
            Ok(()) => status.set_text(tr("Saved. It applies to the open games in a few seconds.")),
            Err(e) => status.set_text(&format!("{}: {e}", path.display())),
        }
    };

    let enabled = gtk::CheckButton::with_label(tr("Show the performance counter in games"));
    enabled.set_active(current.enabled);
    enabled.connect_toggled({
        let (status, save) = (status.clone(), save);
        move |b| save("Stats", if b.is_active() { "1" } else { "0" }.to_string(), status.clone())
    });
    page.append(&enabled);

    let details = gtk::CheckButton::with_label(tr("Also show average, minimum, maximum and 1% low"));
    details.set_active(current.details);
    details.connect_toggled({
        let (status, save) = (status.clone(), save);
        move |b| save("Details", if b.is_active() { "1" } else { "0" }.to_string(), status.clone())
    });
    page.append(&details);

    let busy = gtk::CheckButton::with_label(tr("Show a processing icon when effects are being compiled or rebuilt"));
    busy.set_tooltip_text(Some(tr("A small spinner in the corner while the program compiles or rebuilds effects for the game (when a definition changes, for example). It works even with the counter off.")));
    busy.set_active(current.busy);
    busy.connect_toggled({
        let (status, save) = (status.clone(), save);
        move |b| save("Busy", if b.is_active() { "1" } else { "0" }.to_string(), status.clone())
    });
    page.append(&busy);

    let corner_line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    corner_line.append(&gtk::Label::new(Some(tr("Corner"))));
    let corner = gtk::DropDown::from_strings(&[tr("Top left"), tr("Top right"), tr("Bottom left"), tr("Bottom right")]);
    corner.set_selected(u32::from(current.corner));
    corner.connect_selected_notify({
        let (status, save) = (status.clone(), save);
        move |d| save("Corner", d.selected().to_string(), status.clone())
    });
    corner_line.append(&corner);
    corner_line.append(&gtk::Label::new(Some(tr("Size"))));
    let scale = gtk::SpinButton::with_range(1.0, 8.0, 1.0);
    crate::shared::icons::arrows(&scale);
    scale.set_value(f64::from(current.scale));
    scale.connect_value_changed({
        let (status, save) = (status.clone(), save);
        move |s| save("Scale", (s.value() as u32).to_string(), status.clone())
    });
    corner_line.append(&scale);
    page.append(&corner_line);
    page.append(&status);
    page
}
