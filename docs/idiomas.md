# Idiomas da interface

O texto-fonte do código é o **inglês**, que é o idioma padrão. Os outros idiomas são catálogos em
`crates/control-core/locales/<código>.tsv` (uma tradução por linha: `inglês<TAB>tradução`; `\n` e `\t` escritos como
`\\n` e `\\t`). Texto sem tradução aparece em inglês.

## Escolher o idioma

- No painel: **Configurações → Geral → Idioma**. A escolha fica em `~/.config/linux-reshade/ui.ini` (`language=pt_BR`) e vale
  na próxima abertura (há o botão "Reiniciar o painel agora").
- Pela variável `LR_LANG` (por exemplo `LR_LANG=pt_BR lrpanel`), que vale mais que o arquivo.
- Sem nenhuma das duas: inglês.

## Escrever texto novo na interface

```rust
use control_core::i18n::{fmt, tr};
label.set_text(tr("Add effect"));                          // texto fixo
status.set_text(&fmt("Compiling… {}/{}", &[&n, &total]));  // com valores: {} na ordem, ou {1} {2} para mudar a ordem
```

- Escreva a chave em inglês e acrescente a linha no `pt_BR.tsv`. `tr` pede `&'static str`: o texto precisa ser literal.
- Texto que só se conhece na hora (guardado em disco, vindo da layer) usa `tr_dyn(&str)`; sem tradução, devolve o mesmo.
- Mensagens da layer do Vulkan e do `postfx` já saem em inglês e são traduzidas no painel quando a frase é fixa.
- Os testes de `control-core` rodam sempre em inglês; os demais assumem o idioma padrão (sem `language` no `ui.ini`).
- Os modos de teste do `lrpanel` (`--smoke`...) e a linha de comando `lrctl` continuam em português por enquanto.

## Acrescentar um idioma

1. Copie `locales/pt_BR.tsv` para `locales/<código>.tsv` e traduza o lado direito.
2. Registre-o em `LANGUAGES` (`crates/control-core/src/i18n.rs`) com o código, o nome no próprio idioma e o `include_str!`.
