# Eficiência dos efeitos e o que o runtime pode oferecer

Medido na GPU real (RTX 3060), 1920x1080, cadeia do preset "10 RTGI completo com especular", profundidade falsa, 60 quadros,
com `LINUX_RESHADE_CHAIN_PROFILE=1` (teste `a_chain_of_compiled_bundles_runs`, usando os bundles do cache, que têm as imagens).
Soma 34 ms; no jogo a cadeia roda mais leve porque a GPU está em outro estado de clock, mas a proporção é a mesma.

| Efeito | ms | Onde gasta |
|---|---|---|
| RTGI Diffuse | 10,1 | traçado (`TraceWrapCubicCS` 5,1), pirâmide de profundidade própria (`DownsampleDepthCS` 1,8), reprojeção temporal 0,9 |
| DOF | 6,0 | momentos em mipmap (`DownsampleMomentsPS` 2,4), bokeh 0,7, filtros |
| RTGI Specular | 5,9 | pirâmide Hi-Z própria (`SpecularHiZDownsampleCS` 2,3), traçado 0,9, radiância em mip 1,0 |
| Launchpad | 4,6 | escrita de features e profundidade (1,8), normais (1,0), albedo (0,9) |
| Sharpen | 2,3 | um passe só (`MainPS`) |
| ExposureFusion | 1,8 | pirâmide Retinex |
| FilmGrain | 1,7 | `ApplyGrainCS` 1,0 + `ResolvePS` 0,8: grão de química do filme |
| ReGrade | 1,6 | aplicação da LUT |

## Aviso sobre estes números

Os milissegundos acima vêm do teste da cadeia fora do jogo e **só valem como proporção**. Lá, um passe que só devolve a imagem
(`tex2Dfetch` e escrita) custa 1,6 ms a 1080p e um que escreve uma cor constante 0,69 ms; o custo cresce com os pixels (0,39 ms a
540p, 1,6 a 1080p, 6,5 a 2160p), mas é muito maior do que a GPU deveria gastar nisso. O teste mede com a GPU num estado diferente do
do jogo, então a conclusão "o Sharpen e o grão custam muito mais do que um sharpen simples" **não está provada**: um passe trivial
já custa 1,6 ms ali. O custo real por efeito vem do jogo aberto (`lrctl stats on`, que lista a GPU de cada etapa).

## Medido no jogo (Roblox/Sober, 1920x1080, preset 10, `lrctl stats on`)

GPU dos efeitos: **18,05 ms** por quadro: RTGI Diffuse 8,07, RTGI Specular 3,34, Launchpad 2,77, DOF 2,17, e menos de 0,5 cada:
Sharpen 0,39, FilmGrain 0,45, SMAA 0,36, ExposureFusion 0,34, ReGrade 0,14. Então o piso de um passe é cerca de 0,1 a 0,4 ms (o teste
fora do jogo inflava isso de 4 a 10 vezes) e Sharpen/grão **não** são o problema: só os dois RTGI, o Launchpad e o DOF pesam.

Nesta cena o jogo roda a 45 quadros/s com e sem efeitos (21,9 ms sem, 23,2 ms com): ela é limitada pelo processador do jogo e a GPU dos
efeitos se esconde atrás disso. Acelerar os efeitos só aparece em cenas limitadas pela GPU (por exemplo o minigame a 240 quadros/s).

## O que se repete entre efeitos (e o runtime pode dar uma vez só)

- **Pirâmides de profundidade**: Launchpad, RTGI Diffuse e RTGI Specular cada um monta a sua (`DownsampleDepth` 1,8 ms,
  `HiZ` 2,3 ms, features do Launchpad 1,8 ms). Um serviço do runtime que entregasse profundidade linear e a pirâmide
  (mín/máx e média) como textura nomeada, calculada uma vez por quadro, tiraria perto de 4 ms desta cadeia.
- **Normais**: o Launchpad as calcula; qualquer efeito de luz que não use o Launchpad as calcularia de novo.
- **Histórico temporal**: cada efeito temporal (RTGI, Specular) cria e troca (ping-pong) as próprias texturas de quadro anterior.
- **Ruído azul**: texturas de arquivo carregadas e amostradas por cada efeito.

## Recursos do runtime que servem a qualquer efeito (sem ser feitos para um efeito só)

1. **Cache de pipelines em disco** (feito). Montar a cadeia do preset 10 levou 320 ms com o cache frio do driver e 116 ms com o
   nosso cache quente; na primeira sessão sem cache do driver (Flatpak) chegou a 4,7 s. Vale para todos os efeitos e atenua o
   congelamento ao trocar de preset.
2. **Serviços de profundidade**: semânticas novas (`LINEARDEPTH`, `DEPTHMIPS`, `NORMALS`) que o runtime preenche uma vez por quadro,
   em meia resolução quando o efeito aceitar. É o recurso de maior retorno medido.
3. **Escala de qualidade global**: uma constante (especialização) que reduz as texturas internas de todos os efeitos (1x, 1/2, 1/4)
   e um modo "orçamento de GPU" que ajusta a escala sozinho com a medição de tempo por passe que a layer já faz.
4. **Histórico gerenciado**: anotação `history = N` numa textura, e o runtime cuida da troca e do reinício quando a resolução muda.
5. **Execução alternada**: anotação para um passe rodar a cada N quadros (ou em tabuleiro) e reaproveitar o resultado, útil para
   GI, bloom, adaptação de exposição.
6. **Formato dos alvos intermediários**: trocar RGBA16F por R11G11B10F nos intermediários cujo alfa nunca é lido (menos banda).
7. **Subgrupos (wave ops)** nos passes de compute quando a GPU tem, para reduções e pirâmides.

## Candidatos a versão nova (do mais caro, melhor do grupo)

1. **GI difusa** (RTGI Diffuse): usar o serviço de profundidade, traçar em meia resolução com reconstrução, histórico gerenciado.
2. **DOF**: momentos em passe único de compute com subgrupos, bokeh em meia resolução.
3. **Sharpen** e **grão**: a medição fora do jogo não mostra que sejam caros (ver o aviso); decidir depois de medir no jogo.

Ordem sugerida: serviços de profundidade (2), escala de qualidade (3), depois a GI.
