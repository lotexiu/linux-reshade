# Caminhos combinados (a retomar)

Ordem combinada em 08/11/2026:
1. ~~Teste no A Hat in Time com `GE-Proton11-7-dxvk-exp`~~ **feito (08/11): não travou**; o DXVK do Experimental não é a causa, sobra o resto do Proton Experimental (ver `docs/depth.md`).
2. **Atualizar a máquina para o Fedora 45** (a versão nova que outras pessoas devem começar a usar) e conferir instalação, layer, atalhos e AppImage nela.
3. **Testar o contador de desempenho e os efeitos básicos em jogo** (ver `docs/efeitos-embutidos.md`).
4. Facilitar o uso e preservar FPS (ver abaixo).

## Antes do resto: arrumar a casa (pedido de 08/11/2026; o usuário vai atualizar a máquina para o Fedora 45 antes de seguir)
1. ~~**Troca de idioma que não aplica direito:**~~ **feito (22/11)**: a janela de Configurações não era do programa (`application()` vazio), então o pedido de troca nunca chegava à principal, e depois, ao associá-la, sobrava uma Configurações antiga; o modelo também guardava textos já traduzidos. Corrigido e coberto por `lrpanel --switch-language` no `ui-smoke`. Descrição original: a janela de Configurações atualiza, mas a janela principal não (a troca cria uma instância nova na configuração em vez de
   atualizar a principal). Validar com calma o fluxo inteiro (catálogo, `LR_LANG`, `ui.ini`, janela principal, páginas em pilha, diálogos abertos) e cobrir com teste.
2. **Reorganização do projeto**, para ficar mais intuitivo e fácil de manter: estrutura de pastas e módulos, menos código repetido, legibilidade, fronteiras claras entre
   crates (layer, postfx, painel, gui, control, ipc...). O usuário acha este projeto o mais diferente e menos intuitivo entre os que conhece.
   **Antes de mexer, fazer uma pesquisa** de como projetos maiores parecidos organizam isso (layers Vulkan, workspaces Rust grandes, apps GTK) e propor o desenho;
   só então migrar, em passos pequenos e com os testes passando a cada um.

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

## Pendente do overlay
- O ícone de "processando" no jogo existe (marcador `busy-*` na pasta de estado enquanto o vigia compila, e a cadeia sendo montada); falta ver em jogo se aparece nos momentos certos.
