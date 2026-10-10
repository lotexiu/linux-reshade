# O que as pessoas dizem do ReShade (pesquisa de 2026-10-10)

Pedido do dono: ver em fóruns e outros lugares o que incomoda no ReShade, o que ele já oferece e o que as pessoas
pedem, para orientar o projeto. A busca foi rápida (fóruns do próprio ReShade, vkBasalt, páginas da comunidade):
são relatos soltos e muitos antigos, **não** estatística. Serve de direção, não de prova.

## O que o ReShade oferece (e o projeto precisa igualar)
Injetor genérico de pós-processamento para D3D9/10/11/12, OpenGL e Vulkan, com a linguagem ReShade FX (sintaxe
HLSL) e licença BSD de 3 cláusulas; presets em `.ini` que se compartilham; interface no jogo para carregar
presets, ligar efeitos e ajustar parâmetros com sliders e interruptores
([visão geral](https://stepmodifications.org/wiki/Guide:ReShade), [TechSpot](https://www.techspot.com/downloads/7206-reshade.html)).
Nosso lado: formato de preset igual, FX compilado, interface parecida. O que falta está em
[docs/CONTINUAR.md](CONTINUAR.md).

## Reclamações recorrentes
- **Versões novas quebram jogos e shaders**: regressões entre versões, jogos que travam ao carregar um shader,
  shaders de versões antigas que deixam de compilar
  ([reclamações comuns](https://www.reshade.me/forum/troubleshooting/3293-common-issues-and-their-solutions?start=20),
  [4.9.1 e Warzone](https://reshade.me/releases/7022-4-9?start=60),
  [travado em "10 effects remaining"](https://reshade.me/forum/troubleshooting/2346-solved-reshade-hanging-at-hangs-at-10-effects-remaining)).
  *Para nós:* o runtime é o mesmo para todos os jogos e a compilação é feita fora do jogo (o jogo nunca trava
  esperando compilar), mas ainda falta mostrar claramente, por técnica, o que foi aplicado e o que foi pulado.
- **Muitos shaders = carregamento lento**: o número de shaders pesa sobretudo no tempo de carga, não nos FPS
  ([load time](https://reshade.me/forum/general-discussion/2494-reshade-3-load-time-is-affected-by-harddisc),
  [mais shaders, pior desempenho?](https://www.reshade.me/forum/troubleshooting/5465-more-shaders-downloaded-worse-performance)).
  *Para nós:* compilar fora do jogo e guardar no cache já ajuda; a análise em segundo plano é a mesma ideia.
- **Queda de FPS que aparece com poucos efeitos** e conflito com overlays de outros programas
  ([Star Citizen](https://reshade.me/forum/troubleshooting/9484-reshade-performance-hit-in-star-citizen),
  [FPS pela metade](https://reshade.me/forum/troubleshooting/2570-reshade-nearly-halving-framerate)).
  *Para nós:* cada etapa da cadeia copia o frame inteiro; **medir o custo na GPU real fica pendente** (já está em
  CONTINUAR.md) e vale antes de pôr cadeias longas em jogos pesados.
- **Detecção de depth buffer frágil**: "No depth buffers found" depois de atualizar, pega o buffer da interface em
  vez da cena 3D, falha com resoluções fora do padrão, MSAA atrapalha, opções de inverter/logarítmico que não
  resolvem ([não acha o depth](https://reshade.me/forum/troubleshooting/9964-reshade-cant-find-the-depth-buffer---what-should-i-do),
  [detecção](https://reshade.me/forum/general-discussion/4083-depth-buffer-detection-modifications?start=900),
  [Sims 3 no Proton](https://reshade.me/forum/troubleshooting/10309-reshade-d3d9-depth-buffer-failing-with-msaa-enabled-in-the-sims-3-linux-proton)).
  *Para nós (marco 5):* é o ponto mais delicado. Convém deixar a escolha do buffer **visível e ajustável por jogo**
  (qual imagem foi escolhida, trocar para outra, inverter/linearizar), e guardar a escolha no preset do jogo.

## Pedidos de interface (o que as pessoas querem do overlay)
- **Ver o que dá para fazer**: reordenar arrastando é funcionalidade escondida; "o usuário deve sempre ver o que
  pode fazer" ([discussão da 3.0](https://www.reshade.me/forum/general-discussion/1977-reshade-3-0-progress-discussion?start=20)).
  *Para nós:* já temos setas visíveis; arrastar entra além delas, não no lugar.
- **Descrição de cada shader e de cada ajuste**, **botão de restaurar os padrões** por shader, clicar no nome para
  abrir os ajustes, janela do jogo continuando ativa ao fechar o overlay
  ([melhorias de qualidade de vida](https://reshade.me/forum/suggestions/3702-four-quality-of-life-improvements-for-the-overlay)).
  *Para nós:* restaurar por efeito já existe; as descrições vêm das anotações `ui_tooltip` do próprio efeito e
  ainda não aparecem na lista de adicionar.
- **Busca que se perde** ao trocar de janela, e janela que precisa ser redimensionada toda vez
  ([feedback pequeno da 3.0.5](https://www.reshade.me/forum/general-discussion/2589-reshade-3-0-5-some-feedback-small-issues?rCH=2)).
  *Para nós:* a janela é separada do jogo (não some ao mudar de foco); lembrar tamanho e posição entre execuções
  ainda não está feito.
- **Gestão de presets e shaders**: presets reproduzíveis que dizem quais shaders usam; "instalar a partir do preset"
  (baixar os shaders que o preset cita); trocar de preset por atalho; organizar a lista por categoria
  ([proposta de gestão](https://www.reshade.me/forum/general-discussion/2041-shader-preset-management-proposition),
  [instalar a partir do preset](https://reshade.me/forum/suggestions/7082-feature-request-install-from-preset),
  [preset por atalho](https://www.reshade.me/forum/suggestions/8175-specific-preset-selection-via-shortcut-key?rCH=2),
  [interface mais amigável](https://reshade.me/forum/suggestions/5466-new-user-friendly-interface-concept)).
  *Para nós:* já avisamos de técnica que não existe; "mostrar quais efeitos faltam e de onde pegar", atalho de
  preset e categorias na lista de adicionar são boas candidatas (a F6 já prevê o atalho de preset).
- **Opcional, não obrigatório**: gerenciar tudo na interface do jogo agrada, desde que seja opcional. Nosso modelo
  (arquivos `.ini` editáveis à mão **e** painel) já atende.

## Linux
O ReShade em si roda no Wine/Proton, mas não em jogos nativos; no Linux o caminho é o **vkBasalt**, que cobre só
Vulkan e tem limites declarados: shaders de várias técnicas não funcionam, o acesso ao depth é experimental e "pode
dar muitos problemas", e o buffer escolhido nem sempre é o desejado
([vkBasalt](https://awesome.ecosyste.ms/projects/github.com%2Fdadschoorse%2Fvkbasalt),
[vkBasalt_overlay](https://gitblind.noratr.app/Boux/vkBasalt_overlay),
[Linux version](https://www.reshade.me/forum/general-discussion/9312-linux-version?rCH=-2)).
No Steam Deck há interesse e soluções soltas (Gamescope com ReShade; o LetMeReShade, arquivado)
([Steam Deck](https://steamcommunity.com/app/1675200/discussions/0/7529517456176879563/),
[LetMeReShade](https://alternativeto.net/software/letmereshade/about)).
*Para nós:* o espaço que o projeto ocupa é exatamente este: **Vulkan nativo e via Proton/DXVK, com várias
técnicas por efeito, presets por jogo e uma interface própria**. Os limites do vkBasalt (várias técnicas, depth) são
o que já tratamos (cadeia) e o que vem a seguir (depth).

## O que isso muda na ordem do trabalho
1. Primeiro, o que já está no plano (análise dos efeitos, estado por técnica, compilar sem travar a janela):
   responde diretamente à reclamação de "não sei o que está sendo aplicado" e de carregamento lento.
2. No depth buffer, **projetar a escolha do buffer como algo visível e ajustável por jogo** desde o começo, porque é
   onde o ReShade mais frustra as pessoas.
3. Candidatos novos, de custo baixo: descrições (`ui_tooltip`) na lista de adicionar, lembrar tamanho e posição da
   janela, "o que falta para este preset" (efeitos e texturas não encontrados).

*Limites desta pesquisa:* poucas buscas, quase todas em fóruns do ReShade; sem dados de uso; vários relatos têm
anos. Vale repetir, mais a fundo, antes de decidir o desenho do depth buffer.
