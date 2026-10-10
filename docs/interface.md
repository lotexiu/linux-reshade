# Interface: o que existe e o que o dono pediu (backlog)

A janela segue a ideia da aba Home do ReShade e vai sendo reajustada com o uso (decisão do dono, 2026-10-10:
"o desenho de tela é difícil de pensar de início e se ajusta pela necessidade"). O que o dono pediu e ainda
**não** foi feito fica aqui, na ordem em que ele falou.

## Já feito
Jogo e preset no alto; busca; "Adicionar efeito"; lista de técnicas em ordem com caixa de seleção, setas,
remover e parâmetros; criar e **apagar** preset (com confirmação e "não perguntar novamente", guardado em
`~/.config/linux-reshade/ui.ini`); remover um efeito do preset **apaga as configurações dele** (desligar não);
a roda do mouse sobre um controle rola a lista (não mexe no controle).

## Pedido, ainda por fazer
1. **Lista "Adicionar efeito" mais útil.**
   - Efeitos incompatíveis (por pedirem um recurso que o runtime ainda não tem: depth, compute...) **em vermelho**.
   - Filtro de **multisseleção** por característica do efeito: usa profundidade, usa cor, imagens de arquivo, etc.
   - Precisa de uma análise de cada efeito (compilar em segundo plano e guardar um resumo no cache), porque o `.fx`
     sozinho não diz o que o runtime recusa. Hoje, ao adicionar um efeito que não compila, o aviso diz o motivo.
2. **Abas**, quando houver mais o que mostrar: **Efeitos** (o que existe hoje), **Configurações** (inclui voltar a
   perguntar antes de apagar um preset) e **Logs**.
3. **Layout em paisagem**: com a janela larga (maximizada), dividir em duas partes, efeitos ativos à esquerda e
   outra coisa à direita (a definir pelo que vier a existir). Hoje os controles esticam e sobra espaço.
4. **Tornar a janela mais intuitiva e fácil de usar** (remodelar). Sem detalhe ainda; o dono quer primeiro que
   funcione.
5. A janela trava enquanto compila ao abrir um preset grande (compilar em segundo plano).

## Observações do dono depois de usar no Haste (2026-10-10)
- **Efeito com várias técnicas = um só efeito** (feito): o `cGaussianBlur` aparecia como dois efeitos independentes e só
  o primeiro tinha os parâmetros. Agora é uma linha por efeito: caixa do efeito liga/desliga todas as técnicas, cada
  técnica tem a própria caixa, os parâmetros são do efeito, mover e remover valem para o efeito inteiro.
- **Cor dos incompatíveis**: manter a do tema, não forçar vermelho (feito).
- **Custo de cada efeito**: o `ZXPose.fx` derrubou um jogo de 200 para 166 fps. Idea: medir o tempo de GPU de cada
  técnica (consultas de timestamp na layer) e mostrar na lista, como a aba Statistics do ReShade. Também ajuda a achar
  os efeitos caros por mau uso (ver a pesquisa em reshade-feedback.md).
- **Efeitos que pegam a interface do jogo** (desfoque, motion blur...): é limitação do ReShade original também, mas
  talvez dê para fazer melhor. Caminho possível, depois do depth buffer: a mesma interceptação das passagens de
  renderização que o depth precisa serve para achar o ponto "a cena 3D acabou e a interface vai começar" e aplicar os
  efeitos ali (o jogo desenha a interface por cima do resultado). Depende de cada jogo e de heurística; a escolha
  ficaria por jogo no preset, com "aplicar antes da interface" ligável. Incerto: precisa de teste em jogos reais.

## Direção de design (referências que o dono mandou, 2026-10-10)
Referências: a própria interface do ReShade (abas Home, Settings, Statistics, Log, About; na Home: seletor de preset
com + e −, busca, "Active to top", "Collapse all", lista de técnicas e, embaixo, os ajustes do efeito com "Reset all to
default" e as definições globais de pré-processador; botão Reload; Performance Mode; na aba Settings: teclas,
caminhos de busca, capturas de tela, aparência), o **lsfg-vk** (lista de perfis à esquerda com Criar/Renomear/Apagar
embaixo, ajustes do perfil à direita) e o **Grimoire** (barra lateral de navegação, busca com filtros e visualização).
Ideias para a nossa janela, quando for a hora de remodelar:
- **Barra lateral à esquerda com os jogos** (como os perfis do lsfg-vk; ● nos abertos, "Preset global" no topo) e, embaixo
  dela, Criar/Renomear/Apagar preset. Isso também resolve o layout em paisagem.
- **Abas à direita**: Efeitos (o que existe hoje) | Configurações | Log | Sobre, nessa ordem de prioridade.
- Da Home do ReShade: "ativos primeiro", "recolher todos", definições de pré-processador globais e por efeito
  (macros, F6), botão de recarregar, "modo desempenho". Da aba Settings: atalhos, caminhos de shaders e texturas,
  capturas de tela.
- Filtros da lista de adicionar mais compactos (hoje ocupam muito): um botão "Filtros" com as opções.
- Nada disso precede o que faz o programa funcionar: depth buffer vem antes da remodelação.

## Notas de projeto
- Há duas "telas" lógicas, diferente do ReShade: a de **adicionar efeitos** (o que existe nas pastas de shaders) e a
  **dos efeitos do preset**, onde se liga e desliga sem remover. Por isso remover é "esquecer" (apaga os valores).
