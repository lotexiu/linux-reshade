# Auditoria da interface (lrpanel) e plano em etapas

Pedido: rever com calma a interface inteira, achar o que é desenhado sem motivo e trazer para o painel o que um app web
(React/Angular) faz de graça: dados em um sentido só, componentes que só refazem o que mudou, debounce, trabalho pesado fora da
tela. Medido em 2026-11-06 (`LRDBG=1 lrpanel`).

## O que se achou

| # | Problema | Evidência | Sintoma para o usuário |
|---|---|---|---|
| 1 | A leitura do estado roda na thread da tela, de 0,5 em 0,5 s, e **refazia tudo** (`build_view`: lê o preset, recalcula os controles de cada efeito, lista os presets do jogo) mesmo sem nada ter mudado | `refresh` levava 3,4 a 4,1 ms por tick, mais 0,25 ms de `apply` | Tela presa por instantes; uso de CPU à toa (1% parado) |
| 2 | `refresh` lia mais de mil arquivos por tick: `available()` (38 pastas, 1169 arquivos .fx) e a contagem das análises | idem | idem |
| 3 | A lista de efeitos era refeita (`rebuild`) quando mudava QUALQUER coisa da assinatura, inclusive ligado/desligado e o estado no jogo | `signature_of` tinha `t.enabled` e `t.runtime` | Cliques perdidos ao ligar vários efeitos seguidos; rolagem voltando ao topo |
| 4 | Sliders gravavam o preset e avisavam a layer a CADA mudança (dezenas por segundo) | `notify_change` chamava o modelo direto | Arrastar fica pesado; o arquivo é regravado sem parar |
| 5 | Menus (popover) que mudam de tamanho ou se aninham | GTK 5414 | Menus que fechavam e não reabriam |
| 6 | `window.rs` é um objeto só com mais de 25 widgets e uma `build_row` de 300 linhas | 1200 linhas | Cada mudança mexe em tudo; os bugs se repetem |
| 7 | O flag `updating` é a defesa contra o eco (modelo muda widget, widget avisa o modelo) | `if !ui.updating.get()` em cada handler | Fonte de eventos ignorados quando se esquece de testá-lo |

## O que já foi feito (esta rodada)
- **Versão do modelo** (`Home::version`): `refresh` só a sobe quando algo que a tela mostra mudou, e a tela pula o desenho se a
  versão é a que ela já mostra. É o `shouldComponentUpdate`/`memo` da tela inteira. Parada, a tela não desenha nada.
- **Tela montada guardada pela chave** `(preset relido, jogos abertos, análises, jogo escolhido, pasta de presets)`: só se
  refaz quando uma delas muda. É o `useMemo`.
- **Leituras de disco guardadas pela data da pasta**: `available()` e a contagem das análises. `refresh`: de 3,4 ms para 1,2 ms.
- **Lista de efeitos por mudança no lugar**: ligado/desligado e o estado no jogo mudam sem refazer a linha (`update_row`).
- **`Throttle`** (`rx.rs`): os sliders gravam no máximo a cada 80 ms, e a última posição sempre vale. É o debounce/throttle.
- **Adicionar efeito virou página** (componente `add_effect.rs`), e o roteiro `--smoke-popovers` cobre o fluxo.

## Plano (cada etapa verificável e entregue sozinha)
1. **Lista com chave (feito)** (como `key=` do React): linhas reaproveitadas por arquivo; adicionar, remover e mover mexem só nas linhas
   afetadas, sem refazer as outras nem os controles abertos.
2. **Componente por parte da janela (a linha de efeito e a página Adicionar efeito feitas)**: cabeçalho, barra do preset, lista de efeitos (+ `EffectRow`), jogos abertos, avisos.
   Cada um com `set(props)` que compara com o que já mostra e ignora o igual. `window.rs` só liga um ao outro.
3. **Ações em vez de chamadas**: os widgets enviam uma `Action` (enum) e um só lugar a aplica no modelo; o `updating` some porque
   o widget só avisa quando o usuário mexe (`connect_*` ligado só às ações de pessoa).
4. **Leitura fora da thread da tela**: uma thread lê disco e camadas, compara com o último estado e só envia um novo `Snapshot`
   (imutável, compartilhado por `Arc`) quando mudou; a tela só aplica. A tela nunca espera disco.
5. **Resto dos menus pequenos** (profundidade, definições, novo preset): páginas ou diálogos se der qualquer problema.

Cada etapa termina com `scripts/dev.sh ui-smoke` e `lrpanel --smoke <Efeito>` passando e, para o que é de clique real, a
conferência do usuário.
