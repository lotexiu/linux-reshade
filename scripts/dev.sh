#!/usr/bin/env bash
# Utilitário de desenvolvimento. Idempotente; serve para trocar de máquina sem esforço.
# Uso: scripts/dev.sh <setup|doctor|build|install|uninstall|run|smoke|hotkey|placement|logs> [args]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LAYER_NAME="VK_LAYER_LOTEXIU_linux_reshade"
LAYER_LIB="liblinux_reshade_layer.so"
ENABLE_VAR="LINUX_RESHADE"
DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
MANIFEST_DIR="$DATA_HOME/vulkan/implicit_layer.d"
MANIFEST="$MANIFEST_DIR/${LAYER_NAME}.json"
LOG_FILE="${LINUX_RESHADE_LOG:-${XDG_RUNTIME_DIR:-/tmp}/linux-reshade.log}"

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
    echo "sudo apt-get install -y build-essential pkg-config vulkan-tools vulkan-validationlayers glslang-tools spirv-tools"
  elif have pacman; then
    echo "sudo pacman -S --needed base-devel pkgconf vulkan-tools vulkan-validation-layers glslang spirv-tools"
  elif have dnf; then
    echo "sudo dnf install -y gcc make pkgconf-pkg-config vulkan-tools vulkan-validation-layers glslang spirv-tools"
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
  elif have cc && have vkcube; then
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

# Build + instala + roda o vkcube; liga e desliga a flag com o lrctl no meio da execução e
# confere que a layer foi carregada, viu os presents e reagiu às duas mudanças.
cmd_smoke() {
  say "o vkcube abre na tela em foco por ~15 s; evite mexer em janelas até terminar"
  cmd_install
  cmd_build -p linux-reshade-control
  local lrctl="$ROOT/target/release/lrctl"
  "$lrctl" off >/dev/null
  : >"$LOG_FILE"
  env "$ENABLE_VAR=1" LINUX_RESHADE_LOG="$LOG_FILE" timeout 60 vkcube --c 1800 >/dev/null 2>&1 &
  local pid=$!
  # espera o primeiro present (até ~10 s) antes de mexer na flag
  for _ in $(seq 1 100); do grep -q "vkQueuePresentKHR #1$" "$LOG_FILE" 2>/dev/null && break; sleep 0.1; done
  sleep 1; "$lrctl" on >/dev/null
  sleep 2; "$lrctl" off >/dev/null
  wait "$pid" || true
  local ok=1
  for pat in "vkCreateInstance ok" "vkCreateDevice ok" "vkQueuePresentKHR #1$" \
             "estado compartilhado aberto" "efeitos LIGADOS" "efeitos DESLIGADOS" "vkDestroyInstance"; do
    if grep -q "$pat" "$LOG_FILE"; then printf '  ok     %s\n' "$pat"; else printf '  FALHOU %s\n' "$pat"; ok=0; fi
  done
  [ "$ok" = 1 ] && say "smoke: passou" || { warn "smoke: falhou (log em $LOG_FILE)"; exit 1; }
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
  trap 'kill "$daemon" "$cube" 2>/dev/null || true' EXIT
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

# Extensão do GNOME Shell que posiciona o vkcube (no Wayland o cliente não escolhe a posição).
PLACEMENT_UUID="vkcube-placement@linux-reshade.dev"
PLACEMENT_DIR="$DATA_HOME/gnome-shell/extensions/$PLACEMENT_UUID"

cmd_placement() {
  case "${1:-}" in
    install)
      mkdir -p "$(dirname "$PLACEMENT_DIR")"
      ln -sfn "$ROOT/dev/gnome-placement" "$PLACEMENT_DIR"
      if gnome-extensions enable "$PLACEMENT_UUID" 2>/dev/null; then
        say "extensão ativada"
      else
        warn "o GNOME ainda não enxergou a extensão: no Wayland é preciso sair da sessão e entrar de novo."
        warn "depois rode: gnome-extensions enable $PLACEMENT_UUID"
      fi ;;
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
  hotkey)    cmd_hotkey ;;
  placement) shift; cmd_placement "$@" ;;
  logs)      shift; cmd_logs "$@" ;;
  *) echo "uso: $0 <setup|doctor|build|install|uninstall|run|smoke|hotkey|placement|logs>" >&2; exit 2 ;;
esac
