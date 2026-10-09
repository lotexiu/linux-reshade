# Estudo: como o ReShade funciona

Resultado da leitura do código-fonte do ReShade (`crosire/reshade`, branch `main`, BSD-3), em 2026-10-09,
para decidir o que copiar, o que mudar e o que descartar. Serve de referência para `docs/presets.md`.

**Limites da leitura** (para ninguém tomar isto por mais do que é): li cerca de metade de `runtime.cpp`
(config, presets, carregamento de efeitos, passes) e do `runtime_gui.cpp` (abas Home e Settings). **Não li**:
o corpo de `create_effect`, o editor de variáveis por `ui_type`, as abas Statistics/Log/About, o tratamento
de `timeout` e `nosave` nos efeitos, a entrada (`input.cpp`) e o addon de profundidade
(`generic_depth`, só vi o que o fórum diz). Onde algo vem de fórum e não do código, está dito.

## 1. Onde ficam as coisas no ReShade

- O `ReShade.ini` fica **ao lado do executável do jogo** (junto da DLL injetada). Cada jogo tem o seu.
  Chaves ausentes caem em uma configuração global (`global_config()`; onde ela fica eu não verifiquei).
- Caminhos relativos (`.\reshade-shaders\Shaders`) valem em relação à pasta da DLL, ou seja, **dentro da
  pasta do jogo**. Reinstalar o jogo apaga shaders, texturas e presets.
- Dá para apontar `EffectSearchPaths`/`TextureSearchPaths` para uma pasta comum (a documentação do
  repositório de shaders manda isso: extrair em qualquer lugar e adicionar os caminhos).
- Um caminho terminado em `**` é buscado **recursivamente**; extensões de efeito: `.fx`.
- Há **um preset ativo por vez** (`PresetPath`), mais uma lista (`PresetFiles`, no `.ini`) que a interface
  usa para trocar. Não existe "pasta de presets por jogo" no ReShade: a ligação jogo→preset é o próprio
  `ReShade.ini` do jogo.

**Diferença do nosso projeto (decidida):** nada fica na pasta do jogo; shaders **globais** por padrão e
uma pasta própria por jogo só quando for preciso; todas as pastas configuráveis (para sobreviver a
reinstalação ou formatação).

## 2. `ReShade.ini` (configuração) — chaves lidas no código

| Seção | Chaves |
|---|---|
| `INPUT` | `ForceShortcutModifiers`, `KeyScreenshot` (padrão: PrintScreen), `KeyEffects`, `KeyNextPreset`, `KeyPreviousPreset`, `KeyReload`, `GamepadNavigation` |
| `GENERAL` | `NoDebugInfo`, `NoEffectCache`, `NoReloadOnInit`, `EffectSearchPaths` (padrão `.\`), `TextureSearchPaths` (padrão `.\`), `PerformanceMode`, `PreprocessorDefinitions` (globais), `SkipLoadingDisabledEffects`, `IntermediateCachePath` (padrão: temp), `StartupPresetPath`, `PresetPath` (padrão `ReShadePreset.ini`), `PresetTransitionDuration` (0–10000 ms), `PresetShortcutKeys`, `PresetShortcutPaths` |
| `SCREENSHOT` | `SavePath`, `SoundPath`, `ClearAlpha`, `FileFormat` (bmp/png/jpeg/jxl), `FileNaming` (padrão `%AppName% %Date% %Time%_%Count%`), `JPEGQuality`, `SaveBeforeShot`, `SavePresetFile`, `PostSaveCommand` (+ `Arguments`, `WorkingDirectory`, `HideWindow`), `SaveOverlayShot` |

Pontos que importam para nós:
- **Atalho por preset** (`PresetShortcutKeys` + `PresetShortcutPaths`): uma tecla leva direto a um preset.
- **Transição entre presets** (`PresetTransitionDuration`): os valores `float` são interpolados.
- **Modo desempenho**: compila com otimização maior e troca os uniforms por constantes.
- **Cache de efeitos**: o identificador do cache mistura app, resolução, formato, versão, modo desempenho,
  GPU, **todas as definições de pré-processador**, o arquivo do efeito + data de modificação e a data de
  cada `.fxh` dos caminhos de busca.

## 3. O arquivo de preset — formato exato

Seção global (sem nome):

| Chave | Significado (do código de `save/load_current_preset`) |
|---|---|
| `Techniques` | só as técnicas **ligadas**, como `Nome@Arquivo.fx` (na leitura, aceita também só `Nome`) |
| `TechniqueSorting` | **todas** as técnicas (inclusive desligadas) na ordem de execução; só é gravada se já existia ou se a ordem difere de `Techniques` |
| `PreprocessorDefinitions` | macros do preset inteiro (some do arquivo se vazia) |
| `Key<Nome@Arquivo.fx>` | **atalho que liga/desliga aquela técnica** (conferido: o ponto que eu tinha marcado como "a verificar") |

Uma seção por efeito, nomeada pelo **nome do arquivo** (`[Sepia.fx]`):

| Chave | Significado |
|---|---|
| `PreprocessorDefinitions` | macros só deste efeito |
| `Key<uniform>` | atalho de uniform que aceita (por exemplo, alternar um `bool`) |
| `<nome do uniform>` | o valor (um número por componente); uniforms com `source` ou `nosave` não são gravados |

Regras:
- **Ordem:** ordenação estável pela lista de `TechniqueSorting`. Técnicas fora dela: ordem de declaração
  dentro do arquivo; entre arquivos, ordem alfabética do `ui_label` (ou do nome). Se o preset não tem
  `TechniqueSorting`, usa o do `ReShade.ini` e, por fim, `Techniques`.
- **Técnica desconhecida** no preset: aviso "uses unknown technique" e o preset fica marcado como incompleto
  (não é erro fatal).
- **Anotação `enabled`** na técnica: liga por padrão ao carregar.
- **Precedência de macros:** efeito > preset > globais (`ReShade.ini`); valor vazio vira `1`.
- Gravação: o ReShade grava a cada alteração (com "Auto Save") e descarrega o cache no `present`.

## 4. Macros predefinidas nos efeitos

`__RESHADE__`, `__RESHADE_PERMUTATION__`, `__RESHADE_PERFORMANCE_MODE__`, `__VENDOR__`, `__DEVICE__`,
`__RENDERER__`, `__APPLICATION__` (hash do nome do executável), `BUFFER_WIDTH`, `BUFFER_HEIGHT`,
`BUFFER_RCP_WIDTH`, `BUFFER_RCP_HEIGHT`, `BUFFER_COLOR_SPACE`, `BUFFER_COLOR_FORMAT`,
`BUFFER_COLOR_BIT_DEPTH`. As `RESHADE_DEPTH_*` (reversed, upside-down, logarithmic, far plane, escala e
deslocamento) **não** são do runtime: vêm do `ReShade.fxh` e o usuário as ajusta como
`PreprocessorDefinitions` do preset. Nos shaders que temos aparecem em 11 ocorrências (contagem de linhas, não de arquivos).

## 5. Técnicas, passes e uniforms

- Técnica: anotações vistas no código: `enabled`, `hidden`, `enabled_in_screenshot`, `timeout`
  (desliga sozinha depois de N ms), `ui_label`, `ui_tooltip`. Na prática quase nenhum shader dos conjuntos
  que temos as usa (um por arquivo é a regra: 20 de 25 arquivos têm uma só técnica; o máximo é cinco).
- Passes: antes de um passe que lê o back buffer o ReShade o **copia** para uma textura; o resultado vai para
  o back buffer ou para render targets; há `clear`, *blending* completo, passes de **compute** e geração
  automática de mipmaps. Nosso `postfx` cobre a cópia e o desenho; **compute e mipmaps não**.
- Fontes de uniform (`source`): `frametime`, `framecount`, `random`, `pingpong`, `date`, `timer`, `key`,
  `mousepoint`, `mousedelta`, `mousebutton`, `mousewheel`, `overlay_open`, `overlay_active`,
  `overlay_hovered`, `screenshot`, `bufready_depth`. Nosso runtime tem as seis primeiras.
  **Mouse/teclado/overlay não têm equivalente simples no Wayland** (uma layer não recebe entrada):
  precisa de decisão (ver `docs/presets.md`).
- `ui_type` nos shaders que temos: `drag`, `slider`, `combo`, `radio`, `input`, `color`, `drag2`, `list`.
  O painel cobre slider, combo, cor, número e interruptor; **`radio`, `list` e `drag2` ainda precisam ser
  conferidos**.

## 6. A interface do ReShade (o que o `lrpanel` deve cobrir)

Abas: **Home**, Settings, Statistics, Log, About (e Add-ons).

Home: barra de preset (anterior/próximo, nome atual com marca de "não salvo", navegador de `.ini`, menu de
contexto para atribuir tecla ao preset); "Auto Save"; restaurar tudo (com confirmação); salvar; **"+"
para criar preset** (herdando o atual ou de um modelo); busca por nome/rótulo; "Active to top";
expandir/recolher tudo; lista de efeitos e técnicas com caixa de seleção e **arrastar para reordenar**;
variáveis abaixo; "Reload" (Ctrl/Shift limpa o cache); "Performance Mode". Avisos: "preset inválido" e "o
preset usa efeitos desconhecidos".

Settings: modos de entrada; teclas (overlay, efeitos, recarregar, preset anterior/próximo); duração da
transição; preset de início; caminhos de efeitos e texturas; "carregar só efeitos ativos"; limpar cache;
capturas de tela (tecla, pasta, nome com macros, formato, qualidade, salvar antes/depois, salvar o preset,
comando pós-captura); idioma; tema; **OSD** opcional (relógio, FPS, tempo de quadro, nome do preset, em um dos
quatro cantos).

## 7. O addon de profundidade (marco 5; só fórum, não li o código)

Fórum: depende de `RESHADE_DEPTH_INPUT_IS_REVERSED`, `…_UPSIDE_DOWN`, `…_LOGARITHMIC` e
`…_LINEARIZATION_FAR_PLANE`, ajustados à mão olhando o efeito `DisplayDepth`; o ReShade tem detecção
heurística do buffer de profundidade, com opção de copiá-lo **antes do clear**. Antes do marco 5 isso exige
leitura do código do addon.

## 8. Referência no Linux: vkBasalt

Layer Vulkan existente que roda efeitos do ReShade. Configuração mínima: `effects=a:b:c` (executados **da
esquerda para a direita**; o mesmo efeito pode se repetir), `reshadeTexturePath`, `reshadeIncludePath`,
`toggleKey`, `enableOnLaunch`, e uma opção por parâmetro de cada efeito interno. Não tem interface, presets
nem pasta por jogo (seleciona o arquivo de config por variável de ambiente). Mostra o piso do que o
usuário espera; o ReShade é o teto.

## 9. O que isso muda na nossa proposta

1. O formato de preset é o do ReShade (agora verificado, inclusive `Key<técnica>`): **copiar**.
2. Macros em três níveis (global < preset < efeito) com recompilação: **copiar**, e incluir as macros no
   identificador do bundle (já é assim para o conteúdo SPIR-V).
3. Shaders: **busca em camadas** (pasta do jogo antes da global), `**` recursivo, caminhos relativos à
   pasta-raiz de dados (não ao jogo).
4. Compilar **fora do jogo** (no controlador) elimina a travada de compilação no primeiro quadro que o
   ReShade tem; é uma melhoria real.
5. Interpolar valores na troca de preset é viável (o controlador publica os valores); fica para depois.
6. Lacunas conhecidas e ainda sem plano: compute shaders, mipmaps, mouse/teclado no Wayland, `ui_type`
   `radio`/`list`/`drag2`, `timeout`, OSD, capturas de tela.
