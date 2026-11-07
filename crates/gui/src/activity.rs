//! Indicador de trabalho em segundo plano no cabeçalho: um spinner e o texto "Compilando: ..." enquanto o daemon ou o painel compilam efeitos.
//! Sem isso, trocar uma definição que precisa de compilação parece não fazer nada por alguns segundos.
//! Componente: recebe os nomes por `set` e não conhece a janela.

use control_core::i18n::{fmt, tr};
use gtk4 as gtk;
use gtk::prelude::*;
use std::rc::Rc;

pub struct Activity {
    pub root: gtk::Box,
    spinner: gtk::Spinner,
    label: gtk::Label,
}

impl Activity {
    pub fn new() -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let spinner = gtk::Spinner::new();
        let label = gtk::Label::new(None);
        label.add_css_class("dim-label");
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_max_width_chars(40);
        root.append(&spinner);
        root.append(&label);
        root.set_visible(false);
        Rc::new(Self { root, spinner, label })
    }

    /// Os efeitos em compilação agora (vazio: esconde o indicador).
    pub fn set(&self, names: &[String]) {
        if names.is_empty() {
            self.spinner.stop();
            self.root.set_visible(false);
            return;
        }
        let text = match names {
            [one] => fmt("Compiling: {}", &[one]),
            [first, rest @ ..] => fmt("Compiling: {} (+{})", &[first, &rest.len()]),
            [] => String::new(),
        };
        self.label.set_text(&text);
        self.label.set_tooltip_text(Some(tr("Effects are being compiled in the background; the change applies when they finish")));
        self.spinner.start();
        self.root.set_visible(true);
    }
}

/// Os efeitos que estão sendo compilados: uma pasta de trabalho por bundle em `<cache>/building` (`<efeito>[.m<hash>]-<L>x<A>.<pid>`).
pub fn compiling(cache: &std::path::Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(cache.join("building")) else { return Vec::new() };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            // Pasta de um processo que já morreu (travou ou foi encerrado no meio): não é compilação em andamento.
            let (head, pid) = name.rsplit_once('.')?;
            if !pid.parse::<u32>().is_ok_and(|p| std::path::Path::new(&format!("/proc/{p}")).exists()) {
                return None;
            }
            let name = head.to_string();
            let name = name.rsplit_once('-').map_or(name.as_str(), |(head, _size)| head).to_string();
            let name = name.split(".m").next().unwrap_or(&name).to_string();
            (!name.is_empty()).then_some(name)
        })
        .collect();
    names.sort();
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    #[test]
    fn names_lose_pid_size_and_variant() {
        let dir = std::env::temp_dir().join(format!("lr-activity-{}", std::process::id()));
        let building = dir.join("building");
        let me = std::process::id();
        std::fs::create_dir_all(building.join(format!("MartysMods_SMAA.mfa2e75dc-800x600.{me}"))).unwrap();
        std::fs::create_dir_all(building.join(format!("Sepia-400x300.{me}"))).unwrap();
        std::fs::create_dir_all(building.join("Morto-400x300.4294967")).unwrap();
        assert_eq!(super::compiling(&dir), vec!["MartysMods_SMAA".to_string(), "Sepia".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
