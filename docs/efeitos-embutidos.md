# Efeitos que vêm com o programa e o contador de desempenho

## Onde ficam
A pasta `effects/` do repositório vai junto com o programa (`share/linux-reshade/effects` no AppImage). O compilador a usa **por último**: um efeito com o mesmo
nome na pasta do usuário vale mais, e quem já tinha uma pasta de dados antes de um efeito embutido existir o recebe sem copiar nada. As texturas deles ficam em
`effects/Textures/` (também por último na busca).

## Os efeitos básicos (escritos para este projeto, sem dependências do ReShade.fxh)
| Efeito | O que faz | Custo |
|---|---|---|
| `BasicSharpen` | nitidez (máscara de 4 vizinhos) com limite de realce | 5 leituras por pixel |
| `BasicVibrance` | vibração, saturação, contraste e brilho | 1 leitura |
| `BasicVignette` | vinheta (raio, suavidade, redonda ou da proporção da tela) | 1 leitura |
| `BasicLUT` | correção de cor por LUT 3D (`Textures/BasicLUT.png`, 16 blocos de 16x16; a que vem é neutra) | 3 leituras |
| `Sepia`, `DepthView`, `ComputeInvert` | exemplos e depuração | |

Para dar um visual ao `BasicLUT`, troque `BasicLUT.png` por uma LUT sua no mesmo formato (256x16).

## O contador de desempenho (`FPSCounter`)
Mostra no canto do jogo: **FPS agora** (suavizado), **média**, **mínimo**, **máximo** e **1% low** dos últimos 5 segundos, e o **tempo de quadro** em ms.
As estatísticas vêm do runtime da layer: as fontes `fps_avg`, `fps_min`, `fps_max` e `fps_low1` (uniforms com `source = "..."`), calculadas numa janela de
5 s e refeitas a cada 10 quadros. Um quadro além de 1 s vale 1 s (pausa de carregamento não é desempenho).

Liga-se em **Configurações ▸ Overlay** (`[OVERLAY]` do `config.ini`: `Stats`, `Corner` 0-3, `Scale` 1-8, `Details`). O daemon acrescenta o efeito **por último** ao plano de cada
jogo (depois dos efeitos do preset), a menos que o preset já o tenha. Limitação: precisa de um preset (do jogo ou o global); sem nenhum, não há plano.
Também dá para pôr o `FPSCounter` num preset como um efeito qualquer.

## Ferramentas de desenvolvimento
- `cargo run --release -p control-core --example compile -- <Efeito> <L> <A>` compila como o painel (com as texturas), usando `LINUX_RESHADE_CACHE` e
  `LINUX_RESHADE_RESOURCES=$PWD`.
- `LINUX_RESHADE_CHAIN_FPS=avg,min,max,low1` no teste `a_chain_of_compiled_bundles_runs` mostra o contador com valores conhecidos (`LINUX_RESHADE_CHAIN_PNG` grava a imagem).
