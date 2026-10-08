# linux-reshade (nome provisório)

Pós-processamento de jogos **nativo no Linux**, com uso quase idêntico ao ReShade do Windows, e
com acesso ao **depth buffer** (o ponto que as alternativas atuais no Linux não cobrem bem).

> Estado: **só anotações** (2026-10-08). Nenhum código, nenhum `cargo init`, nenhum repositório git
> ainda. Este diretório existe para a ideia não se perder. Ver `ROADMAP.md` e `DEPENDENCIAS.md`.

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
