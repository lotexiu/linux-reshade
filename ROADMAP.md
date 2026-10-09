# Roadmap

Marcos em ordem. Cada um deve rodar de ponta a ponta antes do próximo. Atualizar incrementalmente.

0. **Ambiente.** Instalar Rust (rustup), SDK/ferramentas Vulkan (`vulkaninfo`, `glslc`, `spirv-val`),
   e um jogo ou demo Vulkan simples para teste. Opcional agora: `gamescope`. Criar o workspace Cargo
   e `git init`.
   *Feito em 2026-10-09:* git, Rust 1.99 (asdf, fixado em `.tool-versions`), `scripts/dev.sh`. *Falta:* `sudo apt-get install
   build-essential vulkan-tools ...` (ver `scripts/dev.sh setup`).
1. **Layer passthrough.** *(parte feita em 2026-10-09: layer carrega no `vkcube` sob Wayland/NVIDIA, repassa tudo e registra instance/device/swapchain/present; `scripts/dev.sh smoke` verifica. Flag de efeitos em memória compartilhada (crate `ipc`) escrita pelo `lrctl` e lida pela layer a cada present, coberta pelo `smoke`. Atalho global (Ctrl+Shift+F10) via portal GlobalShortcuts, em `lrctl daemon`, testado com `scripts/dev.sh hotkey`: ciclo ligar/desligar visto pela layer. Verificado também dentro do container `soldier` da Steam (`scripts/dev.sh smoke-steam`): layer carregada, estado em `/dev/shm` visível, flag respeitada. Falta: Proton de verdade (sniper), jogos de 32 bits, e o passe que inverte as cores (marco 2).)* Layer Vulkan em Rust (`ash`) que só repassa tudo e é carregada num teste.
   Critério de aceite: `scripts/dev.sh run` mostra a layer no `VK_LOADER_DEBUG=layer` do `vkcube`, o
   log registra cada Present, e um processo separado liga/desliga uma flag em memória compartilhada
   (inverter cores quando ligada) com atalho global. Inclui manifesto JSON, ativação por variável de ambiente (para não rodar em todo processo
   Vulkan) e teste dentro do container do Steam (pressure-vessel).
2. **Passe fullscreen fixo.** *(feito em 2026-10-09: crate `postfx` + integração na layer; Ctrl+Shift+F10 alterna entre normal e invertido no `vkcube`; `scripts/dev.sh validate` roda com a validation layer sem mensagens. Pendente: espaço de cor sRGB, desempenho medido, jogos reais.)* Ler o swapchain e escrever nele (por exemplo inverter cores).
3. **Bundle + um `.fx` simples** *(feito em 2026-10-09: formato v1 em `docs/bundle-format.md`; `tools/fx2spv` (C++, reshadefx BSD-3) compila FX para bundle; crate `fx-bundle` lê e valida; `postfx::Effect` executa; a layer roda o bundle de `LINUX_RESHADE_BUNDLE`. 15 efeitos executam na GPU de software com a validation layer sem mensagens (13 da comunidade, Sépia e Inversão); 5 são recusados com motivo explícito: 3 usam depth (marco 5) e 2 precisam carregar imagem de arquivo. Pendente: carregador de imagens, mipmaps, swapchain sRGB, recompilar o bundle quando a resolução muda.)* (só cor, sem depth). Define o formato do manifesto: passes,
   texturas, samplers, uniforms, técnicas, anotações para a UI, semânticas `COLOR` e `DEPTH`.
4. **App de controle** *(4a feito em 2026-10-09: `lrctl uniforms|get|set|reset` altera parâmetros em tempo real por um bloco seqlock em memória compartilhada, documentado em `docs/ipc.md`. 4b feito: `lrctl use` escolhe o efeito e a layer o troca em tempo de execução, sem editar o Steam. 4c feito: a layer informa resolução, formato e estado de cada swapchain (`lrctl layers`), `lrctl use <Nome>` escolhe o efeito por nome e o vigia (no `daemon`) o compila para cada resolução que aparecer, verificado com dois `vkcube` de tamanhos diferentes (`scripts/dev.sh smoke-auto`). 4d feito: `lrpanel`, interface GTK4 (efeito, parâmetros gerados das anotações `ui_*`, jogos e estado, interruptor de efeitos), com a lógica num crate independente de toolkit (`panel`); verificada de ponta a ponta com um `vkcube` real (`scripts/dev.sh panel-smoke`).)* conversando por socket: parâmetros, presets, recompilação quando um
   `#define` muda (hot reload do bundle).
   *Próximo (`docs/presets.md`; F0 feita: pasta de dados, `config.ini`, `lrctl config`):* efeito e parâmetros por jogo, presets em pasta configurável
   (`preset.ini` global e uma pasta por jogo), vários efeitos encadeados em ordem, paridade com o ReShade.
5. **Depth buffer.** O diferencial. Rastreio de imagens/draws/layouts, escolha heurística,
   cópia antes do clear, Z invertido, formatos, MSAA. Testar com jogos Vulkan nativos e D3D no Proton.
6. **Depois, sem ordem fixa:** formato nativo de efeito além do FX, backend OpenGL, backend Windows,
   fusão de passes e outras otimizações.

## Regra para as libs

Nenhuma lib é criada antes de um marco precisar dela. Lista de candidatas, gatilhos e política de
terceiros em `DEPENDENCIAS.md`.

## Como começar o código (quando for a hora)

- Um **workspace Cargo** na raiz deste diretório. Começar com poucos crates (layer, app de
  controle). Crates novos aparecem quando um marco pedir.
- Libs nascem **dentro do workspace**, como crates internos. Só vão para repositório próprio
  (como o `@lotexiu/typescript`) depois que a API estabilizar.
- Spikes e benchmarks em `.tmp/`, fora do git, no estilo do `@lotexiu/typescript`.
