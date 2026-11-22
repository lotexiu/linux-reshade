# Caminhos combinados (a retomar)

Ordem combinada em 08/11/2026:
1. **Teste no A Hat in Time** com a ferramenta `GE-Proton11-7-dxvk-exp` (GE-Proton com o DXVK do Proton Experimental): trocar macros 10 vezes (LOGARITHMIC, REVERSED,
   `DIFFUSE_GI_Q`), 30 s entre elas, `LINUX_RESHADE_LOG=/tmp/lr.log`. Se travar, a causa é o DXVK do Experimental; senão, outra parte do Proton.
2. **Atualizar a máquina para o Fedora 45** (a versão nova que outras pessoas devem começar a usar) e conferir instalação, layer, atalhos e AppImage nela.
3. **Testar o contador de desempenho e os efeitos básicos em jogo** (ver `docs/efeitos-embutidos.md`).
4. Facilitar o uso e preservar FPS (ver abaixo).

## Facilitar o uso
- Botão que copia `LINUX_RESHADE=1 %command%` para as opções de inicialização da Steam.
- Lembrar a profundidade escolhida por jogo (`game.ini`); combinar imagem + ponto de captura (hoje a escolha é exclusiva no arquivo).
- Lista de passes sem corte (campo estruturado no status em vez do texto de 700 caracteres).
- `lrctl doctor` completo e "Exportar diagnóstico" (distribuição, driver, Vulkan, layer, spirv-opt, portal, Xid recentes, log da layer).
- Aviso no painel para o Proton Experimental (troca de definições ao vivo pode derrubar a GPU) e detecção de Xid.
- Presets prontos por faixa (Leve, Médio, Pesado); importar/exportar presets do ReShade.

## Preservar FPS (ideias, da que mais ajuda à que menos)
1. Mostrar o custo de GPU de cada efeito no painel (a layer já mede por etapa).
2. Guarda de FPS: se o tempo de quadro passar de um limite, reduzir a qualidade (macro) ou desligar o efeito mais caro, e voltar quando sobrar folga.
3. Pular trabalho que não aparece (menus sem cena 3D; só copiar profundidade se algum efeito a usa).
4. Texturas temporárias de efeitos diferentes dividindo a mesma memória.

## Adiados
- Ubuntu (visto em um teste: atalhos não funcionaram e as mudanças de efeito só valiam na abertura do jogo): depois do Fedora 45; ver `docs/instalacao.md`.
- Seleção automática de profundidade por pontuação e estabilidade (média móvel por estratégia); só depois de dados de vários jogos.
