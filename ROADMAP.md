# Roadmap

Marcos em ordem. Cada um deve rodar de ponta a ponta antes do próximo. Atualizar incrementalmente.

0. **Ambiente.** Instalar Rust (rustup), SDK/ferramentas Vulkan (`vulkaninfo`, `glslc`, `spirv-val`),
   e um jogo ou demo Vulkan simples para teste. Opcional agora: `gamescope`. Criar o workspace Cargo
   e `git init`.
   *Feito em 2026-10-09:* git, Rust 1.99 (asdf, fixado em `.tool-versions`), `scripts/dev.sh`. *Falta:* `sudo apt-get install
   build-essential vulkan-tools ...` (ver `scripts/dev.sh setup`).
1. **Layer passthrough.** *(parte feita em 2026-10-09: layer carrega no `vkcube` sob Wayland/NVIDIA, repassa tudo e registra instance/device/swapchain/present; `scripts/dev.sh smoke` verifica. Falta: flag em memória compartilhada + atalho global, e teste no pressure-vessel do Steam.)* Layer Vulkan em Rust (`ash`) que só repassa tudo e é carregada num teste.
   Critério de aceite: `scripts/dev.sh run` mostra a layer no `VK_LOADER_DEBUG=layer` do `vkcube`, o
   log registra cada Present, e um processo separado liga/desliga uma flag em memória compartilhada
   (inverter cores quando ligada) com atalho global. Inclui manifesto JSON, ativação por variável de ambiente (para não rodar em todo processo
   Vulkan) e teste dentro do container do Steam (pressure-vessel).
2. **Passe fullscreen fixo.** Ler o swapchain e escrever nele (por exemplo inverter cores).
3. **Bundle + um `.fx` simples** (só cor, sem depth). Define o formato do manifesto: passes,
   texturas, samplers, uniforms, técnicas, anotações para a UI, semânticas `COLOR` e `DEPTH`.
4. **App de controle** conversando por socket: parâmetros, presets, recompilação quando um
   `#define` muda (hot reload do bundle).
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
