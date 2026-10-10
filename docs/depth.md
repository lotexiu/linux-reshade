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

## Passos (cada um verificável e commitado)
- **D1, só observar:** ganchos de `vkCreateImage`, `vkCreateImageView`, `vkCreateFramebuffer`, `vkCreateRenderPass`(2),
  início/fim de passe (os dois tipos), `vkCmdExecuteCommands` e desenhos; contadores baratos e ligados só quando pedido
  (variável de ambiente no D1); as candidatas aparecem em `lrctl layers` com os dados do item 8. Teste: `vkcube`
  (uma candidata, com desenhos) e, depois, o Haste. Nenhuma mudança no que o jogo desenha.
- **F6, macros que recompilam:** `PreprocessorDefinitions` nos três níveis (global, preset, efeito) chegam ao
  `fx2spv -D` e invalidam o bundle.
- **D2, copiar e entregar:** aprender num quadro, copiar uma vez no fim do passe no seguinte, corrigir `storeOp`, e a textura `DEPTH` no `postfx`. Teste: um efeito
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
