# linux-reshade (nome provisório)

Pós-processamento de jogos **nativo no Linux**, com uso quase idêntico ao ReShade do Windows, e
com acesso ao **depth buffer** (o ponto que as alternativas atuais no Linux não cobrem bem).

> Estado: **marco 1 em andamento** (2026-10-09): a layer passthrough já carrega no `vkcube`. Rust 1.99 via asdf (`.tool-versions`), `scripts/dev.sh` criado; workspace Cargo com `vk-layer` e `layer`. Ver `ROADMAP.md` e `DEPENDENCIAS.md`.

## Por que existe

O ReShade depende de hook binário e de DLLs do Windows, e não roda nativamente no Linux. No Linux
o caminho oficial é outro: **layers Vulkan**. Como DXVK (D3D9/10/11) e VKD3D-Proton (D3D12)
traduzem D3D para Vulkan, uma layer Vulkan vê jogos Vulkan nativos **e** jogos Windows no Proton,
sem injeção de DLL.

Objetivo secundário e deliberado: os módulos que esta ferramenta exigir viram **libs próprias
reutilizáveis** (ver `DEPENDENCIAS.md`). As libs nascem da necessidade do projeto, não antes dela.

## Decisões tomadas

| Decisão | Motivo |
|---|---|
| Linux nativo primeiro; Windows talvez um dia | O Windows exigiria hooks por API (D3D11/12), muito mais trabalho. Vulkan layer também existe no Windows, então não fecha a porta. |
| **Layer Vulkan implícita**, sem hook binário | Mecanismo oficial do loader (manifesto JSON + `vkNegotiateLoaderLayerInterfaceVersion`). |
| Linguagem principal: **Rust** | O rastreio de depth é máquina de estados (enums + `match` exaustivo ajudam). A layer roda dentro do jogo, então crash e data race derrubam o jogo. Núcleo reaproveitável (Blender via PyO3, Godot via gdext etc.). |
| **C++ só no front-end de FX**, como **ferramenta separada** | Compatibilidade com os `.fx` da comunidade depende de reproduzir o compilador do ReShade, que é C++. **Nunca linkado na layer**, nenhum C++ roda dentro do processo do jogo. |
| Layer lê **bundle pronto** (SPIR-V + manifesto), não compila FX | Menos código no processo do jogo, carga mais rápida, hot reload feito de fora. |
| **FX como entrada** (compatibilidade); formato nativo próprio **depois** | A biblioteca de efeitos em FX é o valor. Formato novo só quando o runtime já funcionar. |
| Controle por **processo separado** (socket Unix / memória compartilhada) | Layer não recebe input, e no Wayland não há captura global. App de controle (egui) edita parâmetros e presets e recompila quando `#define` muda. |
| Interface do backend atrás de uma **trait** | Mantém o núcleo (grafo de efeitos, parâmetros, presets) independente de Vulkan. |
| **Efeitos rodam na layer; só controle e UI ficam fora** (2026-10-09) | Passes gravados no mesmo command buffer, antes do `vkQueuePresentKHR`: zero frame de atraso (sem ghosting). Processo separado exigiria dma-buf + semáforos e pelo menos um frame de latência. Parâmetros vão por memória compartilhada, lidos uma vez por frame. Socket só para comandos raros. |
| **A layer não toca em input** (2026-10-09) | Reduz superfície no processo do jogo (anti-cheat) e simplifica a layer. Captura de teclado/mouse e atalhos ficam no app de controle. |
| **Wayland é requisito**, X11 é o ambiente atual (2026-10-09) | Atalhos globais precisam de backends trocáveis atrás de uma trait: X11 (XInput2/XGrabKey), portal GlobalShortcuts e/ou evdev no Wayland. Verificado em 2026-10-09 (GNOME 50.1): o frontend (`GlobalShortcuts` v1) e o backend `xdg-desktop-portal-gnome` expõem a interface; o comportamento real (diálogo de permissão, atribuição da tecla pelo usuário, `app_id` de processo fora de sandbox) ainda não foi testado. |
| Scopes de atalho: `Global` e `Janela visível` | `Global` = atalho registrado no sistema. `Janela visível` = eventos normais da janela do app de controle quando em foco. |
| **Depth é opt-in por jogo**, desligado por padrão (2026-10-09) | É a feature mais sensível para anti-cheat (permite efeitos que "enxergam" através de paredes). |
| OpenGL nativo **fica para depois** | Não há mecanismo de layer. Exigiria `LD_PRELOAD` e hook de `glXSwapBuffers`/`eglSwapBuffers`, ou Zink. |

## Arquitetura alvo

```
 ferramenta de build (C++)         layer Vulkan (Rust, dentro do jogo)        app de controle (Rust)
 fx2spv: .fx ──▶ bundle ──────────▶ carrega bundle, roda os passes   ◀─socket─▶ UI, presets, recompilação
        (SPIR-V + manifesto)         rastreia depth, escreve no swapchain       (egui)
```

Compatibilidade de depth com FX: o FX pede o depth como textura com semântica `DEPTH` e o
backbuffer com `COLOR`. O manifesto precisa registrar essas semânticas e a layer fornece a textura.

## Diferencial: depth buffer

Não existe "API de depth". A layer precisa acompanhar criação de imagens com formato de depth,
`vkCmdDraw*`, attachments (render pass ou dynamic rendering) e transições de layout, escolher qual
é o depth do mundo 3D por heurística e **copiar a imagem antes de o jogo limpá-la**. Também há Z
invertido, formatos variados e MSAA. É heurística por jogo, e deve ter módulo próprio e testável.

## A verificar (veio de memória, não foi checado)

- Se o `vkBasalt` realmente não lê depth.
- Se o `gamescope` tem opção de efeitos ReShade (`--reshade-effect`) e até onde ela vai.
- Licença do código do ReShade (acredito BSD-3) e se o front-end de FX é reutilizável como ferramenta.
- Que a semântica de depth no FX é mesmo `: DEPTH`.

## Ambiente desta máquina (checado em 2026-10-08)

- Sessão **X11**; layers em `/usr/share/vulkan/implicit_layer.d/`: `nvidia_layers.json` e
  `VkLayer_MESA_device_select.json`. Há driver NVIDIA além do Mesa.
- **Não instalados:** `cargo`/`rustc`/`rustup`, `vulkaninfo`, `gamescope`, `glslc`, `dxc`,
  `spirv-val`, `cmake`, `g++`/`clang`. Instalar antes do primeiro marco.

## Perguntas em aberto

- Nome definitivo do projeto e licença.
- Hospedagem (os outros projetos estão no GitHub do `lotexiu`).
- Driver alvo inicial: NVIDIA (o que há aqui) ou Mesa também?
- No FX→SPIR-V: reaproveitar o front-end do ReShade, ou escrever o nosso depois?
