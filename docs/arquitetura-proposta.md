# Proposta de organização do projeto (para aprovar antes de migrar)

Pedido de 22/11/2026: deixar o projeto mais intuitivo e fácil de manter (pastas, módulos, menos repetição, legibilidade). Esta página junta o que foi
pesquisado, o que se mediu no código e uma sequência de passos pequenos. **Nada foi migrado ainda.**

## 1. O que a pesquisa diz (resumo)
- **Workspace plano.** Para projetos de 10 mil a 1 milhão de linhas (este tem ~32 mil em Rust), a recomendação de quem mantém o rust-analyzer é uma pasta `crates/` plana, com um
  crate por pasta e **nomes com prefixo** (o namespace do Cargo é plano; pastas aninhadas criam uma segunda hierarquia que sai de sincronia). Fonte:
  [Large Rust Workspaces (matklad)](https://matklad.github.io/2021/08/22/large-rust-workspaces.html); as diretrizes de Rust da Microsoft dizem o mesmo
  ([Project Guidelines](https://microsoft.github.io/rust-guidelines/guidelines/project/)). Já fazemos o plano; falta o prefixo e a coerência dos nomes.
- **Layers do Vulkan** (vkBasalt, MangoHud) são uma biblioteca `.so` mais um manifesto JSON, ativados por variável de ambiente, sem daemon; a configuração é lida do disco.
  Nosso desenho (layer + daemon + painel por memória compartilhada) é mais rico, então a separação entre o que roda **dentro do jogo** e o que roda **fora** precisa ficar óbvia.
- **Apps GTK em Rust** não têm uma regra oficial de separar lógica e interface (só o padrão `mod imp` por tipo e as pastas `data/`, `po/` dos modelos do GNOME Builder). A regra que já
  seguimos (modelo independente de toolkit no crate `panel`, uma página/componente por arquivo) é a que vale manter.
- Limite: não achei uma fonte autorizada para o desenho interno de um *post-processing layer*; as decisões abaixo vêm do código daqui, não de um padrão externo.

## 2. O que se mediu aqui (e por que parece "diferente")
| Achado | Dado |
|---|---|
| Um crate faz coisa demais | `control-core`: 5.169 linhas, 21 arquivos: compilador de efeitos, análise, plano dos jogos, vigia, instalação, Flatpak, capturas, parâmetros... |
| Nomes sem regra | pacotes `linux-reshade-layer/-gui/-control` convivem com `ini`, `ipc`, `panel`, `preset`, `config`; a pasta `control` é o `lrctl`, `gui` é o `lrpanel` |
| Dependências estranhas | o CLI (`control`) depende do crate de modelo de tela (`panel`); `gui` fala com `control-core` direto além de `panel`; `i18n` depende de `config`; `ipc` depende de `i18n` |
| Arquivos enormes | `layer/depth/hooks.rs` 1.469 · `layer/effect.rs` 1.240 · `panel/home.rs` 1.179 · `postfx/effect/mod.rs` 1.133 · `gui/window.rs` 1.101 · `control-core/compiler.rs` 1.071 · `gui/shortcuts_dialog.rs` 1.023 |
| Pequenas repetições | `modified(`, `one_line(`, `expand(`, resolução das pastas XDG (`config`, `install`, `ipc`, o daemon); `page_box`/`hint`/`config_file` só vivem no diálogo de Configurações |
| Ferramentas de desenvolvimento | `scripts/dev.sh`: 837 linhas e 32 comandos num arquivo só |
| Raiz e docs | `ROADMAP.md`, `DEPENDENCIAS.md`, `README.md` na raiz; 18 arquivos soltos em `docs/` sem índice; dois roteiros (`ROADMAP.md` histórico e `docs/ROADMAP.md` de próximos passos); `build/`, `dev/`, `dist/` |

## 3. Princípios
1. **Dentro do jogo x fora do jogo** (layer, postfx, vk-layer, ipc) separados do resto, por nome e por regra de dependência.
2. **Núcleo sem toolkit.** Nada de GTK fora de `gui`. O que a interface precisa vem de `panel` (e, no futuro, de uma API local: ver o item da interface web em `docs/ROADMAP.md`).
3. **Um arquivo, uma responsabilidade** (já é a regra da interface: um componente por arquivo), e **um lugar para cada coisa repetida**.
4. **Passos pequenos**, cada um com os testes passando e um commit; dá para parar em qualquer ponto.

## 4. Desenho proposto (pastas planas, nomes com prefixo `lr-`)
```
crates/
  # base (sem dependência interna, ou só entre si)
  lr-ini  lr-paths  lr-i18n  lr-image
  # formatos e protocolo
  lr-bundle (fx-bundle)  lr-preset  lr-ipc
  # dentro do jogo
  lr-vk (vk-layer)  lr-postfx  lr-layer (cdylib)
  # núcleo (sem interface, sem CLI)
  lr-config  lr-compile  lr-plan  lr-system  lr-shortcuts  lr-shortcuts-portal
  # modelo da tela e aplicativos
  lr-panel (modelo)  lr-cli (lrctl + daemon)  lr-gtk (lrpanel)
```
- `lr-paths` junta de uma vez as pastas XDG e a de estado (hoje espalhadas), e os pequenos auxiliares repetidos.
- `control-core` se divide em: `lr-compile` (compilador, análise, manutenção do cache, patches), `lr-plan` (plano dos jogos, vigia, parâmetros, seleção) e `lr-system`
  (instalação, localização, Flatpak, lista de layers, capturas). O CLI deixa de depender de `panel` (o que ele usa de lá vai para `lr-plan`).
- Os binários mantêm os nomes `lrctl` e `lrpanel`; só os pacotes e as pastas mudam.
- Arquivos grandes viram pastas de módulos (uma responsabilidade cada):
  - `lr-gtk`: `window/` (cabeçalho, jogos, linha da profundidade...) e `settings/` com **uma página por arquivo** (geral, atalhos, capturas, pastas, desempenho, overlay, cache, sistema, sobre, log);
  - `lr-layer`: `effect/` (ciclo de vida da cadeia, troca, estatísticas de FPS, fontes do runtime) e `depth/hooks/` por família de função (passes, limpezas, consultas, cópias);
  - `lr-panel`: `home/` por assunto (jogo e preset, efeitos, profundidade, mensagens).
- Ferramentas: `scripts/dev.sh` vira um despachante fino sobre `scripts/lib/*.sh` (um arquivo por assunto: smoke, fx, xid, appimage); os testes continuam os mesmos.
- Documentação: `docs/README.md` como índice, e três pastas: `usuario/` (instalação, idiomas, presets, efeitos embutidos), `arquitetura/` (ipc, bundle-format, depth, interface) e `projeto/` (roteiros, levantamentos,
  upstream). Um só roteiro (os "próximos passos" entram no `ROADMAP.md`, e o histórico dos marcos vira `docs/projeto/historico.md`).

## 5. Sequência (cada passo: testes passando + um commit)
0. **Documentação e raiz** (sem código): índice, juntar os dois roteiros, mover os arquivos de `docs/`. Risco baixo.
1. **Consolidar o que se repete:** `lr-paths` e os auxiliares. Risco baixo (testes cobrem).
2. **Quebrar os arquivos enormes** em módulos, sem mudar comportamento. Risco baixo a médio (mecânico).
3. **Dividir `control-core`** em três crates e tirar a dependência `control → panel`. Risco médio (muitos `use`), mas o compilador acusa tudo.
4. **Renomear** pastas e pacotes para o prefixo `lr-` (e ajustar `scripts/appimage.sh`, docs e CI). Risco médio, totalmente mecânico.
5. **Modularizar `scripts/dev.sh`.** Risco baixo.

## 6. O que NÃO muda
O formato do bundle, o protocolo de memória compartilhada, o comportamento da layer, os nomes `lrctl`/`lrpanel`, os arquivos do usuário (`config.ini`, presets, `game.ini`) e o formato do `ui.ini`.

## 7. Decisões que ficam com você
- Ir pelos passos 0 a 2 agora (baixo risco) e deixar 3 a 5 para depois do teste em jogo do overlay?
- O prefixo `lr-` serve, ou prefere outro (`linux-reshade-` por extenso nas pastas)?
- Manter os comentários e a documentação em português (como está) ou migrar para inglês? (Não mexo se não pedir.)
