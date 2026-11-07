//! Setas dos campos numéricos (`GtkSpinButton`). Alguns temas de ícones (o `Tela`, por exemplo) trazem `value-increase-symbolic` como um "+"
//! de cor fixa cinza-escura dentro do SVG, que some sobre o fundo escuro. As setas próprias seguem a cor do texto: ficam gravadas numa pasta
//! de dados do programa (estrutura do tema `hicolor`, que o GTK sempre consulta) e são postas nos botões de cada campo por [`arrows`].

use gtk4 as gtk;
use gtk::prelude::*;

const UP: &str = "linux-reshade-arrow-up-symbolic";
const DOWN: &str = "linux-reshade-arrow-down-symbolic";

const ICONS: [(&str, &str); 2] = [
    ("linux-reshade-arrow-up-symbolic.svg", include_str!("../../../packaging/icons/linux-reshade-arrow-up-symbolic.svg")),
    ("linux-reshade-arrow-down-symbolic.svg", include_str!("../../../packaging/icons/linux-reshade-arrow-down-symbolic.svg")),
];

fn base() -> Option<std::path::PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME").map(std::path::PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share")))?;
    Some(data.join(config::LOCATION_DIR).join("icons"))
}

/// Grava os ícones (se faltam ou mudaram) e põe a pasta no caminho de busca do GTK.
pub fn install() {
    let Some(base) = base() else { return };
    let dir = base.join("hicolor/scalable/actions");
    if std::fs::create_dir_all(&dir).is_ok() {
        for (name, svg) in ICONS {
            let path = dir.join(name);
            if std::fs::read_to_string(&path).ok().as_deref() != Some(svg) {
                let _ = std::fs::write(&path, svg);
            }
        }
    }
    let _ = control_core::install::ensure_icon();
    gtk::Window::set_default_icon_name("linux-reshade");
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::IconTheme::for_display(&display).add_search_path(&base);
    }
}

/// Troca os ícones dos botões do campo numérico pelas setas próprias (para cima e para baixo).
pub fn arrows(spin: &gtk::SpinButton) {
    let mut child = spin.first_child();
    while let Some(c) = child {
        if let Some(button) = c.downcast_ref::<gtk::Button>()
            && let Some(image) = button.first_child().and_downcast::<gtk::Image>()
        {
            image.set_icon_name(Some(if button.has_css_class("up") { UP } else { DOWN }));
        }
        child = c.next_sibling();
    }
}

/// Os ícones embutidos estão disponíveis no tema em uso?
pub fn available() -> Vec<(&'static str, bool)> {
    let Some(display) = gtk::gdk::Display::default() else { return Vec::new() };
    let theme = gtk::IconTheme::for_display(&display);
    [UP, DOWN].into_iter().map(|name| (name, theme.has_icon(name))).collect()
}
