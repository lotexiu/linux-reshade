# fx2spv

Compila um efeito ReShade FX para um **bundle** (`manifest.json` + um `.spv` por entry point; formato
em [`docs/bundle-format.md`](../../docs/bundle-format.md)). É uma ferramenta separada: nada dela é
ligado à layer, que só lê o bundle.

```
scripts/dev.sh fx-tool                       # baixa as dependências fixadas e compila (git + g++)
build/fx2spv/fx2spv --width 1920 --height 1080 -I dir-de-headers -o saida/ efeito.fx
scripts/dev.sh fx-test [Efeito...]           # compila efeitos de exemplo e valida o SPIR-V
```

Opções: `-I dir` (include), `-D NOME[=VALOR]`, `--width/--height` (valores de `BUFFER_WIDTH/HEIGHT`
com que o efeito é compilado), `--spirv-version 1.0|1.3` (padrão 1.0), `--debug`.

## Como funciona

Reproduz o fluxo do runtime do ReShade no Vulkan: pré-processador (com as mesmas macros `BUFFER_*`),
parser e gerador de SPIR-V do `reshadefx`, e um módulo SPIR-V por entry point. Depois escreve o
manifesto. O diretório de saída é publicado por renomeação, então ninguém vê um bundle pela metade.

## Licença e dependências

O `reshadefx` (`source/effect_*` do ReShade, © Patrick Mours) é **BSD-3-Clause**. Ele **não é copiado
para este repositório**: o `build.sh` o baixa, no commit fixado em [`deps.sh`](deps.sh), para
`third_party/` (ignorado pelo git) e o compila junto com o `main.cpp` daqui. Quem distribuir o
binário `fx2spv` precisa manter o aviso de copyright e a licença do ReShade, conforme a BSD-3.
Os headers do SPIR-V vêm do repositório `KhronosGroup/SPIRV-Headers`.

Os efeitos de `reshade-shaders` usados em `fx-test` têm licenças próprias (indicadas no cabeçalho de
cada arquivo); são baixados só para teste e nunca versionados aqui.

## Remendos sobre o ReShade fixado

`patches/*.patch` são aplicados por `build.sh` sobre `third_party/reshade` (uma vez por checkout; se não
se aplicarem ao commit fixado, a construção falha). Hoje há um:

- `0001`: `#include` escrito para o Windows. `\` vira `/` e o arquivo é procurado sem diferenciar maiúsculas
  (`.\MartysMods\mmx_global.fxh`, `Reshade.fxh`, `FXShaders/` contra `FxShaders`). Numa pasta real com 152
  efeitos, subiu de 108 para 135 os que compilam; os 17 restantes usam imagens de armazenamento (compute).
