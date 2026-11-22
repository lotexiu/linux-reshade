//! A janela, no estilo da aba Home do ReShade: jogo e preset no alto, busca e "adicionar efeito", e a lista de
//! técnicas em ordem de execução, cada uma com caixa de seleção, setas, remover e os parâmetros do efeito.
//! Nenhuma regra de negócio mora aqui; tudo o que se decide está em `panel::Home`.

use crate::features::effects::add_effect::AddEffectView;
use crate::features::effects::effect_row::{EffectRow, RowCtx};
use crate::features::flatpak::flatpak_view::FlatpakView;
use crate::features::games::open_games::GamesView;
use gtk4 as gtk;
use crate::features::depth::depth_view::DepthView;
use control_core::i18n::{fmt, tr};
use crate::shared::macros::MacroEditor;
use gtk::glib;
use gtk::prelude::*;
use panel::{EffectGroup, Home};
use preset::TechId;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::{Rc, Weak};
use std::time::Duration;

mod apply;
mod build;
mod connect;
mod testing;

/// Intervalo entre duas leituras do estado (jogos, flag, mudanças feitas pelo `lrctl` ou editando o arquivo).
const REFRESH: Duration = Duration::from_millis(500);
fn global_game() -> &'static str {
    tr("Global preset")
}
/// O último item da lista de jogos: abre a página que liga o linux-reshade em apps Flatpak.
fn add_flatpak() -> &'static str {
    tr("＋ Add Flatpak game…")
}
fn inherit_preset() -> &'static str {
    tr("(use the global preset)")
}

pub struct Ui {
    pub window: gtk::ApplicationWindow,
    home: Rc<RefCell<Home>>,
    enabled: gtk::Switch,
    game: gtk::DropDown,
    /// O jogo de cada item da lista (`None`: o preset global).
    game_ids: RefCell<Vec<Option<String>>>,
    preset: gtk::DropDown,
    preset_names: RefCell<Vec<String>>,
    preset_note: gtk::Label,
    depth_note: gtk::Label,
    depth_view: Rc<DepthView>,
    depth_button: gtk::Button,
    new_preset: gtk::MenuButton,
    macros: Rc<MacroEditor>,
    new_name: gtk::Entry,
    new_create: gtk::Button,
    search: gtk::SearchEntry,
    list: gtk::ListBox,
    scroller: gtk::ScrolledWindow,
    delete_preset: gtk::Button,
    empty: gtk::Label,
    rows: RefCell<Vec<EffectRow>>,
    /// Descreve o que a lista mostra (ordem, rótulos, estados): se mudar, a lista é refeita.
    signature: RefCell<String>,
    /// Efeitos (arquivos) com os parâmetros abertos: sobrevive às refeituras da lista.
    expanded: Rc<RefCell<HashSet<String>>>,
    /// A página "Adicionar efeito" e a pilha que a alterna com a lista do preset.
    add_view: Rc<AddEffectView>,
    stack: gtk::Stack,
    flatpak_view: Rc<FlatpakView>,
    /// Aparece quando o jogo escolhido é um app Flatpak configurado: abre a página onde se desfaz a configuração.
    flatpak_button: gtk::Button,
    /// A versão do modelo que a tela já mostra: se a leitura periódica não a mudou, não há o que redesenhar.
    applied_version: Cell<u64>,
    /// O indicador "Compilando..." do cabeçalho e a pasta de cache que ele vigia.
    activity: Rc<crate::shared::activity::Activity>,
    cache_dir: std::path::PathBuf,
    games_view: RefCell<GamesView>,
    notice: gtk::Label,
    /// Verdadeiro enquanto a janela atualiza os widgets a partir do modelo.
    updating: Cell<bool>,
}

/// Marca "a janela está mudando widgets" enquanto vive; ao sair (por qualquer caminho) desmarca. Aninhado, o de fora é que desmarca.
struct Programmatic<'a> {
    flag: &'a Cell<bool>,
    was: bool,
}

impl<'a> Programmatic<'a> {
    fn enter(flag: &'a Cell<bool>) -> Self {
        let was = flag.replace(true);
        Self { flag, was }
    }
}

impl Drop for Programmatic<'_> {
    fn drop(&mut self) {
        self.flag.set(self.was);
    }
}

/// Quem refaz a tela depois de uma ação; preenchido quando a `Ui` existe (as ações nascem antes dela).
type Hook = Rc<RefCell<Option<Box<dyn Fn()>>>>;

pub(crate) fn margins(w: &impl IsA<gtk::Widget>, all: i32) {
    w.set_margin_top(all);
    w.set_margin_bottom(all);
    w.set_margin_start(all);
    w.set_margin_end(all);
}

fn caption(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_halign(gtk::Align::Start);
    l.add_css_class("dim-label");
    l.set_width_chars(7);
    l.set_xalign(0.0);
    l
}

impl Ui {
    pub fn new(app: &gtk::Application, home: Home) -> Rc<Ui> {
        Self::with_home(app, Rc::new(RefCell::new(home)))
    }

    /// O modelo da janela: quem refaz a janela (a troca de idioma) o passa à nova, que continua de onde a outra estava.
    pub fn home(&self) -> Rc<RefCell<Home>> {
        self.home.clone()
    }

    /// Põe no seletor o jogo que o modelo tem escolhido (depois de o último item, "adicionar Flatpak", ter sido clicado).
    fn apply_game_selection(self: &Rc<Self>) {
        let _programmatic = Programmatic::enter(&self.updating);
        let selected = {
            let home = self.home.borrow();
            let game = home.snapshot().game.clone();
            self.game_ids.borrow().iter().position(|g| *g == game).unwrap_or(0) as u32
        };
        self.game.set_selected(selected);
    }

    /// Mostra uma das páginas da janela (`"main"`: a lista do preset; `"add"`: adicionar efeito).
    pub fn show_page(self: &Rc<Self>, name: &str) {
        self.stack.set_visible_child_name(name);
        if name == "flatpak" {
            self.flatpak_view.reload();
            self.flatpak_view.focus_search();
        }
        if name == "depth" {
            let ui = self.clone();
            glib::idle_add_local_once(move || ui.apply()); // monta a lista já, sem esperar a próxima atualização
        }
        if name == "add" {
            self.add_view.focus_search();
            let ui = self.clone();
            glib::idle_add_local_once(move || ui.apply()); // começa a montar a lista já, sem esperar a próxima atualização
        }
    }

    /// Uma ação do modelo seguida de refazer a tela (as linhas da lista usam isto).
    fn act(self: &Rc<Self>, action: impl FnOnce(&mut Home)) {
        action(&mut self.home.borrow_mut());
        self.apply();
    }

    /// Mostra/oculta as linhas conforme a busca.
    fn apply_search(&self) {
        let q = self.search.text().to_lowercase();
        for row in self.rows.borrow().iter() {
            let hit = q.is_empty() || row.haystack.contains(&q);
            row.row.set_visible(hit);
        }
    }
}
