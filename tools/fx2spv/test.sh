#!/usr/bin/env bash
# Compila efeitos de exemplo com o fx2spv e valida cada módulo SPIR-V com o spirv-val.
# Os efeitos próprios ficam em effects/; os da comunidade vêm do repositório reshade-shaders.
set -uo pipefail

# shellcheck source=lib.sh
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

[ -x "$TOOL" ] || { echo "rode tools/fx2spv/build.sh primeiro" >&2; exit 1; }
ensure_shaders

fail=0
check() { # check <arquivo.fx> [opções do fx2spv...]
  local fx="$1"; shift
  local name; name="$(basename "$fx" .fx)"
  if ! "$TOOL" --width 1920 --height 1080 -I "$SLIM/Shaders" -I "$LEGACY/Shaders" -o "$OUT/$name" "$@" "$fx" 2>"$OUT/$name.log"; then
    printf '  FALHOU  %s\n' "$name"; sed 's/^/          /' "$OUT/$name.log" | head -5; fail=1; return
  fi
  for spv in "$OUT/$name"/*.spv; do
    if ! spirv-val --target-env vulkan1.0 "$spv" 2>"$OUT/$name.val"; then
      printf '  INVÁLIDO %s (%s)\n' "$name" "$(basename "$spv")"; head -3 "$OUT/$name.val" | sed 's/^/          /'; fail=1; return
    fi
  done
  printf '  ok      %s\n' "$name"
}

# Sem argumentos, usa um conjunto de efeitos reais que cobre multipasso, texturas e vários uniforms.
if [ $# -eq 0 ]; then
  set -- AdaptiveSharpen Colourfulness Denoise FilmicPass FineSharp GaussianBlur HSLShift \
         HighPassSharpen LevelsPlus NightVision MagicBloom Bloom Deband Daltonize LUT
fi

mkdir -p "$OUT"
for fx in "$ROOT"/effects/*.fx; do check "$fx"; done
for name in "$@"; do
  if [ -f "$LEGACY/Shaders/$name.fx" ]; then check "$LEGACY/Shaders/$name.fx"; else check "$SLIM/Shaders/$name.fx"; fi
done
# #include escrito para o Windows (barra invertida, maiúsculas trocadas) tem de achar o arquivo (remendo em patches/).
inc_dir="$OUT/include-windows"; rm -rf "$inc_dir"; mkdir -p "$inc_dir/Sub"
printf 'float4 cor() { return float4(1.0, 0.5, 0.25, 1.0); }\n' > "$inc_dir/Sub/Cor.fxh"
cat > "$inc_dir/Teste.fx" <<'FX'
#include ".\sub\COR.fxh"
#include "reshade.fxh"
float4 PS_Teste(float4 pos : SV_Position, float2 uv : TEXCOORD) : SV_Target { return cor() * tex2D(ReShade::BackBuffer, uv); }
technique Teste { pass { VertexShader = PostProcessVS; PixelShader = PS_Teste; } }
FX
check "$inc_dir/Teste.fx"

exit $fail
