#!/usr/bin/env bash
# Baixa as dependências (fixadas por commit) para third_party/ e compila o fx2spv em build/fx2spv/.
# Só precisa de git e g++ (sem cmake).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
# shellcheck source=deps.sh
. "$HERE/deps.sh"

TP="$ROOT/third_party"
OUT="$ROOT/build/fx2spv"
mkdir -p "$TP" "$OUT"

# fetch <nome> <url> <commit> [diretório-esparso...]: clona só o necessário, no commit exato.
fetch() {
  local name="$1" url="$2" commit="$3"; shift 3
  local dir="$TP/$name"
  if [ -d "$dir/.git" ] && [ "$(git -C "$dir" rev-parse HEAD 2>/dev/null)" = "$commit" ]; then return; fi
  rm -rf "$dir"; mkdir -p "$dir"
  git -C "$dir" init -q
  git -C "$dir" remote add origin "$url"
  if [ $# -gt 0 ]; then
    git -C "$dir" sparse-checkout init --cone
    git -C "$dir" sparse-checkout set "$@"
    git -C "$dir" fetch -q --depth 1 --filter=blob:none origin "$commit"
  else
    git -C "$dir" fetch -q --depth 1 origin "$commit"
  fi
  git -C "$dir" checkout -q FETCH_HEAD
  echo "baixado $name @ ${commit:0:10}"
}

fetch reshade "$RESHADE_URL" "$RESHADE_COMMIT" source
fetch spirv-headers "$SPIRV_HEADERS_URL" "$SPIRV_HEADERS_COMMIT" include

# Remendos nossos sobre o ReShade fixado (tools/fx2spv/patches/*.patch), aplicados uma vez por checkout.
for patch in "$HERE"/patches/*.patch; do
  [ -f "$patch" ] || continue
  if git -C "$TP/reshade" apply --check "$patch" 2>/dev/null; then
    git -C "$TP/reshade" apply "$patch"
    echo "remendo aplicado: $(basename "$patch")"
  elif ! git -C "$TP/reshade" apply --check --reverse "$patch" 2>/dev/null; then
    echo "o remendo $(basename "$patch") não se aplica ao ReShade fixado" >&2
    exit 1
  fi
done

SRC="$TP/reshade/source"
FILES=(effect_lexer effect_parser_exp effect_parser_stmt effect_preprocessor effect_symbol_table effect_expression effect_codegen_spirv)
OBJS=()
for f in "${FILES[@]}"; do
  obj="$OUT/$f.o"
  # Recompila só o que mudou (ou se o commit das dependências mudou: o caminho do .o não muda, então compara o stamp).
  if [ ! -f "$obj" ] || [ "$SRC/$f.cpp" -nt "$obj" ] || [ "$HERE/deps.sh" -nt "$obj" ]; then
    echo "compilando $f"
    g++ -std=c++20 -O2 -w -c "$SRC/$f.cpp" -o "$obj" \
      -I "$SRC" -I "$TP/spirv-headers/include/spirv/unified1"
  fi
  OBJS+=("$obj")
done

g++ -std=c++20 -O2 -c "$HERE/main.cpp" -o "$OUT/main.o" \
  -I "$SRC" -I "$TP/spirv-headers/include/spirv/unified1" \
  -DRESHADE_COMMIT="\"$RESHADE_COMMIT\"" -DSPIRV_HEADERS_COMMIT="\"$SPIRV_HEADERS_COMMIT\""
g++ -o "$OUT/fx2spv" "$OUT/main.o" "${OBJS[@]}"
echo "ok: $OUT/fx2spv"
