#!/usr/bin/env python3
"""Mede quanto a GPU gasta por quadro com um preset: compila os efeitos dele (fx2spv) e roda a cadeia na GPU real
(teste `a_chain_of_compiled_bundles_runs`, com profundidade falsa). Uso:

    tools/presets/medir.py <preset.ini>... [--size 1920x1080] [--shaders /mnt/D/ReShade/Shaders]

O preset usa os valores padrão dos efeitos (mais as macros das seções `[Efeito.fx]`). Os tempos são só de GPU e só de uma estimativa: o teste não carrega as imagens de arquivo (efeitos que dependem delas, como o SMAA, aparecem em PULADAS e não entram na conta). Para o custo de verdade, use a aba Desempenho com o jogo aberto.
"""
import argparse, os, re, subprocess, sys, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

def parse(path):
    techniques, macros, section = [], {}, None
    for line in open(path, encoding="utf-8", errors="replace"):
        line = line.strip()
        if m := re.match(r"\[(.+)\]$", line):
            section = m.group(1)
        elif section is None and line.startswith("Techniques="):
            techniques = [t for t in line[11:].split(",") if t]
        elif section and "=" in line:
            k, v = line.split("=", 1)
            macros.setdefault(section, []).append((k, v))
    return techniques, macros

def find(shaders, name):
    for d, _, fs in os.walk(shaders):
        if name in fs:
            return os.path.join(d, name)

def measure(preset, shaders, size):
    techniques, macros = parse(preset)
    w, h = size.split("x")
    files = []
    for t in techniques:
        f = t.split("@", 1)[1] if "@" in t else t + ".fx"
        if f not in files:
            files.append(f)
    includes = [a for d, _, _ in os.walk(shaders) for a in ("-I", d)]
    out = tempfile.mkdtemp(prefix="medir-")
    dirs = []
    for f in files:
        src = find(shaders, f)
        if not src:
            return f"{f}: não achei"
        dest = os.path.join(out, f[:-3])
        cmd = [os.path.join(ROOT, "build/fx2spv/fx2spv"), "--width", w, "--height", h, *includes]
        text = open(src, encoding="utf-8", errors="replace").read()
        for k, v in macros.get(f, []):
            # As seções do preset guardam macros e valores de uniforms; só os nomes definidos com `#define` no efeito são macros.
            if re.search(rf"#\s*(ifndef|define|ifdef)\s+{re.escape(k)}\b", text):
                cmd += ["-D", f"{k}={v}"]
        r = subprocess.run(cmd + ["-o", dest, src], capture_output=True, text=True)
        if r.returncode:
            return f"{f}: não compila: {r.stderr.strip().splitlines()[:1]}"
        dirs.append(dest)
    env = dict(os.environ, LINUX_RESHADE_CHAIN_BUNDLES=":".join(dirs), LINUX_RESHADE_CHAIN_SIZE=size, LINUX_RESHADE_TEST_HARDWARE="1",
               LINUX_RESHADE_CHAIN_PROFILE="1", LINUX_RESHADE_CHAIN_FRAMES="40")
    r = subprocess.run(["cargo", "test", "-q", "-p", "postfx", "--test", "community", "a_chain_of_compiled_bundles_runs", "--", "--nocapture"],
                       cwd=ROOT, env=env, capture_output=True, text=True)
    text = r.stderr + r.stdout
    total = re.search(r"PERFIL por passe \(média, total ([\d.]+) ms\)", text)
    skipped = re.findall(r"pulada: (.*)", text)
    line = f"{os.path.basename(preset)[:-4]}: " + (f"{total.group(1)} ms de GPU por quadro a {size}" if total else "sem medição (ver erro)")
    if skipped:
        line += f"  PULADAS: {skipped}"
    if not total:
        line += "\n" + "\n".join(text.strip().splitlines()[-6:])
    return line

if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("presets", nargs="+")
    ap.add_argument("--size", default="1920x1080")
    ap.add_argument("--shaders", default="/mnt/D/ReShade/Shaders")
    a = ap.parse_args()
    for p in a.presets:
        print(measure(p, a.shaders, a.size), flush=True)
