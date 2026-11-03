#!/usr/bin/env python3
"""Copia os presets de uma pasta (padrão /mnt/D/ReShade/Presets) para a pasta de presets de um jogo, sem sobrescrever os que já existem.

    tools/presets/instalar.py <id-do-jogo> [--de /mnt/D/ReShade/Presets] [--dados ~/.local/share/linux-reshade]
"""
import argparse, os, shutil, sys

ap = argparse.ArgumentParser()
ap.add_argument("jogo")
ap.add_argument("--de", default="/mnt/D/ReShade/Presets")
ap.add_argument("--dados", default=os.path.expanduser("~/.local/share/linux-reshade"))
a = ap.parse_args()
dest = os.path.join(a.dados, "Presets", a.jogo)
os.makedirs(dest, exist_ok=True)
for f in sorted(os.listdir(a.de)):
    if not f.endswith(".ini"):
        continue
    target = os.path.join(dest, f)
    if os.path.exists(target):
        print(f"já existe, mantido: {f}")
    else:
        shutil.copy2(os.path.join(a.de, f), target)
        print(f"copiado: {f}")
