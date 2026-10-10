# Depth buffer (marco 5): desenho

Pedido do dono: o depth buffer é o diferencial do projeto; 38 dos 150 efeitos da pasta dele pedem depth. Pesquisa
(docs/reshade-feedback.md): no ReShade a detecção é o ponto que mais frustra (não acha, pega a interface em vez da
cena, MSAA atrapalha). No vkBasalt o depth é experimental e "pode dar muitos problemas". Por isso o desenho abaixo
torna a escolha do buffer **visível e ajustável por jogo**. Teste de referência: Haste (Unity, Vulkan nativo, tela
inicial em 3D, abre rápido) e o `vkcube` (tem depth).

## Como a layer acha o depth (heurística, como o ReShade)
1. `vkCreateImage`: guarda as imagens com formato de profundidade (`D16`, `D32`, `D24S8`, `D32S8`...) e uso
   `DEPTH_STENCIL_ATTACHMENT`. A layer acrescenta `TRANSFER_SRC` (e `SAMPLED`) ao uso das candidatas, como já faz com o swapchain.
2. `vkCmdBeginRenderPass` e `vkCmdBeginRendering` (renderização dinâmica): ligam a imagem de profundidade usada ao passe.
3. `vkCmdDraw*`: conta desenhos e vértices por imagem de profundidade, por quadro.
4. Candidata = a imagem com mais desenhos/vértices no quadro, com proporção parecida com a do swapchain (a do ReShade
   também assume resolução parecida; resoluções estranhas são um motivo conhecido de falha).
5. A layer **informa as candidatas** no status (tamanho, formato, desenhos, qual foi escolhida) e o painel as mostra.

## Como o efeito recebe o depth
- Ao fim do último passe de renderização que escreve a imagem escolhida (antes de o jogo a limpar ou reutilizar), a
  layer grava no command buffer do jogo uma **cópia** dela para uma imagem de profundidade própria (barreiras antes e
  depois). Copiar no meio do command buffer do jogo é a parte mais delicada: precisa de teste com a validation layer.
- No `postfx`, a textura com semântica `DEPTH` aponta para essa cópia (vista do aspecto de profundidade) em vez da
  recusa de hoje.
- O ReShade ajusta o depth por **macros** do preset: `RESHADE_DEPTH_INPUT_IS_REVERSED`, `..._UPSIDE_DOWN`,
  `..._LOGARITHMIC`, `RESHADE_DEPTH_LINEARIZATION_FAR_PLANE`. Isso depende da **F6** (macros que recompilam o efeito):
  ela entra antes de o efeito poder usar o depth direito.

## Passos (cada um verificável e commitado)
- **D1, só observar:** ganchos de `vkCreateImage`, início de passe e desenho; as candidatas aparecem em `lrctl layers`.
  Teste: `vkcube` (uma candidata, com desenhos) e, depois, o Haste. Nenhuma mudança no que o jogo desenha.
- **F6, macros que recompilam:** `PreprocessorDefinitions` nos três níveis (global, preset, efeito) chegam ao
  `fx2spv -D` e invalidam o bundle.
- **D2, copiar e entregar:** a cópia no command buffer do jogo e a textura `DEPTH` no `postfx`. Teste: um efeito
  simples que desenha a profundidade como cinza (o `DisplayDepth.fx` do próprio ReShade) no vkcube e no Haste.
- **D3, escolha no painel:** lista das candidatas por jogo (tamanho, formato, desenhos), "automático" ou fixar uma, inverter/logarítmico/
  distância do plano distante; a escolha vai para o preset do jogo.

## Riscos (ditos de antemão)
- Jogos que reusam a mesma imagem de profundidade para a cena e para a interface, ou que a limpam antes de o quadro acabar:
  a cópia tem de acontecer no ponto certo, e a heurística pode errar.
- Jogos com depth em formato que não pode ser copiado como está ou sem `SAMPLED`: tratar como indisponível, com o motivo.
- MSAA: depth multiamostrado precisa de resolve; fora do primeiro passo.
- Custo: uma cópia por quadro da imagem de profundidade.
- Isso mexe em command buffers do jogo: o mais arriscado de todo o projeto; exige a validation layer em cada passo.
