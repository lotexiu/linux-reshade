//! Apoio aos testes automáticos: aciona os widgets de verdade e confere o que o modelo viu.

use super::*;

impl Ui {
    /// O que a tela sabe dos jogos, em texto (para os testes automáticos conferirem de fora).
    /// Mexe na página "Adicionar efeito" como uma pessoa (abrir, abrir os filtros, escolher uma pasta, digitar, voltar, reabrir) e
    /// devolve o que deu errado: uma página que some, um campo que não aparece. Os avisos e erros do GTK durante o roteiro também
    /// contam (é assim que um widget com dois pais aparece).
    pub async fn smoke_popovers(self: &Rc<Self>) -> Vec<String> {
        use std::time::Duration;
        let mut problems = Vec::new();
        let step = |ms: u64| glib::timeout_future(Duration::from_millis(ms));
        let warnings_before = crate::gtk_warnings();
        let on_add = |ui: &Ui| ui.stack.visible_child_name().as_deref() == Some("add");

        self.show_page("add");
        step(800).await;
        if !on_add(self) {
            problems.push("a página de adicionar efeito não abriu".to_string());
            return problems;
        }
        if !self.add_view.is_search_shown() {
            problems.push("o campo de busca dos efeitos não aparece".to_string());
        }
        if !self.add_view.is_filters_button_shown() {
            problems.push("o botão Filtros não aparece".to_string());
        }
        if self.add_view.rows_built() == 0 && !self.home.borrow().snapshot().effects.is_empty() {
            problems.push("a lista de efeitos não foi montada".to_string());
        }

        self.add_view.open_filters(true);
        step(500).await;
        if !on_add(self) {
            problems.push("abrir os filtros tirou a página".to_string());
            return problems;
        }
        if !self.add_view.is_folder_list_shown() {
            problems.push("a lista de pastas não apareceu ao abrir os filtros".to_string());
        }
        self.add_view.select_folder(1);
        step(400).await;
        self.add_view.type_query("a");
        step(400).await;
        if !on_add(self) {
            problems.push("escolher uma pasta ou digitar fechou a página".to_string());
            return problems;
        }
        self.add_view.open_filters(false);
        step(300).await;
        self.add_view.type_query("");
        self.add_view.select_folder(0);
        self.show_page("main");
        step(500).await;
        if on_add(self) {
            problems.push("voltar não mostrou a lista do preset".to_string());
        }
        self.show_page("add");
        step(500).await;
        if !on_add(self) {
            problems.push("a página não reabriu depois de fechada".to_string());
        }
        self.show_page("main");
        step(300).await;

        // A página dos apps Flatpak, pelo último item da lista de jogos (a escolha volta ao jogo que estava).
        let before = self.game.selected();
        self.game.set_selected(self.game_ids.borrow().len() as u32);
        step(2500).await;
        if self.stack.visible_child_name().as_deref() != Some("flatpak") {
            problems.push("o último item da lista de jogos não abriu a página dos apps Flatpak".to_string());
        }
        if self.game.selected() != before {
            problems.push("a lista de jogos não voltou ao jogo anterior".to_string());
        }
        if control_core::flatpak::available() && self.flatpak_view.app_count() == 0 {
            problems.push("a página dos apps Flatpak não listou nenhum app".to_string());
        }
        self.show_page("main");
        step(300).await;

        // A página "Depth Buffer" com dados de mentira (o roteiro não tem um jogo aberto): as opções, os pontos de captura e voltar.
        let choices = vec![
            panel::DepthChoice { number: 1, label: "1920x1080 D24_UNORM_S8 · 411 draws, 12 passes, normal".into(), automatic: true, unusable: None, blocking: false },
            panel::DepthChoice { number: 2, label: "1120x1120 D24_UNORM_S8 · 10 draws, 1 passes, normal".into(), automatic: false, unusable: Some("different aspect ratio than the screen".into()), blocking: false },
        ];
        let clears = vec![panel::DepthClear { draws_before: 0, splittable: true }, panel::DepthClear { draws_before: 276, splittable: false }];
        let passes = vec![panel::DepthPass { draws: 311, load: "Clear".into(), store: "Store".into() }, panel::DepthPass { draws: 233, load: "Load".into(), store: "Store".into() }];
        let picks = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        self.depth_view.connect_changed({
            let picks = picks.clone();
            move |p| picks.borrow_mut().push(p)
        });
        self.show_page("depth");
        step(400).await;
        if self.stack.visible_child_name().as_deref() != Some("depth") {
            problems.push("a página Depth Buffer não abriu".to_string());
        }
        self.depth_view.set(&choices, &clears, &passes, ipc::DepthPick::Auto, true);
        step(300).await;
        // Automático (1) + pontos: automático, 2 limpezas, passe (4) + 2 imagens = 7
        if self.depth_view.rows_built() < 6 {
            problems.push(format!("a lista da profundidade tem só {} opções", self.depth_view.rows_built()));
        }
        self.depth_view.set(&choices, &clears, &passes, ipc::DepthPick::Clear(1), true);
        step(300).await;
        self.depth_view.set(&choices, &clears, &passes, ipc::DepthPick::Candidate(2), true);
        step(300).await;
        if !picks.borrow().is_empty() {
            problems.push("refazer a lista da profundidade foi tratado como uma escolha da pessoa".to_string());
        }
        if self.stack.visible_child_name().as_deref() != Some("depth") {
            problems.push("refazer a lista da profundidade tirou a página".to_string());
        }
        self.show_page("main");
        step(300).await;
        self.depth_view.set(&[], &[], &[], ipc::DepthPick::Auto, true);

        let new = crate::gtk_warnings().saturating_sub(warnings_before);
        if new > 0 {
            problems.push(format!("o GTK reclamou {new} vez(es) durante o roteiro (veja os avisos acima)"));
        }
        problems
    }

    pub fn macros_popup(&self) {
        self.macros.button.popup();
    }

    pub fn describe_games(&self) -> String {
        let home = self.home.borrow();
        let s = home.snapshot();
        let games: Vec<String> = s.games.iter().map(|g| format!("{}{}", g.id, if g.open { "*" } else { "" })).collect();
        // O que o seletor de jogos mostra de verdade (o widget, não o modelo).
        let shown: Vec<String> = self
            .game
            .model()
            .map(|m| (0..m.n_items()).filter_map(|i| m.item(i).and_downcast::<gtk::StringObject>().map(|o| o.string().to_string())).collect())
            .unwrap_or_default();
        format!("jogos: [{}] camadas: {} seletor: {:?}", games.join(", "), s.layers.len(), shown)
    }

    // ---- Apoio aos testes automáticos: aciona os widgets de verdade e confere o que o modelo viu. ----

    /// Escolhe um jogo, como pela lista.
    pub fn select_game(self: &Rc<Self>, id: &str) {
        self.act(|h| h.select_game(Some(id)));
    }

    /// Adiciona um efeito ao preset do jogo escolhido, como pelo botão. `false` se o efeito não existe.
    pub fn choose_effect(self: &Rc<Self>, name: &str) -> bool {
        // A lista da página "Adicionar efeito" só é montada quando ela está na tela: vai direto ao modelo (o mesmo que o clique faz).
        if self.home.borrow().snapshot().effects.iter().any(|e| e == name) {
            self.act(|h| h.add_effect(name));
            return true;
        }
        false
    }

    /// Tira do preset as técnicas do efeito (desfaz `choose_effect`).
    pub fn remove_effect(self: &Rc<Self>, name: &str) {
        let file = format!("{name}.fx");
        let ids: Vec<TechId> = self.home.borrow().snapshot().techniques.iter().filter(|t| t.file == file).map(|t| t.id.clone()).collect();
        for id in ids {
            self.act(|h| h.remove_technique(&id));
        }
    }

    /// Percorre a interface como um usuário e devolve o que deu errado (vazio = tudo certo). Deixa o preset como estava.
    pub fn smoke(self: &Rc<Self>, effect_name: &str) -> Vec<String> {
        let mut problems = Vec::new();
        let mut check = |ok: bool, what: &str| {
            if !ok {
                problems.push(what.to_string());
            }
        };
        let file = format!("{effect_name}.fx");
        let find = |ui: &Ui| ui.home.borrow().snapshot().techniques.iter().find(|t| t.file == file).cloned();

        // 1. adicionar o efeito
        check(self.choose_effect(effect_name), &format!("o efeito {effect_name:?} não está na lista"));
        let Some(t) = find(self) else {
            check(false, "o efeito não apareceu na lista de técnicas");
            return problems;
        };
        check(t.enabled && self.home.borrow().snapshot().notice.is_none(), &format!("estado ao adicionar: {:?}", self.home.borrow().snapshot().notice));
        check(self.rows.borrow().iter().any(|r| r.file == file), "a linha do efeito não foi desenhada");

        // 2. desligar e ligar pela caixa de seleção
        let check_button = self.rows.borrow().iter().find(|r| r.file == file).map(|r| r.check.clone());
        if let Some(c) = check_button {
            c.set_active(false);
            check(find(self).is_some_and(|t| !t.enabled), "a caixa não desligou a técnica");
        }
        let row_before = self.rows.borrow().iter().find(|r| r.file == file).map(|r| r.row.clone());
        let check_button = self.rows.borrow().iter().find(|r| r.file == file).map(|r| r.check.clone());
        if let Some(c) = check_button {
            // Ligar e desligar várias vezes seguidas: o estado acompanha cada clique e a linha NÃO é refeita (refazer troca a caixa
            // sob o ponteiro e o clique seguinte se perde).
            for n in 0..6 {
                c.set_active(n % 2 == 1);
                check(find(self).is_some_and(|t| t.enabled == (n % 2 == 1)), &format!("a caixa não acompanhou o clique {n}"));
            }
            check(find(self).is_some_and(|t| t.enabled), "a caixa não religou a técnica");
            let row_after = self.rows.borrow().iter().find(|r| r.file == file).map(|r| r.row.clone());
            check(row_before.is_some() && row_before == row_after, "ligar e desligar refez a linha do efeito");
        }

        // 3. mexer num slider e ver o modelo receber; depois restaurar
        let scales = self.rows.borrow().iter().find(|r| r.file == file).and_then(|r| r.controls.as_ref().map(|c| c.borrow().first_scales())).unwrap_or_default();
        if let Some(scale) = scales.first() {
            let adj = scale.adjustment();
            let target = adj.lower() + (adj.upper() - adj.lower()) * 0.25;
            scale.set_value(target);
            let seen = find(self).and_then(|t| t.controls.first().map(|c| c.value[0]));
            check(seen.is_some_and(|v| (v - target).abs() < 0.02), &format!("o modelo viu {seen:?}, esperava ~{target}"));
            self.act(|h| h.reset_effect(&file));
            check(find(self).is_some_and(|t| t.controls.iter().all(|c| c.is_default())), "restaurar não voltou aos padrões");
        } else {
            check(false, "o efeito não gerou nenhum controle");
        }

        // 3b. criar um preset pelo botão "Criar" do popover (precisa de um jogo escolhido)
        if self.home.borrow().snapshot().game.is_some() {
            self.new_name.set_text("smoke-novo");
            self.new_create.emit_clicked();
            let active = self.home.borrow().snapshot().active_preset.clone();
            check(active.as_deref() == Some("smoke-novo"), &format!("o botão Criar não criou o preset (ativo: {active:?}, aviso: {:?})", self.home.borrow().snapshot().notice));
            check(self.new_name.text().is_empty(), "o campo do nome não foi limpo");
            check(self.delete_preset.is_sensitive(), "a lixeira do preset não ficou ativa");
            self.act(|h| h.delete_preset("smoke-novo"));
            check(!self.home.borrow().snapshot().presets.iter().any(|p| p == "smoke-novo"), "o preset não foi apagado");
        }

        // 3c. as definições do preset, pelo editor (liga/desliga e texto livre)
        self.macros.set_text("");
        self.act(|h| h.set_preset_macros("RESHADE_DEPTH_INPUT_IS_UPSIDE_DOWN=1,SMOKE_Q=2"));
        self.apply();
        check(self.macros.text() == "RESHADE_DEPTH_INPUT_IS_UPSIDE_DOWN=1,SMOKE_Q=2", &format!("o editor não mostra as definições do modelo: {:?}", self.macros.text()));
        self.act(|h| h.set_preset_macros(""));
        self.apply();
        check(self.macros.text().is_empty(), "o editor não limpou depois de apagar as definições");

        // 4. o interruptor de efeitos ligados
        self.enabled.set_active(true);
        check(self.home.borrow().snapshot().enabled, "o interruptor não ligou os efeitos");
        self.enabled.set_active(false);
        check(!self.home.borrow().snapshot().enabled, "o interruptor não desligou os efeitos");

        // 4b. acrescentar e tirar OUTRO efeito não refaz a linha do primeiro (a lista reaproveita as linhas, como o `key=` do React)
        let first = self.rows.borrow().iter().find(|r| r.file == file).map(|r| r.row.clone());
        if effect_name != "Tonemap" && self.choose_effect("Tonemap") {
            check(self.rows.borrow().iter().any(|r| r.file == "Tonemap.fx"), "o segundo efeito não apareceu na lista");
            let same = self.rows.borrow().iter().find(|r| r.file == file).map(|r| r.row.clone());
            check(first.is_some() && first == same, "acrescentar outro efeito refez a linha do primeiro");
            let order: Vec<String> = self.rows.borrow().iter().map(|r| r.file.clone()).collect();
            let shown: Vec<String> = std::iter::successors(self.list.first_child(), |w| w.next_sibling()).map(|w| w.widget_name().to_string()).collect();
            check(shown.len() == order.len(), &format!("a lista mostra {} linhas, o modelo tem {}", shown.len(), order.len()));
            // Mover um efeito para cima: a lista segue a ordem do modelo e as linhas continuam as mesmas.
            let tonemap_row = self.rows.borrow().iter().find(|r| r.file == "Tonemap.fx").map(|r| r.row.clone());
            self.act(|h| h.move_effect("Tonemap.fx", -1));
            let model: Vec<String> = self.home.borrow().snapshot().groups.iter().map(|g| g.file.clone()).collect();
            let rows: Vec<String> = self.rows.borrow().iter().map(|r| r.file.clone()).collect();
            let shown: Vec<gtk::Widget> = std::iter::successors(self.list.first_child(), |w| w.next_sibling()).collect();
            let shown_rows: Vec<gtk::Widget> = self.rows.borrow().iter().map(|r| r.row.clone().upcast()).collect();
            check(model == rows, &format!("a ordem das linhas {rows:?} não segue o modelo {model:?}"));
            check(shown == shown_rows, "a ordem na tela não segue a das linhas");
            check(tonemap_row.is_some() && tonemap_row == self.rows.borrow().iter().find(|r| r.file == "Tonemap.fx").map(|r| r.row.clone()), "mover refez a linha");
            self.remove_effect("Tonemap");
            let same = self.rows.borrow().iter().find(|r| r.file == file).map(|r| r.row.clone());
            check(first.is_some() && first == same, "tirar outro efeito refez a linha do primeiro");
            check(!self.rows.borrow().iter().any(|r| r.file == "Tonemap.fx"), "o segundo efeito não saiu da lista");
        }

        // 5. deixar o preset como estava
        self.remove_effect(effect_name);
        check(find(self).is_none(), "o efeito não saiu do preset");
        problems
    }
}
