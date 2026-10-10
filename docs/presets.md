# Presets, shaders, jogos e cadeia de efeitos

Estado: **proposta, revisão 2** (2026-10-09); F0 e F1 implementadas, o resto ainda sem código. A base factual (o que o ReShade faz, com a
fonte) está em [reshade-estudo.md](reshade-estudo.md); este documento diz o que copiamos e o que muda.

Pedidos que moldam o desenho: efeito e parâmetros **por jogo**; nada gravado na pasta do jogo; **shaders
globais por padrão**, com pasta própria por jogo só quando um shader não funcionar naquele jogo; **todas as
pastas configuráveis**, para sobreviver a reinstalar o jogo ou formatar a máquina; vários presets, vários
efeitos em ordem, e paridade com o ReShade onde isso for bom.

## Onde fica o quê

Há uma **pasta de dados** escolhida pelo usuário (exemplo: `/mnt/D/reshade`). Tudo o que importa preservar
mora nela; o que fica em `~/.config` é só um ponteiro de uma linha.

```
~/.config/linux-reshade/location.ini     data_dir=/mnt/D/reshade        (único arquivo fora da pasta de dados)

/mnt/D/reshade/
  config.ini                  ajustes globais (caminhos, atalhos, macros globais, mapa de jogos)
  Shaders/  Textures/         efeitos e imagens GLOBAIS (o conteúdo do reshade-shaders; busca recursiva)
  Presets/
    preset.ini                preset global: vale para jogo sem preset próprio
    RocketLeague/             uma pasta por jogo
      game.ini                preset ativo + ajustes só deste jogo
      Noturno.ini             presets do jogo (pode haver vários)
      Cores vivas.ini
      Shaders/  Textures/     opcionais: se existirem, são procurados ANTES dos globais
    vkcube/
      ...
```

- Cada pasta tem um padrão (`<data_dir>/Shaders`, `<data_dir>/Presets`…) e **pode ser trocada** em
  `config.ini` (`effect_paths`, `texture_paths`, `presets_dir`), inclusive para fora da pasta de dados, como
  o `/mnt/D/presets` do pedido. As listas aceitam vários caminhos e o sufixo `**` (recursivo), como no ReShade.
- Caminhos **relativos** valem em relação ao `config.ini` (e não ao jogo, como no ReShade): copiar a pasta de
  dados para outra máquina ou outro ponto de montagem continua funcionando.
- Busca em camadas: pasta do jogo → caminhos globais. O primeiro arquivo com aquele nome vence. Assim um
  shader problemático num jogo é substituído só ali, e o resto continua global.
- Nada é gravado na pasta do jogo. Quem escreve nesses arquivos é o `lrctl`/`lrpanel`, nunca a layer.
- Reinstalar o sistema: instalar o projeto, criar o `location.ini` (uma linha) e pronto.

## Formato do preset: o do ReShade

Verificado no código do ReShade (ver o estudo, seção 3). Um preset da comunidade funciona sem conversão.

```ini
PreprocessorDefinitions=RESHADE_DEPTH_INPUT_IS_REVERSED=1
Techniques=Sepia@Sepia.fx,Bloom@Bloom.fx
TechniqueSorting=Sepia@Sepia.fx,Bloom@Bloom.fx
KeySepia@Sepia.fx=122,0,0,0

[Sepia.fx]
Intensity=0.8
PreprocessorDefinitions=
```

- `Techniques` = ligadas; `TechniqueSorting` = ordem de todas; `Key<técnica>` = atalho da técnica;
  `[Arquivo.fx]` = valores dos uniforms e macros daquele efeito.
- **Ordem = a da lista**: a primeira roda primeiro e a de baixo recebe o resultado dela (é a semântica do
  ReShade e do vkBasalt).
- Macros: efeito > preset > globais (`config.ini`); valor vazio vale `1`. Mudar uma macro recompila.
- Chaves e comentários que não conhecemos são **preservados** ao regravar; a gravação é atômica.
- Técnica desconhecida no preset: aviso e o preset segue (como no ReShade), sem falhar.

`game.ini` (nosso, o ReShade não tem equivalente porque cada jogo tem seu `ReShade.ini`):
`preset=Noturno.ini`, atalhos de preset, macros do jogo, e, se preciso, o id que o identifica.

## Identificar o jogo

A layer informa o nome do executável sem extensão (`exe=` no status; sob Proton, o `.exe` do jogo, lido do
primeiro argumento do processo) e o nome do `VkApplicationInfo` (`app=`). O id é o executável e, sem ele, o
`app`. (O mapa `[Games]` do `config.ini` para corrigir um id continua previsto, ainda não existe.) Sem pasta correspondente, vale o preset
global. A pasta só é criada quando o usuário salva algo.

## Como isso conversa com a layer

A layer **não lê `.ini` nem procura shaders**: código dentro do jogo fica pequeno. O controlador
(`lrctl daemon` / `lrpanel`):

1. descobre o jogo de cada layer viva e escolhe o preset (do jogo → global);
2. compila cada técnica **antes** de o jogo precisar, para a resolução dele (o ReShade compila dentro do
   jogo, no primeiro quadro, e isso trava; aqui não);
3. publica **por processo** um plano: lista ordenada de técnicas (bundle, técnica, ligada) e o bloco de
   uniforms de cada uma, em `ctl-<id>` na memória compartilhada, com seqlock como os blobs de hoje;
4. grava no `.ini` quando o usuário muda algo, e **relê** quando o arquivo muda no disco.

A escolha e os parâmetros globais de hoje viram o plano do "jogo sem preset": nada quebra.

## Cadeia na layer

Cada técnica é uma etapa; a N lê o resultado da N-1 (a primeira lê o frame do jogo) e a última escreve no
que vai para a tela, com *ping-pong* entre duas imagens. É o que o ReShade faz (copia o back buffer antes de
um passe que o lê). Texturas de mesmo nome em efeitos diferentes são compartilhadas no ReShade: ficam para
depois. Compute shaders e mipmaps também não existem no nosso `postfx` e ficam fora deste plano.

## Linha de comando

```sh
lrctl preset new Noturno --game vkcube      # cria <Presets>/vkcube/Noturno.ini copiando o que vale agora, e o escolhe
lrctl preset enable Sepia --game vkcube     # liga uma técnica (Nome@Arquivo.fx, ou só Nome)
lrctl preset move Colourfulness 1 --game vkcube
lrctl preset set Sepia.fx Strength 0.4 --game vkcube
lrctl preset show --game vkcube             # técnicas em ordem, [x] ligada
lrctl preset use none --game vkcube         # volta ao preset global
```

`--file <arquivo.ini>` edita um arquivo qualquer. Com o `lrctl daemon` rodando, o jogo aberto passa a usar a
mudança assim que o arquivo é gravado. Sem `--game`/`--file` o comando diz o que falta.

## Fases (commits pequenos, cada fase com testes próprios)

- **F0 — configuração e caminhos.** *(feita em 2026-10-09: crates `ini` e `config`; `lrctl config` e
  `lrctl config init <pasta>`; o compilador usa a configuração e vê shaders novos sem reiniciar. Falta: a
  busca em camadas por jogo só é usada quando existir jogo identificado, na F4.)* Crate para
  `location.ini`/`config.ini`, caminhos em camadas, `**`, relativos ao arquivo.
- **F1 — plano por processo no ipc.** *(feita em 2026-10-09: canal `ctl-<id>` por layer com escolha e
  parâmetros próprios, o vigia compila para a escolha efetiva de cada jogo, `lrctl ... --layer <pid|nome>`;
  verificada com dois `vkcube` reais em `scripts/dev.sh smoke-layer`. Falta: a interface gráfica ainda
  só mexe nos valores de todos; isso é a F5.)* Escolha e parâmetros por layer;
- **F2 — crate `preset`.** *(feita em 2026-10-09: ler/gravar o formato, técnicas e ordem
  (`resolve`, `set_enabled`, `move_technique`), macros em três níveis, valores de uniform, `Key<técnica>`;
  ida e volta byte a byte conferida com 4 presets reais; `preset::game` resolve jogo → arquivo via `game.ini`.)*
  Parser/gravador do formato acima, testado com presets reais; resolução jogo → arquivo.
- **F3 — cadeia na layer.** *(parte do `postfx` feita em 2026-10-10: `postfx::Chain` roda várias técnicas em
  ordem, num único submit; testada na GPU de software com dois efeitos em ordem trocada contra o cálculo na
  CPU. A layer usa `Chain`: com plano publicado constrói uma etapa por técnica; sem plano, a escolha única é uma
  cadeia de uma etapa.)*
  Várias técnicas em ordem; `postfx` ganha o encadeamento e a escolha de técnica
  no bundle. Teste: dois efeitos em ordem trocada dão imagens diferentes (golden).
- **F4 — plano a partir do preset.** *(feita em 2026-10-10: `control_core::game_plan` monta o plano do preset
  (técnicas ligadas na ordem, bundle compilado, imagem do buffer de uniforms com os valores do preset);
  `control_core::games` o publica no canal de cada jogo aberto, relê quando o arquivo muda, compila cada
  resolução e limpa o plano se o preset some; `scripts/dev.sh smoke-preset` confere de ponta a ponta com um
  vkcube. Falta: macros que recompilam (F6) e o id do jogo pelo nome do executável.)* O controlador monta o
  plano de cada jogo e o mantém atualizado.
- **F5 — interface.** Jogos selecionáveis; técnicas com ligar/desligar e arrastar para reordenar; busca;
  seletor de preset, "+" (herdando o atual), salvar e restaurar tudo.
- **F6 — macros por nível** (recompila), atalhos de preset, anterior/próximo, transição suave de valores.
- **F7 — o resto:** atalhos por técnica, capturas de tela, OSD, imagens (LUTs), fontes de mouse/teclado.

## Decisões em aberto

1. **Pasta de dados única com `location.ini`** (recomendado) ou só variáveis de ambiente e `config.ini` em
   `~/.config`? A segunda perde a configuração ao formatar.
2. **Entrada no Wayland** (`mousepoint`, `key`…): uma layer não recebe entrada. Opções: ignorar essas fontes
   (valor padrão, como hoje); ou o controlador ler o mouse por um canal permitido (portal) — a decidir só
   se algum shader que você use precisar.
3. **Id do jogo** = nome do executável (recomendado), corrigível no `config.ini`.
4. **O mesmo efeito duas vezes na cadeia** (o vkBasalt permite; o ReShade não, porque o nome da técnica
   é único): não suportar na primeira versão.
