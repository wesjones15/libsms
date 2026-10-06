#!/usr/bin/env python3
"""Extracts the files libsms reads from your own copy of Super Mario Sunshine.

    python tools/extract_assets.py "path/to/Super Mario Sunshine.iso"

Writes (git-ignored, never commit it) to ./assets:

    assets/params_mario/   tuning .prm files        (params.szs:mario/)
    assets/bck/ bmd/ bas/ btp/ watergun2/ ...        (all of mario.szs: clips, models,
                           clip sound keys, face textures, FLUDD)
    assets/AudioRes/       wave banks, sequences, streams (disc AudioRes/), plus
                           msound.aaf (nintendo.szs:audi/msound.aaf)

Nothing here is distributed with the project; the disc is the only source. Needs
decomp-toolkit (dtk) to read the disc image and the Yaz0 archives inside it. It is found via
--dtk, the DTK environment variable, PATH, or the build/tools folder of a built decomp checkout
next to this repository (../sms-main, ../pikmin-main, ../pikmin2-main). Python 3.8+, standard library
only. Disc formats dtk reads: ISO, RVZ, WIA, WBFS, CISO, GCZ, NFS, TGC.
"""

import argparse
import hashlib
import os
import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent

# (archive inside files/data, path inside the archive, destination under --out)
# An empty inner path is the whole archive. Matches libsms::extract::EXTRACTS.
EXTRACTS = [
    ("params.szs", "mario/", "params_mario"),
    ("params.szs", "npc/", "params_npc"),
    ("params.szs", "enemy/", "params_enemy"),
    ("mario.szs", "", ""),
    ("nintendo.szs", "audi/msound.aaf", "AudioRes/msound.aaf"),
    ("yoshi.szs", "", "yoshi"),
]

# Disc folders copied as they are.
DISC_FOLDERS = ["AudioRes/"]

# Files that must exist afterwards for the sim and viewer to work.
REQUIRED = [
    "params_mario/run.prm",
    "params_mario/jump.prm",
    "params_mario/mario.prm",
    "params_mario/watergun/nozzledeform.prm",
    "bck/ma_wait.bck",
    "bck/ma_run2.bck",
    "bmd/ma_mdl1.bmd",
    "bmd/ma_cap1.bmd",
    "bmd/ma_hnd2r.bmd",
    "bas/ma_jump.bas",
    "btp/ma_wink_tx.btp",
    "watergun2/body/wg_mdl1.bmd",
    "watergun2/normal_wg/normal_wg.bmd",
    "AudioRes/msound.aaf",
    "AudioRes/mSound.asn",
    "AudioRes/Seqs/sequence.arc",
    "params_npc/npcmontem.prm",
    "npc/montemcommon/mom_wait.bck",
    "npc/montewcommon/mow_wait.bck",
    "npc/montema/moma_model.bmd",
    "npc/montew/mow_model.bmd",
    "scene/mapobj/fruitpapaya.bmd",
    "scene/mapobj/eggyoshi_normal.bmd",
    "yoshi/yoshi_model.bmd",
    "enemy/hamukuri/default.bmd",
]

# Folders gathered from every stage archive into one: (prefix, destination). Matches
# libsms::extract::SCENE_FOLDERS. monte* are the Piantas.
SCENE_FOLDERS = [("monte", "npc"), ("hamukuri", "enemy")]
# Single files gathered from the stage archives: (folder/name prefix, destination). The fruit
# Mario picks up (TResetFruit, MapObjBall.cpp) and Yoshi's egg (TEggYoshi, Item.cpp).
SCENE_FILES = [("mapobj/fruit", "scene"), ("mapobj/eggyoshi", "scene")]

GAME_IDS = {
    b"GMSJ01": "Japan (NTSC-J)",
    b"GMSE01": "USA (NTSC-U)",
    b"GMSP01": "Europe (PAL)",
}


def find_dtk(explicit):
    cands = []
    if explicit:
        cands.append(Path(explicit))
    if os.environ.get("DTK"):
        cands.append(Path(os.environ["DTK"]))
    on_path = shutil.which("dtk")
    if on_path:
        cands.append(Path(on_path))
    exe = "dtk.exe" if os.name == "nt" else "dtk"
    for rel in [
        "../sms-main/build/tools",
        "../pikmin-main/build/tools",
        "../pikmin2-main/build/tools",
    ]:
        cands.append((ROOT / rel / exe).resolve())
    for c in cands:
        if c.is_file():
            return str(c)
    return None


def read_game_id(iso):
    """Plain GameCube images start with the 6-character game ID; compressed ones don't."""
    try:
        with open(iso, "rb") as f:
            head = f.read(6)
    except OSError as e:
        sys.exit(f"cannot read {iso}: {e}")
    return head if head in GAME_IDS else None


def run(cmd, check=True):
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        if not check:
            return ""
        sys.stderr.write(r.stdout + r.stderr)
        sys.exit(f"command failed ({r.returncode}): {' '.join(cmd)}")
    return r.stdout


def normalise(dest):
    """Some dtk builds write archive paths with backslashes into the file name; fold those
    into real subfolders so the layout is the same everywhere."""
    for p in list(Path(dest).rglob("*")):
        if p.is_file() and "\\" in p.name:
            target = p.parent.joinpath(*[s for s in p.name.split("\\") if s])
            target.parent.mkdir(parents=True, exist_ok=True)
            p.replace(target)
    # tidy empty directories left behind
    for p in sorted(Path(dest).rglob("*"), reverse=True):
        if p.is_dir() and not any(p.iterdir()):
            p.rmdir()


def sha1(path):
    h = hashlib.sha1()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("iso", help="your Super Mario Sunshine disc image")
    ap.add_argument("--out", default=str(ROOT / "assets"), help="output folder (default: ./assets)")
    ap.add_argument("--dtk", help="path to decomp-toolkit's dtk executable")
    ap.add_argument("--force", action="store_true", help="re-extract even if assets already exist")
    args = ap.parse_args()

    iso = Path(args.iso).expanduser().resolve()
    if not iso.is_file():
        sys.exit(f"not a file: {iso}")
    out = Path(args.out).resolve()

    if not args.force and all((out / r).is_file() for r in REQUIRED):
        print(f"{out} already has everything; use --force to redo it.")
        return

    dtk = find_dtk(args.dtk)
    if not dtk:
        sys.exit(
            "decomp-toolkit (dtk) not found. Download a release from "
            "https://github.com/encounter/decomp-toolkit/releases and pass --dtk, set DTK, "
            "or put it on PATH."
        )

    gid = read_game_id(iso)
    if gid:
        print(f"disc: {gid.decode()} {GAME_IDS[gid]}")
    else:
        print("disc: compressed or unrecognised header; letting dtk decide")

    listing = run([dtk, "vfs", "ls", f"{iso}:files/data/"])
    for needed in ("params.szs", "mario.szs"):
        if needed not in listing:
            sys.exit(f"{needed} is not in files/data of this image; is it Super Mario Sunshine?")

    if out.exists() and args.force:
        shutil.rmtree(out)
    out.mkdir(parents=True, exist_ok=True)

    for archive, inner, dest in EXTRACTS:
        src = f"{iso}:files/data/{archive}:{inner}"
        dst = out / dest
        folder = inner == "" or inner.endswith("/")
        if folder:
            dst.mkdir(parents=True, exist_ok=True)
            target = str(dst) + os.sep
        else:
            dst.parent.mkdir(parents=True, exist_ok=True)
            target = str(dst)
        print(f"extracting {archive}:{inner}")
        run([dtk, "vfs", "cp", src, target])
        if folder:
            normalise(dst)

    scenes = [
        line.split("|")[1].strip()
        for line in run([dtk, "vfs", "ls", f"{iso}:files/data/scene/"]).splitlines()
        if "|" in line and line.split("|")[1].strip().endswith(".szs")
    ]
    for prefix, dest in SCENE_FOLDERS:
        print(f"extracting {prefix}* from {len(scenes)} stage archives")
        for scene in scenes:
            listing = run([dtk, "vfs", "ls", f"{iso}:files/data/scene/{scene}:"])
            folders = {
                line.split("|")[1].strip().rstrip("/")
                for line in listing.splitlines()
                if "|" in line and line.split("|")[1].strip().startswith(prefix)
            }
            for folder in sorted(folders):
                dst = out / dest / folder
                if dst.exists():
                    continue
                dst.mkdir(parents=True, exist_ok=True)
                run([dtk, "vfs", "cp", f"{iso}:files/data/scene/{scene}:{folder}/", str(dst) + os.sep])
                normalise(dst)

    for prefix, dest in SCENE_FILES:
        folder, stem = prefix.rsplit("/", 1)
        print(f"extracting {prefix}* from {len(scenes)} stage archives")
        for scene in scenes:
            listing = run([dtk, "vfs", "ls", f"{iso}:files/data/scene/{scene}:{folder}/"], check=False)
            for line in listing.splitlines():
                if "|" not in line:
                    continue
                name = line.split("|")[1].strip()
                if not name.startswith(stem):
                    continue
                dst = out / dest / folder / name.lower()
                if dst.exists():
                    continue
                dst.parent.mkdir(parents=True, exist_ok=True)
                run([dtk, "vfs", "cp", f"{iso}:files/data/scene/{scene}:{folder}/{name}", str(dst)])

    for folder in DISC_FOLDERS:
        dst = out / folder
        dst.mkdir(parents=True, exist_ok=True)
        print(f"extracting {folder}")
        run([dtk, "vfs", "cp", f"{iso}:files/{folder}", str(dst) + os.sep])
        normalise(dst)

    missing = [r for r in REQUIRED if not (out / r).is_file()]
    if missing:
        sys.exit("extraction finished but these files are missing:\n  " + "\n  ".join(missing))

    n_prm = len(list((out / "params_mario").rglob("*.prm")))
    n_bck = len(list((out / "bck").glob("*.bck")))
    (out / "SOURCE.txt").write_text(
        f"extracted from: {iso.name}\n"
        f"game id: {gid.decode() if gid else 'unknown'}\n"
        f"image sha1: {sha1(iso)}\n"
        f"files: {n_prm} prm, {n_bck} bck, ma_mdl1.bmd\n"
    )
    print(f"done: {n_prm} .prm files, {n_bck} animation clips, Mario's models and sound into {out}")
    print("This folder is git-ignored. Do not commit or share it.")


if __name__ == "__main__":
    main()
