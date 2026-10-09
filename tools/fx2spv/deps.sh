# Dependências do fx2spv, fixadas por commit. Lido por build.sh.
RESHADE_URL=https://github.com/crosire/reshade.git
RESHADE_COMMIT=7bf9de8b33bcc76c3177007e65d73c72dd0f34c0
SPIRV_HEADERS_URL=https://github.com/KhronosGroup/SPIRV-Headers.git
SPIRV_HEADERS_COMMIT=86f980c731e62ae4eaf383d320449d71687936bf
# Só para testes (não são dependências de build): efeitos reais da comunidade.
# Dois ramos do mesmo repositório: "slim" traz os headers (ReShade.fxh etc.) e poucos efeitos;
# "legacy" traz o conjunto clássico de efeitos, que usa esses headers.
SHADERS_URL=https://github.com/crosire/reshade-shaders.git
SHADERS_SLIM_COMMIT=fd0022170615ce0d8162d219bff07232fa6dd84f
SHADERS_LEGACY_COMMIT=bcb5ba54199f4455026dd8ba66dc1b74461d3152
