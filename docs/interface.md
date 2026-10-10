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

## Notas de projeto
- Há duas "telas" lógicas, diferente do ReShade: a de **adicionar efeitos** (o que existe nas pastas de shaders) e a
  **dos efeitos do preset**, onde se liga e desliga sem remover. Por isso remover é "esquecer" (apaga os valores).
