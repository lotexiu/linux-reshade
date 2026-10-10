# Ponto de retomada

Escrito em 2026-10-09, no fim da F1. Serve a quem for continuar (o dono do projeto em outra máquina, ou uma
nova sessão do Claude): diz onde paramos, o que vem agora, como trabalhar aqui e o que já deu errado.
A memória do Claude fica na máquina dele (`~/.claude/...`), então o que importa dela está **repetido aqui**.

Se for uma nova sessão do Claude: leia, nesta ordem, `README.md`, este arquivo, `docs/presets.md`,
`docs/reshade-estudo.md` e `docs/ipc.md`; depois `git log --oneline | head -30`.

## Onde paramos

Últimos commits relevantes (do mais novo): documentação e `smoke-layer` (`a86ceac`), CLI `--layer`
(`b7474fb`), vigia por jogo (`8756a45`), layer lê o canal do jogo (`d514b66`), canal `ctl-<id>` no ipc
(`a86e4cc`), `lrctl config` (`e558501`), crates `config` e `ini`.

| Fase (`docs/presets.md`) | Estado |
|---|---|
| Marcos 0 a 4 do `ROADMAP.md` (layer, passe fixo, bundle, controle, GUI) | **feitos** |
| F0 configuração e caminhos (`ini`, `config`, `lrctl config`) | **feita** |
| F1 escolha e parâmetros por jogo (`ctl-<id>`, `--layer`) | **feita** |
| F2 crate `preset` (ler/gravar o formato do ReShade) | **feita** (`preset`, `preset::game`) |
| F3 cadeia de vários efeitos na layer | **feita** (`postfx::Chain`, a layer usa o plano) |
| F4 plano de cada jogo montado a partir do preset | **feita** (`game_plan`, `games`, `smoke-preset`) |
| F5 interface por jogo (lista selecionável, técnicas, presets) | a fazer |
| F6 macros por nível, atalhos de preset, transição suave | a fazer |
| F7 o resto do inventário (atalhos por técnica, capturas, OSD, imagens, mouse/teclado) | a fazer |
| Marco 5: depth buffer | não iniciado (é o diferencial do projeto) |

Verificado ao fim da F1 (todos passaram): `cargo test --workspace --exclude linux-reshade-gui`, e os testes de
ponta a ponta `validate` (validation layer sem mensagens), `smoke-steam`, `smoke-auto`, `smoke-layer`.

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

## Próximo passo: F2, o crate `preset`

Objetivo: ler e gravar presets no formato do ReShade, sem dependências novas (usa o crate `ini`).

- API esperada: `Preset::load/parse/save`, técnicas ligadas e ordem (`techniques()`, com `Nome@Arquivo.fx`),
  `set_enabled`, `move_technique`, macros em três níveis (global < preset < efeito; valor vazio vale `1`),
  valores de uniform por efeito (`values(effect_file)`), `Key<técnica>`.
- Regras já lidas no código do ReShade (`docs/reshade-estudo.md`, seção 3): `Techniques` só tem as ligadas;
  `TechniqueSorting` tem todas, só é gravada se já existia ou se a ordem difere; técnica desconhecida = aviso, não erro;
  ordenação estável pela lista, e técnicas fora dela seguem a ordem de declaração no arquivo e, entre arquivos,
  a ordem alfabética do rótulo; uniforms com `source` ou `nosave` não são gravados.
- Testes: ida e volta byte a byte de presets reais da comunidade (**baixar alguns** exemplos de presets; ainda não
  há nenhum no repositório), preservar comentários e chaves desconhecidas, CRLF, e as regras acima.
- Depois: resolução jogo → arquivo (`<Presets>/<Jogo>/game.ini` com `preset=`), ainda na F2 ou no início da F4.

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
