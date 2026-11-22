# Proposta de organização do projeto (para aprovar antes de migrar)

Pedido de 22/11/2026: deixar o projeto mais intuitivo e fácil de manter (pastas, módulos, menos repetição, legibilidade). Esta página junta o que foi
pesquisado, o que se mediu no código e uma sequência de passos pequenos. **Nada foi migrado ainda.**

## 1. O que quase todo projeto segue (Angular, React, Next.js)
Em vez de procurar projetos parecidos com o nosso (não existe), a pesquisa foi no que as estruturas mais comuns têm em comum. Resumo, com as fontes:
- **Por funcionalidade (feature-first).** Cada funcionalidade tem a sua pasta, com tudo o que ela precisa (telas, serviços, modelos). Angular ("organize por feature") e
  [Bulletproof React](https://cdn.jsdelivr.net/npm/bulletproof-react666@0.1.0/docs/project-structure.md) fazem assim; as funcionalidades não importam umas às outras.
- **`core` e `shared`.** O `core` guarda o que é do aplicativo inteiro (serviços únicos, infraestrutura); o `shared` guarda o que várias funcionalidades reusam (componentes e utilitários),
  **sem regra de negócio**. ([guias de estrutura do Angular](https://itnext.io/choosing-a-highly-scalable-folder-structure-in-angular-d987de65ec7))
- **Dependência em um só sentido:** `shared` → `features` → `app`. Quem fica embaixo não conhece quem fica em cima; costuma-se impor com uma regra de lint.
- **LIFT (Angular):** *Locate* (achar o código rápido), *Identify* (o nome diz o que é), manter a estrutura **Flat** (rasa) e **DRY**.
- **Colocação (Next.js):** o que só uma parte usa mora ao lado dela (testes, auxiliares), e pastas privadas/grupos organizam sem mudar o comportamento. A própria documentação diz que não há
  uma estrutura única certa: **escolha uma e seja consistente**. ([projeto do Next.js](https://nextjs.org/docs/app/getting-started/project-structure))
- **Monorepo:** `apps/` (o que se entrega) e `packages/` (bibliotecas compartilhadas), a convenção de Turborepo/Nx, a mesma do seu `ts-monorepo`.
- Para Rust em especial, a pasta `crates/` plana com nomes de pacote com prefixo é o que se recomenda para esse tamanho
  ([Large Rust Workspaces](https://matklad.github.io/2021/08/22/large-rust-workspaces.html)).
Limite: boa parte disso vem de guias e artigos, não de um documento oficial único. O que se repete em todos é o que importa: **funcionalidade, `core`/`shared`, dependência de um sentido, nome que diz o que é**.

## 2. O que se mediu aqui (e por que parece "diferente")
| Achado | Dado |
|---|---|
| Um crate faz coisa demais | `control-core`: 5.169 linhas, 21 arquivos: compilador de efeitos, análise, plano dos jogos, vigia, instalação, Flatpak, capturas, parâmetros... |
| Nomes sem regra | pacotes `linux-reshade-layer/-gui/-control` convivem com `ini`, `ipc`, `panel`, `preset`, `config`; a pasta `control` é o `lrctl`, `gui` é o `lrpanel` |
| Dependências estranhas | o CLI (`control`) depende do crate de modelo de tela (`panel`); `gui` fala com `control-core` direto além de `panel`; `i18n` depende de `config`; `ipc` depende de `i18n` |
| Arquivos enormes | `layer/depth/hooks.rs` 1.469 · `layer/effect.rs` 1.240 · `panel/home.rs` 1.179 · `postfx/effect/mod.rs` 1.133 · `gui/window.rs` 1.101 · `control-core/compiler.rs` 1.071 · `gui/shortcuts_dialog.rs` 1.023 |
| Pequenas repetições | `modified(`, `one_line(`, `expand(`, resolução das pastas XDG (`config`, `install`, `ipc`, o daemon); `page_box`/`hint`/`config_file` só vivem no diálogo de Configurações |
| Ferramentas de desenvolvimento | `scripts/dev.sh`: 837 linhas e 32 comandos num arquivo só |
| Raiz e docs (antes do passo 0) | `ROADMAP.md`, `DEPENDENCIAS.md` e `README.md` na raiz; 18 arquivos soltos em `docs/` sem índice; dois roteiros (o histórico, na raiz, e o de próximos passos, em `docs/`); `build/`, `dev/`, `dist/` |

## 3. Mapa: o que cada convenção vira aqui
| Convenção | Aqui |
|---|---|
| `apps/` x `packages/` | `apps/` = o que se entrega (`lrctl`, `lrpanel`, a layer); `crates/` = bibliotecas compartilhadas |
| `shared` (sem regra de negócio) | base: `lr-ini`, `lr-paths`, `lr-i18n`, `lr-image`, e os widgets reusáveis da interface |
| `core` (regra de negócio, sem interface) | `lr-compile`, `lr-plan`, `lr-system`, `lr-config`, `lr-bundle`, `lr-preset`, `lr-ipc`, `lr-shortcuts*` |
| `features/<nome>` | dentro de cada app: efeitos, presets, jogos, profundidade, atalhos, configurações, overlay, Flatpak |
| `app` (monta tudo) | a janela principal e o `main` de cada app |
| dependência `shared → core → features → app` | regra conferida por um teste (um script que lê o `cargo metadata` e falha se um crate de baixo depender de um de cima) |
| LIFT (nome diz o que é, estrutura rasa) | um arquivo, uma responsabilidade; `view.rs`, `note.rs`, `plan.rs`: o nome é o papel |

## 4. Desenho proposto
```
apps/
  lrctl/     CLI + daemon (bin lrctl)           src/{app, features/{...}}
  lrpanel/   interface GTK (bin lrpanel)         src/{app, features/{effects,presets,games,depth,settings,overlay,flatpak}, shared/widgets}
  layer/     cdylib que roda dentro do jogo      src/{...}  (depth/, effect/ ... já são funcionalidades)
crates/      (planas, prefixo lr-)
  lr-ini  lr-paths  lr-i18n  lr-image                       base (shared)
  lr-config  lr-preset  lr-bundle  lr-ipc                    formatos e protocolo
  lr-vk  lr-postfx                                           para a layer
  lr-compile  lr-plan  lr-system  lr-shortcuts  lr-shortcuts-portal   núcleo
  lr-panel                                                   modelo da tela (sem toolkit)
```
- `lr-paths` junta as pastas XDG e a de estado (hoje espalhadas em `config`, `install`, `ipc` e no daemon) e os pequenos auxiliares repetidos.
- `control-core` se divide em `lr-compile` (compilador, análise, manutenção do cache, patches), `lr-plan` (plano dos jogos, vigia, parâmetros, seleção) e `lr-system`
  (instalação, localização, Flatpak, lista de layers, capturas). O CLI deixa de depender de `panel`.
- **Cada app por funcionalidade** (o equivalente de `features/`):
  - `lrpanel`: `features/settings/` com **uma página por arquivo** (geral, atalhos, capturas, pastas, desempenho, overlay, cache, sistema, sobre, log), `features/depth/`, `features/effects/`... e `shared/widgets/` para o que se repete (`page_box`, `hint`, linha de pasta);
  - `layer`: `effect/` (ciclo de vida da cadeia, troca, estatísticas de FPS, fontes) e `depth/` já é uma funcionalidade, com `hooks/` por família de função (passes, limpezas, consultas, cópias);
  - `lr-panel`: `home/` por assunto (jogo e preset, efeitos, profundidade, mensagens).
- Ferramentas: `scripts/dev.sh` vira um despachante fino sobre `scripts/lib/*.sh` (um arquivo por assunto: smoke, fx, xid, appimage).
- Documentação: `docs/README.md` como índice e três pastas: `usuario/`, `arquitetura/`, `projeto/`; um só `ROADMAP.md` (os próximos passos, na raiz); o histórico dos marcos vira `docs/projeto/historico.md`.

## 5. Sequência (cada passo: testes passando + um commit)
0. ~~**Documentação e raiz**~~ **feito (22/11)**: índice em `docs/README.md`, um só `ROADMAP.md`, arquivos em `docs/{usuario,arquitetura,projeto}`.
1. **Consolidar o que se repete:** `lr-paths` e os auxiliares. Risco baixo (testes cobrem).
2. **Quebrar os arquivos enormes** em módulos por funcionalidade (`features/`, `shared/`), sem mudar comportamento. Risco baixo a médio (mecânico).
3. **Dividir `control-core`** em três crates e tirar a dependência `control → panel`. Risco médio (muitos `use`), mas o compilador acusa tudo.
4. **Mover os aplicativos para `apps/` e renomear os pacotes para o prefixo `lr-`** (e ajustar `scripts/appimage.sh` e docs); acrescentar o teste das regras de dependência. Risco médio, mecânico.
5. **Modularizar `scripts/dev.sh`.** Risco baixo.

## 6. O que NÃO muda
O formato do bundle, o protocolo de memória compartilhada, o comportamento da layer, os nomes `lrctl`/`lrpanel`, os arquivos do usuário (`config.ini`, presets, `game.ini`) e o formato do `ui.ini`.

## 7. Decisões que ficam com você
- Ir pelos passos 0 a 2 agora (baixo risco) e deixar 3 a 5 para depois do teste em jogo do overlay?
- A divisão `apps/` + `crates/` com prefixo `lr-` serve? (É a convenção dos monorepos; o seu `ts-monorepo` usa `packages/`, e eu poderia usar `packages/` no lugar de `crates/` para ficar igual.)
- Manter os comentários e a documentação em português (como está) ou migrar para inglês? (Não mexo se não pedir.)
