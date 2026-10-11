#!/usr/bin/env bash
# Utilitário de desenvolvimento. Idempotente; serve para trocar de máquina sem esforço.
# Uso: scripts/dev.sh <setup|doctor|build|install|uninstall|run|smoke|validate|smoke-steam|smoke-auto|smoke-layer|smoke-preset|smoke-depth|smoke-depth-effect|smoke-compute|smoke-dynamic|daemon|panel|panel-smoke|hotkey|game-options|fx-tool|fx-compile|fx-test|fx-survey|fx-golden|placement|logs> [args]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LAYER_NAME="VK_LAYER_LOTEXIU_linux_reshade"
LAYER_LIB="liblinux_reshade_layer.so"
ENABLE_VAR="LINUX_RESHADE"
DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
MANIFEST_DIR="$DATA_HOME/vulkan/implicit_layer.d"
MANIFEST="$MANIFEST_DIR/${LAYER_NAME}.json"
# O log fica no $HOME porque os containers da Steam compartilham o $HOME, mas não o /run/user.
LOG_FILE="${LINUX_RESHADE_LOG:-${XDG_STATE_HOME:-$HOME/.local/state}/linux-reshade/layer.log}"
mkdir -p "$(dirname "$LOG_FILE")"
STEAM_SOLDIER="${STEAM_ROOT:-$HOME/.local/share/Steam}/steamapps/common/SteamLinuxRuntime_soldier/run-in-soldier"

# Rust vem do asdf (versão fixada em .tool-versions); sem asdf, cai no rustup.
# shellcheck disable=SC1091
[ -f "$HOME/.asdf/asdf.sh" ] && . "$HOME/.asdf/asdf.sh"
export PATH="$HOME/.cargo/bin:$PATH"

say() { printf '\033[1;36m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m!!\033[0m %s\n' "$*" >&2; }
have() { command -v "$1" >/dev/null 2>&1; }

# Pacotes de sistema por gerenciador. Rust não entra aqui (asdf/rustup, sem root).
pkg_cmd() {
  if have apt-get; then
    echo "sudo apt-get install -y build-essential pkg-config vulkan-tools vulkan-validationlayers glslang-tools spirv-tools libgtk-4-dev"
  elif have pacman; then
    echo "sudo pacman -S --needed base-devel pkgconf vulkan-tools vulkan-validation-layers glslang spirv-tools gtk4"
  elif have dnf; then
    echo "sudo dnf install -y gcc make pkgconf-pkg-config vulkan-tools vulkan-validation-layers glslang spirv-tools gtk4-devel"
  else
    echo ""
  fi
}

cmd_setup() {
  if have asdf; then
    say "Rust via asdf (.tool-versions)"
    asdf plugin list | grep -qx rust || asdf plugin add rust
    (cd "$ROOT" && asdf install rust)
  elif ! have rustc; then
    say "asdf ausente; instalando Rust via rustup (usuário, sem root)"
    curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --no-modify-path
  fi
  local cmd; cmd="$(pkg_cmd)"
  if [ -z "$cmd" ]; then
    warn "Gerenciador de pacotes não reconhecido. Instale manualmente: compilador C, vulkan-tools (vkcube)."
  elif have cc && have vkcube && pkg-config --exists gtk4 2>/dev/null; then
    say "Dependências de sistema já presentes"
  else
    say "Faltam pacotes de sistema (precisa de root):"
    echo "    $cmd"
    if [ -t 0 ] && [ "${1:-}" != "--no-install" ]; then
      read -r -p "Rodar agora? [s/N] " ans
      [[ "$ans" =~ ^[sSyY]$ ]] && eval "$cmd"
    fi
  fi
  cmd_doctor
}

cmd_doctor() {
  say "Ambiente"
  printf '  sessão:      %s (%s)\n' "${XDG_SESSION_TYPE:-?}" "${XDG_CURRENT_DESKTOP:-?}"
  printf '  distro:      %s\n' "$(. /etc/os-release && echo "$PRETTY_NAME")"
  printf '  kernel:      %s\n' "$(uname -r)"
  printf '  gpu:         %s\n' "$(lspci 2>/dev/null | grep -iE 'vga|3d' | sed 's/.*: //' | paste -sd';' || echo '?')"
  if pkg-config --exists gtk4 2>/dev/null; then printf '  %-12s ok (%s)\n' gtk4 "$(pkg-config --modversion gtk4)"; else printf '  %-12s \033[1;31mFALTA\033[0m (libgtk-4-dev)\n' gtk4; fi
  for t in rustc cargo cc vkcube vulkaninfo glslangValidator spirv-val; do
    if have "$t"; then printf '  %-12s ok\n' "$t"; else printf '  %-12s \033[1;31mFALTA\033[0m\n' "$t"; fi
  done
  if [ -f "$MANIFEST" ]; then printf '  manifesto:   instalado (%s)\n' "$MANIFEST"; else printf '  manifesto:   não instalado\n'; fi
}

cmd_build() { (cd "$ROOT" && cargo build --release "$@"); }

cmd_install() {
  cmd_build -p linux-reshade-layer
  local lib="$ROOT/target/release/$LAYER_LIB"
  [ -f "$lib" ] || { warn "não achei $lib"; exit 1; }
  # Após um travamento do PC o .so pode ficar zerado e o Cargo não o refaz; o loader só diz "invalid ELF header".
  if [ "$(head -c 4 "$lib" | tail -c 3)" != "ELF" ]; then
    warn "$lib está corrompido; refazendo"
    (cd "$ROOT" && cargo clean -p linux-reshade-layer --release) && cmd_build -p linux-reshade-layer
  fi
  mkdir -p "$MANIFEST_DIR"
  cat >"$MANIFEST" <<EOF
{
  "file_format_version": "1.2.0",
  "layer": {
    "name": "$LAYER_NAME",
    "type": "GLOBAL",
    "library_path": "$lib",
    "api_version": "1.3.0",
    "implementation_version": "1",
    "description": "linux-reshade (desenvolvimento)",
    "functions": {
      "vkNegotiateLoaderLayerInterfaceVersion": "vkNegotiateLoaderLayerInterfaceVersion"
    },
    "enable_environment": { "$ENABLE_VAR": "1" },
    "disable_environment": { "${ENABLE_VAR}_DISABLE": "1" }
  }
}
EOF
  say "Manifesto instalado em $MANIFEST"
}

cmd_uninstall() { rm -f "$MANIFEST"; say "Manifesto removido"; }

cmd_run() {
  have vkcube || { warn "vkcube não instalado (rode: scripts/dev.sh setup)"; exit 1; }
  [ $# -gt 0 ] || set -- --c 600   # sem argumentos: encerra o vkcube após 600 frames
  : >"$LOG_FILE"
  say "vkcube com a layer ativa; log em $LOG_FILE"
  env "$ENABLE_VAR=1" LINUX_RESHADE_LOG="$LOG_FILE" timeout 60 vkcube "$@" >/dev/null 2>&1 || true
  cat "$LOG_FILE"
}

# Build + instala + roda o vkcube (comando em "$@"); liga e desliga a flag com o lrctl no meio da
# execução e confere que a layer foi carregada, viu os presents e reagiu às duas mudanças.
# Compila dois efeitos para a janela de 500x500 do vkcube (o Sépia, com um parâmetro, e a Inversão).
SMOKE_BUNDLE="$ROOT/build/bundles/Sepia-500x500"
SMOKE_BUNDLE_2="$ROOT/build/bundles/Invert-500x500"
prepare_smoke_bundle() {
  [ -x "$ROOT/build/fx2spv/fx2spv" ] || cmd_fx_tool >&2
  "$ROOT/tools/fx2spv/compile.sh" Sepia 500 500 >/dev/null
  "$ROOT/tools/fx2spv/compile.sh" "$ROOT/crates/postfx/tests/fx/Invert.fx" 500 500 >/dev/null
}

# Deixa o canal com a layer como estava antes do teste: nenhum efeito escolhido e efeitos desligados.
smoke_cleanup() {
  local lrctl="$ROOT/target/release/lrctl"
  [ -x "$lrctl" ] || return 0
  "$lrctl" use none >/dev/null 2>&1 || true
  "$lrctl" off >/dev/null 2>&1 || true
}

# smoke_with <rótulo> <exige-término-limpo 0|1> <timeout-s> <comando...>
smoke_with() {
  local label="$1" strict="$2" limit="$3"; shift 3
  say "smoke ($label): o vkcube abre na tela em foco por ~20 s; evite mexer em janelas até terminar"
  cmd_install
  cmd_build -p linux-reshade-control
  prepare_smoke_bundle
  local lrctl="$ROOT/target/release/lrctl"
  # Sempre deixa o canal limpo ao sair (inclusive em falha): uma escolha esquecida valeria para os jogos de verdade.
  trap smoke_cleanup EXIT
  "$lrctl" off >/dev/null
  "$lrctl" use none >/dev/null
  : >"$LOG_FILE"
  # SMOKE_VALIDATE=1 liga a validation layer do Vulkan; a saída do vkcube fica em $LOG_FILE.out.
  # O bundle NÃO vai pelo ambiente: quem escolhe o efeito é o app de controle (lrctl use).
  local validate_env=()
  [ "${SMOKE_VALIDATE:-0}" = 1 ] && validate_env=("VK_LOADER_LAYERS_ENABLE=*validation*")
  env -u LINUX_RESHADE_BUNDLE "$ENABLE_VAR=1" LINUX_RESHADE_LOG="$LOG_FILE" "${validate_env[@]}" timeout "$limit" "$@" --c 2400 >"$LOG_FILE.out" 2>&1 &
  local pid=$!
  # espera o primeiro present (até ~15 s) antes de mexer no canal
  for _ in $(seq 1 150); do grep -q "vkQueuePresentKHR #1$" "$LOG_FILE" 2>/dev/null && break; sleep 0.1; done
  "$lrctl" use "$SMOKE_BUNDLE" >/dev/null
  # publica um parâmetro antes de ligar: a layer precisa aplicá-lo ao criar o efeito
  "$lrctl" set Strength 0.5 >/dev/null
  # liga, desliga, liga de novo (reaproveita os recursos já criados) e desliga
  sleep 1; "$lrctl" on >/dev/null
  sleep 1.5; "$lrctl" off >/dev/null
  sleep 1; "$lrctl" on >/dev/null
  # troca de efeito com ele ligado: a layer descarta o Sépia e constrói a Inversão
  sleep 1.5; "$lrctl" use "$SMOKE_BUNDLE_2" >/dev/null
  sleep 2; "$lrctl" off >/dev/null
  local rc=0; wait "$pid" || rc=$?
  "$lrctl" use "$SMOKE_BUNDLE" >/dev/null && "$lrctl" reset >/dev/null   # limpa o parâmetro do Sépia
  [ "$rc" = 0 ] || warn "o vkcube terminou com código $rc (124 = estourou o timeout; 143/137 = morto por sinal)"
  local ok=1 patterns=("vkCreateInstance ok" "vkCreateDevice ok" "vkQueuePresentKHR #1$" \
             "estado compartilhado aberto" "efeitos LIGADOS" "efeitos DESLIGADOS" "efeito criado"
             "parâmetros do app de controle aplicados" "o bundle mudou" "Invert-500x500")
  [ "$strict" = 1 ] && patterns+=("vkDestroyInstance")
  if [ "${SMOKE_VALIDATE:-0}" = 1 ]; then
    local issues; issues=$(grep -ciE "VUID|Validation (Error|Warning)" "$LOG_FILE.out" || true)
    if [ "$issues" = 0 ]; then printf '  ok     validation sem mensagens\n'; else
      printf '  FALHOU validation: %s mensagens (veja %s)\n' "$issues" "$LOG_FILE.out"; ok=0; fi
  fi
  # dois efeitos precisam ter sido construídos: o Sépia e, depois da troca, a Inversão
  local built; built=$(grep -c "efeito criado" "$LOG_FILE" || true)
  if [ "$built" -ge 2 ]; then printf '  ok     efeitos construídos: %s\n' "$built"; else printf '  FALHOU efeitos construídos: %s (esperava 2 ou mais)\n' "$built"; ok=0; fi
  for pat in "${patterns[@]}"; do
    if grep -q "$pat" "$LOG_FILE"; then printf '  ok     %s\n' "$pat"; else printf '  FALHOU %s\n' "$pat"; ok=0; fi
  done
  [ "$ok" = 1 ] && say "smoke ($label): passou" || { warn "smoke ($label): falhou (log em $LOG_FILE)"; exit 1; }
}

# --wsi wayland explícito: o caminho XWayland (--wsi xcb) trava nesta máquina mesmo sem a layer.
cmd_smoke() { smoke_with "host" 1 60 vkcube --wsi wayland --width 500 --height 500; }

# O mesmo, com a validation layer: falha se ela acusar qualquer uso incorreto da API.
cmd_validate() { SMOKE_VALIDATE=1 smoke_with "host + validation" 1 60 vkcube --wsi wayland --width 500 --height 500; }

# Encerra o que o smoke-auto abriu e deixa o canal limpo.
# Isola um teste da máquina de quem o roda: os efeitos são os do clone (efeitos próprios e a coleção clássica) e
# os presets vêm de uma pasta vazia, então a pasta de dados e os presets reais do usuário não interferem.
smoke_isolate() { # smoke_isolate <pasta temporária>
  local legacy="$ROOT/third_party/reshade-shaders-legacy/Shaders" slim="$ROOT/third_party/reshade-shaders-slim/Shaders"
  [ -d "$legacy" ] || cmd_fx_tool
  mkdir -p "$1/Presets"
  # Estado compartilhado só deste teste: o daemon do usuário (se estiver rodando) não enxerga nem mexe nos jogos dele.
  export LINUX_RESHADE_STATE_DIR="$1/state"
  export LINUX_RESHADE_CACHE="$1/cache" LINUX_RESHADE_PRESETS="$1/Presets"
  export LINUX_RESHADE_EFFECTS="$ROOT/effects:$legacy" LINUX_RESHADE_INCLUDES="$slim:$legacy"
}

smoke_auto_cleanup() {
  [ "${#SMOKE_PIDS[@]}" -gt 0 ] && kill "${SMOKE_PIDS[@]}" 2>/dev/null || true
  smoke_cleanup
}

# Teste da compilação automática. O efeito é escolhido POR NOME com um vkcube de 500x500 aberto (o próprio
# `lrctl use` compila essa resolução); depois abre-se um de 320x240, que só o vigia pode atender, e as
# duas layers precisam ficar prontas. Usa um cache isolado.
cmd_smoke_auto() {
  say "smoke (auto): abre dois vkcube (500x500 e depois 320x240) por ~25 s; evite mexer em janelas até terminar"
  cmd_install
  cmd_build -p linux-reshade-control
  local lrctl="$ROOT/target/release/lrctl" cache; cache="$(mktemp -d)"
  smoke_isolate "$cache"
  # Os PIDs ficam numa variável global: o trap EXIT roda depois desta função terminar, quando as locais já não existem.
  SMOKE_PIDS=()
  trap smoke_auto_cleanup EXIT
  "$lrctl" off >/dev/null; "$lrctl" use none >/dev/null
  : >"$LOG_FILE"
  "$lrctl" watch >"$cache/watch.log" 2>&1 &
  SMOKE_PIDS+=($!)
  local cube_env=(env -u LINUX_RESHADE_BUNDLE "$ENABLE_VAR=1" LINUX_RESHADE_LOG="$LOG_FILE")

  # wait_layers <resolução> <texto>: espera (até ~20 s) uma linha de `lrctl layers` com os dois
  wait_layers() {
    for _ in $(seq 1 200); do "$lrctl" layers | grep "$1" | grep -q "$2" && return 0; sleep 0.1; done
    return 1
  }
  local ok=1
  check() { # check <descrição> <comando de teste>
    if eval "$2"; then printf '  ok     %s\n' "$1"; else printf '  FALHOU %s\n' "$1"; ok=0; fi
  }

  # 1. um jogo de 500x500, sem efeito; escolhe-se o Sépia por nome (o `use` compila 500x500)
  "${cube_env[@]}" timeout 60 vkcube --wsi wayland --width 500 --height 500 --c 1800 >/dev/null 2>&1 &
  local cube1=$!; SMOKE_PIDS+=("$cube1")
  wait_layers 500x500 "nenhum efeito" || true
  "$lrctl" use Sepia
  check "500x500 ficou pronto" 'wait_layers 500x500 pronto'

  # 2. um jogo de OUTRA resolução aparece depois: só o vigia pode compilá-la
  "${cube_env[@]}" timeout 60 vkcube --wsi wayland --width 320 --height 240 --c 1500 >/dev/null 2>&1 &
  local cube2=$!; SMOKE_PIDS+=("$cube2")
  check "320x240 ficou pronto (compilado pelo vigia)" 'wait_layers 320x240 pronto'
  "$lrctl" layers

  # 3. um parâmetro ajustado com o efeito escolhido POR NOME vale para as duas resoluções (cada uma tem um bundle
  #    de id diferente; os parâmetros são identificados pelo efeito e pelo layout, não pelo bundle)
  check "ajustar um parâmetro com a escolha por nome" '"$lrctl" set Strength 0.5 >/dev/null'
  "$lrctl" on >/dev/null; sleep 2; "$lrctl" off >/dev/null
  wait "$cube1" "$cube2" 2>/dev/null || true

  check "o parâmetro foi aplicado nos DOIS jogos" '[ "$(grep "parâmetros do app de controle aplicados" "$LOG_FILE" | grep -o "pid=[0-9]*" | sort -u | wc -l)" -ge 2 ]'
  check "o vigia compilou 320x240" 'grep -q "compilado Sepia para 320x240" "$cache/watch.log"'
  check "bundle 500x500 no cache" '[ -f "$cache/cache/Sepia-500x500/manifest.json" ]'
  check "bundle 320x240 no cache" '[ -f "$cache/cache/Sepia-320x240/manifest.json" ]'
  check "o efeito foi construído nos dois jogos" '[ "$(grep -c "efeito criado" "$LOG_FILE")" -ge 2 ]'
  check "a layer de 320x240 esperou o bundle antes de ele existir" 'grep -q "swapchain 320x240: ainda não há bundle, esperando" "$LOG_FILE"'
  "$lrctl" reset >/dev/null; "$lrctl" use none >/dev/null   # não deixa o Strength ajustado no estado de quem usa a máquina
  [ "$ok" = 1 ] && say "smoke (auto): passou" || { warn "smoke (auto): falhou (logs: $LOG_FILE e $cache/watch.log)"; exit 1; }
}

# Teste da escolha por jogo. Dois vkcube idênticos abertos juntos: o efeito é escolhido para UM deles
# (`--layer <pid>`) e só ele pode ficar pronto; depois a escolha de todos não passa por cima da do jogo, `none`
# tira o efeito só de um, `inherit` o devolve, e um parâmetro ajustado num jogo não chega ao outro.
cmd_smoke_layer() {
  say "smoke (por jogo): abre dois vkcube (400x300) por ~30 s; evite mexer em janelas até terminar"
  cmd_install
  cmd_build -p linux-reshade-control
  local lrctl="$ROOT/target/release/lrctl" cache; cache="$(mktemp -d)"
  smoke_isolate "$cache"
  SMOKE_PIDS=()
  trap smoke_auto_cleanup EXIT
  "$lrctl" off >/dev/null
  # parte de um estado conhecido: o parâmetro de todos nos padrões do efeito
  "$lrctl" use Sepia >/dev/null; "$lrctl" reset >/dev/null; "$lrctl" use none >/dev/null
  : >"$LOG_FILE"
  "$lrctl" watch >"$cache/watch.log" 2>&1 &
  SMOKE_PIDS+=($!)
  local cube_env=(env -u LINUX_RESHADE_BUNDLE "$ENABLE_VAR=1" LINUX_RESHADE_LOG="$LOG_FILE")
  for _ in 1 2; do
    "${cube_env[@]}" timeout 90 vkcube --wsi wayland --width 400 --height 300 --c 4000 >/dev/null 2>&1 &
    SMOKE_PIDS+=($!)
  done

  local ok=1
  check() { if eval "$2"; then printf '  ok     %s\n' "$1"; else printf '  FALHOU %s\n' "$1"; ok=0; fi; }
  # state_of <pid> <texto>: o jogo <pid> mostra <texto> em `lrctl layers`
  state_of() { "$lrctl" layers | awk -v p="$1" '$2==p' | grep -q "$2"; }
  wait_state() { for _ in $(seq 1 200); do state_of "$1" "$2" && return 0; sleep 0.1; done; return 1; }
  for _ in $(seq 1 150); do [ "$("$lrctl" layers | awk 'NR>1 && $2 ~ /^[0-9]+$/' | wc -l)" -ge 2 ] && break; sleep 0.1; done
  local pids; pids=($("$lrctl" layers | awk 'NR>1 && $2 ~ /^[0-9]+$/ {print $2}' | sort -n))
  local A=${pids[0]:-0} B=${pids[1]:-0}
  check "dois jogos com a layer (pids $A e $B)" '[ "$A" != 0 ] && [ "$B" != 0 ] && [ "$A" != "$B" ]'
  "$lrctl" on >/dev/null

  # 1. efeito só para A
  "$lrctl" use Sepia --layer "$A" >/dev/null
  check "A ficou pronto" 'wait_state "$A" pronto'
  sleep 1
  check "B continua sem efeito" 'state_of "$B" "nenhum efeito"'

  # 2. escolha de todos: B passa a seguir; A mantém a sua
  "$lrctl" use Sepia >/dev/null
  check "B passou a seguir a escolha de todos" 'wait_state "$B" pronto'
  check "A segue pronto" 'state_of "$A" pronto'

  # 3. sem efeito só em B
  "$lrctl" use none --layer "$B" >/dev/null
  check "B ficou sem efeito (escolha de todos continua valendo)" 'wait_state "$B" "nenhum efeito"'
  check "A segue pronto" 'state_of "$A" pronto'
  check "o 'current' do jogo diz que é escolha própria" '"$lrctl" current --layer "$B" | grep -q "sem efeito"'

  # 4. herdar de volta
  "$lrctl" use inherit --layer "$B" >/dev/null
  check "B voltou a seguir todos" 'wait_state "$B" pronto'

  # 5. parâmetros por jogo: o ajuste de A não muda o valor de todos nem o de B
  "$lrctl" set Strength 0.5 --layer "$A" >/dev/null
  sleep 2
  check "o parâmetro do jogo A foi aplicado em A" 'grep "parâmetros do app de controle aplicados" "$LOG_FILE" | grep -q "pid=$A"'
  check "A vale 0.5" '[ "$("$lrctl" get Strength --layer "$A")" = 0.5 ]'
  check "B e todos seguem no padrão do efeito (1)" '[ "$("$lrctl" get Strength --layer "$B")" = 1 ] && [ "$("$lrctl" get Strength)" = 1 ]'
  # um ajuste global chega a B (que herda) mas não passa por cima do de A
  "$lrctl" set Strength 0.2 >/dev/null
  check "ajuste de todos chega a B e não muda A" '[ "$("$lrctl" get Strength --layer "$B")" = 0.2 ] && [ "$("$lrctl" get Strength --layer "$A")" = 0.5 ]'

  "$lrctl" reset --layer "$A" >/dev/null; "$lrctl" reset >/dev/null; "$lrctl" use none >/dev/null; "$lrctl" off >/dev/null
  [ "$ok" = 1 ] && say "smoke (por jogo): passou" || { warn "smoke (por jogo): falhou (logs: $LOG_FILE e $cache/watch.log)"; "$lrctl" layers; exit 1; }
}

# Teste do preset por jogo. Um vkcube com a layer e o preset dele (Presets/vkcube) de DUAS técnicas (Sépia e Colourfulness):
# o vigia publica o plano, a layer constrói a cadeia de 2 etapas; editar o preset muda a cadeia, mudar só um
# valor não a reconstrói, e apagar o preset volta à escolha única. Pastas de efeitos e de presets isoladas.
cmd_smoke_preset() {
  say "smoke (preset): abre um vkcube (400x300) por ~25 s; evite mexer em janelas até terminar"
  cmd_install
  cmd_build -p linux-reshade-control
  local lrctl="$ROOT/target/release/lrctl" tmp; tmp="$(mktemp -d)"
  smoke_isolate "$tmp"
  SMOKE_PIDS=()
  trap smoke_auto_cleanup EXIT
  "$lrctl" off >/dev/null; "$lrctl" use none >/dev/null
  : >"$LOG_FILE"
  # o preset é do jogo: Presets/vkcube/game.ini aponta para Noturno.ini (o id é o nome do executável)
  mkdir -p "$tmp/Presets/vkcube"; printf 'preset=Noturno.ini\n' >"$tmp/Presets/vkcube/game.ini"
  local preset="$tmp/Presets/vkcube/Noturno.ini"
  printf 'Techniques=Sepia@Sepia.fx,Colourfulness@Colourfulness.fx\n\n[Sepia.fx]\nStrength=0.5\n' >"$preset"
  "$lrctl" watch >"$tmp/watch.log" 2>&1 &
  SMOKE_PIDS+=($!)
  env -u LINUX_RESHADE_BUNDLE "$ENABLE_VAR=1" LINUX_RESHADE_LOG="$LOG_FILE" timeout 60 vkcube --wsi wayland --width 400 --height 300 --c 3000 >/dev/null 2>&1 &
  SMOKE_PIDS+=($!)

  local ok=1
  check() { if eval "$2"; then printf '  ok     %s\n' "$1"; else printf '  FALHOU %s\n' "$1"; ok=0; fi; }
  waitfor() { for _ in $(seq 1 250); do eval "$1" && return 0; sleep 0.1; done; return 1; }
  chains() { grep -c "cadeia de $1 etapa(s) criada" "$LOG_FILE"; }
  "$lrctl" on >/dev/null

  check "o vigia publicou o plano de 2 etapas" 'waitfor "grep -q \"(2 etapa\|: 2 etapa\" \"$tmp/watch.log\""'
  check "a layer recebeu o plano de 2 etapas" 'waitfor "grep -q \"plano do jogo: 2 etapa\" \"$LOG_FILE\""'
  check "a cadeia de 2 etapas foi construída" 'waitfor "[ \$(chains 2) -ge 1 ]"'
  check "o jogo aparece como pronto" '"$lrctl" layers | grep -q pronto'
  check "a layer contou ao app de controle que as 2 técnicas foram aplicadas" 'waitfor "[ \$(\"$lrctl\" layers | grep -c aplicada) = 2 ]"'
  check "os dois efeitos foram compilados" '[ -f "$tmp/cache/Sepia-400x300/manifest.json" ] && [ -f "$tmp/cache/Colourfulness-400x300/manifest.json" ]'

  # editar o preset: uma técnica só
  sleep 1.2
  printf 'Techniques=Colourfulness@Colourfulness.fx\n' >"$preset"
  check "a layer recebeu o plano de 1 etapa" 'waitfor "grep -q \"plano do jogo: 1 etapa\" \"$LOG_FILE\""'
  check "a cadeia foi reconstruída com 1 etapa" 'waitfor "[ \$(chains 1) -ge 1 ]"'

  # mudar só um valor de uniform: o plano muda, a cadeia não é reconstruída
  sleep 1.2
  local before; before="$(grep -c "etapa(s) criada" "$LOG_FILE")"
  printf 'Techniques=Colourfulness@Colourfulness.fx\n\n[Colourfulness.fx]\ncolourfulness=0.5\n' >"$preset"
  sleep 2
  check "só um valor mudou: a cadeia não foi reconstruída" '[ "$(grep -c "etapa(s) criada" "$LOG_FILE")" = "$before" ]'

  # uma macro do preset: recompila o efeito (outro bundle, com o nome da variante) e a layer reconstrói a cadeia
  sleep 1.2
  local before_macro; before_macro="$(grep -c "etapa(s) criada" "$LOG_FILE")"
  printf 'PreprocessorDefinitions=SMOKE_Q=2\nTechniques=Colourfulness@Colourfulness.fx\n' >"$preset"
  check "a macro recompilou o efeito (bundle da variante)" 'waitfor "ls \"$tmp\"/cache | grep -q \"^Colourfulness\\.m.*-400x300\$\""'
  check "a layer carregou o bundle da variante" 'waitfor "grep -q \"bundle carregado: .*Colourfulness\\.m\" \"$LOG_FILE\""'
  check "a layer reconstruiu a cadeia com o bundle novo" 'waitfor "[ \$(grep -c \"etapa(s) criada\" \"$LOG_FILE\") -gt $before_macro ]"'

  # apagar o preset: volta à escolha única
  rm -f "$preset"
  check "sem preset a layer volta à escolha única" 'waitfor "grep -q \"sem plano do jogo\" \"$LOG_FILE\""'

  "$lrctl" off >/dev/null; "$lrctl" use none >/dev/null
  [ "$ok" = 1 ] && say "smoke (preset): passou" || { warn "smoke (preset): falhou (logs: $LOG_FILE e $tmp/watch.log)"; "$lrctl" layers; exit 1; }
}

# Teste da observação de profundidade (D1): um vkcube com a layer e a observação ligada; `lrctl layers` precisa listar a
# imagem de profundidade do cubo como candidata escolhida, com desenhos, e a validation layer não pode reclamar de nada.
cmd_smoke_depth() {
  say "smoke (profundidade): abre um vkcube (400x300) por ~15 s; evite mexer em janelas até terminar"
  cmd_install
  cmd_build -p linux-reshade-control
  local lrctl="$ROOT/target/release/lrctl" out; out="$(mktemp -d)"
  : >"$LOG_FILE"
  env -u LINUX_RESHADE_BUNDLE "$ENABLE_VAR=1" LINUX_RESHADE_DEPTH_OBSERVE=1 LINUX_RESHADE_LOG="$LOG_FILE" VK_LOADER_LAYERS_ENABLE='*validation*' \
    timeout 40 vkcube --wsi wayland --width 400 --height 300 --c 700 >"$out/vkcube.txt" 2>&1 &
  SMOKE_PIDS=($!)
  trap smoke_auto_cleanup EXIT
  local ok=1
  check() { if eval "$2"; then printf '  ok     %s\n' "$1"; else printf '  FALHOU %s\n' "$1"; ok=0; fi; }
  waitfor() { for _ in $(seq 1 150); do eval "$1" && return 0; sleep 0.1; done; return 1; }
  check "a layer listou uma candidata de profundidade escolhida" 'waitfor "\"$lrctl\" layers | grep -qE \"^  \\* \\[[0-9]+\\] 400x300\""'
  check "a candidata tem desenhos e vértices" '"$lrctl" layers | grep -E "^  \* \[[0-9]+\] 400x300" | grep -qE "[1-9][0-9]* desenhos, [1-9][0-9]* vértices"'
  check "o passe do jogo limpa a profundidade ao começar" '"$lrctl" layers | grep -E "^  \* \[[0-9]+\] 400x300" | grep -q "clear ->"'
  check "o log da layer registrou a escolha" 'grep -q "profundidade escolhida: 400x300" "$LOG_FILE"'
  wait "${SMOKE_PIDS[0]}" 2>/dev/null || true
  check "a validation layer não reclamou de nada" '! grep -q "VUID" "$out/vkcube.txt"'
  [ "$ok" = 1 ] && say "smoke (profundidade): passou" || { warn "smoke (profundidade): falhou ($out)"; "$lrctl" layers; exit 1; }
}

# Teste do efeito que usa a profundidade (D2a): um vkcube com a layer ARMADA (acrescenta SAMPLED à profundidade) e a
# gravação forçada da profundidade (o vkcube a descarta), um preset com o DepthView e o vigia. A técnica precisa
# ficar "aplicada" (a layer amostrou a imagem original) e a validation layer não pode reclamar de nada.
cmd_smoke_depth_effect() {
  say "smoke (efeito de profundidade): abre um vkcube (400x300) por ~25 s; evite mexer em janelas até terminar"
  cmd_install
  cmd_build -p linux-reshade-control
  local lrctl="$ROOT/target/release/lrctl" tmp; tmp="$(mktemp -d)"
  smoke_isolate "$tmp"
  mkdir -p "$tmp/Presets/vkcube"
  printf 'preset=Prof.ini\n' >"$tmp/Presets/vkcube/game.ini"
  printf 'Techniques=DepthView@DepthView.fx\n' >"$tmp/Presets/vkcube/Prof.ini"
  SMOKE_PIDS=()
  trap smoke_auto_cleanup EXIT
  "$lrctl" off >/dev/null; "$lrctl" use none >/dev/null
  : >"$LOG_FILE"
  "$lrctl" watch >"$tmp/watch.log" 2>&1 &
  SMOKE_PIDS+=($!)
  env -u LINUX_RESHADE_BUNDLE "$ENABLE_VAR=1" LINUX_RESHADE_DEPTH=1 LINUX_RESHADE_DEPTH_FORCE_STORE=1 LINUX_RESHADE_LOG="$LOG_FILE" \
    VK_LOADER_LAYERS_ENABLE='*validation*' timeout 60 vkcube --wsi wayland --width 400 --height 300 --c 1500 >"$tmp/vkcube.txt" 2>&1 &
  SMOKE_PIDS+=($!)
  local ok=1
  check() { if eval "$2"; then printf '  ok     %s\n' "$1"; else printf '  FALHOU %s\n' "$1"; ok=0; fi; }
  waitfor() { for _ in $(seq 1 250); do eval "$1" && return 0; sleep 0.1; done; return 1; }
  "$lrctl" on >/dev/null
  check "a profundidade do vkcube ficou amostrável" 'waitfor "\"$lrctl\" layers | grep -E \"^  \\* \\[[0-9]+\\] 400x300\" | grep -q amostrável"'
  check "a técnica de profundidade foi aplicada" 'waitfor "\"$lrctl\" layers | grep -q \"DepthView  aplicada\""'
  check "a cadeia foi construída com a profundidade" 'grep -q "cadeia de 1 etapa(s) criada" "$LOG_FILE"'
  # a foto da profundidade depois do 1º passe (cópia dentro do command buffer do jogo) também é amostrada
  "$lrctl" depth pick pass 1 >/dev/null
  check "a layer recebeu o pedido de passe" 'waitfor "grep -q \"pedido para a profundidade: candidata None, passe Some(1)\" \"$LOG_FILE\""'
  sleep 2
  check "a técnica continua aplicada com a foto do passe" '"$lrctl" layers | grep -q "DepthView  aplicada"'
  "$lrctl" depth pick auto >/dev/null
  check "a layer voltou à escolha automática" 'waitfor "grep -q \"pedido para a profundidade: candidata None, passe None\" \"$LOG_FILE\""'
  sleep 2
  "$lrctl" off >/dev/null
  wait "${SMOKE_PIDS[1]}" 2>/dev/null || true
  check "a validation layer não reclamou de nada" '! grep -q "VUID" "$tmp/vkcube.txt"'
  [ "$ok" = 1 ] && say "smoke (efeito de profundidade): passou" || { warn "smoke (efeito de profundidade): falhou ($tmp, $LOG_FILE)"; "$lrctl" layers; grep -E "pulada|profundidade|cadeia" "$LOG_FILE" | tail -8; exit 1; }
}

cmd_smoke_compute() {
  say "smoke (compute): abre um vkcube (400x300) por ~25 s; evite mexer em janelas até terminar"
  cmd_install
  cmd_build -p linux-reshade-control
  local lrctl="$ROOT/target/release/lrctl" tmp; tmp="$(mktemp -d)"
  smoke_isolate "$tmp"
  mkdir -p "$tmp/Presets/vkcube"
  printf 'preset=Compute.ini\n' >"$tmp/Presets/vkcube/game.ini"
  printf 'Techniques=ComputeInvert@ComputeInvert.fx\n' >"$tmp/Presets/vkcube/Compute.ini"
  SMOKE_PIDS=()
  trap smoke_auto_cleanup EXIT
  "$lrctl" off >/dev/null; "$lrctl" use none >/dev/null
  : >"$LOG_FILE"
  "$lrctl" watch >"$tmp/watch.log" 2>&1 &
  SMOKE_PIDS+=($!)
  env -u LINUX_RESHADE_BUNDLE "$ENABLE_VAR=1" LINUX_RESHADE_LOG="$LOG_FILE" \
    VK_LOADER_LAYERS_ENABLE='*validation*' timeout 60 vkcube --wsi wayland --width 400 --height 300 --c 1500 >"$tmp/vkcube.txt" 2>&1 &
  SMOKE_PIDS+=($!)
  local ok=1
  check() { if eval "$2"; then printf '  ok     %s\n' "$1"; else printf '  FALHOU %s\n' "$1"; ok=0; fi; }
  waitfor() { for _ in $(seq 1 250); do eval "$1" && return 0; sleep 0.1; done; return 1; }
  "$lrctl" on >/dev/null
  check "a técnica de compute foi aplicada" 'waitfor "\"$lrctl\" layers | grep -q \"ComputeInvert  aplicada\""'
  check "a cadeia de compute foi construída" 'grep -q "cadeia de 1 etapa(s) criada" "$LOG_FILE"'
  sleep 3
  "$lrctl" off >/dev/null
  wait "${SMOKE_PIDS[1]}" 2>/dev/null || true
  check "a validation layer não reclamou de nada" '! grep -q "VUID" "$tmp/vkcube.txt"'
  [ "$ok" = 1 ] && say "smoke (compute): passou" || { warn "smoke (compute): falhou ($tmp, $LOG_FILE)"; "$lrctl" layers; grep -E "pulada|cadeia" "$LOG_FILE" | tail -8; exit 1; }
}

# Teste da escolha por limpeza de profundidade: um app de renderização dinâmica (como o DXVK) com limpezas DENTRO do passe, numa
# superfície headless, com a validation layer. Confere a foto gravada antes da limpeza escolhida e nenhum erro de validação.
cmd_smoke_dynamic() {
  say "smoke (renderização dinâmica): roda um app headless de 150 quadros na GPU de software"
  cmd_install
  cargo build --release -p linux-reshade-layer --example dynamic_clear >/dev/null 2>&1 || { warn "não compilei o exemplo"; exit 1; }
  local tmp out; tmp="$(mktemp -d)"
  out="$(LINUX_RESHADE=1 LINUX_RESHADE_DEPTH=1 LINUX_RESHADE_STATE_DIR="$tmp" LINUX_RESHADE_LOG="$tmp/log.txt" VK_LOADER_LAYERS_ENABLE='*validation*' timeout 120 "$ROOT/target/release/examples/dynamic_clear" 2>&1)"
  local ok=1
  check() { if eval "$2"; then printf '  ok     %s\n' "$1"; else printf '  FALHOU %s\n' "$1"; ok=0; fi; }
  check "o app terminou os quadros" 'grep -q "dynamic_clear: 150 quadros" <<<"$out"'
  check "a foto da profundidade foi gravada antes da limpeza 2 (a que apaga mais desenhos)" 'grep -q "foto da profundidade gravada antes da limpeza 2" "$tmp/log.txt"'
  check "a validation layer não reclamou de nada" '! grep -q "VUID" <<<"$out"'
  [ "$ok" = 1 ] && say "smoke (renderização dinâmica): passou" || { warn "smoke (renderização dinâmica): falhou ($tmp)"; echo "$out" | head -10; exit 1; }
}

# Um daemon antigo (o binário foi recompilado depois de ele subir) continua executando o código velho: ele precisa ser
# trocado. Devolve sucesso se havia algum desatualizado e o matou.
kill_stale_daemons() {
  local pid exe stale=1
  for pid in $(pgrep -f "release/lrctl daemon" || true); do
    exe="$(readlink "/proc/$pid/exe" 2>/dev/null || true)"
    case "$exe" in
      *lrctl*"(deleted)"*) kill "$pid" 2>/dev/null && stale=0 ;;
      *lrctl) [ "$ROOT/target/release/lrctl" -nt "/proc/$pid" ] && kill "$pid" 2>/dev/null && stale=0 ;;
    esac
  done
  return $stale
}

# Sobe o daemon em segundo plano se não há um atual (o painel só edita arquivos; quem aplica o preset é o daemon).
ensure_daemon() {
  if kill_stale_daemons; then
    say "daemon antigo (binário recompilado) encerrado; subindo o novo"
    sleep 0.5
  fi
  if ! pgrep -f "release/lrctl daemon" >/dev/null; then
    mkdir -p "$(dirname "$LOG_FILE")"
    say "subindo o daemon em segundo plano (log: $(dirname "$LOG_FILE")/daemon.log)"
    nohup "$ROOT/target/release/lrctl" daemon >"$(dirname "$LOG_FILE")/daemon.log" 2>&1 &
  fi
}

# O app de controle residente: atalho global e vigia (compila os efeitos e aplica o preset dos jogos abertos).
cmd_daemon() {
  cmd_build -p linux-reshade-control >/dev/null
  kill_stale_daemons && sleep 0.5
  exec "$ROOT/target/release/lrctl" daemon
}

# Abre a interface gráfica (compila se preciso). Se o daemon não está rodando, sobe um em segundo plano: sem ele o
# preset não é aplicado nos jogos (o painel só edita os arquivos).
cmd_panel() {
  cmd_build -p linux-reshade-gui -p linux-reshade-control >/dev/null
  ensure_daemon
  exec "$ROOT/target/release/lrpanel" "$@"
}

# Teste de ponta a ponta da interface: um vkcube de verdade com a layer, o vigia, e a interface escolhendo o
# Sépia e mantendo a janela aberta. O script confere, de fora, que o jogo ficou "pronto" e que o efeito foi construído.
cmd_panel_smoke() {
  say "smoke (interface): abre um vkcube e a janela do lrpanel por ~10 s; evite mexer em janelas até terminar"
  cmd_install
  cmd_build -p linux-reshade-control -p linux-reshade-gui
  local lrctl="$ROOT/target/release/lrctl" panel="$ROOT/target/release/lrpanel" cache; cache="$(mktemp -d)"
  smoke_isolate "$cache"
  SMOKE_PIDS=()
  trap smoke_auto_cleanup EXIT
  "$lrctl" off >/dev/null; "$lrctl" use none >/dev/null
  : >"$LOG_FILE"
  "$lrctl" watch >"$cache/watch.log" 2>&1 &
  SMOKE_PIDS+=($!)
  env -u LINUX_RESHADE_BUNDLE "$ENABLE_VAR=1" LINUX_RESHADE_LOG="$LOG_FILE" timeout 60 vkcube --wsi wayland --width 500 --height 500 --c 2400 >/dev/null 2>&1 &
  SMOKE_PIDS+=($!)

  local ok=1
  check() { if eval "$2"; then printf '  ok     %s\n' "$1"; else printf '  FALHOU %s\n' "$1"; ok=0; fi; }
  wait_for() { for _ in $(seq 1 100); do eval "$1" && return 0; sleep 0.1; done; return 1; }

  wait_for '"$lrctl" layers | grep -q 500x500' || true
  # 1. a interface, aberta por 7 s com o Sépia adicionado ao preset do jogo aberto (vkcube), e uma captura de tela
  "$panel" --effect Sepia --screenshot "$cache/painel.png" --hold 7000 >"$cache/panel.log" 2>&1 &
  local panel_pid=$!
  check "a interface adicionou o efeito ao preset do jogo (visível ao lrctl)" 'wait_for '"'"'"$lrctl" preset show --game vkcube | grep -q "\[x\] Sepia"'"'"
  check "o jogo ficou pronto para o efeito" 'wait_for '"'"'"$lrctl" layers | grep -q pronto'"'"
  "$lrctl" on >/dev/null; sleep 1.2; "$lrctl" off >/dev/null
  check "o efeito foi construído no jogo" 'grep -q "efeito criado" "$LOG_FILE"'
  wait "$panel_pid" || true
  check "a captura da janela foi gravada" '[ -s "$cache/painel.png" ]'
  check "a interface deixou o preset como estava" '! "$lrctl" preset show --game vkcube | grep -q "Sepia"'

  # 1b. no uso normal ninguém segura a interface além do programa: ela precisa continuar viva (botões e atualização)
  check "a interface continua viva sozinha (uso normal)" '"$panel" --keepalive-check >"$cache/keepalive.log" 2>&1'

  # 2. a interface acionada como um usuário (lista, slider, restaurar, interruptor)
  check "o teste de fumaça da interface" '"$panel" --smoke Sepia >"$cache/smoke.log" 2>&1'
  [ -s "$cache/smoke.log" ] && sed 's/^/        /' "$cache/smoke.log"
  printf '  captura: %s\n' "$cache/painel.png"
  [ "$ok" = 1 ] && say "smoke (interface): passou" || { warn "smoke (interface): falhou (logs em $cache)"; exit 1; }
}

# Mesmo teste, mas dentro do container soldier da Steam (pressure-vessel).
cmd_smoke_steam() {
  [ -x "$STEAM_SOLDIER" ] || { warn "não achei $STEAM_SOLDIER (Steam Linux Runtime soldier instalado?)"; exit 1; }
  smoke_with "container soldier" 1 60 "$STEAM_SOLDIER" -- vkcube-wayland --width 500 --height 500
}

# Teste interativo do atalho global: o vkcube roda, o lrctl daemon escuta, e você aperta o atalho.
# Não faz parte do smoke porque depende de uma pessoa (e, na primeira vez, do diálogo do GNOME).
cmd_hotkey() {
  cmd_install
  cmd_build -p linux-reshade-control
  local lrctl="$ROOT/target/release/lrctl" daemon_log="${LOG_FILE%.log}-daemon.log"
  "$lrctl" off >/dev/null
  : >"$LOG_FILE"; : >"$daemon_log"
  "$lrctl" daemon >"$daemon_log" 2>&1 &
  local daemon=$!
  env "$ENABLE_VAR=1" LINUX_RESHADE_LOG="$LOG_FILE" timeout 90 vkcube --c 5400 >/dev/null 2>&1 &
  local cube=$!
  # aspas duplas: expande já, porque as variáveis locais não existem mais quando o trap roda
  trap "kill $daemon $cube 2>/dev/null || true" EXIT
  for _ in $(seq 1 100); do grep -q "atalho" "$daemon_log" && break; sleep 0.2; done
  cat "$daemon_log"
  say "aperte o atalho acima (aceite o diálogo do GNOME se aparecer); aguardando até 60 s"
  for _ in $(seq 1 120); do grep -q "efeitos LIGADOS" "$LOG_FILE" && break; sleep 0.5; done
  if grep -q "efeitos LIGADOS" "$LOG_FILE"; then
    say "ligado pelo atalho. Aperte de novo para desligar (aguardando até 30 s)"
    for _ in $(seq 1 60); do grep -q "efeitos DESLIGADOS" "$LOG_FILE" && break; sleep 0.5; done
  fi
  echo "--- daemon"; cat "$daemon_log"; echo "--- layer"; grep -E "efeitos|estado" "$LOG_FILE"
  grep -q "efeitos LIGADOS" "$LOG_FILE" && grep -q "efeitos DESLIGADOS" "$LOG_FILE" \
    && say "hotkey: passou" || { warn "hotkey: não viu o ciclo ligar/desligar"; exit 1; }
}

# Ferramenta fx2spv (C++): compila efeitos .fx para bundles. Usa git e g++ (sem cmake).
cmd_fx_tool() { "$ROOT/tools/fx2spv/build.sh"; }
# Imprime a linha para as opções de lançamento do Steam (Propriedades > Geral > Opções de lançamento).
# Sem argumento: só a layer ligada e o log (sonda: descobre a API e a resolução, sem alterar o jogo).
# Com um diretório de bundle: o efeito fica disponível (ligue/desligue com Ctrl+Shift+F10).
cmd_game_options() {
  local line="$ENABLE_VAR=1 LINUX_RESHADE_LOG=\"$LOG_FILE\""
  [ $# -gt 0 ] && line="$line LINUX_RESHADE_BUNDLE=\"$(realpath "$1")\""
  echo "$line %command%"
}

# Compila um efeito para uma resolução: fx-compile <Efeito|arquivo.fx> <largura> <altura>. Imprime o bundle.
cmd_fx_compile() { "$ROOT/tools/fx2spv/compile.sh" "$@"; }
# Compila efeitos de exemplo (próprios e da comunidade) e valida os módulos SPIR-V.
cmd_fx_test() { "$ROOT/tools/fx2spv/test.sh" "$@"; }

# Levantamento: compila todos os efeitos de uma pasta e os executa na GPU de software (ver tools/fx2spv/survey.sh).
cmd_fx_survey() { "$ROOT/tools/fx2spv/survey.sh" "$@"; }

# Regenera os bundles de referência (golden) usados pelos testes, a partir dos .fx de origem.
cmd_fx_golden() {
  [ -x "$ROOT/build/fx2spv/fx2spv" ] || cmd_fx_tool
  local fx="$ROOT/build/fx2spv/fx2spv"
  "$fx" --width 500 --height 500 -o "$ROOT/crates/fx-bundle/tests/data/Sepia" "$ROOT/effects/Sepia.fx"
  for name in Sepia; do
    "$fx" --width 8 --height 4 -o "$ROOT/crates/postfx/tests/data/$name" "$ROOT/effects/$name.fx"
  done
  for name in Invert TwoPass Timer Mip Tex Depth Compute Volume ShareProducer ShareConsumer Ipc Ipc2; do
    "$fx" --width 8 --height 4 -o "$ROOT/crates/postfx/tests/data/$name" "$ROOT/crates/postfx/tests/fx/$name.fx"
  done
}

# Extensão do GNOME Shell que posiciona o vkcube (no Wayland o cliente não escolhe a posição).
PLACEMENT_UUID="vkcube-placement@linux-reshade.dev"
PLACEMENT_DIR="$DATA_HOME/gnome-shell/extensions/$PLACEMENT_UUID"

cmd_placement() {
  case "${1:-}" in
    install)
      mkdir -p "$(dirname "$PLACEMENT_DIR")"
      ln -sfn "$ROOT/dev/gnome-placement" "$PLACEMENT_DIR"
      # Não ativa sozinho: é código dentro do compositor e já derrubou a sessão uma vez (2026-10-09).
      warn "extensão instalada, mas DESATIVADA. No Wayland ela só carrega após sair e entrar na sessão."
      warn "para ativar (por sua conta e risco): gnome-extensions enable $PLACEMENT_UUID" ;;
    uninstall)
      gnome-extensions disable "$PLACEMENT_UUID" 2>/dev/null || true
      rm -f "$PLACEMENT_DIR"; say "extensão removida" ;;
    *) echo "uso: $0 placement <install|uninstall>" >&2; exit 2 ;;
  esac
}

cmd_logs() { tail -n "${1:-50}" -f "$LOG_FILE"; }

case "${1:-}" in
  setup)     shift; cmd_setup "$@" ;;
  doctor)    cmd_doctor ;;
  build)     shift; cmd_build "$@" ;;
  install)   cmd_install ;;
  uninstall) cmd_uninstall ;;
  run)       shift; cmd_run "$@" ;;
  smoke)     cmd_smoke ;;
  validate)  cmd_validate ;;
  smoke-steam) cmd_smoke_steam ;;
  smoke-auto) cmd_smoke_auto ;;
  smoke-layer) cmd_smoke_layer ;;
  smoke-preset) cmd_smoke_preset ;;
  smoke-depth) cmd_smoke_depth ;;
  smoke-depth-effect) cmd_smoke_depth_effect ;;
  smoke-compute) cmd_smoke_compute ;;
  smoke-dynamic) cmd_smoke_dynamic ;;
  daemon)    cmd_daemon ;;
  panel)     shift; cmd_panel "$@" ;;
  panel-smoke) cmd_panel_smoke ;;
  hotkey)    cmd_hotkey ;;
  fx-tool)   cmd_fx_tool ;;
  game-options) shift; cmd_game_options "$@" ;;
  fx-compile) shift; cmd_fx_compile "$@" ;;
  fx-test)   shift; cmd_fx_test "$@" ;;
  fx-survey) shift; cmd_fx_survey "$@" ;;
  fx-golden) cmd_fx_golden ;;
  placement) shift; cmd_placement "$@" ;;
  logs)      shift; cmd_logs "$@" ;;
  *) echo "uso: $0 <setup|doctor|build|install|uninstall|run|smoke|validate|smoke-steam|smoke-auto|smoke-layer|smoke-preset|smoke-depth|smoke-depth-effect|daemon|panel|panel-smoke|hotkey|game-options|fx-tool|fx-compile|fx-test|fx-survey|fx-golden|placement|logs>" >&2; exit 2 ;;
esac
