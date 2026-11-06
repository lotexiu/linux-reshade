# Idiomas da interface

O texto-fonte do código é o **inglês**, que é o idioma padrão. Cada outro idioma é um arquivo
`crates/i18n/locales/<código>.tsv` (uma tradução por linha: `inglês<TAB>tradução`; `\n` e `\t` escritos como `\\n` e `\\t`).
A linha `# language: Nome do idioma` dá o nome que aparece na lista. Texto sem tradução aparece em inglês, nunca vazio.

## Escolher o idioma

- No painel: **Configurações → Geral → Idioma**. Vale na hora (a janela é refeita no mesmo lugar) e fica em
  `~/.config/linux-reshade/ui.ini` (`language=pt_BR`).
- Pela variável `LR_LANG` (por exemplo `LR_LANG=pt_BR lrctl layers`), que vale mais que o arquivo.
- Sem nenhuma das duas: inglês. Sob o `cargo` (test, run) o arquivo é ignorado e vale só `LR_LANG`.

## Acrescentar um idioma

Basta o arquivo. Nada no código muda:

1. Copie `crates/i18n/locales/pt_BR.tsv` para `crates/i18n/locales/<código>.tsv`, troque a linha `# language:` e traduza o lado direito.
2. Recompile: o `build.rs` do crate `i18n` embute todo `locales/*.tsv`.

Sem recompilar, um `<código>.tsv` em `~/.local/share/linux-reshade/locales/` (ou na pasta de `LINUX_RESHADE_LOCALES`) também aparece na
lista e vence o embutido de mesmo código.

## Escrever texto novo

```rust
use control_core::i18n::{fmt, tr};          // (nos crates de baixo nível: use i18n::{fmt, tr})
label.set_text(tr("Add effect"));                          // texto fixo (precisa ser literal)
status.set_text(&fmt("Compiling… {}/{}", &[&n, &total]));  // com valores: {} na ordem, ou {1} {2} para mudar a ordem
```

A chave é o texto em inglês; acrescente a linha no `pt_BR.tsv` (e nos outros idiomas).

- Texto que só se conhece na hora (guardado em disco, vindo da layer) usa `tr_dyn(&str)`: exato, ou por modelo (`step {}: …` casa
  com `step 2: …` e traduz também o que está dentro dos `{}`).
- A layer do Vulkan e o `postfx` mandam seus motivos em inglês; o painel os traduz com `tr_dyn`.
- Tabelas `const` guardam o texto em inglês e chamam `tr` na hora de usar.

## Conferir

- `lrpanel --dump-texts` lista todo texto visível (janela, páginas e abas das Configurações); compare `LR_LANG=en` com o outro idioma.
- `lrpanel --switch-language <código>` troca o idioma com a janela aberta e confere que ficou uma só janela, com os textos novos.
- Os testes de `control-core` e dos crates de baixo nível rodam em inglês; os de `panel` e `gui` também, desde que não haja `LR_LANG`.

## O que não é traduzido

O log de diagnóstico da layer e do daemon (`layer.log`), os modos de teste do `lrpanel` (`--smoke`…) e o nome de arquivo do preset padrão
(`Padrão.ini`, que é dado do usuário).
