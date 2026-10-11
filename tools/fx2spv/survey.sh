#!/usr/bin/env bash
# Levantamento: compila TODOS os .fx de uma pasta de efeitos (a do dono, por padrão a de `lrctl config`) e executa cada
# bundle na GPU de software (lavapipe), dando uma imagem de profundidade aos que pedem. Mostra quantos rodam e por que
# os outros não. Uso: survey.sh [pasta-de-efeitos]
set -uo pipefail
# shellcheck source=lib.sh
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

[ -x "$TOOL" ] || { echo "rode tools/fx2spv/build.sh primeiro" >&2; exit 1; }
SRC="${1:-/mnt/D/Outros/RT/reshade-shaders/Shaders}"
[ -d "$SRC" ] || { echo "pasta de efeitos não existe: $SRC" >&2; exit 1; }
SURVEY="$ROOT/build/survey"
rm -rf "$SURVEY"; mkdir -p "$SURVEY"

includes=(-I "$SRC")
while IFS= read -r d; do includes+=(-I "$d"); done < <(find "$SRC" -type d | sort)

compile_one() {
  local fx="$1" name; name="$(basename "$fx" .fx)"
  if "$TOOL" --width 1920 --height 1080 "${@:2}" -o "$SURVEY/$name" "$fx" 2>"$SURVEY/$name.log"; then
    for spv in "$SURVEY/$name"/*.spv; do
      spirv-val --target-env vulkan1.0 "$spv" 2>"$SURVEY/$name.val" || { echo "INVALIDO $name" >>"$SURVEY/_compile.txt"; return; }
    done
    echo "ok $name" >>"$SURVEY/_compile.txt"
  else
    echo "FALHOU $name: $(head -c 160 "$SURVEY/$name.log" | tr '\n' ' ')" >>"$SURVEY/_compile.txt"
  fi
}
export -f compile_one; export TOOL SURVEY

find "$SRC" -name '*.fx' | sort | xargs -P 4 -I{} bash -c 'compile_one "$@"' _ {} "${includes[@]}"
total=$(find "$SRC" -name '*.fx' | wc -l)
echo "compilados: $(grep -c '^ok ' "$SURVEY/_compile.txt") de $total"
grep -v '^ok ' "$SURVEY/_compile.txt" | sort | sed 's/^/  /' | head -80

echo; echo "execução na GPU de software:"
LINUX_RESHADE_SURVEY_BUNDLES="$SURVEY" cargo test -q -p postfx --test community -- --nocapture 2>&1 | grep -vE "^running|^test |^$|warning" | tee "$SURVEY/_run.txt" | head -150
