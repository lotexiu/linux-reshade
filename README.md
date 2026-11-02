# linux-reshade (nome provisório)

Pós-processamento de jogos **nativo no Linux**, com uso quase idêntico ao ReShade do Windows, e com acesso
ao **depth buffer** (o ponto que as alternativas atuais no Linux não cobrem bem; ainda não feito, marco 5).

> **Retomando o trabalho (outra máquina ou outra sessão do Claude)?** Leia primeiro
> [`docs/CONTINUAR.md`](docs/CONTINUAR.md): onde paramos, o que vem agora, combinados de trabalho e armadilhas.

## Estado (2026-10-09)

Funciona de ponta a ponta, verificado com `vkcube` e, antes, com jogos reais (Idols of Ash em Vulkan nativo,
Rocket League via Proton, só offline):

- **Layer Vulkan** implícita (Rust) que aplica efeitos FX no `vkQueuePresentKHR`, sem frame de atraso.
- **Compilador FX → bundle** (`tools/fx2spv`, C++, ferramenta separada) com os efeitos `.fx` da comunidade.
- **Controle em tempo real** por linha de comando (`lrctl`) e por interface gráfica GTK4 (`lrpanel`):
  escolher o efeito, ajustar parâmetros, ligar/desligar (atalho global **Ctrl+Shift+F10** no Wayland, configurável: botão do teclado no painel ou `[INPUT]` do `config.ini`; aceita também teclas como `'`/`OEM_7`, e atalhos de próximo/anterior preset).
- **Por jogo (F1):** efeito e parâmetros próprios de cada jogo aberto (`lrctl ... --layer <pid|nome>`),
  além da escolha para todos. A interface gráfica ainda não usa isso.
- **Pastas configuráveis (F0):** pasta de dados única (shaders, texturas, presets) que sobrevive a reinstalar
  o jogo ou formatar a máquina (`lrctl config init <pasta>`).

Em andamento: presets por jogo no formato do ReShade, vários efeitos em cadeia, interface por jogo.
Ver [`ROADMAP.md`](ROADMAP.md) e [`docs/presets.md`](docs/presets.md).

## Começar numa máquina nova

Requisitos: Linux com Vulkan (testado: Ubuntu 26.04, GNOME 50 Wayland, NVIDIA 595), `git` e `g++` (sem cmake).

```sh
git clone https://github.com/lotexiu/linux-reshade.git && cd linux-reshade
scripts/dev.sh setup        # Rust (asdf, versão em .tool-versions) + pacotes de sistema (pede sudo se faltar algo)
scripts/dev.sh doctor       # confere o ambiente
scripts/dev.sh fx-tool      # baixa as dependências fixadas do fx2spv e o compila (build/fx2spv/fx2spv)
scripts/dev.sh install      # compila a layer e instala o manifesto em ~/.local/share/vulkan/implicit_layer.d
cargo build --release       # lrctl e lrpanel em target/release/
```

Pacotes de sistema (Ubuntu): `build-essential pkg-config vulkan-tools vulkan-validationlayers glslang-tools
spirv-tools libgtk-4-dev`. Rust vem do `asdf` (ou `rustup`, se o `asdf` não existir).

Primeiro uso:

```sh
LINUX_RESHADE=1 vkcube --wsi wayland     # a layer só carrega com LINUX_RESHADE=1 (use --wsi wayland; ver "Achados")
lrctl daemon &                           # atalho global + compila os efeitos para cada janela
lrctl use Sepia && lrctl on              # escolhe um efeito e liga
lrpanel                                  # a interface gráfica
```

Em jogos da Steam, nas opções de lançamento: `LINUX_RESHADE=1 %command%` (`scripts/dev.sh game-options`).

## Uso

```sh
lrctl on | off | toggle | status       # liga/desliga os efeitos (de todos os jogos)
lrctl layers                           # jogos com a layer ativa: PID, resolução, formato, estado do efeito
lrctl effects                          # efeitos disponíveis
lrctl use <Nome>                       # escolhe o efeito (todos os jogos); use none desfaz
lrctl uniforms | get | set | reset     # parâmetros do efeito, em tempo real
lrctl use Sepia --layer 1234           # SÓ no jogo de pid 1234 (ou --layer RocketLeague)
lrctl use none --layer 1234            # sem efeito só nesse jogo
lrctl use inherit --layer 1234         # volta a seguir a escolha de todos
lrctl config                           # onde estão os efeitos, texturas, presets e cache
lrctl config init /mnt/D/reshade       # prepara a pasta de dados e a escolhe
```

Variáveis de ambiente úteis: `LINUX_RESHADE_DATA` (pasta de dados), `LINUX_RESHADE_EFFECTS` (lista `:` de
pastas de efeitos), `LINUX_RESHADE_CACHE`, `LINUX_RESHADE_LOG` (log da layer). A lista completa está em
`docs/presets.md` e no código de `crates/config`.

## Mapa do repositório

| Caminho | O quê |
|---|---|
| `crates/vk-layer` | base de qualquer layer Vulkan: dispatch, cadeia de create-info, negociação com o loader |
| `crates/layer` | a layer (`liblinux_reshade_layer.so`): present, swapchain, efeitos, canais com o controle |
| `crates/postfx` | executa um efeito (passes, texturas, uniforms) em Vulkan; testado em GPU de software |
| `crates/fx-bundle` | formato do bundle (`docs/bundle-format.md`): leitor, uniforms, ids |
| `crates/ipc` | memória compartilhada entre controle e layer (`docs/ipc.md`): estado, parâmetros, escolha, status, canal por jogo |
| `crates/ini` | `.ini` que preserva comentários, ordem e chaves desconhecidas (base de config e presets) |
| `crates/config` | pasta de dados, `config.ini`, caminhos de busca em camadas |
| `crates/control-core` | a lógica do controle, sem interface: compilador, vigia, escolha, parâmetros |
| `crates/control` | `lrctl` (linha de comando e daemon) |
| `crates/shortcuts`, `crates/shortcuts-portal` | atalho global (trait + backend do portal XDG GlobalShortcuts) |
| `crates/panel` | o modelo da interface, independente de toolkit |
| `crates/gui` | `lrpanel` (GTK4) |
| `tools/fx2spv` | compilador FX → bundle (C++, separado; baixa o `reshadefx` BSD-3 para `third_party/`) |
| `effects/` | efeitos próprios (`Sepia.fx`) |
| `dev/gnome-placement` | extensão do GNOME para posicionar janelas de teste (**desativada**: já derrubou o GNOME) |
| `scripts/dev.sh` | setup, doctor, build, testes de ponta a ponta |

## Testes

```sh
cargo test --workspace --exclude linux-reshade-gui     # unitários e integração (sem janela; usa lavapipe para a GPU)
scripts/dev.sh smoke          # vkcube real + layer: liga/desliga, parâmetros, troca de efeito
scripts/dev.sh validate       # o mesmo com a validation layer do Vulkan (precisa ficar sem mensagens)
scripts/dev.sh smoke-steam    # dentro do contêiner da Steam (pressure-vessel)
scripts/dev.sh smoke-auto     # dois vkcube de resoluções diferentes: compilação automática pelo vigia
scripts/dev.sh smoke-layer    # dois vkcube: efeito só em um deles, parâmetros por jogo
scripts/dev.sh panel-smoke    # a interface gráfica de ponta a ponta
scripts/dev.sh fx-test        # compila efeitos da comunidade e valida o SPIR-V
```

Os `smoke-*` abrem janelas por ~10 a 30 s: evite mexer em janelas até terminarem.

## Documentação

| Arquivo | Conteúdo |
|---|---|
| [`docs/CONTINUAR.md`](docs/CONTINUAR.md) | **ponto de retomada**: estado, próximos passos, combinados, armadilhas |
| [`ROADMAP.md`](ROADMAP.md) | os marcos e o que cada um entregou |
| [`docs/presets.md`](docs/presets.md) | desenho de presets, shaders, jogos e cadeia de efeitos (fases F0..F7) |
| [`docs/reshade-estudo.md`](docs/reshade-estudo.md) | como o ReShade funciona, lido do código-fonte, com os limites da leitura |
| [`docs/ipc.md`](docs/ipc.md) | o canal entre controle e layer |
| [`docs/bundle-format.md`](docs/bundle-format.md) | o formato do bundle |
| [`DEPENDENCIAS.md`](DEPENDENCIAS.md) | política e decisões sobre dependências e libs próprias |
| [`tools/fx2spv/README.md`](tools/fx2spv/README.md) | o compilador FX |

## Por que existe

O ReShade depende de hook binário e de DLLs do Windows, e não roda nativamente no Linux. No Linux o caminho
oficial é outro: **layers Vulkan**. Como DXVK (D3D9/10/11) e VKD3D-Proton (D3D12) traduzem D3D para Vulkan,
uma layer Vulkan vê jogos Vulkan nativos **e** jogos Windows no Proton, sem injeção de DLL.

Objetivo secundário e deliberado: os módulos que esta ferramenta exigir viram **libs próprias reutilizáveis**
(ver `DEPENDENCIAS.md`). As libs nascem da necessidade do projeto, não antes dela.

## Decisões tomadas

| Decisão | Motivo |
|---|---|
| Linux nativo primeiro; Windows talvez um dia | O Windows exigiria hooks por API (D3D11/12), muito mais trabalho. Vulkan layer também existe no Windows, então não fecha a porta. |
| **Layer Vulkan implícita**, sem hook binário | Mecanismo oficial do loader (manifesto JSON + `vkNegotiateLoaderLayerInterfaceVersion`). |
| Linguagem principal: **Rust** | O rastreio de depth é máquina de estados (enums + `match` exaustivo ajudam). A layer roda dentro do jogo, então crash e data race derrubam o jogo. Núcleo reaproveitável. |
| **C++ só no front-end de FX**, como **ferramenta separada** | Compatibilidade com os `.fx` da comunidade depende de reproduzir o compilador do ReShade, que é C++ (BSD-3). **Nunca linkado na layer**: nenhum C++ roda dentro do processo do jogo. |
| Layer lê **bundle pronto** (SPIR-V + manifesto), não compila FX | Menos código no processo do jogo, carga mais rápida, recompilação feita de fora. |
| **FX como entrada** (compatibilidade); formato nativo próprio **depois** | A biblioteca de efeitos em FX é o valor. Formato novo só quando o runtime já estiver completo. |
| **Efeitos rodam na layer; só controle e UI ficam fora** | Passes gravados no mesmo command buffer, antes do `vkQueuePresentKHR`: zero frame de atraso (sem ghosting). Processo separado exigiria dma-buf + semáforos e pelo menos um frame de latência. |
| Controle por **processo separado**, memória compartilhada em `/dev/shm` | Parâmetros e escolha são lidos uma vez por frame, sem lock nem syscall. `/dev/shm` (e não `/run/user`) porque o pressure-vessel da Steam o expõe. |
| **A layer não toca em input** | Reduz superfície no processo do jogo (anti-cheat). Teclado e atalhos ficam no app de controle. |
| **Wayland é o alvo**; X11 só se um dia for preciso | Atalho global pelo portal XDG GlobalShortcuts (zbus); GNOME mostra um diálogo de confirmação na primeira vez. O atalho é só uma sugestão: o usuário pode trocá-lo. |
| Interface em **GTK4**, lógica num crate independente de toolkit | Menos crates entre os toolkits de janela e o melhor comportamento no GNOME/Wayland (medido em `DEPENDENCIAS.md`). |
| **Nada na pasta do jogo**; pasta de dados escolhida pelo usuário | Reinstalar o jogo ou formatar a máquina não perde shaders nem presets. Shaders globais por padrão, pasta própria por jogo só quando um shader não funciona ali. |
| **Preset no formato do ReShade** | Presets da comunidade funcionam sem conversão (formato verificado no código do ReShade). |
| **Depth é opt-in por jogo**, desligado por padrão | É a feature mais sensível para anti-cheat (permite efeitos que "enxergam" através de paredes). |
| OpenGL nativo **fica para depois** | Não há mecanismo de layer; exigiria `LD_PRELOAD` e hook de `glXSwapBuffers`/`eglSwapBuffers`, ou Zink. |
| Anti-cheat: **só jogos offline** nos testes | Rocket League apenas em modo offline. Nada de testar em partida online. |

## Arquitetura

```
 fx2spv (C++, separado)           layer Vulkan (Rust, dentro do jogo)         controle (Rust, outro processo)
 .fx ──▶ bundle (SPIR-V +  ─────▶ lê o bundle, roda os passes no present ◀──▶ lrctl / lrpanel / daemon
         manifest.json)            lê escolha e parâmetros da memória          escolhe, ajusta, compila por
                                   compartilhada (global e por jogo)            resolução, atalho global
```

O controle compila cada efeito para a resolução de cada jogo aberto e publica o caminho do bundle; a layer o
carrega e troca em tempo de execução. Detalhes em `docs/ipc.md`.

## Diferencial: depth buffer (marco 5, não iniciado)

Não existe "API de depth". A layer precisa acompanhar criação de imagens com formato de depth, `vkCmdDraw*`,
attachments (render pass ou dynamic rendering) e transições de layout, escolher qual é o depth do mundo 3D por
heurística e **copiar a imagem antes de o jogo limpá-la**. Também há Z invertido, formatos variados e MSAA. É
heurística por jogo, com módulo próprio e testável. Hoje os efeitos que amostram depth (como o Bloom) são
recusados com uma mensagem clara.

## Achados de ambiente (Ubuntu 26.04 / GNOME 50 / NVIDIA 595)

- **Contêineres da Steam (pressure-vessel):** a layer carrega e funciona dentro do runtime `soldier` (exige
  `GLIBC_2.34`). O `/run/user/<uid>` **não** é visível lá; `/dev/shm`, `/tmp`, `$HOME` e `/mnt` são.
- **XWayland trava na apresentação** neste ambiente: `vkcube --wsi xcb` não termina (nem sem a layer);
  `--wsi wayland` termina. Importa para jogos no Proton (X11 por padrão): testar com jogo real.
- **Ctrl+Alt+F1..F12 troca de terminal virtual** e travou a máquina uma vez; nunca sugerir esses atalhos.
- **A extensão `vkcube-placement` derrubou o GNOME Shell** (SIGSEGV) e a sessão inteira; ficou desativada.
  Código dentro do compositor não tem segunda chance.
- Um travamento do PC pode deixar artefatos de build zerados e o Cargo não os refaz;
  `scripts/dev.sh install` detecta e refaz o `.so`.

## A verificar

- Se o `vkBasalt` lê depth: o `.conf` de exemplo dele tem uma opção `depthCapture` (vista, **não testada**).
- Se o `gamescope` tem efeitos ReShade (`--reshade-effect`) e até onde vai.
- Como o addon `generic_depth` do ReShade escolhe o buffer de profundidade (código **não lido**; só o que o
  fórum diz). Ler antes do marco 5.

## Perguntas em aberto

- Nome definitivo do projeto e licença do código próprio (o `reshadefx` que o `fx2spv` usa é BSD-3).
- Driver alvo: NVIDIA (o que há aqui) é o testado; Mesa não foi testado.
- Compute shaders, mipmaps e fontes de mouse/teclado dos efeitos: sem plano (ver `docs/CONTINUAR.md`).
