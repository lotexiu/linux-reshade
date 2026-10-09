#!/usr/bin/env bash
# Compila um efeito para uma resolução e imprime o diretório do bundle.
# Uso: compile.sh <Efeito|arquivo.fx> <largura> <altura> [opções do fx2spv...]
set -euo pipefail
# shellcheck source=lib.sh
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

[ $# -ge 3 ] || { echo "uso: $0 <Efeito|arquivo.fx> <largura> <altura> [opções do fx2spv]" >&2; exit 2; }
name="$1"; width="$2"; height="$3"; shift 3

[ -x "$TOOL" ] || "$HERE/build.sh" >&2
ensure_shaders
fx="$(find_effect "$name")" || { echo "efeito não encontrado: $name" >&2; exit 1; }
bundle="$OUT/$(basename "$fx" .fx)-${width}x${height}"
mkdir -p "$OUT"
mapfile -t includes < <(include_flags)
"$TOOL" --width "$width" --height "$height" "${includes[@]}" -o "$bundle" "$@" "$fx" >&2
echo "$bundle"
