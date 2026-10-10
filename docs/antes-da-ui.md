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
