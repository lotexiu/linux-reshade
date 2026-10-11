# Levantamento dos efeitos do dono (2026-10-10)

`scripts/dev.sh fx-survey` compila todos os `.fx` da pasta de efeitos (os 152 do dono) e executa cada bundle na GPU de
software (lavapipe), com uma imagem de profundidade falsa para os que a pedem. Não confere pixels: confere que o
runtime cria, grava, submete e destrói o efeito.

## Depois do compute e das texturas 1D/3D (2026-10-10, mesma máquina)

De 152 efeitos: **119 executam** na GPU de software (todas as técnicas de cada um são criadas e submetidas); **27**
só ficam de fora do levantamento por faltar a imagem de uma textura (o app de controle as monta no uso, então
rodariam); **6** são recusados de verdade:

| Efeito | Por quê |
|---|---|
| `cBlending`, `cFrameBlending` | escrita sRGB numa textura de render target |
| `kDatamosh` | leitura sRGB de uma textura de render target |
| `MartysMods_LUTMANAGER` | semântica própria (`MARTY_LUT_MANAGER`) |
| `Scatter`, `dh_uber_rt` | um passe amostra a própria textura de saída (realimentação no mesmo passe) |

Dos efeitos de compute (18 antes): todos criam e executam agora (`MartysMods_MXAO`, `REGRADE`, `SMAA`, `INSIGHT`,
`ColorSort_CS`, `Frequency_CS`, `RealLongExposure`...), e os que precisavam de texturas 1D/3D também
(`DEPTHOFFIELD`, `FFTBLOOM`, `FILMGRAIN`, `PARALLAX_DOF`, `RTGI_DIFFUSE`, `lilium__hdr_and_sdr_analysis`).

Falta conferir, e **não foi testado em jogo nem em GPU real**: o compute só rodou na lavapipe; a validation layer não
reclamou do runtime, mas reclamou de três coisas dos shaders (não do runtime): `Offset` fora do gather (a layer agora
liga `shaderImageGatherExtended` no dispositivo do jogo), memória compartilhada acima de 32 KiB num efeito (a lavapipe
limita a 32 KiB; as GPUs NVIDIA permitem 48 KiB) e SPIR-V que o `spirv-val` recusa em 6 efeitos
(`MartysMods_EXPOSUREFUSION`, `FFTBLOOM`, `INSIGHT`, `vort_HDR`, `vort_Motion`, `lilium__hdr_and_sdr_analysis`: um
`OpFunctionCall` com argumento de tipo diferente do parâmetro, defeito do gerador de SPIR-V do compilador).

## Antes (só o levantamento inicial)

| Resultado | Quantos | O que é |
|---|---|---|
| Executam | **107** | inclui os de profundidade (`AdaptiveFog`, `CinematicDOF` não: ver imagens abaixo), `qUINT_*`, `cShade`, `lilium`... |
| Recusados só por faltar a imagem de uma textura | 19 | `LUT`, `SMAA`, `Heightfog`, `NGLighting`... O levantamento não monta as imagens (o app de controle monta no uso), então rodariam. `Quark_Fast_GI`/`Quark_Plus_RT` apontam para um arquivo `?`. |
| **Não compilam: imagens de armazenamento (compute)** | **18** | `ColorSort_CS`, `Frequency_CS`, `Gravity_CS`, `RealLongExposure`, `lilium__hdr_and_sdr_analysis` e 13 do Marty (`DEPTHOFFIELD`, `FFTBLOOM`, `FILMGRAIN`, `INSIGHT`, `MXAO`, `PARALLAX_DOF`, `REGRADE`, `REGRADE+`, `RELIGHT`, `RTGI_DIFFUSE`, `RTGI_SPECULAR`, `SMAA`) |
| Passe amostra a própria textura de saída | 2 | `Scatter`, `dh_uber_rt` (realimentação no mesmo passe) |
| Escrita sRGB numa textura | 2 | `cBlending`, `cFrameBlending` |
| Leitura sRGB de textura de render target | 1 | `kDatamosh` |
| Semântica própria | 1 | `MartysMods_LUTMANAGER` |
| SPIR-V que o `spirv-val` recusa (mas roda na lavapipe) | 3 | `MartysMods_EXPOSUREFUSION`, `vort_HDR`, `vort_Motion` |

Conclusão: **o maior bloqueio é o compute** (18 efeitos, quase todos da suíte do Marty que o dono usa). É o próximo passo.
