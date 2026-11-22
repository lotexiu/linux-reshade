//! Aba Sobre.

use gtk::prelude::*;
use gtk4 as gtk;
use control_core::i18n::{fmt, tr};
use super::widgets::*;

pub(super) fn about_page() -> gtk::Box {
    let page = page_box();
    let config = config::Config::load();
    let show = |p: Option<&std::path::Path>| p.map_or(tr("(not configured)").to_string(), |p| p.display().to_string());
    let text = fmt(
        "linux-reshade {}\nReShade-compatible Vulkan layer for Linux.\n\nData folder: {}\nconfig.ini: {}\nPresets: {}\nCompiled effects cache: {}\nScreenshots: {}\nShared state (memory): {}\n\nReShade options and what is missing: docs/usuario/reshade-opcoes.md",
        &[
            &env!("CARGO_PKG_VERSION"),
            &show(config.data_dir.as_deref()),
            &show(config.config_path().as_deref()),
            &show(config.presets_dir.as_deref()),
            &config.cache_dir.display(),
            &config.screenshot_dir.display(),
            &ipc::state_dir().display(),
        ],
    );
    let label = gtk::Label::new(Some(&text));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_selectable(true);
    no_autoselect(&label);
    page.append(&label);
    page
}
