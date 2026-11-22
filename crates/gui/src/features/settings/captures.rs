//! Aba Capturas de tela.

use gtk::prelude::*;
use gtk4 as gtk;
use control_core::i18n::tr;
use super::widgets::*;

pub(super) fn captures_page(window: &gtk::Window) -> gtk::Box {
    let page = page_box();
    page.append(&hint(tr("The capture key is in the Shortcuts tab. Each game with the layer open saves its own frame.")));
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_wrap(true);
    let default = config::Config::load().screenshot_dir.to_string_lossy().into_owned();
    folder_row(&page, window, tr("Captures folder"), tr("Each game gets a subfolder; files carry the date and time."), ini_text("SCREENSHOT", "SavePath", &default), false, |text| {
        let path = config_file().ok_or(tr("no data folder: run `lrctl config init`"))?;
        config::set_value(&path, "SCREENSHOT", "SavePath", text).map_err(|e| format!("{}: {e}", path.display()))
    }, &status);
    let both = gtk::CheckButton::with_label(tr("Also save the frame without effects (to compare), ending in \"_no-effects\""));
    both.set_active(config::Config::load().screenshot_before);
    {
        let status = status.clone();
        both.connect_toggled(move |b| {
            let Some(path) = config_file() else { return };
            match config::set_value(&path, "SCREENSHOT", "SaveBeforeShot", if b.is_active() { "1" } else { "0" }) {
                Ok(()) => status.set_text(tr("Saved.")),
                Err(e) => status.set_text(&format!("{}: {e}", path.display())),
            }
        });
    }
    page.append(&both);
    page.append(&status);
    page
}
