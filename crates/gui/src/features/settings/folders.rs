//! Aba Pastas: efeitos, texturas, presets e o cache dos compilados.

use gtk::prelude::*;
use gtk4 as gtk;
use control_core::i18n::tr;
use super::widgets::*;

pub(super) fn folders_page(window: &gtk::Window) -> gtk::Box {
    let page = page_box();
    page.append(&hint(tr("The folders are kept in config.ini. An entry ending in /** includes the subfolders. Relative paths (./) count from the config.ini folder.")));
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_wrap(true);
    let list = |section: &'static str, key: &'static str| {
        move |text: &str| -> Result<(), String> {
            let path = config_file().ok_or(tr("no data folder: run `lrctl config init`"))?;
            let items: Vec<String> = text.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            config::set_list(&path, section, key, &items).map_err(|e| format!("{}: {e}", path.display()))
        }
    };
    let single = |section: &'static str, key: &'static str| {
        move |text: &str| -> Result<(), String> {
            let path = config_file().ok_or(tr("no data folder: run `lrctl config init`"))?;
            config::set_value(&path, section, key, text).map_err(|e| format!("{}: {e}", path.display()))
        }
    };
    folder_row(&page, window, tr("Effects (.fx)"), tr("Folders where effects are searched for (the first with the name wins)."), ini_list("GENERAL", "EffectSearchPaths", "./Shaders/**"), true, list("GENERAL", "EffectSearchPaths"), &status);
    folder_row(&page, window, tr("Effect textures"), tr("Folders of the images the effects load (LUTs, noise...)."), ini_list("GENERAL", "TextureSearchPaths", "./Textures/**"), true, list("GENERAL", "TextureSearchPaths"), &status);
    folder_row(&page, window, tr("Presets"), tr("Where the global preset and each game's folder live."), ini_text("GENERAL", "PresetsPath", "./Presets"), false, single("GENERAL", "PresetsPath"), &status);
    folder_row(&page, window, tr("Compiled effects (cache)"), tr("Where the compiled effects are kept; it can be deleted freely."), ini_text("GENERAL", "IntermediateCachePath", &config::Config::load().cache_dir.to_string_lossy()), false, single("GENERAL", "IntermediateCachePath"), &status);
    let all = gtk::CheckButton::with_label(tr("Analyze every effect in the background (otherwise only those used by a preset)"));
    all.set_tooltip_text(Some(tr("The analysis compiles each effect once to tell which ones are compatible. With hundreds of effects that takes minutes of CPU; the ones outside presets are analyzed when you pick them.")));
    all.set_active(config::Config::load().analyze_all);
    {
        let status = status.clone();
        all.connect_toggled(move |b| {
            let Some(path) = config_file() else { return };
            match config::set_value(&path, "GENERAL", "AnalyzeAllEffects", if b.is_active() { "1" } else { "0" }) {
                Ok(()) => status.set_text(tr("Saved.")),
                Err(e) => status.set_text(&format!("{}: {e}", path.display())),
            }
        });
    }
    page.append(&all);
    folder_row(&page, window, tr("Skip in the background analysis"), tr("Effects in these folders are not analyzed ahead of time (a preset that uses them still compiles them)."), ini_list("GENERAL", "AnalysisExclude", ""), true, list("GENERAL", "AnalysisExclude"), &status);
    page.append(&status);
    page
}
