//! A janela: monta os widgets e os liga ao modelo. Nenhuma regra de negócio mora aqui; tudo o que se
//! decide está em `panel::Model`.

use crate::controls::ControlsView;
use crate::games::GamesView;
use gtk4 as gtk;
use gtk::glib;
use gtk::prelude::*;
use panel::{Model, Selection};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

/// Intervalo entre duas leituras do estado (jogos, flag, mudanças feitas pelo `lrctl`).
const REFRESH: Duration = Duration::from_millis(500);
/// O item que desfaz a escolha de efeito.
const NONE_ITEM: &str = "(nenhum)";

pub struct Ui {
    pub window: gtk::ApplicationWindow,
    model: Rc<RefCell<Model>>,
    enabled: gtk::Switch,
    effect: gtk::DropDown,
    /// Os efeitos que a lista mostra hoje (sem o item "(nenhum)").
    effect_names: RefCell<Vec<String>>,
    chosen: gtk::Label,
    /// O título da seção de parâmetros: só aparece quando há o que ajustar.
    controls_title: gtk::Label,
    controls: RefCell<ControlsView>,
    games: RefCell<GamesView>,
    notice: gtk::Label,
    reset_all: gtk::Button,
    /// Verdadeiro enquanto a janela atualiza os widgets a partir do modelo.
    updating: Cell<bool>,
}

/// Quem refaz a tela depois de uma ação; preenchido quando a `Ui` existe (as ações nascem antes dela).
type Hook = Rc<RefCell<Option<Box<dyn Fn()>>>>;

fn heading(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_halign(gtk::Align::Start);
    l.add_css_class("title-4");
    l
}

impl Ui {
    pub fn new(app: &gtk::Application, model: Model) -> Rc<Ui> {
        let model = Rc::new(RefCell::new(model));
        let hook: Hook = Rc::new(RefCell::new(None));
        let redraw = {
            let hook = hook.clone();
            move || {
                if let Some(f) = &*hook.borrow() {
                    f();
                }
            }
        };

        // Um controle do efeito mudou: vai para o modelo e a tela é refeita.
        let controls = ControlsView::new({
            let (model, redraw) = (model.clone(), redraw.clone());
            Rc::new(move |name: &str, values: &[f64]| {
                model.borrow_mut().set_control(name, values);
                redraw();
            })
        });

        let enabled = gtk::Switch::new();
        enabled.set_valign(gtk::Align::Center);
        let effect = gtk::DropDown::from_strings(&[NONE_ITEM]);
        effect.set_hexpand(true);
        let chosen = gtk::Label::new(None);
        chosen.set_halign(gtk::Align::Start);
        chosen.add_css_class("dim-label");
        let notice = gtk::Label::new(None);
        notice.set_halign(gtk::Align::Start);
        notice.set_wrap(true);
        notice.add_css_class("warning");
        let reset_all = gtk::Button::with_label("Restaurar todos os padrões");
        reset_all.set_halign(gtk::Align::Start);

        let page = gtk::Box::new(gtk::Orientation::Vertical, 12);
        for margin in [gtk::Box::set_margin_top, gtk::Box::set_margin_bottom, gtk::Box::set_margin_start, gtk::Box::set_margin_end] {
            margin(&page, 16);
        }
        page.append(&heading("Efeito"));
        page.append(&effect);
        page.append(&chosen);
        let controls_title = heading("Parâmetros");
        page.append(&controls_title);
        page.append(&controls.root);
        page.append(&reset_all);
        page.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        page.append(&heading("Jogos"));
        let games = GamesView::new();
        page.append(&games.root);
        page.append(&notice);

        let scroller = gtk::ScrolledWindow::new();
        scroller.set_child(Some(&page));
        scroller.set_vexpand(true);
        scroller.set_hscrollbar_policy(gtk::PolicyType::Never);

        let header = gtk::HeaderBar::new();
        let toggle = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        toggle.append(&gtk::Label::new(Some("Efeitos ligados")));
        toggle.append(&enabled);
        header.pack_end(&toggle);

        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("linux-reshade")
            .default_width(520)
            .default_height(720)
            .child(&scroller)
            .build();
        window.set_titlebar(Some(&header));

        let ui = Rc::new(Ui {
            window,
            model,
            enabled,
            effect,
            effect_names: RefCell::new(Vec::new()),
            chosen,
            controls_title,
            controls: RefCell::new(controls),
            games: RefCell::new(games),
            notice,
            reset_all,
            updating: Cell::new(false),
        });

        // O gancho que refaz a tela, guardado só como referência fraca para não formar um ciclo.
        let weak: Weak<Ui> = Rc::downgrade(&ui);
        *hook.borrow_mut() = Some(Box::new(move || {
            if let Some(ui) = weak.upgrade() {
                ui.apply();
            }
        }));

        ui.connect(redraw);
        ui.apply();
        ui
    }

    /// Liga os widgets da janela às ações do modelo.
    fn connect(self: &Rc<Self>, redraw: impl Fn() + Clone + 'static) {
        let weak = Rc::downgrade(self);

        self.enabled.connect_active_notify({
            let (weak, redraw) = (weak.clone(), redraw.clone());
            move |sw| {
                if let Some(ui) = weak.upgrade() {
                    if !ui.updating.get() {
                        ui.model.borrow_mut().set_enabled(sw.is_active());
                        redraw();
                    }
                }
            }
        });

        self.effect.connect_selected_notify({
            let (weak, redraw) = (weak.clone(), redraw.clone());
            move |dd| {
                let Some(ui) = weak.upgrade() else { return };
                if ui.updating.get() {
                    return;
                }
                let index = dd.selected() as usize;
                let name = index.checked_sub(1).and_then(|i| ui.effect_names.borrow().get(i).cloned());
                match name {
                    Some(name) => ui.model.borrow_mut().select_effect(&name),
                    None => ui.model.borrow_mut().clear_selection(),
                }
                redraw();
            }
        });

        self.reset_all.connect_clicked({
            let (weak, redraw) = (weak.clone(), redraw.clone());
            move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.model.borrow_mut().reset_controls();
                    redraw();
                }
            }
        });

        // Lê o estado de tempos em tempos: jogos que abrem e fecham, o atalho global, o `lrctl`.
        glib::timeout_add_local(REFRESH, move || match weak.upgrade() {
            Some(ui) => {
                ui.model.borrow_mut().refresh();
                ui.apply();
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });
    }

    /// Empurra o estado do modelo para os widgets.
    fn apply(&self) {
        self.updating.set(true);
        {
            let model = self.model.borrow();
            let s = model.snapshot();

            if s.enabled != self.enabled.is_active() {
                self.enabled.set_active(s.enabled);
            }

            // A lista de efeitos (só é refeita se mudou).
            if *self.effect_names.borrow() != s.effects {
                let mut items = vec![NONE_ITEM.to_string()];
                items.extend(s.effects.iter().cloned());
                let refs: Vec<&str> = items.iter().map(String::as_str).collect();
                self.effect.set_model(Some(&gtk::StringList::new(&refs)));
                *self.effect_names.borrow_mut() = s.effects.clone();
            }
            let (index, text) = match &s.selection {
                Selection::None => (0, "Nenhum efeito escolhido.".to_string()),
                Selection::Effect(name) => (
                    s.effects.iter().position(|e| e == name).map_or(0, |i| i as u32 + 1),
                    format!("Escolhido: {name}. É compilado para a resolução de cada jogo aberto."),
                ),
                Selection::Fixed(dir) => (0, format!("Bundle fixo (escolhido pelo lrctl): {}", dir.display())),
            };
            if self.effect.selected() != index {
                self.effect.set_selected(index);
            }
            self.chosen.set_text(&text);

            self.controls.borrow_mut().show(&s.controls);
            self.reset_all.set_visible(!s.controls.is_empty());
            self.controls_title.set_visible(!s.controls.is_empty());
            self.games.borrow_mut().show(&s.layers);

            self.notice.set_visible(s.notice.is_some());
            self.notice.set_text(s.notice.as_deref().unwrap_or(""));
        }
        self.updating.set(false);
    }

    /// Escolhe um efeito na lista, como o usuário faria. `false` se o efeito não está na lista.
    pub fn choose_effect(&self, name: &str) -> bool {
        let position = self.effect_names.borrow().iter().position(|n| n == name);
        match position {
            Some(p) => {
                self.effect.set_selected(p as u32 + 1);
                true
            }
            None => false,
        }
    }

    /// Escolhe "(nenhum)" na lista.
    pub fn choose_no_effect(&self) {
        self.effect.set_selected(0);
    }

    // ---- Apoio ao teste de fumaça: aciona os widgets de verdade e confere o que o modelo viu. ----

    /// Percorre a interface como um usuário e devolve o que deu errado (vazio = tudo certo).
    pub fn smoke(&self, effect_name: &str) -> Vec<String> {
        let mut problems = Vec::new();
        let mut check = |ok: bool, what: &str| {
            if !ok {
                problems.push(what.to_string());
            }
        };

        // 1. escolher o efeito na lista
        let listed = self.choose_effect(effect_name);
        check(listed, &format!("o efeito {effect_name:?} não está na lista"));
        if !listed {
            return problems;
        }
        let selected = self.model.borrow().snapshot().selection.clone();
        check(selected == Selection::Effect(effect_name.into()), &format!("escolha esperada {effect_name:?}, veio {selected:?}"));
        let notice = self.model.borrow().snapshot().notice.clone();
        check(notice.is_none(), &format!("aviso ao escolher: {notice:?}"));

        // 2. mexer num slider e ver o modelo receber
        let scales = self.controls.borrow().first_scales();
        check(self.controls.borrow().row_count() > 0, "o efeito não gerou nenhum controle");
        if let Some(scale) = scales.first() {
            let adj = scale.adjustment();
            let target = adj.lower() + (adj.upper() - adj.lower()) * 0.25;
            scale.set_value(target);
            let seen = self.model.borrow().snapshot().controls.first().map(|c| c.value[0]);
            check(seen.is_some_and(|v| (v - target).abs() < 0.02), &format!("o modelo viu {seen:?}, esperava ~{target}"));

            // 3. o botão de restaurar volta ao padrão
            // (o empréstimo precisa acabar antes do clique: a ação do botão usa `controls` de novo)
            let reset = self.controls.borrow().first_reset();
            if let Some(reset) = reset {
                reset.emit_clicked();
                let c = self.model.borrow().snapshot().controls.first().cloned();
                check(c.as_ref().is_some_and(|c| c.is_default()), &format!("depois de restaurar: {c:?}"));
            }
        }

        // 4. o interruptor de efeitos ligados
        self.enabled.set_active(true);
        check(self.model.borrow().snapshot().enabled, "o interruptor não ligou os efeitos");
        self.enabled.set_active(false);
        check(!self.model.borrow().snapshot().enabled, "o interruptor não desligou os efeitos");

        // 5. deixar tudo como estava
        self.model.borrow_mut().reset_controls();
        self.choose_no_effect();
        let selected = self.model.borrow().snapshot().selection.clone();
        check(selected == Selection::None, &format!("a escolha não foi desfeita: {selected:?}"));
        problems
    }
}
