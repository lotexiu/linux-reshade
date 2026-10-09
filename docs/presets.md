# Presets, jogos e cadeia de efeitos

Estado: **proposta** (2026-10-09), ainda sem código. Nasceu de dois pedidos: efeito **por jogo** e
**paridade com o ReShade original** (vários presets, vários efeitos encadeados, ordem de prioridade),
com os arquivos de configuração fora da pasta do jogo, numa pasta que o usuário escolhe.

## O que o ReShade original tem (inventário)

Marcado com a fase em que entra neste projeto. "—" = decidido não copiar.

| Recurso do ReShade | Aqui |
|---|---|
| Preset = arquivo `.ini` com a lista de técnicas ligadas, a ordem, os valores dos uniforms e os `#define`s | F2 |
| Vários presets por jogo, troca em tempo de execução (próximo/anterior, escolher na lista) | F5, F6 |
| Vários efeitos ligados ao mesmo tempo, cada um com seus valores | F3 |
| Ordem de execução: cada técnica lê o resultado da anterior | F3 |
| Ligar/desligar uma técnica sem tirá-la da lista; atalho próprio por técnica | F5 (atalho: F7) |
| Reordenar técnicas | F5 |
| `PreprocessorDefinitions` por preset (mudar uma macro recompila o efeito) | F6 |
| `ui_*` (label, min, max, items, type…) gerando os controles | feito (4d) |
| Fontes de uniform: timer, frametime, framecount, date, random, pingpong | feito |
| Fontes de uniform: `mouse_point`, `mouse_button`, `key`, `overlay_open`, `depth`… | F7 (depth: marco 5) |
| Texturas vindas de arquivo (LUTs, ruído) | F7 |
| Caminhos de busca de shaders/texturas (`ShaderPaths`, `TexturePaths`) | F2 (configuráveis) |
| Captura de tela com e sem efeito, atalho de recarregar efeitos | F7 |
| Ajustes globais (`ReShade.ini`: teclas, caminhos, modo desempenho) | F2 (arquivo de config do `lrctl`) |
| Overlay desenhado dentro do jogo | — (a interface é um processo separado: mais seguro com anti-cheat) |
| API: DX9/10/11/12/OpenGL | — (Vulkan; jogos D3D passam por DXVK/VKD3D) |
| Add-ons | — |

## Onde ficam os arquivos

A pasta é escolhida pelo usuário (variável `LINUX_RESHADE_PRESETS` ou `presets_dir=` no arquivo de config
do `lrctl`; sem nenhuma das duas usa `~/.config/linux-reshade/presets`). Exemplo com `/mnt/D/presets`:

```
/mnt/D/presets/
  config.ini                 ajustes globais (caminhos de efeitos, atalhos)
  preset.ini                 preset global: vale para jogo sem preset próprio
  RocketLeague/              uma pasta por jogo (nome = id do jogo, abaixo)
    game.ini                 qual preset está ativo + ajustes só deste jogo
    Noturno.ini              um preset (pode haver vários)
    Cores vivas.ini
  vkcube/
    ...
```

Nada é gravado na pasta do jogo. Quem escreve nesses arquivos é o `lrctl`/`lrpanel`, nunca a layer.

## Formato do preset: o mesmo do ReShade

Recomendação: **ler e escrever o formato do `ReShadePreset.ini`**, para os presets da comunidade
funcionarem sem conversão:

```ini
PreprocessorDefinitions=
Techniques=Sepia,Bloom@Bloom.fx
TechniqueSorting=Sepia,Bloom@Bloom.fx

[Sepia.fx]
Intensity=0.8
```

- `Techniques` = as técnicas ligadas; `TechniqueSorting` = a ordem de todas (inclusive as desligadas).
- Seção `[Arquivo.fx]` = valores dos uniforms daquele efeito (o que `lrctl set` faz hoje).
- Chaves e comentários desconhecidos são **preservados** ao regravar (não perder o que o usuário ou outra
  ferramenta escreveu).
- A gravação é atômica (arquivo temporário + `rename`).
- A parte de atalhos por técnica (`Key…`) precisa ser conferida contra o código do ReShade antes de
  implementar (F7); o que está acima é o que já é seguro afirmar.

## Identificar o jogo

A layer já informa o nome (`VkApplicationInfo` ou nome do executável). Para escolher a pasta:

1. entrada em `config.ini` (`[Games]` mapeando um id para uma pasta, para o usuário corrigir);
2. nome do executável sem extensão (`RocketLeague.exe` → `RocketLeague`; via Proton é o `.exe`);
3. nome da aplicação do `VkApplicationInfo`.

Sem pasta correspondente, vale o preset global; a pasta só é criada quando o usuário salva algo.

## Como isso conversa com a layer

A layer **não lê `.ini`**: código dentro do jogo fica pequeno. O controlador (`lrctl daemon` / `lrpanel`):

1. descobre o jogo de cada layer viva e escolhe o preset (jogo ativo → senão global);
2. compila cada técnica para a resolução do jogo (como já faz com a escolha global);
3. publica **por processo** um plano: lista ordenada de técnicas (bundle + técnica + ligada?) e o bloco
   de uniforms de cada uma, num arquivo `ctl-<id>` em `/dev/shm`, com seqlock como os blobs de hoje;
4. grava de volta no `.ini` quando o usuário muda algo.

A escolha e os parâmetros globais de hoje viram o plano do "jogo sem preset" (o comportamento atual continua).

## Cadeia na layer

Cada técnica é uma etapa; a etapa N lê o resultado da N-1 (a primeira lê o frame do jogo) e a última
escreve no que vai para a tela, com *ping-pong* entre duas imagens. É a semântica do ReShade, em que
`BackBuffer` é "o que as técnicas anteriores produziram". Texturas declaradas com o mesmo nome em efeitos
diferentes são compartilhadas no ReShade; ficam para a F7 (primeiro cada efeito com as suas).

## Fases (cada uma com commits pequenos e testes próprios)

- **F1 — plano por processo no ipc.** Escolha e parâmetros por layer, com os globais como padrão;
  `lrctl use|set|get --layer <id>`. Sem `.ini` ainda.
- **F2 — crate `preset`.** Parser/gravador do formato acima, sem dependências novas se possível, testado
  com presets reais do ReShade; configuração da pasta de presets; resolução jogo → arquivo.
- **F3 — cadeia na layer.** Várias técnicas em ordem; o `postfx` ganha o encadeamento e a escolha de
  técnica dentro do bundle. Teste: dois efeitos em ordem trocada dão imagens diferentes (golden).
- **F4 — plano a partir do preset.** O controlador monta o plano de cada jogo a partir do `.ini` e o
  mantém atualizado quando o arquivo muda.
- **F5 — interface.** Jogos selecionáveis; lista de técnicas com ligar/desligar e reordenar; seletor e
  "salvar como" de presets.
- **F6 — macros por preset** (recompila) e troca rápida de preset.
- **F7 — o resto do inventário:** atalhos por técnica, fontes de mouse/teclado, imagens, capturas.

## Decisões em aberto

1. Formato do ReShade como está (recomendado) ou formato próprio?
2. "Maior prioridade afeta o próximo" = a técnica do topo da lista roda primeiro e as de baixo recebem o
   resultado dela (como no ReShade). Confirmar que é isso.
3. Id do jogo = nome do executável (recomendado) com correção manual em `config.ini`.
4. Várias pastas de presets (por exemplo, uma por perfil) ou uma só?
