# Ponto de retomada

Escrito em 2026-10-09 e atualizado em 2026-10-10 (fim da F4). Serve a quem for continuar (o dono do projeto em outra máquina, ou uma
nova sessão do Claude): diz onde paramos, o que vem agora, como trabalhar aqui e o que já deu errado.
A memória do Claude fica na máquina dele (`~/.claude/...`), então o que importa dela está **repetido aqui**.

Se for uma nova sessão do Claude: leia, nesta ordem, `README.md`, este arquivo, `docs/presets.md`,
`docs/reshade-estudo.md` e `docs/ipc.md`; depois `git log --oneline | head -30`.

## Onde paramos

Fim da F4: um preset do formato do ReShade vale por jogo, o vigia o transforma em plano, e a layer executa a
cadeia. Commits de referência: `smoke-preset` (`f5eae79`), `lrctl preset` (`7ae4c6a`), sRGB (`5a1fccc`), mipmaps
(`103fb53`), texturas de arquivo (`a52dff8`, `94079c7`).

| Fase (`docs/presets.md`) | Estado |
|---|---|
| Marcos 0 a 4 do `ROADMAP.md` (layer, passe fixo, bundle, controle, GUI) | **feitos** |
| F0 configuração e caminhos | **feita** |
| F1 escolha e parâmetros por jogo | **feita** |
| F2 crate `preset` e `game.ini` | **feita** |
| F3 cadeia de vários efeitos (`postfx::Chain`) | **feita**; etapa não suportada é pulada com aviso |
| F4 plano a partir do preset (`game_plan`, `games`) | **feita**; `lrctl preset ...` edita |
| F5 interface por jogo (no estilo da aba Home do ReShade) | **primeira versão feita** (`panel::Home`, `lrpanel`); ver pendências abaixo |
| F6 macros por nível (recompilam) | **feita**; atalhos de preset e transição suave ainda não |
| D1 observar a profundidade (`LINUX_RESHADE_DEPTH_OBSERVE=1`) | **feita**, testada no vkcube e no Haste; ver `docs/depth.md` |
| F7 o resto do inventário (atalhos por técnica, capturas, OSD, mouse/teclado) | a fazer |
| D2 profundidade amostrada (original ou foto de um passe) | **feita**; regra automática do `DontCare` validada no Haste (2026-10-10); `docs/depth.md` |
| D3 profundidade por jogo | parcial: marcador `depth-arm-<jogo>` arma o jogo sozinho; o painel mostra o estado da profundidade e as macros (`UPSIDE_DOWN` à mão, no preset); falta escolher candidata/passe no painel |
| Compute e texturas 1D/3D | **feitos** (2026-10-10): 119 de 152 efeitos executam na lavapipe; `smoke-compute` passa na GPU real (vkcube); falta testar os efeitos do Marty num jogo; `docs/levantamento.md` |
| Efeito antes da UI | adiado: no Haste a UI já está na imagem de render (`docs/antes-da-ui.md`) |

Também feito nesta etapa: sRGB (8 bits), mipmaps, texturas de arquivo (PNG), o remendo de `#include` do Windows
no `fx2spv` (`tools/fx2spv/patches`), a pasta de dados padrão (`~/.local/share/linux-reshade`), o id do jogo
pelo executável (`exe=` no status).

Verificado ao fim: `cargo test --workspace --exclude linux-reshade-gui`, e de ponta a ponta `validate`,
`smoke-auto`, `smoke-layer` e `smoke-preset` (todos passaram). **Não rodados depois das mudanças**: `smoke-steam`
e `panel-smoke`. Numa cadeia real de 4 a 7 efeitos da pasta do dono, na GPU NVIDIA com a validation layer: sem
mensagens.

## Decisões já tomadas (não reabrir sem motivo)

1. **Pasta de dados única** escolhida pelo usuário, com um `location.ini` de uma linha em `~/.config/linux-reshade/`
   apontando para ela; cada caminho (shaders, texturas, presets, cache) também é configurável. Caminhos relativos
   valem a partir do `config.ini`. Layout em `docs/presets.md`.
2. **Shaders globais por padrão**; `<Presets>/<Jogo>/Shaders` (se existir) é procurada antes das globais.
3. **Preset no formato do `ReShadePreset.ini`** (`Techniques`, `TechniqueSorting`, `PreprocessorDefinitions`,
   `Key<técnica>`, seção `[Arquivo.fx]`). Ordem de execução = ordem da lista (a primeira roda primeiro).
4. **Id do jogo** = nome do executável sem extensão (corrigível em `config.ini`), depois o nome do `VkApplicationInfo`.
5. **A layer não lê `.ini` nem procura shaders**: só o controle faz isso e publica por memória compartilhada.
6. **Mouse/teclado nos efeitos** (`mousepoint`, `key`...): ficam com o valor padrão até algum shader usado precisar
   (uma layer não recebe entrada no Wayland).
7. **O mesmo efeito duas vezes na cadeia**: não suportado na primeira versão (o ReShade também não permite).
8. **Liga/desliga é global** (o atalho liga e desliga tudo); só a escolha e os parâmetros são por jogo.
9. Presets por jogo vivem fora da pasta do jogo; reinstalar o jogo ou a máquina não os perde.

## Próximo passo

1. **F5, refinar a interface** (o dono quer algo parecido com o ReShade e ajustar com o uso): arrastar para
   reordenar (hoje são setas), compilar sem travar a janela (hoje `Home::prepare_effects` compila na thread da
   interface ao abrir um preset), abas Configurações/Log/Sobre, "salvar como"/restaurar o preset, atalho por
   técnica, e mostrar o estado de cada técnica no jogo aberto (aplicada, pulada, com problema).
2. **F6, macros**: `PreprocessorDefinitions` em três níveis precisa recompilar o efeito (hoje `game_plan` as ignora
   e o `fx2spv` aceita `-D`?). Conferir.
3. **Marco 5, depth buffer**: o maior ganho em efeitos (40 de 152) e o diferencial do projeto.
4. Dívidas pequenas: JPEG/DDS nas texturas; sRGB de render target; **testar o alvo sRGB num jogo real** (Haste, `B8G8R8A8_SRGB`: só foi testado na GPU de software); `[Games]` no `config.ini` para corrigir um id;
   a interface ainda não conhece planos (só a escolha única).

## Combinados de trabalho (do dono do projeto)

- **Commits pequenos, um assunto cada**, para poder reverter. Mensagens em português, estilo
  `feat(crate): ...`, `fix(...)`, `docs: ...`, `test: ...`.
- **Modular ao máximo**: um crate por função, sem repetir código; "primeiro fazer existir"; não pular etapas.
- **Cuidado com dependências**: medir (`cargo tree`: nº de crates, licença, mantenedor) e registrar em
  `DEPENDENCIAS.md` **antes** de adotar. A layer (dentro do jogo) exige critério mais duro. As crates novas da
  F0/F1 (`ini`, `config`) não adicionam nenhuma dependência de terceiros.
- **Testar de verdade** (GPU de software lavapipe, validation layer, os `smoke-*`) antes de declarar pronto, e
  **dizer o que não foi testado**.
- Responder em português do Brasil, direto, sem enrolar; dizer honestamente o que não funcionou.
- **Não rodar nada que abra janela ou mexa na GPU** sem avisar, principalmente depois de um travamento.
- Para o Claude: terminar os commits com a linha `Co-Authored-By: Claude ... <noreply@anthropic.com>` que o
  ambiente pedir; não fazer `git push` sem o dono pedir.
- Performance (custo de GPU/FPS) **fica para depois**. Por ora só há `vkcube` para testar; jogos reais só quando o
  dono puder (Rocket League **só offline**, por causa de anti-cheat).

## Armadilhas já vividas

- **Ctrl+Alt+F1..F12 trocou de VT e travou a máquina.** O atalho do projeto é Ctrl+Shift+F10. Nunca sugerir Ctrl+Alt+Fn.
- **Extensão GNOME `dev/gnome-placement` derrubou o compositor** e a sessão; está desativada. Não reativar sem cuidado.
- `vkcube --wsi xcb` (XWayland) e `vkcube --validate` travam neste ambiente, mesmo sem a layer. Usar `--wsi wayland` e
  `VK_LOADER_LAYERS_ENABLE='*validation*'`.
- Depois de um travamento do PC os artefatos de build podem ficar zerados e o Cargo não os refaz
  (`scripts/dev.sh install` detecta o `.so`).
- `pkill -f` já matou o próprio shell duas vezes: matar por PID exato (`pgrep -x`).
- Smoke que deixa estado para trás (escolha de efeito, parâmetro, processo): os testes limpam por `trap` e restauram
  o parâmetro no fim; se criar um teste novo, faça igual. O estado vive em `/dev/shm/linux-reshade-<uid>/`.
- Rust: o `rust-analyzer` do VSCode às vezes mostra diagnósticos velhos; confirmar com `cargo build`.
- Arquivo de teste com `ETXTBSY` ao executar logo depois de gravar: já tratado em `Compiler` (`run_retrying_busy`).
- Pacotes de sistema (`sudo`) o dono instala à mão; o agente não tem sudo sem senha.

## Pendências e dívidas conhecidas

- **A confirmar no jogo (pedido do dono, 2026-10-10):** sliders decimais agora têm passo fino (1/1000 da faixa) e o
  vigia olha o preset a cada 60 ms (era 200 ms); conferir se mexer nos parâmetros de um efeito (ex.: `HeightFog`) fica
  suave e sem atraso perceptível. Se ainda houver degrau, o próximo passo é interpolar os uniforms na layer.

- **Interface gráfica:** só mexe na escolha e nos valores de *todos* os jogos; selecionar um jogo é a F5. O dono relatou
  duas coisas ao testar o `lrpanel`: (a) o painel aberto antes do jogo não listava o jogo novo: **não consegui reproduzir**
  (com um teste real o `vkcube` aparece em ~5 s); se voltar, perguntar como o painel foi aberto, se havia mais de um
  `lrctl daemon`, e se estava minimizado; (b) o menu mostrava "Bloom" com o texto "Nenhum efeito escolhido": **sem
  explicação**; o Bloom amostra depth e é recusado (esperado). O texto vermelho do estado do jogo sai cortado na lista.
- **Lacunas do runtime** (medido em 2026-10-10 com os 152 efeitos da pasta do dono, numa GPU de software com a
  validation layer: **82 rodam**; os outros 70): **40 pedem depth** (marco 5), **8 usam imagens de arquivo** (LUTs,
  ruído; PNG já funciona: o app de controle decodifica para o bundle; falta JPEG/DDS), 2 escrevem sRGB numa textura, 1
  lê sRGB de uma textura, 1 usa outra semântica. Os 17 de compute (`storage`) nem compilam ainda. Ainda sem plano:
  fontes de uniform `mouse*`/`key`/`overlay_*`/`screenshot`/`bufready_depth`, `timeout` de técnica, OSD, capturas
  de tela, layer de 32 bits. O compilador (`fx2spv`) emite `Offset` fora de `OpImage*Gather` em ~10 efeitos: a
  validation layer reclama (VUID-RuntimeSpirv-Offset-10213); a NVIDIA aceita, mas é inválido pela especificação.
- `cargo clippy` aponta avisos antigos em `ipc` (`blob.rs`, `lib.rs`) e `vk-layer`; os crates novos estão limpos.
- `cargo audit` / `cargo deny` ainda não foram rodados.
- Rocket League via Proton e Idols of Ash foram testados **antes** da F0/F1; a escolha por jogo não foi testada em jogo real.
- Janela do jogo no monitor errado (Rocket League): problema do ambiente, não do projeto.

## Em outra máquina: o que refazer

O estado de uma máquina não vai no git. Ao chegar:

1. `scripts/dev.sh setup`, `doctor`, `fx-tool`, `install`, `cargo build --release` (ver README).
2. `lrctl config init <pasta>` e copiar para ela os shaders/presets (se a pasta de dados já existe de antes, só
   apontar o `location.ini` para ela com o mesmo comando; ele não sobrescreve nada).
3. Subir `lrctl daemon` (atalho global Ctrl+Shift+F10; o GNOME pede confirmação na primeira vez).
4. Rodar `cargo test --workspace --exclude linux-reshade-gui` e `scripts/dev.sh validate` para conferir o ambiente
   antes de mexer em qualquer coisa.
5. O que **não** vem junto: `third_party/` e `build/` (ignorados pelo git; `fx-tool` refaz), o estado em `/dev/shm`,
   o manifesto em `~/.local/share/vulkan/implicit_layer.d/` (o `install` refaz) e a memória local do Claude.

## Comandos úteis

```sh
scripts/dev.sh logs                 # log da layer (~/.local/state/linux-reshade/layer.log)
lrctl layers                        # o que está aberto, com PID
lrctl config                        # de onde vêm efeitos, presets e cache
VK_LOADER_DEBUG=layer LINUX_RESHADE=1 vkcube --wsi wayland --c 100   # o loader carregou a layer?
target/release/lrpanel --effect Sepia --screenshot /tmp/painel.png --hold 2000 --wait 3000   # captura da interface
```

## Roblox (Sober, Flatpak): o que se aprendeu

- A layer só carrega como extensão VulkanLayer do Flatpak (`./scripts/dev.sh flatpak-layer org.vinegarhq.Sober --apply`). O script agora recria o repositório local a cada vez: antes o flatpak seguia instalando o commit velho ("nada a atualizar"). Confira com `sha1sum` da .so instalada.
- Oscilação da profundidade tinha duas causas: (1) a candidata escolhida pelo número sumia quando o jogo recriava a imagem (mapas de sombra em cascata) e a escolha caía na heurística; agora herda a imagem de mesmo tamanho/formato (`picked_image`); (2) o daemon remontava o plano toda vez que a inversão (`reversed`) piscava entre `Some` e `None`, refazendo a cadeia; agora só muda depois de 25 ticks iguais (`REVERSED_TICKS`).
- `lrctl layers` mostra "presents sem efeito/com efeito": se o primeiro número cresce durante a oscilação, a layer está pulando o efeito.
- MSAA em qualidade máxima funciona com a layer atual (resolve da amostra 0).

## Atalhos configuráveis (2026-11-02)

- `config.ini`, seção `[INPUT]`: `KeyEffects` (ligar/desligar, de fábrica `CTRL+SHIFT+F10`), `KeyNextPreset`, `KeyPreviousPreset`. O valor é o formato XDG do portal; `shortcuts::trigger::normalize` entende `'`, `OEM_7`, `Ctrl+Shift+F10`... A janela de atalhos do painel (botão do teclado) captura a tecla e grava.
- O portal só usa a tecla sugerida na primeira vez que vê um id (confirmado no KDE). Para trocar, o daemon anota o que registrou (`shortcuts-bound.ini`, ao lado do cache), esquece o atalho pelo `KGlobalAccel` do KDE (`shortcuts_portal::forget`) e registra de novo; o daemon se reexecuta sozinho ao ver o `[INPUT]` mudar. No GNOME não há "esquecer": a tecla só muda pelas configurações do sistema.
- Uma tecla sem modificador (como `'`) vale em TODOS os programas enquanto o daemon roda; a janela avisa.

## Regra da interface: sem popover dentro de popover
Um popover (ou `DropDown`, que abre um) dentro de outro deixa o de fora aberto depois do clique que fecha o de dentro: a
janela parece travada. Dentro de um popover use `crates/gui/src/disclosure.rs` (botão que revela o conteúdo no lugar) ou
botões/listas comuns, como `depth_picker.rs` e `macros.rs` já fazem.
