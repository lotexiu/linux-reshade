# Levantamento dos efeitos do dono (2026-10-10)

`scripts/dev.sh fx-survey` compila todos os `.fx` da pasta de efeitos (os 152 do dono) e executa cada bundle na GPU de
software (lavapipe), com uma imagem de profundidade falsa para os que a pedem. Não confere pixels: confere que o
runtime cria, grava, submete e destrói o efeito.

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
