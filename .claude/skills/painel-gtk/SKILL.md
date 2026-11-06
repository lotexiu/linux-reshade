---
name: painel-gtk
description: Regras e fluxo para mexer na interface GTK4 do lrpanel (crates/gui). Use antes de criar ou mudar qualquer menu, popover, página, filtro, diálogo ou lista da janela.
---

# Interface do painel (GTK4, Wayland/KDE)

Leia `docs/CONTINUAR.md` ("Arquitetura da interface") antes de começar. Resumo:

1. Fluxo com lista, busca, filtros ou conteúdo que cresce: **página de um `gtk::Stack`**, nunca popover. Modelo: `crates/gui/src/add_effect.rs`.
2. Popover só para algo pequeno e fixo (um campo e um botão, um aviso). **Nunca** aninhado, nunca com `DropDown` dentro, nunca com seção
   que abre/fecha. Dentro de menu, "mais opções" = `disclosure.rs`.
3. Componente = um arquivo, uma struct em `Rc<Self>`, dados entram por `set(...)`, eventos saem por callbacks; não conhece a janela.
   `window.rs` só monta e liga componentes. Callbacks que precisam da janela usam referência fraca (`Weak<Ui>`).
4. Widget só tem um pai: tire do pai antigo (`remove`) ANTES de colocar no novo, senão o GTK recusa e o widget some.
5. Listas grandes (mil linhas ou mais): monte em pedaços por `glib::idle_add_local`, só quando visíveis.
6. Refazer uma lista não pode levar a rolagem ao topo: guarde e devolva o valor do `vadjustment`.

## Antes de entregar
- Todo fluxo novo ganha passos em `Ui::smoke_popovers` (`crates/gui/src/window.rs`), acionados com `--smoke-popovers`.
- Rode `scripts/dev.sh ui-smoke`: deve imprimir `lrpanel smoke-popovers: ok`. Ele falha se algo some ou se o GTK emite aviso/erro.
- `lrpanel --smoke <Efeito> --game <id>` confere o resto (adicionar, ligar/desligar, sliders, presets).
- Diga ao usuário o que NÃO foi testado (clique real com o ponteiro não é simulável aqui).

## Texto visível

Todo texto que a pessoa lê passa por `control_core::i18n::tr` (fixo) ou `fmt` (com valores); a chave é o texto em inglês e a
tradução entra em `crates/control-core/locales/pt_BR.tsv`. Ver `docs/idiomas.md`.
