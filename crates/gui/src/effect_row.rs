//! Uma linha da lista de efeitos do preset: a caixa de ligar/desligar, a bolinha de estado, o nome, os botões e o painel de
//! parâmetros. Não conhece a janela: tudo o que a pessoa faz sai por [`RowCtx`] (o que a linha pode pedir) e o que muda no modelo
//! entra por [`EffectRow::update`], que mexe só no que mudou (a linha não é refeita a cada atualização).

use crate::controls::ControlsView;
use crate::macros::MacroEditor;
use gtk::prelude::*;
use gtk4 as gtk;
use panel::{EffectGroup, Home};
use preset::TechId;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use control_core::i18n::{fmt, tr};

/// O que uma linha pode pedir à janela e o que precisa saber dela.
pub struct RowCtx {
    /// Aplica uma ação ao modelo (e redesenha).
    pub act: Rc<dyn Fn(Box<dyn FnOnce(&mut Home)>)>,
    /// A janela está pondo valores do modelo nos widgets: os avisos dos widgets não são da pessoa.
    pub programmatic: Rc<dyn Fn() -> bool>,
    /// As definições de pré-processador do efeito, como o modelo as tem agora.
    pub current_macros: Rc<dyn Fn(&str) -> String>,
    /// Efeitos (arquivos) com os parâmetros abertos: sobrevive à troca das linhas.
    pub expanded: Rc<RefCell<HashSet<String>>>,
    pub scroller: gtk::ScrolledWindow,
}

/// Os widgets de um efeito da lista (com as técnicas dele).
pub struct EffectRow {
    /// Arquivo do efeito (`cGaussianBlur.fx`).
    pub file: String,
    /// O que a busca compara: nome do efeito, arquivo e nomes das técnicas.
    pub haystack: String,
    pub check: gtk::CheckButton,
    /// As caixas de cada técnica (só nos efeitos com várias), para atualizar o estado sem refazer a linha.
    pub sub_checks: Vec<(TechId, gtk::CheckButton)>,
    pub dot: gtk::Label,
    pub alert_slot: gtk::Box,
    pub controls: Option<RefCell<ControlsView>>,
    pub row: gtk::ListBoxRow,
    /// O que a linha mostra e só muda refazendo-a (nomes, teclas, definições...): a mesma assinatura, a linha serve (ver `signature_of`).
    pub signature: String,
    up: gtk::Button,
    down: gtk::Button,
}

impl EffectRow {
    /// O que exige refazer a linha: o arquivo, as definições, os nomes e as teclas das técnicas. Ligado/desligado e o estado no jogo NÃO
    /// entram: mudam no lugar (`update`), porque refazer a linha troca a caixa sob o ponteiro e o clique seguinte se perde.
    pub fn signature_of(g: &EffectGroup) -> String {
        let multi = g.techniques.len() > 1;
        let techniques: Vec<String> = g
            .techniques
            .iter()
            // Num efeito com várias técnicas, o estado no jogo de cada uma está escrito dentro do painel dele.
            .map(|t| format!("{}:{}:{}:{}", t.id, t.label, t.key, if multi { format!("{:?}", t.runtime) } else { String::new() }))
            .collect();
        format!("{}|{}|{}", g.file, g.macros, techniques.join(";"))
    }

    /// Põe a linha na posição `index` (de `0` a `last`): só o que depende dela, os botões de subir e descer.
    pub fn set_position(&self, index: usize, last: usize) {
        self.up.set_sensitive(index > 0);
        self.down.set_sensitive(index < last);
    }

    /// O texto e a cor do que a layer fez com uma técnica no jogo aberto.
    fn runtime_label(state: &ipc::TechniqueState) -> gtk::Label {
        let (text, class) = match state {
            ipc::TechniqueState::Applied => (tr("● applied in the game").to_string(), "success"),
            ipc::TechniqueState::Skipped(why) => (fmt("skipped in the game: {}", &[&control_core::i18n::tr_dyn(why)]), "error"),
        };
        let l = gtk::Label::new(Some(&text));
        l.set_halign(gtk::Align::Start);
        l.set_xalign(0.0);
        l.set_wrap(true);
        l.add_css_class(class);
        l.add_css_class("caption");
        l
    }

    /// A caixa de ligar/desligar com uma área de clique do tamanho do círculo de destaque do tema: clicar em qualquer ponto dele liga ou
    /// desliga (a caixa sozinha só responde sobre o quadradinho, e o destaque aparece antes, com o mouse ao lado).
    fn big_hit_area(check: &gtk::CheckButton) -> gtk::Box {
        let hit = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        hit.set_size_request(40, 40);
        hit.set_hexpand(false);
        hit.set_halign(gtk::Align::Start);
        hit.set_valign(gtk::Align::Center);
        check.set_halign(gtk::Align::Center);
        check.set_valign(gtk::Align::Center);
        check.set_hexpand(true); // centraliza o quadradinho dentro da área fixa (a área em si não cresce)
        check.set_can_target(false); // o clique é da área, não só do quadradinho
        hit.append(check);
        hit.set_tooltip_text(check.tooltip_text().as_deref());
        let click = gtk::GestureClick::new();
        click.set_button(1);
        click.connect_released({
            let check = check.clone();
            move |gesture, _, x, y| {
                let inside = gesture.widget().is_some_and(|w| x >= 0.0 && y >= 0.0 && x <= f64::from(w.width()) && y <= f64::from(w.height()));
                if inside {
                    check.set_active(!check.is_active());
                }
            }
        });
        hit.add_controller(click);
        let motion = gtk::EventControllerMotion::new();
        motion.connect_enter({
            let check = check.clone();
            move |_, _, _| check.set_state_flags(gtk::StateFlags::PRELIGHT, false)
        });
        motion.connect_leave({
            let check = check.clone();
            move |_| check.unset_state_flags(gtk::StateFlags::PRELIGHT)
        });
        hit.add_controller(motion);
        hit
    }

    /// A bolinha à esquerda do nome mostra o estado: apagada (desligado ou sem jogo), verde (aplicada no jogo), amarela (aviso) ou
    /// vermelha (pulada no jogo). O texto só aparece quando há algo a dizer, no ícone de alerta à direita (abre ao clicar). Muda só o
    /// que mudou: o ícone aberto não é recriado a cada atualização.
    fn apply_status(dot: &gtk::Label, slot: &gtk::Box, g: &EffectGroup) {
        let skipped = g.techniques.iter().find_map(|t| match &t.runtime {
            Some(ipc::TechniqueState::Skipped(why)) => Some(why.clone()),
            _ => None,
        });
        let applied = g.techniques.iter().filter(|t| matches!(t.runtime, Some(ipc::TechniqueState::Applied))).count();
        let problem = g.techniques.iter().find_map(|t| t.problem.clone());
        let (class, tip) = match (&skipped, &problem, applied > 0) {
            (Some(_), _, _) => ("error", tr("Skipped in the game")),
            (None, Some(_), _) => ("warning", tr("Has a warning")),
            (None, None, true) => ("success", tr("Applied in the game")),
            _ => ("dim-label", tr("Not applied right now")),
        };
        for c in ["error", "warning", "success", "dim-label"] {
            dot.remove_css_class(c);
        }
        dot.add_css_class(class);
        dot.set_tooltip_text(Some(tip));
        let text = skipped.as_ref().map(|w| fmt("Skipped in the game: {}", &[&control_core::i18n::tr_dyn(w)])).or(problem);
        // O texto atual do aviso fica no nome do espaço (`widget_name`): só refaz o ícone quando ele muda.
        let shown = text.clone().unwrap_or_default();
        if slot.widget_name() == shown {
            return;
        }
        slot.set_widget_name(&shown);
        while let Some(child) = slot.first_child() {
            slot.remove(&child);
        }
        if let Some(text) = text {
            let button = gtk::MenuButton::new();
            button.set_icon_name("dialog-warning-symbolic");
            button.add_css_class("flat");
            button.add_css_class(if skipped.is_some() { "error" } else { "warning" });
            button.set_valign(gtk::Align::Center);
            button.set_tooltip_text(Some(tr("Show the warning")));
            let label = gtk::Label::new(Some(&text));
            label.set_wrap(true);
            label.set_max_width_chars(48);
            label.set_xalign(0.0);
            crate::window::margins(&label, 8);
            let popover = gtk::Popover::new();
            popover.set_child(Some(&label));
            button.set_popover(Some(&popover));
            slot.append(&button);
        }
    }

    /// Põe na linha o que muda sem refazê-la: as caixas de ligado/desligado e o estado no jogo.
    pub fn update(row: &EffectRow, g: &EffectGroup) {
        row.check.set_active(g.any_enabled());
        row.check.set_inconsistent(g.any_enabled() && !g.all_enabled());
        for (id, sub) in &row.sub_checks {
            if let Some(t) = g.techniques.iter().find(|t| t.id == *id) {
                sub.set_active(t.enabled);
            }
        }
        Self::apply_status(&row.dot, &row.alert_slot, g);
    }

    pub fn new(ctx: &Rc<RowCtx>, index: usize, last: usize, g: &EffectGroup) -> EffectRow {
        let file = g.file.clone();
        let multi = g.techniques.len() > 1;

        // A caixa do efeito liga e desliga todas as técnicas dele (marcada pela metade se só algumas estão ligadas).
        let check = gtk::CheckButton::new();
        check.set_active(g.any_enabled());
        check.set_inconsistent(g.any_enabled() && !g.all_enabled());
        check.set_valign(gtk::Align::Center);
        check.set_tooltip_text(Some(tr("Turn the effect on or off")));
        check.connect_toggled({
            let (ctx, file) = (ctx.clone(), file.clone());
            move |c| {
                if !(ctx.programmatic)() {
                    let (on, file) = (c.is_active(), file.clone());
                    (ctx.act)(Box::new(move |h| h.set_effect_enabled(&file, on)));
                }
            }
        });

        let hit = Self::big_hit_area(&check);

        let title = gtk::Label::new(Some(&g.name));
        title.set_halign(gtk::Align::Start);
        title.set_xalign(0.0);
        title.add_css_class("heading");
        let subtitle = gtk::Label::new(Some(&g.file));
        subtitle.set_halign(gtk::Align::Start);
        subtitle.add_css_class("dim-label");
        subtitle.add_css_class("caption");
        let names = gtk::Box::new(gtk::Orientation::Vertical, 0);
        names.set_hexpand(true);
        names.append(&title);
        names.append(&subtitle);

        // A bolinha à esquerda do nome mostra o estado e o ícone de alerta à direita traz o texto, quando há (ver `apply_status`).
        let dot = gtk::Label::new(Some("●"));
        dot.set_valign(gtk::Align::Center);
        let alert_slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        Self::apply_status(&dot, &alert_slot, g);

        let icon_button = |icon: &str, tip: &str| {
            let b = gtk::Button::from_icon_name(icon);
            b.set_tooltip_text(Some(tip));
            b.add_css_class("flat");
            b.set_valign(gtk::Align::Center);
            b
        };
        let up = icon_button("go-up-symbolic", tr("Run earlier"));
        up.set_sensitive(index > 0);
        up.connect_clicked({
            let (ctx, file) = (ctx.clone(), file.clone());
            move |_| {
                let file = file.clone();
                (ctx.act)(Box::new(move |h| h.move_effect(&file, -1)));
            }
        });
        let down = icon_button("go-down-symbolic", tr("Run later"));
        down.set_sensitive(index < last);
        down.connect_clicked({
            let (ctx, file) = (ctx.clone(), file.clone());
            move |_| {
                let file = file.clone();
                (ctx.act)(Box::new(move |h| h.move_effect(&file, 1)));
            }
        });
        let remove = icon_button("user-trash-symbolic", tr("Remove the effect from the preset (deletes its settings)"));
        remove.connect_clicked({
            let (ctx, file) = (ctx.clone(), file.clone());
            move |_| {
                let file = file.clone();
                (ctx.act)(Box::new(move |h| h.remove_effect(&file)));
            }
        });

        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        head.append(&hit);
        head.append(&dot);
        head.append(&names);
        head.append(&alert_slot);

        // O painel que abre e fecha (⋮): as técnicas do efeito (se são várias) e os parâmetros dele.
        let controls = (!g.controls.is_empty()).then(|| {
            let view = ControlsView::new(
                {
                    let (ctx, file) = (ctx.clone(), file.clone());
                    Rc::new(move |name: &str, values: &[f64]| {
                        let (file, name, values) = (file.clone(), name.to_string(), values.to_vec());
                        (ctx.act)(Box::new(move |h| h.set_control(&file, &name, &values)));
                    })
                },
                ctx.scroller.clone(),
            );
            RefCell::new(view)
        });
        let outer = gtk::Box::new(gtk::Orientation::Vertical, 4);
        crate::window::margins(&outer, 8);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        top.append(&head);
        let has_panel = true; // as definições de pré-processador do efeito estão sempre no painel
        let revealer = gtk::Revealer::new();
        let mut sub_checks: Vec<(TechId, gtk::CheckButton)> = Vec::new();
        if has_panel {
            let panel = gtk::Box::new(gtk::Orientation::Vertical, 8);
            panel.set_margin_start(32);
            panel.set_margin_top(6);
            if multi {
                let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
                let heading = gtk::Label::new(Some(tr("Techniques")));
                heading.set_halign(gtk::Align::Start);
                heading.add_css_class("dim-label");
                heading.add_css_class("caption");
                list.append(&heading);
                for t in &g.techniques {
                    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                    let sub = gtk::CheckButton::with_label(&t.label);
                    sub_checks.push((t.id.clone(), sub.clone()));
                    sub.set_active(t.enabled);
                    sub.set_tooltip_text(Some(tr("Turn only this technique on or off")));
                    sub.connect_toggled({
                        let (ctx, id) = (ctx.clone(), t.id.clone());
                        move |c| {
                            if !(ctx.programmatic)() {
                                let (on, id) = (c.is_active(), id.clone());
                                (ctx.act)(Box::new(move |h| h.set_technique_enabled(&id, on)));
                            }
                        }
                    });
                    line.append(&sub);
                    if let Some(state) = &t.runtime {
                        line.append(&Self::runtime_label(state));
                    }
                    list.append(&line);
                }
                panel.append(&list);
            }
            // O atalho que liga e desliga cada técnica (vai no `Key<técnica>` do preset, como no ReShade).
            for t in &g.techniques {
                let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                let caption = gtk::Label::new(Some(&if multi { fmt("Shortcut for {}", &[&t.label]) } else { tr("Shortcut to turn on/off").to_string() }));
                caption.set_xalign(0.0);
                caption.set_hexpand(true);
                let button = gtk::Button::with_label(&if t.key.is_empty() { tr("No shortcut").to_string() } else { shortcuts::trigger::display(&t.key) });
                button.set_tooltip_text(Some(tr("Global key that turns this effect on and off (the daemon registers it; works with the game in focus)")));
                button.connect_clicked({
                    let (ctx, id, title) = (ctx.clone(), t.id.clone(), t.label.clone());
                    move |button| {
                        let window = button.root().and_downcast::<gtk::Window>();
                        let Some(window) = window else { return };
                        let (ctx, id) = (ctx.clone(), id.clone());
                        crate::shortcuts_dialog::capture_key(&window, &fmt("Shortcut for {}", &[&title]), move |text| {
                            let id = id.clone();
                            (ctx.act)(Box::new(move |h| h.set_technique_key(&id, &text)));
                        });
                    }
                });
                line.append(&caption);
                line.append(&button);
                panel.append(&line);
            }
            // Definições de pré-processador só deste efeito (mudar recompila o efeito).
            let macros = MacroEditor::new(tr("Effect definitions…"), tr("Preprocessor definitions for this effect only; they take precedence over the preset's. Changing them recompiles the effect."));
            macros.set_text(&g.macros);
            macros.button.set_halign(gtk::Align::Start);
            macros.connect_changed({
                let (ctx, file) = (ctx.clone(), file.clone());
                move |text| {
                    if !(ctx.programmatic)() {
                        // Compara com o que o modelo tem AGORA (não com o que havia quando a lista foi desenhada): senão voltar ao
                        // valor antigo, ou mudar duas vezes, não aplicava.
                        let current = (ctx.current_macros)(&file);
                        if text != current {
                            let (file, text) = (file.clone(), text.clone());
                            (ctx.act)(Box::new(move |h| h.set_effect_macros(&file, &text)));
                        }
                    }
                }
            });
            panel.append(&macros.button);
            if let Some(view) = &controls {
                panel.append(&view.borrow().root);
                let reset = gtk::Button::with_label(tr("Restore the effect's defaults"));
                reset.set_halign(gtk::Align::Start);
                reset.connect_clicked({
                    let (ctx, file) = (ctx.clone(), file.clone());
                    move |_| {
                        let file = file.clone();
                        (ctx.act)(Box::new(move |h| h.reset_effect(&file)));
                    }
                });
                panel.append(&reset);
            }
            revealer.set_child(Some(&panel));
            let open = ctx.expanded.borrow().contains(&file);
            revealer.set_reveal_child(open);
            let toggle = gtk::ToggleButton::new();
            toggle.set_icon_name("view-more-symbolic");
            toggle.set_tooltip_text(Some(if multi { tr("Techniques and parameters") } else { tr("Parameters") }));
            toggle.add_css_class("flat");
            toggle.set_valign(gtk::Align::Center);
            toggle.set_active(open);
            toggle.connect_toggled({
                let (ctx, file, revealer) = (ctx.clone(), file.clone(), revealer.clone());
                move |b| {
                    revealer.set_reveal_child(b.is_active());
                    if b.is_active() {
                        ctx.expanded.borrow_mut().insert(file.clone());
                    } else {
                        ctx.expanded.borrow_mut().remove(&file);
                    }
                }
            });
            top.append(&toggle);
        }
        top.append(&up);
        top.append(&down);
        top.append(&remove);
        outer.append(&top);
        if has_panel {
            outer.append(&revealer);
        }

        let row = gtk::ListBoxRow::new();
        row.set_activatable(false);
        row.set_child(Some(&outer));
        let haystack = std::iter::once(g.name.clone()).chain(std::iter::once(g.file.clone())).chain(g.techniques.iter().flat_map(|t| [t.label.clone(), t.id.name.clone()])).collect::<Vec<_>>().join(" ").to_lowercase();
        EffectRow { file: g.file.clone(), haystack, check, sub_checks, dot, alert_slot, controls, row, signature: Self::signature_of(g), up, down }
    }
}
