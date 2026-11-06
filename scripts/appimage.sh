#!/usr/bin/env bash
# Monta o AppImage do linux-reshade em dist/: o painel (lrpanel), a linha de comando (lrctl), a camada do Vulkan, o compilador de efeitos
# (fx2spv) e os efeitos que vêm com o programa. Usa o linuxdeploy (com o plugin do GTK) para juntar as bibliotecas e o appimagetool para
# empacotar; baixa os dois em dist/tools na primeira vez.
#
# Uso: scripts/appimage.sh [--appdir-only]   (--appdir-only monta só a pasta AppDir, sem baixar nada nem empacotar: serve para testar o AppRun)
# A distribuição que roda o AppImage precisa ter a glibc pelo menos tão nova quanto a da máquina que o montou.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
# shellcheck disable=SC1091
[ -f "$HOME/.asdf/asdf.sh" ] && . "$HOME/.asdf/asdf.sh"
export PATH="$HOME/.cargo/bin:$PATH"
say() { printf '\033[1;36m==>\033[0m %s\n' "$*"; }

VERSION="$(awk -F'"' '/^version/{print $2; exit}' Cargo.toml)"
DIST="$ROOT/dist"; APPDIR="$DIST/AppDir"; TOOLS="$DIST/tools"
mkdir -p "$TOOLS"

say "compilando (release)"
cargo build --release -p linux-reshade-control -p linux-reshade-gui -p linux-reshade-layer
[ -x build/fx2spv/fx2spv ] || tools/fx2spv/build.sh

say "AppDir"
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/lib" "$APPDIR/usr/share/linux-reshade/effects" "$APPDIR/usr/share/applications" "$APPDIR/usr/share/icons/hicolor/scalable/apps"
install -m755 target/release/lrpanel target/release/lrctl "$APPDIR/usr/bin/"
install -m755 build/fx2spv/fx2spv "$APPDIR/usr/bin/fx2spv"
install -m755 "target/release/liblinux_reshade_layer.so" "$APPDIR/usr/lib/"
# O otimizador de SPIR-V é opcional (deixa a GPU de 5 a 10% mais rápida); entra se existe na máquina.
for tool in spirv-opt spirv-val; do command -v "$tool" >/dev/null && install -m755 "$(command -v "$tool")" "$APPDIR/usr/bin/"; done || true
cp -r effects/. "$APPDIR/usr/share/linux-reshade/effects/"
for branch in slim legacy; do
  src="third_party/reshade-shaders-$branch"
  [ -d "$src" ] || continue
  mkdir -p "$APPDIR/usr/share/linux-reshade/$src"
  for part in Shaders Textures; do [ -d "$src/$part" ] && cp -r "$src/$part" "$APPDIR/usr/share/linux-reshade/$src/"; done
done
install -m644 packaging/linux-reshade.desktop "$APPDIR/usr/share/applications/linux-reshade.desktop"
install -m644 packaging/linux-reshade.svg "$APPDIR/usr/share/icons/hicolor/scalable/apps/linux-reshade.svg"

if [ "${1:-}" = "--appdir-only" ]; then
  rm -f "$APPDIR/AppRun"; install -m755 packaging/AppRun "$APPDIR/AppRun"
  say "AppDir pronto em $APPDIR (sem as bibliotecas do GTK: usa as do sistema)"
  exit 0
fi

fetch() { [ -f "$TOOLS/$1" ] || { say "baixando $1"; curl -fsSL -o "$TOOLS/$1" "$2"; chmod +x "$TOOLS/$1"; }; }
fetch linuxdeploy-x86_64.AppImage https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage
fetch linuxdeploy-plugin-gtk.sh https://raw.githubusercontent.com/linuxdeploy/linuxdeploy-plugin-gtk/master/linuxdeploy-plugin-gtk.sh
fetch appimagetool-x86_64.AppImage https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage

say "juntando as bibliotecas (linuxdeploy + GTK)"
export APPIMAGE_EXTRACT_AND_RUN=1 DEPLOY_GTK_VERSION=4 NO_STRIP=1 PATH="$TOOLS:$PATH"
"$TOOLS/linuxdeploy-x86_64.AppImage" --appdir "$APPDIR" \
  --executable "$APPDIR/usr/bin/lrpanel" --executable "$APPDIR/usr/bin/lrctl" --executable "$APPDIR/usr/bin/fx2spv" \
  --library "$APPDIR/usr/lib/liblinux_reshade_layer.so" \
  --desktop-file "$APPDIR/usr/share/applications/linux-reshade.desktop" --icon-file packaging/linux-reshade.svg --plugin gtk

# O AppRun próprio (o do linuxdeploy só abre um executável).
rm -f "$APPDIR/AppRun"
install -m755 packaging/AppRun "$APPDIR/AppRun"

say "empacotando"
OUT="$DIST/linux-reshade-$VERSION-x86_64.AppImage"
# Um AppImage anterior no mesmo caminho é trocado: sem isto o empacotador pode deixar o arquivo velho (e o painel, ao abrir, só
# troca o daemon e a camada de quem roda a build nova se o arquivo for mesmo o novo).
rm -f "$OUT"
ARCH=x86_64 "$TOOLS/appimagetool-x86_64.AppImage" --no-appstream "$APPDIR" "$OUT"
say "pronto: $OUT ($(du -h "$OUT" | cut -f1))"
