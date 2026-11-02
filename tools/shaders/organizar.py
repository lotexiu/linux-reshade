#!/usr/bin/env python3
"""Junta duas coleções de shaders do ReShade numa pasta nova e organizada, SEM tocar nas originais.

Uso: organizar.py --a <coleção A> --b <coleção B> --out <pasta nova> [--dry-run]

A = a pasta que a pessoa já usa (a estrutura dela é mantida). B = uma coleção nova (por exemplo a do Leshade, que vem numa pasta só).
Regras:
  - arquivo de A fica no mesmo caminho relativo;
  - arquivo de B com o mesmo NOME de um de A: se o conteúdo é igual (ignorando fim de linha e espaços no fim), fica um só; se difere,
    ganha o de B (coleção mais nova), exceto nos efeitos do Marty (iMMERSE/MartysMods*) e nas texturas, onde ganha o de A; o perdedor vai
    para `_alternativas/`;
  - o resto de B é separado por categoria (CRT, Bloom, Cor...), os LUTs `* MLUT` vão para `MLUT/`;
  - sai um inventário em `_inventario/` com o que foi feito com cada arquivo.
"""
import argparse
import csv
import hashlib
import os
import re
import shutil
import sys
from collections import defaultdict

TEXT = (".fx", ".fxh")


def norm_hash(path):
    data = open(path, "rb").read()
    if path.lower().endswith(TEXT):
        text = data.decode("utf-8", "replace").replace("\r\n", "\n").replace("\r", "\n")
        data = "\n".join(line.rstrip() for line in text.split("\n")).strip().encode()
    return hashlib.sha1(data).hexdigest()


def scan(root):
    out = {}
    for dp, dn, fn in os.walk(root):
        dn.sort()
        for f in sorted(fn):
            p = os.path.join(dp, f)
            out[os.path.relpath(p, root)] = p
    return out


# Categoria de um efeito solto de B, pelo nome. A primeira regra que casa vale.
CATEGORIES = [
    ("MLUT", r".* MLUT\.(fx|png)(\.fx)?$|^RevoLUTion_MLUT"),
    ("Cor/PD80", r"^PD80_"),
    ("qUINT", r"^qUINT_"),
    ("HDR/lilium", r"^lilium__"),
    ("iMMERSE", r"^MartysMods"),
    ("CRT_Retro", r"^(CRT|crt|Cathode|ColorMAME|BasicCRT|Deconverge|DOSGame|EGAfilter|GTU|LeiFX|MAMEPostProc|MattiasCRT|MC(Amber|Green|Orange)|MetaCRT|N64|NTSC|ntsc|PAL|R57_PAL|Phosphor|PowerVR2|RGBLCD|RetroCRT|scanlines|sgenpt|TVCRTPixels|VHS|vt220|zfast_crt|lcd-grid|3DFX|nGlide|HQ4X|CoolRetroTerminal|mdapt)"),
    ("Bloom", r"^(Bloom|Bessel_Bloom|HDRBloom|MagicBloom|Emphasize)"),
    ("Desfoque_DOF", r"^(Defocus|DOF|LightDoF|CinematicDOF|fastBlur|GaussianBlur|BilateralCS|FakeMotionBlur|HDRMotionBlur|DirectionalDepthBlur|RealLongExposure)"),
    ("Nitidez", r"^(AdaptiveSharpen|CAS|FineSharp|HighPassSharpen|rj_sharpen|SharpContrast|warpsharp|LumaSharpen)"),
    ("AntiAliasing", r"^(CMAA|FXAA|SMAA)"),
    ("Cor", r"^(Chromaticity|Colourfulness|ContrastStretch|Daltonize|HDRSaturation|HSLShift|ImageAdjustment|LevelsPlus|Levels|LUT|MultiLUT|FilmicPass|pColors|ArtifactColors|SweetFX Technicolor|Technicolor|Glamayre|Sepia|Vibrance|Tonemap|LiftGammaGain|Curves|Monochrome|DPX|ColorMatrix|ColorSort|FakeHDR|Vignette|ChromaticAberration)"),
    ("Estilo", r"^(FilmGrain|Glitch|JPEG|Halftone|cmyk_halftone|Oilify|DeepFry|MMJCelShader|pColorNoise|pPalettePosterize|BilateralComic|Cartoon|ASCII|Nostalgia|PandaFX|Droste|Gravity|GoldenRatio|Frequency_CS|Layer|Border|MagicBorder|Splitscreen)"),
    ("Profundidade_Luz", r"^(AmbientLight|Dehaze|DisplayDepth|ReVeil|ReflectiveBumpMapping|NightVision|Denoise|Deband|LocalContrast|AdaptiveFog|DepthHaze|DepthDarkness|Heightfog|NormalMap|CobraMask)"),
    ("Utilidades", r"^(Mouse|UIMask|Detect_menus|pCamera|VRS_Map|CobraUtility)"),
]

# Cabeçalhos soltos de B e onde moram.
HEADER_HOME = [
    (r"^PD80_", "Cor/PD80"),
    (r"^qUINT_", "qUINT"),
    (r"^_BaseLUT", "MLUT"),
    (r"^(HDRShadersFunctions|Oklab)", "HDR"),
    (r"^(VRS_Map|pUtils|CobraUtility)", "Utilidades"),
    (r"^(FXAA|SMAA)", "AntiAliasing"),
]

# Pastas soltas de B e a categoria dos efeitos que as usam (vão juntas: os #include são relativos a elas).
FOLDER_HOME = {"crt-royale": "CRT_Retro", "lilium__include": "HDR/lilium", "MartysMods": "iMMERSE", "Pumbo": "Estilo", "SweetFX": "Cor"}

A_WINS = re.compile(r"(^|/)iMMERSE/|MartysMods", re.I)


def a_dest(rel):
    """Onde um arquivo de A fica no destino: os soltos em `Shaders/` vão para a categoria (os `#include` se resolvem pelas pastas de
    busca, não pelo caminho); o resto mantém o caminho."""
    parts = rel.split(os.sep)
    if len(parts) == 2 and parts[0] == "Shaders":
        name = parts[1]
        home = header_home(name) if name.lower().endswith(".fxh") else category_of(name)
        return os.path.join("Shaders", home, name)
    return rel


def category_of(name):
    for cat, pat in CATEGORIES:
        if re.search(pat, name):
            return cat
    return "Outros"


def header_home(name):
    for pat, cat in HEADER_HOME:
        if re.search(pat, name):
            return cat
    return "Base"


def tidy_loose(out):
    """Move (renomeia, sem apagar) os arquivos soltos de `Shaders/` do destino para a categoria deles e atualiza o inventário."""
    shaders = os.path.join(out, "Shaders")
    moved = {}
    for name in sorted(os.listdir(shaders)):
        path = os.path.join(shaders, name)
        if not os.path.isfile(path):
            continue
        rel = os.path.join("Shaders", name)
        dest = a_dest(rel)
        if dest == rel:
            continue
        os.makedirs(os.path.dirname(os.path.join(out, dest)), exist_ok=True)
        os.rename(path, os.path.join(out, dest))
        moved[rel] = dest
    csv_path = os.path.join(out, "_inventario", "origem.csv")
    rows = list(csv.reader(open(csv_path)))
    with open(csv_path, "w", newline="") as f:
        w = csv.writer(f)
        for row in rows:
            w.writerow([moved.get(row[0], row[0])] + row[1:])
    with open(os.path.join(out, "_inventario", "movidos_depois.csv"), "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["de", "para"])
        w.writerows(sorted(moved.items()))
    print("movidos:", len(moved))
    return 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--a", required=True)
    ap.add_argument("--b", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--arrumar-soltos", action="store_true", help="só move, na pasta de destino que já existe, os arquivos soltos de Shaders/ para as categorias (não apaga nada)")
    args = ap.parse_args()
    if args.arrumar_soltos:
        return tidy_loose(args.out)
    a, b = scan(args.a), scan(args.b)
    a_by_name = defaultdict(list)
    for rel in a:
        a_by_name[os.path.basename(rel).lower()].append(rel)

    plan = {}  # destino relativo -> (origem, de onde)
    notes = []  # (arquivo, decisão, detalhe)
    alternatives = []  # (destino em _alternativas, origem)

    # Famílias que vão juntas (os arquivos dependem uns dos outros): se alguma difere, a de B substitui a de A inteira.
    def family(rel):
        base = os.path.basename(rel).lower()
        return "lilium" if base.startswith("lilium__") or "lilium__include" in rel.split(os.sep) else None

    a_family = {rel for rel in a if family(rel)}
    b_family = {rel for rel in b if family(rel)}
    replace_family = False
    if a_family and b_family:
        a_names = {os.path.basename(r).lower(): r for r in a_family}
        replace_family = any(os.path.basename(r).lower() not in a_names or norm_hash(a[a_names[os.path.basename(r).lower()]]) != norm_hash(b[r]) for r in b_family)
    for rel, path in a.items():
        if replace_family and rel in a_family:
            alternatives.append((os.path.join("_alternativas", "A", rel), path))
            notes.append((rel, "família lilium__: a de B (mais nova) substituiu a de A inteira; a de A foi para _alternativas", "HDR/lilium"))
            continue
        parts_a = rel.split(os.sep)
        if len(parts_a) == 2 and parts_a[0] == "Shaders":
            # Os arquivos soltos de A também se organizam (os `#include` se resolvem pelas pastas de busca, não pelo caminho).
            name = parts_a[1]
            home = header_home(name) if name.lower().endswith(".fxh") else category_of(name)
            plan[os.path.join("Shaders", home, name)] = (path, "A")
            notes.append((rel, "arquivo solto de A: organizado por categoria", os.path.join("Shaders", home, name)))
            continue
        plan[a_dest(rel)] = (path, "A")
        if a_dest(rel) != rel:
            notes.append((rel, "arquivo solto de A: organizado por categoria", a_dest(rel)))

    for rel, path in b.items():
        parts = rel.split(os.sep)
        top, base = parts[0], os.path.basename(rel)
        in_shaders = top == "Shaders"
        in_textures = top == "Textures"
        if replace_family and rel in b_family:
            sub = parts[1:] if parts[1] != "lilium__include" else parts[1:]
            plan[os.path.join("Shaders", "HDR", "lilium", *sub)] = (path, "B")
            notes.append((rel, "novo (família lilium__ de B)", os.path.join("Shaders", "HDR", "lilium", *sub)))
            continue
        # 1) mesmo nome de um arquivo de A: decide entre os dois (a versão vencedora fica no lugar de A).
        twins = a_by_name.get(base.lower(), []) if (in_shaders or in_textures) else []
        twins = [t for t in twins if t.split(os.sep)[0] == top]
        if twins:
            target = twins[0]
            target_dest = a_dest(target)
            same = norm_hash(a[target]) == norm_hash(path)
            if same:
                notes.append((rel, "igual ao de A (ignorado)", target_dest))
                continue
            if in_textures or A_WINS.search(target) or A_WINS.search(rel):
                alternatives.append((os.path.join("_alternativas", "B", rel), path))
                notes.append((rel, "difere de A: ganhou A (textura ou Marty)", target_dest))
            else:
                alternatives.append((os.path.join("_alternativas", "A", target), a[target]))
                plan[target_dest] = (path, "B")
                notes.append((rel, "difere de A: ganhou B (coleção mais nova); o de A foi para _alternativas", target_dest))
            continue
        # 2) arquivo novo de B: vai para a categoria.
        if in_textures:
            tail = os.sep.join(parts[1:])
            dest = os.path.join("Textures", "MLUT", tail) if re.search(r" MLUT\.png$|RevoLUTion", base) else os.path.join("Textures", tail)
        elif in_shaders and len(parts) == 2:
            if base.lower().endswith(".fxh"):
                dest = os.path.join("Shaders", header_home(base), base)
            else:
                dest = os.path.join("Shaders", category_of(base), base)
        elif in_shaders:
            folder = parts[1]
            home = FOLDER_HOME.get(folder)
            dest = os.path.join("Shaders", home, *parts[1:]) if home else os.path.join("Shaders", "Outros", *parts[1:])
        else:
            dest = rel
        if dest in plan:
            if norm_hash(plan[dest][0]) == norm_hash(path):
                notes.append((rel, "duplicado dentro de B (ignorado)", dest))
                continue
            alternatives.append((os.path.join("_alternativas", "B", rel), path))
            notes.append((rel, "conflito de destino: ficou como alternativa", dest))
            continue
        plan[dest] = (path, "B")
        notes.append((rel, "novo", dest))

    # Um efeito de pasta de A que o B classificou (ex.: MartysMods/...) já foi tratado acima por nome; o resto está em `plan`.
    counts = defaultdict(int)
    for dest, (_, origin) in plan.items():
        counts[(origin, dest.split(os.sep)[0] + "/" + (dest.split(os.sep)[1] if dest.startswith("Shaders") and len(dest.split(os.sep)) > 2 else ""))] += 1
    print("arquivos no destino:", len(plan), "| alternativas:", len(alternatives))
    for (origin, group), n in sorted(counts.items()):
        print(f"  {origin} {group:<28} {n}")
    kinds = defaultdict(int)
    for _, decision, _ in notes:
        kinds[decision.split(":")[0]] += 1
    print("decisões:", dict(kinds))
    if args.dry_run:
        return 0

    out = args.out
    if os.path.exists(out) and os.listdir(out):
        print("a pasta de destino já existe e não está vazia:", out, file=sys.stderr)
        return 2
    for dest, (src, _) in plan.items():
        target = os.path.join(out, dest)
        os.makedirs(os.path.dirname(target), exist_ok=True)
        shutil.copy2(src, target)
    for dest, src in alternatives:
        target = os.path.join(out, dest)
        os.makedirs(os.path.dirname(target), exist_ok=True)
        shutil.copy2(src, target)
    inv = os.path.join(out, "_inventario")
    os.makedirs(inv, exist_ok=True)
    with open(os.path.join(inv, "decisoes.csv"), "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["arquivo_de_B", "decisao", "destino_ou_par_em_A"])
        w.writerows(sorted(notes))
    with open(os.path.join(inv, "origem.csv"), "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["destino", "origem", "arquivo_original"])
        for dest, (src, origin) in sorted(plan.items()):
            w.writerow([dest, origin, src])
    return 0


if __name__ == "__main__":
    sys.exit(main())
