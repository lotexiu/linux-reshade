# Depth buffer (marco 5): desenho

Pedido do dono: o depth buffer é o diferencial do projeto; 38 dos 150 efeitos da pasta dele pedem depth. Pesquisa
(docs/reshade-feedback.md): no ReShade a detecção é o ponto que mais frustra (não acha, pega a interface em vez da
cena, MSAA atrapalha). No vkBasalt o depth é experimental e "pode dar muitos problemas". Por isso o desenho abaixo
torna a escolha do buffer **visível e ajustável por jogo**. Teste de referência: Haste (Unity, Vulkan nativo, tela
inicial em 3D, abre rápido) e o `vkcube` (tem depth).

## O que o ReShade faz de verdade (lido no código)
Lido em `examples/09-depth/generic_depth_addon.cpp` do ReShade
([fonte](https://github.com/crosire/reshade/blob/main/examples/09-depth/generic_depth_addon.cpp)); a documentação do add-on
só descreve opções ([Depth3D](https://www.depth3d.info/generic-depth-mod.html), [guia do Marty](https://guides.martysmods.com/reshade/depth/)).
- Ao **criar** a imagem de profundidade, acrescenta `copy_source` (em Vulkan e D3D12 **sempre** trabalha com uma
  cópia, por causa de aliasing de recursos).
- A cada **desenho**, soma vértices e chamadas por imagem de profundidade (`vertices`, `drawcalls`,
  `drawcalls_indirect`) e guarda o último viewport. Se há muitos desenhos indiretos, compara por chamadas; senão, por vértices.
- Quando o jogo **limpa** a profundidade (ou a troca por outra, ou faz um desenho de tela cheia), copia a imagem para
  a cópia **antes** da limpeza, se a carga de trabalho desde a última limpeza for **maior ou igual** à melhor deste
  quadro. Ou seja, pode copiar várias vezes por quadro e o último "melhor" vence. Ignora limpezas depois de pouco trabalho
  e viewports pequenos (sombras renderizadas na mesma imagem).
- No **present**, escolhe a candidata: descarta as sem desenhos, as que só apareceram agora, as multiamostradas (sem
  resolve) e as de **proporção diferente** da tela (tolerância de 0,1 na razão largura/altura e razão de tamanho entre
  0,5 e 1,85); entre as que sobram, a de mais vértices. Há uma escolha manual por sessão (por *handle*) e um "índice de limpeza" por jogo.
- Com MSAA, faz *resolve* (precisa de `VK_KHR_depth_stencil_resolve`). Se o passe descarta a profundidade ao terminar
  (`storeOp` = descartar), o próprio código admite que uma cópia depois "provavelmente terá dados quebrados".

## O que dá para fazer melhor numa layer Vulkan nativa
A pergunta do dono foi "quando e como obter o depth, com pouco consumo e de forma estável". Respostas, em ordem:
1. **Só quando alguém precisa.** O ReShade rastreia sempre. Aqui o plano já diz se alguma técnica ligada lê `DEPTH`
   (`analysis`: característica "usa profundidade"). Sem técnica de profundidade: **custo zero** (nem cópia, nem contagem).
   Com ela, 2 quadros de aquecimento para aprender.
2. **Aprender num quadro e copiar uma vez no seguinte.** Em vez de copiar a cada limpeza e deixar o último "melhor"
   valer, o quadro N só *observa* (qual passe de renderização, em que posição dentro do quadro, foi o da cena principal) e
   o quadro N+1 copia **uma vez**, no fim desse passe (posição conhecida). Se as estatísticas deixarem de bater, volta a aprender.
3. **O melhor ponto para copiar é o fim do passe da cena** (`vkCmdEndRenderPass`/`vkCmdEndRendering`): ali o layout da
   imagem é conhecido (o `finalLayout` do passe, ou o `imageLayout` do anexo), então a barreira é correta sem adivinhar.
   Amostrar a imagem original no momento do present só serve quando o jogo não a toca depois, e o layout aí é incerto.
4. **Corrigir o `storeOp`**: se o passe da cena descarta a profundidade ao fim, a layer troca para guardar (nos passes
   candidatos, na criação do render pass ou ao copiar o `VkRenderingInfo`), em vez de torcer para dar certo.
5. **Descobrir sozinha o que o ReShade pede ao usuário**: profundidade **invertida** (reversed-Z) sai do `depthCompareOp` dos
   pipelines (`GREATER`) e do valor de limpeza (0 em vez de 1); "de cabeça para baixo" e "logarítmica" ficam como
   ajuste manual. Isso elimina a configuração que mais confunde (o ReShade a deixa nas macros
   `RESHADE_DEPTH_INPUT_IS_REVERSED` etc.).
6. **A escolha persiste por jogo**, por assinatura (formato, tamanho, posição do passe), não por *handle* (que muda a
   cada execução), e fica no preset do jogo; o painel mostra as candidatas e permite fixar uma.
7. **Dois tipos de passe**: tratar `vkCmdBeginRenderPass`(2/KHR) e `vkCmdBeginRendering`(KHR). Passes dentro de
   *secondary command buffers* (`vkCmdExecuteCommands`) contam para o passe do primário (o DXVK e a Unity usam).
8. **Informações que precisamos observar**: por imagem de profundidade: formato, tamanho, amostras, uso, e por quadro
   número de desenhos e vértices, viewport, posição do passe no quadro, `loadOp`/`storeOp`/valor de limpeza e layout;
   por pipeline: `depthCompareOp` e se escreve profundidade; do swapchain: tamanho.
9. **MSAA fica de fora do primeiro passo** (precisa de resolve de profundidade); a candidata multiamostrada é
   marcada "indisponível: MSAA", com o motivo visível, não ignorada em silêncio.

*Limite da pesquisa:* não consegui ler o código do vkBasalt (`depthCapture`, "pode dar muitos problemas fora da NVIDIA");
a conclusão sobre ele vem só do FAQ resumido por terceiros
([vkBasalt](https://awesome.ecosyste.ms/projects/github.com%2Fdadschoorse%2Fvkbasalt)). O que a Vulkan exige para copiar
profundidade (aspecto `DEPTH`, uso `TRANSFER_SRC`, `D16`/`D32` com amostragem obrigatória) vem do
[guia da Khronos](https://docs.vulkan.org/guide/latest/depth.html). Unity expõe `_CameraDepthTexture` por um passe de
profundidade ou do G-buffer ([doc da Unity](https://docs.unity3d.com/2018.3/Documentation/Manual/SL-CameraDepthTexture.html)),
o que muda de jogo para jogo: motivo a mais para observar antes de copiar.

## Pergunta do dono: a cópia é mesmo necessária? (2026-10-11)
**Não sempre.** A cópia do ReShade (em Vulkan, sempre) existe por três motivos, e dá para tratar cada um sem copiar:
1. *Uso:* a imagem precisa ter `SAMPLED` para um shader lê-la. A layer acrescenta o uso na criação (armado com
   `LINUX_RESHADE_DEPTH=1`; sem MSAA e só se o formato aceita).
2. *Layout:* para amostrar, a imagem tem de estar em layout de leitura, e o jogo a deixa onde quiser. A layer acompanha o
   layout (fim dos passes e barreiras, na ordem dos envios à fila), então sabe o layout exato no present: leva a imagem
   ao layout de leitura antes dos efeitos e a devolve depois, sem mudar nada para o jogo. **Foi implementado (D2a).**
3. *Tempo de vida:* o conteúdo pode ser limpo ou descartado antes do fim do quadro. É o único caso em que copiar é
   preciso: copiar no ponto certo (antes da limpeza), dentro do command buffer do jogo, onde o layout é conhecido
   (D2b, ainda não feito). No Haste a profundidade da cena sobrevive até o fim do quadro (`store`), então a cópia nem seria usada.

Custo estimado de uma cópia (não medido): profundidade D32 a 1920x1080 são ~8,3 MB lidos e ~8,3 MB escritos, uns
0,05 ms numa GPU de ~360 GB/s, mais a espera entre passes; a 1280x720 (FSR do Haste), menos da metade. Para um jogo de
200 fps (5 ms por quadro), do que se espera algo em torno de 1 a 4 por cento no pior caso. Amostrar a original custa zero
de banda extra. A medição de verdade fica para a medição de custo por técnica (timestamp queries; ver interface.md).

## Haste (Unity, Vulkan nativo): o que se mediu (2026-10-10)

Teste ao vivo com `lrctl depth pick pass N` (foto da profundidade depois do N-ésimo passe) e uma captura de tela de cada uma.

- A imagem original lida no fim do quadro (e as candidatas 1 e o "passe 1") saía **estragada**: um desenho torto e grande
  cobria a cena (o "quadrado" que acompanha a câmera). Os passes 2 a 48 eram idênticos e corretos; o 49 e o 50 estragavam.
- O número de passes varia entre quadros (11 numa sessão, 50 em outra): **escolher por número de passe não é estável**.
- Regra automática adotada (sem pedido): a profundidade é a foto do passe anterior ao **primeiro passe, depois do primeiro,
  que começa com `DontCare`** (o jogo descarta o conteúdo e recomeça a imagem: outra câmera ou camada no mesmo alvo).
  Exige o modo armado (`LINUX_RESHADE_DEPTH=1`); sem ele vale a imagem original. Regra empírica, validada em um jogo só.
  A causa exata do passe 49 não foi achada (os contadores de comparação por passe vieram zerados).
- Orientação: o buffer do Unity é invertido (Reversed) **e** de cabeça para baixo. A layer detecta o Reversed
  (macro automática); o `RESHADE_DEPTH_INPUT_IS_UPSIDE_DOWN=1` precisa ser posto à mão (campo Macros do painel).
  Não há como detectar de forma confiável: vira escolha por jogo (D3). Com o "Show live preview" ligado os controles
  `iUIReversed`/`iUIUpsideDown` valem; com ele desligado valem só as macros.
- Efeitos de profundidade reais no Haste (2026-10-10, regra do `DontCare`, `UPSIDE_DOWN=1`, preset de teste apagado depois):
  `DepthHaze`, `AdaptiveFog` (névoa por distância, correta), `CinematicDOF` (desfoque) e `DepthDarkness` (tela preta é o
  padrão do próprio efeito: foco em 0,0005 do plano distante) funcionam. `HeightFog` não aplica: pede `fognoise.jpg` (JPEG
  ainda não suportado). O DOF borra também o texto da UI do jogo: falta o modo "antes da UI". `smoke-depth` e
  `smoke-depth-effect` passaram depois da regra.

- ATLYSS (Proton, 2026-10-11): a candidata automática tinha 2 passes; o 1º é a cena e o 2º, com `Clear`, desenha outra
  câmera (os personagens da criação) no mesmo alvo. O painel (botão "Profundidade…") mostrou isso e "depois do passe 1" deu a
  profundidade certa. A regra automática agora também recomeça em `Clear` (como o "copiar antes de limpar" do ReShade):
  `observe::scene_pass`.
- **Escolha por limpeza (2026-10-11, inspirada no "copiar antes de limpar" do ReShade):** no ATLYSS (DXVK) o ReShade lista
  `CLEAR 1/2/3` com 4, 1399 e 1 desenhos; o número de passes de renderização muda de quadro para quadro (o DXVK os divide como
  quer), o de limpezas não. A layer agora conta as limpezas da profundidade observada (`vkCmdBeginRendering` que recomeça a
  imagem com `Clear` ou `DontCare`, `vkCmdClearAttachments` com profundidade dentro do passe, `vkCmdClearDepthStencilImage`),
  guarda quantos desenhos cada uma apaga e copia a profundidade **logo antes** da que apaga mais (com histerese de 8 quadros).
  Uma limpeza dentro de um passe de renderização dinâmica é partida: `vkCmdEndRendering`, cópia, `vkCmdBeginRendering` com
  `LOAD`. Só vale para passes sem estruturas encadeadas e sem suspender/retomar; sem isso a limpeza não é "partível" e a
  regra antiga (passe) continua. Pedido manual: `lrctl depth pick clear N` ou "Antes da limpeza" no painel.
  Testado com `smoke-dynamic` (app headless com limpezas dentro do passe, validation layer limpa na lavapipe); **não** com o
  jogo ainda.
- Falha achada no teste: uma cadeia que falhava por falta de profundidade nunca era tentada de novo (corrigido).

## Passos (cada um verificável e commitado)
- **D1, só observar:** ganchos de `vkCreateImage`, `vkCreateImageView`, `vkCreateFramebuffer`, `vkCreateRenderPass`(2),
  início/fim de passe (os dois tipos), `vkCmdExecuteCommands` e desenhos; contadores baratos e ligados só quando pedido
  (variável de ambiente no D1); as candidatas aparecem em `lrctl layers` com os dados do item 8. Teste: `vkcube`
  (uma candidata, com desenhos) e, depois, o Haste. Nenhuma mudança no que o jogo desenha.
- **F6, macros que recompilam:** `PreprocessorDefinitions` nos três níveis (global, preset, efeito) chegam ao
  `fx2spv -D` e invalidam o bundle.
- **D2a (feito):** amostrar a original: uso `SAMPLED` (armado), layout acompanhado, `DepthInput` no `postfx`, a layer entrega a candidata escolhida, reconstrói a cadeia quando ela muda e libera a view se o jogo destruir a imagem. Teste: `smoke-depth-effect` (vkcube com `LINUX_RESHADE_DEPTH_FORCE_STORE=1`, que guarda a profundidade que o vkcube descartaria), com a validation layer sem mensagens.
- **D2b (a fazer):** cópia antes da limpeza para jogos que limpam a profundidade no meio do quadro.
- **D2c (a fazer):** macros de profundidade automáticas (invertida detectada) e o `DisplayDepth` do ReShade.
- **D2, original:** copiar e entregar: aprender num quadro, copiar uma vez no fim do passe no seguinte, corrigir `storeOp`, e a textura `DEPTH` no `postfx`. Teste: um efeito
  simples que desenha a profundidade como cinza (o `DisplayDepth.fx` do próprio ReShade) no vkcube e no Haste.
- **D3, escolha no painel:** lista das candidatas por jogo (tamanho, formato, desenhos), "automático" ou fixar uma, "de cabeça para baixo",
  logarítmico e distância do plano distante (a profundidade invertida é detectada sozinha, com opção de forçar); a escolha vai para o preset do jogo.

## Riscos (ditos de antemão)
- Jogos que reusam a mesma imagem de profundidade para a cena e para a interface, ou que a limpam antes de o quadro acabar:
  a cópia tem de acontecer no ponto certo, e a heurística pode errar.
- Jogos com depth em formato que não pode ser copiado como está ou sem `SAMPLED`: tratar como indisponível, com o motivo.
- MSAA: depth multiamostrado precisa de resolve; fora do primeiro passo.
- Custo: uma cópia por quadro da imagem de profundidade.
- Isso mexe em command buffers do jogo: o mais arriscado de todo o projeto; exige a validation layer em cada passo.

## Modo cópia (o desenho do ReShade), 2026-11-06
Depois de seis Xid 69 num dia (Slime Rancher 2 e A Hat in Time) e da leitura da fonte do add-on (`generic_depth_addon.cpp`), a layer passou
a seguir o ReShade nos pontos em que divergia de um jeito arriscado. Cada ponto tem uma variável de ambiente para voltar ao de antes.

- **Os efeitos leem sempre uma cópia (a foto), nunca a imagem do jogo.** Antes a cadeia lia a imagem original, com a barreira de layout
  gravada uma vez, na criação da cadeia, a partir do layout que a layer rastreava; se o jogo deixasse a imagem noutro layout num quadro
  (carregamento, menu, passe a menos), a barreira mentia. Agora, a cada present, a layer grava (num command buffer próprio, à frente dos
  efeitos no mesmo envio) a cópia original → foto com o layout **daquele momento**, e a foto está sempre no layout de leitura que a
  própria layer gravou. É a "cópia na hora dos efeitos" do ReShade. `LINUX_RESHADE_DEPTH_DIRECT=1` volta a amostrar a original.
- **A cópia da hora dos efeitos só acontece se nenhuma regra copiou no quadro** (limpeza, passe), como o `copied_during_frame` do ReShade.
  Isso também elimina a foto velha: se a limpeza escolhida não existe num quadro, a foto vem da profundidade viva.
- **A partição do passe numa `vkCmdClearAttachments` continua (é o que corrigiu o A Hat in Time), mas só quando é válida.** O ReShade não
  copia dentro de um passe ativo (é inválido). A layer parte o passe (`vkCmdEndRendering`, cópia, `vkCmdBeginRendering` com `LOAD`) só se:
  nenhuma consulta, renderização condicional ou transform feedback foi aberta no passe (têm de acabar na mesma instância; o DXVK usa os
  três: a layer agora conta `vkCmdBeginQuery`/`EndQuery` e os `EXT`), e a profundidade (e o estêncil) são guardadas ao fim da primeira
  metade (um `storeOp` `DONT_CARE`/`NONE` do jogo vira `STORE` na imagem observada). Se não é válido, o passe não é partido, a layer avisa
  no log (`passe não partido: ...`) e a cópia da hora dos efeitos cobre. O `lrctl layers` mostra `passes partidos/pulados`.
  `LINUX_RESHADE_DEPTH_NO_SPLIT=1` desliga a partição (para comparar).
- Custo da cópia: ~8 MB lidos e ~8 MB escritos por quadro a 1080p (D32), na ordem de 0,05 a 0,15 ms numa RTX 3060, e só com algum efeito
  de profundidade ligado.
- Os roteiros `smoke-depth-effect` (vkcube, validation layer) e `smoke-dynamic` (partição, consulta aberta, `DONT_CARE` e MSAA) passam.

## Ajustes na hora, vários jogos, Xid (2026-11-06)
- `lrctl depth policy [split|copy|snapshot=on|off|default ...] [--layer jogo]` muda a política da profundidade **no jogo aberto, sem reiniciar** (a
  layer lê `depth-policy-<id>` a cada 20 quadros): `split` (partir o passe numa limpeza), `copy` (os efeitos leem a cópia; `off` = a imagem do jogo) e
  `snapshot` (qualquer foto). Sem argumentos mostra o que vale em cada jogo. Serve para comparar estratégias ao vivo em vários jogos.
- `scripts/dev.sh xid-watch` segue os Xid da NVIDIA ao vivo e, a cada um, grava em `~/.local/state/linux-reshade/xid-watch.log` o jogo (pelo pid), a
  política em vigor, o último estado publicado pela layer e o fim do log dela. Para vários jogos ao mesmo tempo (cada um com `LINUX_RESHADE_LOG` no
  mesmo arquivo: as linhas levam o pid).
- Uma candidata que "não serve" por proporção diferente ou sem desenhos agora **pode ser escolhida** (em amarelo no painel: "pode não dar o
  resultado esperado"); só a MSAA sem `SAMPLED` (impossível de amostrar) continua bloqueada.

## O quadrado do Haste voltava (2026-11-07)
O "quadrado" do Haste é um desenho torto e grande que cobre a cena e que entra na profundidade **depois** do passe da cena (ver "Haste" acima); a regra do
passe (foto depois do passe N, antes do primeiro `DontCare`) existe para evitá-lo. Com o modo cópia a cópia da hora dos efeitos passou a entrar nos
quadros em que a regra não copiou (o número de passes do Haste varia entre quadros): era a profundidade viva de agora, já com o quadrado. Agora, se uma
regra de ponto (limpeza ou passe) está ativa e a foto já foi gravada, o quadro sem cópia da regra **mantém a foto anterior**; a cópia viva só preenche a
primeira vez ou quando nenhuma regra vale.

## Um trinco por fila (2026-11-07)
Os Xid 69 de 07/11 só aconteciam com o preset pesado e o Proton Experimental (janela e tela cheia); com o GE-Proton 11-7 ou com o preset leve o jogo não
travava. Uma `VkQueue` não pode ser usada por duas threads ao mesmo tempo, e a layer submete o trabalho dos efeitos (e o `vkQueueWaitIdle` da criação da
cadeia) na fila do jogo de dentro do gancho do present, enquanto o jogo submete na mesma fila de outra thread; o DXVK sincroniza o próprio submit e o
próprio present, não o nosso submit extra. Agora existe um trinco por fila (`queue_lock.rs`) que os submits do jogo (os ganchos `vkQueueSubmit` e
`vkQueueSubmit2`) e os nossos (efeitos, criação da cadeia, captura de tela) passam a respeitar. O present em si não é trancado (ele pode bloquear).

## DXVK novo e o present (2026-11-07)
O DXVK do Proton Experimental (3.1.1-97) cria o swapchain com `PRESENT_ID_2`/`PRESENT_WAIT_2` e usa `VK_EXT_swapchain_maintenance1` (fence de apresentação e modo
de apresentação em cada present: estruturas `1000275001`/`1000275003`, mais `1000479001`); o GE-Proton 11-7 (DXVK 3.1-19) não. O preset pesado só travava no Experimental. A layer
trocava os semáforos de espera do present pelo próprio. Modo `resignal` (padrão desde 2026-11-07; `LINUX_RESHADE_RESIGNAL=0` ao abrir o jogo ou `lrctl depth policy resignal=off` voltam ao antigo): o envio
dos efeitos espera os semáforos do jogo e os sinaliza de novo, e o present continua esperando os que o jogo entregou. O `vkCreateDevice`, o pedido de swapchain e os primeiros
presents vão para o log (extensões, filas e estruturas encadeadas) para comparar os dois Protons.

**Resultado (2026-11-07):** com `resignal` o Slime Rancher 2 no Proton Experimental, preset `Padrão` de 9 efeitos, passou do título e do carregamento sem Xid (antes travava sempre em até 3 minutos). Foi o que resolveu o travamento nesse cenário.

## 2026-11-07 (tarde): A Hat in Time trava com o Proton Experimental, não com o GE-Proton

- Preset pesado de 9 efeitos (MartysMods, com profundidade) no A Hat in Time: Xid 69 (`c797`, offset `2158`, data `9e04c002`) 8 vezes em ~2 horas no
  **Proton Experimental**, mesmo com o modo `resignal`. Com o **GE-Proton** o mesmo preset, as mesmas trocas de definição e a mesma sessão não travam.
- Não é a troca de definições: um dos Xid veio 35 s depois da última reconstrução da cadeia, e vários vieram quando o jogo criava muitos
  pipelines (área nova). Também não é o lsfg-vk (desligado), nem o cache (limpo), nem a compilação (8 threads, prioridade normal).
- O pedido de swapchain e o present têm as mesmas estruturas encadeadas nos dois Protons (`swapchain_maintenance1`, fence de apresentação,
  `present_id`) e as 53 extensões do dispositivo são iguais: a diferença está dentro do DXVK (3.1.1-97 do Experimental x 3.1-19 do GE).
- Descartado: reconstrução incremental com a cadeia velha desenhando (corrigida), `LINUX_RESHADE_QUIET_SWAP=1` (cadeia velha parada na criação),
  o travamento do PC inteiro ao rodar com a validation layer (não repetir).
- Aberto: bisseção no Experimental com `lrctl depth policy` (split/copy/snapshot/resignal) e com presets sem profundidade, para saber se é o caminho da
  profundidade ou o do present. Recomendação enquanto isso: GE-Proton.

### Fechamento do dia (07/11): é o Proton Experimental, não a layer nem os shaders

- No GE-Proton o A Hat in Time aguenta qualquer troca de definição (macros de profundidade e `DIFFUSE_GI_Q`, rápidas ou lentas), com o mesmo preset.
  No Proton Experimental, qualquer troca de definição (que recria a cadeia) leva ao Xid 69 em 2 a 35 s; o preset completo sem trocas fica estável.
- Os 4 conjuntos de shaders que a layer carregou nos travamentos rodam 600 quadros a 1920x1080 sem erro no harness (`LINUX_RESHADE_CHAIN_BUNDLES`),
  com profundidade D24S8: não são os shaders. A captura de profundidade é idêntica nos dois Protons.
- Descartado por teste: destruir a cadeia velha antes/depois (`LATE_FREE`, `SAFE_SWAP`), a cadeia velha desenhando junto (`QUIET_SWAP`), reaproveitar efeitos
  entre cadeias, recriar tudo do zero (agora o padrão para mudança de macros), lsfg-vk, cache, threads de compilação.
- Em aberto: o que o DXVK do Experimental faz que o do GE não. Recomendação: GE-Proton para usar a layer com troca de definições ao vivo.
