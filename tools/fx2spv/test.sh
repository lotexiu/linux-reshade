#!/usr/bin/env bash
# Compila efeitos de exemplo com o fx2spv e valida cada módulo SPIR-V com o spirv-val.
# Os efeitos próprios ficam em effects/; os da comunidade vêm do repositório reshade-shaders.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
# shellcheck source=deps.sh
. "$HERE/deps.sh"
TOOL="$ROOT/build/fx2spv/fx2spv"
OUT="$ROOT/build/bundles"
SLIM="$ROOT/third_party/reshade-shaders-slim"
LEGACY="$ROOT/third_party/reshade-shaders-legacy"

[ -x "$TOOL" ] || { echo "rode tools/fx2spv/build.sh primeiro" >&2; exit 1; }

fetch_shaders() { # fetch_shaders <diretório> <commit>
  local dir="$1" commit="$2"
  [ -d "$dir/.git" ] && [ "$(git -C "$dir" rev-parse HEAD)" = "$commit" ] && return
  rm -rf "$dir"; mkdir -p "$dir"
  git -C "$dir" init -q && git -C "$dir" remote add origin "$SHADERS_URL"
  git -C "$dir" fetch -q --depth 1 origin "$commit" && git -C "$dir" checkout -q FETCH_HEAD
}
fetch_shaders "$SLIM" "$SHADERS_SLIM_COMMIT"
fetch_shaders "$LEGACY" "$SHADERS_LEGACY_COMMIT"

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
exit $fail
