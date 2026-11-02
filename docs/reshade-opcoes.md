# Opções do ReShade original x linux-reshade

O que o ReShade 6 oferece nas abas Home/Settings/Statistics/Log/About e onde isso está aqui. Atualizado em 2026-11-02.

| Opção do ReShade | Aqui |
|---|---|
| Pastas de efeitos, texturas, presets e cache (`EffectSearchPaths`, `TextureSearchPaths`, `PresetPath`, `IntermediateCachePath`) | **feito**: aba Pastas das Configurações (grava o `config.ini`; escolhe pasta por diálogo) |
| Tecla de ligar/desligar efeitos (`KeyEffects`) | **feito**: aba Atalhos (aceita `'`, `OEM_7`, combinações) |
| Tecla de próximo/anterior preset (`KeyNextPreset`/`KeyPreviousPreset`) | **feito** |
| Tecla de captura (`KeyScreenshot`), pasta (`SavePath`) e guardar o quadro sem efeitos (`SaveBeforeShot`) | **feito**: PNG por jogo, com e sem efeitos, com o mesmo nome base (`..._sem-efeitos.png`); também `lrctl screenshot` |
| Presets por jogo, escolha e criação | **feito** (painel e `lrctl preset`) |
| Macros (PreprocessorDefinitions) globais, por preset e por efeito | **feito** |
| Controles de cada efeito (sliders com valor digitável, combos, cores...) | **feito** |
| Escolha da profundidade (candidatas, passe, limpeza) | **feito** |
| Tecla para recarregar os efeitos (`KeyReload`) | **feito**: apaga os compilados dos jogos abertos e o vigia refaz (pega o `.fx` editado) |
| Tecla por técnica (`Key<técnica>`) | **feito**: botão em cada efeito do painel; grava o `Key<técnica>` do preset no formato do ReShade (presets do Windows valem) e o daemon registra a tecla |
| Modo de desempenho (constantes no lugar de uniforms) | falta |
| Formato da captura (JPEG/BMP), `SavePresetFile`, `SaveOverlayShot` | falta (só PNG, sem overlay: o painel é outra janela) |
| OSD: FPS e tempo de quadro | **feito** como efeito: `FPSCounter` (canto e tamanho configuráveis); relógio falta |
| Estatísticas (tempo por técnica, uso do processador pela layer) | **feito**: `lrctl stats on` + aba Desempenho (GPU por efeito, processador por quadro); memória falta |
| Abas Log e Sobre | **feito**: abas Registro e Sobre nas Configurações |
| Transição suave entre presets, atalhos de preset por tecla própria | falta |
| Uniforms de mouse/teclado (`mousepoint`, `key`, `overlay_open`...) | `overlay_active`/`overlay_open` feitos (auxílio do autofoco); mouse e teclado não: a layer não recebe entrada |
| Add-ons | não se aplica |

A captura de tela vale para todo jogo aberto com a layer (a Wayland não diz qual janela tem o foco).
