# Efeitos antes da interface do jogo (desenho)

Problema (relatado pelo dono): os efeitos pegam a UI do jogo (o `CinematicDOF` borra o texto do menu). Hoje a cadeia
roda em `vkQueuePresentKHR`, sobre a imagem final, que já tem a UI.

## Ideia: foto da cena + máscara

Sem inserir comandos no meio do command buffer do jogo (arriscado: sincronização, formatos HDR, MSAA):

1. Quando o passe que termina a cena na imagem do swapchain acaba (o último antes da UI), a layer grava no próprio
   command buffer uma cópia da imagem para uma imagem sua `S` (como já faz com a profundidade, `snapshot.rs`).
2. No present a cadeia roda sobre a cópia da cena (`S`), não sobre a imagem final `O`.
3. Um passe final compõe na imagem do swapchain: onde `O == S` (nada da UI desenhou) vale o resultado do efeito `E`;
   onde `O != S` (a UI desenhou, inclusive translúcida) vale o `O` original.

Custo: uma cópia a mais por quadro e um passe de composição; só no modo "antes da UI". Efeitos que usam profundidade
continuam iguais (a profundidade é a da cena).

Limites ditos de antemão: a UI translúcida sobre a cena (um painel escurecido, como o menu de pausa) fica sem efeito
(correto: é UI) mas a cena embaixo dela também (`O != S`); jogos que desenham a UI numa imagem própria e a compõem
depois não têm um "antes da UI" na imagem do swapchain (nesse caso o modo não se aplica).

## Passos

1. **Observar** (feito): `depth/screen.rs` anota os passes que desenham no swapchain (quantos desenhos, `loadOp`) e os
   publica em `lrctl layers` na linha `tela: ...`. Serve para ver, em cada jogo, onde a cena termina e a UI começa.
2. Escolher o ponto (primeiro passe no swapchain; os seguintes com `Load` são UI) e gravar a cópia `S` ali.
3. A cadeia sobre `S` e a composição por máscara (passe novo no `postfx`).
4. Opção por jogo no painel ("aplicar antes da UI") e por efeito, se fizer sentido.

## Medido no Haste (2026-10-10): a ideia da máscara não serve nele

`lrctl layers` mostrou, para a imagem do swapchain num quadro da tela inicial (cena 3D + texto):
`barreira->COLOR_ATTACHMENT, barreira->COLOR_ATTACHMENT, passe de 1 desenho com Load, barreira->PRESENT`. Nenhum blit,
cópia ou resolução, e um único passe na tela, com um desenho só (um quadrado cheio): é o passe final do jogo (o
FSR/escala amostrando uma imagem em resolução de render). **A UI já está dentro dessa imagem de render**, desenhada pelos
passes anteriores; no swapchain não há "antes da UI" para fotografar: `O == S` sempre.

Para esse tipo de jogo (Unity URP com UI em overlay na imagem da câmera) só funciona injetar o efeito **dentro do
fluxo do jogo**, na imagem de render, depois do último passe que usa a profundidade e antes dos passes de UI (que
carregam essa imagem com `Load`). Isso pede: gravar comandos no command buffer do jogo (a cadeia hoje tem command
buffers prontos por imagem do swapchain), sincronizar os uniforms por quadro em voo, acrescentar usos (`SAMPLED`/
`STORAGE`) à imagem de render na criação (modo armado), e lidar com formato HDR (efeitos do ReShade supõem LDR) e com a
resolução de render menor que a tela. É um passo grande e específico por jogo; fica **adiado** até a profundidade por
jogo (D3) e a base de efeitos estarem firmes. O observador da tela (`depth/screen.rs`) continua útil para diagnosticar.

Pista do dono (a verificar): num efeito de raytracing do Marty com visualização de depuração, mexer nos parâmetros mostra
a diferença exatamente onde a UI do jogo está; pode indicar que a UI entra na imagem que o efeito lê. Testar com um
efeito de profundidade e um jogo aberto antes de investir na injeção no fluxo.
