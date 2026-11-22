//! Aba Cache: o que está compilado e analisado, e como limpar.

use gtk::prelude::*;
use gtk4 as gtk;
use std::cell::RefCell;
use std::rc::Rc;
use control_core::i18n::{fmt, tr};
use super::widgets::*;

/// Quanto ocupa o cache dos efeitos compilados, em texto.
pub(super) fn cache_usage_text(compiler: &control_core::compiler::Compiler) -> String {
    let u = control_core::maintenance::usage(compiler);
    format!("{}\n{}", compiler.cache.display(), fmt("{} compiled effects, {} analyses, {} MB", &[&u.bundles, &u.analyses, &format!("{:.1}", u.bytes as f64 / 1e6)]))
}

/// Os nomes dos efeitos do preset global (sem repetir), para medir a compilação deles.
pub(super) fn global_preset_effects() -> Vec<String> {
    let Some(dir) = config::Config::load().presets_dir else { return Vec::new() };
    let Ok(preset) = control_core::presets::load(&dir.join("preset.ini")) else { return Vec::new() };
    let mut names: Vec<String> = Vec::new();
    for row in control_core::presets::rows(&preset) {
        if let Some(file) = row.id.file {
            let name = file.trim_end_matches(".fx").to_string();
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

pub(super) fn cache_page() -> gtk::Box {
    use control_core::maintenance::{self, Clear};
    let page = page_box();
    page.append(&hint(tr("Compiled effects and compatibility analyses are kept. Forced clearing is for testing compilation from scratch; whatever an open game uses is recompiled right after.")));
    let compiler = Rc::new(control_core::compiler::Compiler::from_env());
    let usage = gtk::Label::new(None);
    usage.set_xalign(0.0);
    usage.set_selectable(true);
    no_autoselect(&usage);
    let refresh_usage = {
        let (usage, compiler) = (usage.clone(), compiler.clone());
        move || usage.set_text(&match &*compiler {
            Ok(c) => cache_usage_text(c),
            Err(e) => fmt("No compiler: {}", &[&e]),
        })
    };
    refresh_usage();
    page.append(&usage);

    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_wrap(true);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    for (label, what, tip) in [
        (tr("Clear compiled"), Clear::Bundles, tr("Deletes the compiled effects (the analyses stay)")),
        (tr("Clear analyses"), Clear::Analyses, tr("Deletes the compatibility analyses; the panel redoes them in the background")),
        (tr("Clear everything"), Clear::All, tr("Deletes the compiled effects and the analyses")),
    ] {
        let button = gtk::Button::with_label(label);
        button.set_tooltip_text(Some(tip));
        let (compiler, status, refresh_usage) = (compiler.clone(), status.clone(), refresh_usage.clone());
        let asked: std::cell::Cell<Option<std::time::Instant>> = std::cell::Cell::new(None);
        button.connect_clicked(move |_| {
            if let Ok(c) = &*compiler {
                // Com um jogo aberto, apagar os compilados o deixa sem efeito até o daemon recompilar (e uma troca nessa hora é onde mais
                // se mexe): pede um segundo clique em até 8 s.
                let games = ipc::live_layers().map(|l| l.len()).unwrap_or(0);
                if games > 0 && !matches!(what, Clear::Analyses) {
                    let confirmed = asked.get().is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(8));
                    if !confirmed {
                        asked.set(Some(std::time::Instant::now()));
                        status.set_text(&fmt("{} game(s) with the layer are open: its effects go away until they are compiled again. Click again to clear anyway.", &[&games]));
                        return;
                    }
                    asked.set(None);
                }
                let n = maintenance::clear(c, what);
                status.set_text(&fmt("{} items deleted.", &[&n]));
                refresh_usage();
            }
        });
        buttons.append(&button);
    }
    page.append(&buttons);

    page.append(&hint(tr("Measuring compilation deletes the compiled versions of the global preset's effects and compiles them again, one at a time (1920x1080, no macros), timing each one.")));
    let measure = gtk::Button::with_label(tr("Recompile the global preset and measure"));
    measure.set_halign(gtk::Align::Start);
    let report = gtk::Label::new(None);
    report.set_xalign(0.0);
    report.set_yalign(0.0);
    report.set_selectable(true);
    no_autoselect(&report);
    report.add_css_class("monospace");
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_child(Some(&report));
    scroller.set_vexpand(true);
    {
        let (compiler, status, report, refresh_usage, measure_button) = (compiler.clone(), status.clone(), report.clone(), refresh_usage.clone(), measure.clone());
        measure.connect_clicked(move |_| {
            if compiler.is_err() {
                return;
            }
            let effects = global_preset_effects();
            if effects.is_empty() {
                status.set_text(tr("The global preset has no effects."));
                return;
            }
            measure_button.set_sensitive(false);
            status.set_text(&fmt("Compiling {} effects…", &[&effects.len()]));
            report.set_text("");
            let total = effects.len();
            let (tx, rx) = std::sync::mpsc::channel::<maintenance::Timing>();
            let worker = control_core::compiler::Compiler::from_env();
            std::thread::spawn(move || {
                let Ok(worker) = worker else { return };
                maintenance::time_compiles(&worker, &effects, |t| {
                    let _ = tx.send(t.clone());
                });
            });
            let done: Rc<RefCell<Vec<maintenance::Timing>>> = Rc::new(RefCell::new(Vec::new()));
            let (status, report, refresh_usage, measure_button) = (status.clone(), report.clone(), refresh_usage.clone(), measure_button.clone());
            gtk::glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
                let mut finished = false;
                loop {
                    match rx.try_recv() {
                        Ok(t) => done.borrow_mut().push(t),
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            finished = true;
                            break;
                        }
                    }
                }
                let n = done.borrow().len();
                if finished || n >= total {
                    report.set_text(&maintenance::report(&done.borrow()));
                    status.set_text(tr("Done."));
                    measure_button.set_sensitive(true);
                    refresh_usage();
                    return gtk::glib::ControlFlow::Break;
                }
                status.set_text(&fmt("Compiling… {}/{}", &[&n, &total]));
                gtk::glib::ControlFlow::Continue
            });
        });
    }
    page.append(&measure);
    page.append(&status);
    page.append(&scroller);
    page
}
