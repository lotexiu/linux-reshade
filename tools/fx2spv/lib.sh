# Funções compartilhadas pelos scripts do fx2spv (test.sh, compile.sh). Use com `source`.
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
# shellcheck source=deps.sh
. "$HERE/deps.sh"

TOOL="$ROOT/build/fx2spv/fx2spv"
OUT="$ROOT/build/bundles"
SLIM="$ROOT/third_party/reshade-shaders-slim"
LEGACY="$ROOT/third_party/reshade-shaders-legacy"

# fetch_shaders <diretório> <commit>: baixa o repositório de efeitos da comunidade no commit exato.
fetch_shaders() {
  local dir="$1" commit="$2"
  [ -d "$dir/.git" ] && [ "$(git -C "$dir" rev-parse HEAD)" = "$commit" ] && return
  rm -rf "$dir"; mkdir -p "$dir"
  git -C "$dir" init -q && git -C "$dir" remote add origin "$SHADERS_URL"
  git -C "$dir" fetch -q --depth 1 origin "$commit" && git -C "$dir" checkout -q FETCH_HEAD
}

ensure_shaders() {
  fetch_shaders "$SLIM" "$SHADERS_SLIM_COMMIT"
  fetch_shaders "$LEGACY" "$SHADERS_LEGACY_COMMIT"
}

# find_effect <nome|caminho.fx>: imprime o caminho do .fx (efeitos próprios, depois os da comunidade).
find_effect() {
  local name="$1"
  if [ -f "$name" ]; then echo "$name"; return; fi
  for f in "$ROOT/effects/$name.fx" "$LEGACY/Shaders/$name.fx" "$SLIM/Shaders/$name.fx"; do
    [ -f "$f" ] && { echo "$f"; return; }
  done
  return 1
}

# Opções de include para os efeitos da comunidade (headers no "slim", efeitos no "legacy").
include_flags() { printf -- '-I\n%s\n-I\n%s\n' "$SLIM/Shaders" "$LEGACY/Shaders"; }
