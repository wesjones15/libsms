#!/usr/bin/env python
"""Pick Luigi's Mansion voice clips for each of Mario's voice cues (the voice map's input).

For each Mario cue family (small jump, damage, knocked down ...) you hear Mario's own
variants and a ranked list of Luigi clips, and tick the ones that fit. A clip may serve several
cues and a cue may take several clips: scared and whimper overlap, hurt, big jump and
knocked-down overlap, and the picker never limits a cue to one bucket. Your earlier buckets and
the clip's length only order the list (closest length to one of Mario's variants first, buckets
that usually go with the cue next); "Any length" and "Every bucket" show the rest.

    python libsms/tools/luigi_cue_picker.py [--state picks.json] [--map draft-map.json]

Keys: Up/Down move through clips; Space replays the clip; Enter or p ticks it for this cue; r
rejects it (hidden next time); b plays Mario's variant then the clip, to compare; m plays the
next Mario variant; n marks the cue "no good match"; Tab / Shift+Tab change cue.
Every change is saved, and a draft voice map (cue -> disc waves) is rewritten with it.
Standard library only.
"""
import argparse
import glob
import json
import math
import os
import re
import sys
import tempfile
import wave
import tkinter as tk
from tkinter import ttk

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import luigi_voice_sorter as S  # noqa: E402  (the clip loaders and Mario references)

winsound = S.winsound

# (id, label, Mario cue-name prefixes, buckets that usually go with it). The buckets are only a
# nudge: overlaps are expected, so every bucket stays reachable.
FAMILIES = [
    ("jump_small", "Jump - small (MV21)", ["MV21_JUMP_SMALL", "MA_VO_JUMP_SMALL"],
     ["jump_small", "effort", "jump_mid", "lil_noise", "damage_small"]),
    ("jump_mid", "Jump - mid (MV22)", ["MV22_JUMP_MID", "MA_VO_JUMP_MID"],
     ["jump_mid", "jump_small", "effort", "damage_small", "down", "land_react"]),
    ("jump_large", "Jump - large (MV23)", ["MV23_JUMP_LARGE", "MA_VO_JUMP_BIG"],
     ["jump_large", "jump_mid", "damage_big", "damage_small", "land_react", "scared", "effort"]),
    ("jump_special", "Jump - special / spin (MV24)", ["MV24_JUMP_SPECIAL", "MA_VO_JUMP_SPECIAL"],
     ["jump_special", "jump_huge", "calling_for_mario", "scared", "damage_big", "jump_large"]),
    ("jump_huge", "Jump - huge / triple (MV25A)", ["MV25A_JUMP_HUGE"],
     ["jump_huge", "calling_for_mario", "scared", "down", "jump_special"]),
    ("dive", "Dive / action (MV13)", ["MV13_ACTION_SMALL"],
     ["dive", "effort", "jump_small", "lil_noise"]),
    ("damage_big", "Damage - big (MV01)", ["MV01_DAMAGE_BIG", "MA_VO_DAMAGE_BIG"],
     ["damage_big", "damage_small", "scared", "whimper", "jump_large", "down", "calling_for_mario"]),
    ("damage_mid", "Damage - mid (MV02)", ["MV02_DAMAGE_MID"],
     ["damage_small", "damage_big", "scared", "whimper", "land_react", "effort"]),
    ("damage_small", "Damage - little (MV03)", ["MV03_DAMAGE_LITLE", "MA_VO_DAMAGE_SMALL"],
     ["damage_small", "damage_big", "scared", "whimper", "land_react", "effort"]),
    ("damage_react", "Damage reaction (MV07)", ["MV07_DAMAGE_REACT"],
     ["scared", "whimper", "land_react", "damage_small", "damage_big"]),
    ("down", "Knocked down / dead (MV08A)", ["MV08A_DOWN", "MA_VO_DEAD"],
     ["down", "damage_big", "calling_for_mario", "scared", "jump_mid", "whimper"]),
    ("down_react", "Down reaction (MV08B)", ["MV08B_DOWN_REACT"],
     ["down", "scared", "whimper", "damage_big", "calling_for_mario"]),
    ("cry", "Cry (MV10)", ["MV10A_CRY_SHORT", "MV10B_CRY_JUMP"],
     ["whimper", "scared", "lil_noise", "damage_small"]),
    ("react", "React (MV12 / MV15)", ["MV12_REACT", "MV15_REACT_SMALL"],
     ["scared", "whimper", "land_react", "effort", "lil_noise"]),
    ("effort", "Exert / effort (MV15 / MV17)", ["MV15_EXERT_INST", "MV17_EXERT_REACT"],
     ["effort", "jump_small", "dive", "lil_noise", "land_react"]),
    ("land", "Landing / jump react (MV26)", ["MV26_JUMP_REACT", "MA_VO_LAND_LOW", "MA_VO_LAND_HI"],
     ["land_react", "effort", "damage_small", "scared", "lil_noise"]),
    ("fall", "Fall after (MV18)", ["MV18_FALL_AFTER"], ["scared", "down", "calling_for_mario", "whimper"]),
    ("surprise", "Surprise (MV27 / 28 / 29)", ["MV27_SPRISE", "MV28_SPRISE_SMALL", "MV29_SPRISE_REACT"],
     ["scared", "whimper", "land_react", "lil_noise"]),
    ("fright", "Fright (MV30)", ["MV30_FRIGHT"], ["scared", "whimper", "calling_for_mario"]),
    ("door", "Open door (MV31)", ["MV31_OPEN_DOOR"], ["effort", "call", "lil_noise"]),
    ("hello", "Hello / goodbye (MV_BYEBYE, MV_CHAO)", ["MV_BYEBYE", "MV_CHAO"],
     ["call", "calling_for_mario", "laugh"]),
]

# A clip within about 0.7x..1.45x of a Mario variant's length is a candidate by default, best
# 40 first; "Show 40 more" and "Any length" reach further.
FIT_LIMIT = math.log(1.45)
LIST_STEP = 40
NOT_LIKELY = 0.35  # added to the score for a bucket that does not usually go with the cue


def wav_seconds(path):
    try:
        with wave.open(path) as w:
            return w.getnframes() / float(w.getframerate())
    except (wave.Error, EOFError, OSError):
        return 0.0


class Picks:
    """cue family -> picks, rejects, 'none' flag, note; saved at once."""

    def __init__(self, path, map_path, clips, mario_names):
        self.path, self.map_path = path, map_path
        self.clips = {c.name: c for c in clips}
        self.mario_names = mario_names
        self.cues = {}
        if os.path.isfile(path):
            self.cues = json.load(open(path, encoding="utf-8")).get("cues", {})
        for fid, *_ in FAMILIES:
            self.cues.setdefault(fid, {"picks": [], "rejects": [], "none": False, "note": ""})
        self.save()

    def cue(self, fid):
        return self.cues[fid]

    def save(self):
        self._atomic(self.path, {"version": 1, "cues": self.cues})
        draft = {}
        for fid, label, prefixes, _ in FAMILIES:
            c = self.cues[fid]
            draft[fid] = {
                "label": label,
                "mario_cues": self.mario_names(prefixes),
                "none": c["none"],
                "note": c["note"],
                "picks": [self._ref(n) for n in c["picks"]],
            }
        self._atomic(self.map_path, {"version": 1, "voice_map": draft})

    def _ref(self, name):
        clip = self.clips[name]
        disc = clip.disc or ""
        m = re.match(r"b(\d+)_(\d+)$", disc)
        return {"clip": name, "disc": disc, "bank": int(m[1]) if m else None,
                "index": int(m[2]) if m else None, "secs": round(clip.seconds, 2)}

    @staticmethod
    def _atomic(path, data):
        os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
        fd, tmp = tempfile.mkstemp(dir=os.path.dirname(path) or ".", suffix=".tmp")
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(data, f, indent=1)
        os.replace(tmp, path)


class Picker:
    def __init__(self, root, clips, picks, buckets, bucket_of, mario):
        self.root, self.clips, self.picks = root, clips, picks
        self.buckets, self.bucket_of, self.mario = buckets, bucket_of, mario
        self.by_name = {c.name: c for c in clips}
        self.fam = FAMILIES[0][0]
        self.cur = None  # selected Luigi clip
        self.cap = LIST_STEP  # how many best fits the list shows
        self.variant = 0  # Mario variant `m` plays next
        self.after_id = None
        self.dur = {n: wav_seconds(p) for n, p in mario.items()}
        root.title("Luigi cue picker")
        root.geometry("1320x800")

        top = ttk.Frame(root, padding=6)
        top.pack(fill="x")
        ttk.Label(top, text="Show:").pack(side="left")
        self.view = tk.StringVar(value="Best fits")
        v = ttk.Combobox(top, textvariable=self.view, state="readonly", width=16,
                         values=["Best fits", "Ticked", "Rejected", "Everything"])
        v.pack(side="left", padx=4)
        v.bind("<<ComboboxSelected>>", lambda e: self.fill())
        self.any_len = tk.BooleanVar(value=False)
        self.every_bucket = tk.BooleanVar(value=True)
        self.autoplay = tk.BooleanVar(value=True)
        for text, var in (("Any length", self.any_len), ("Every bucket", self.every_bucket), ("Auto-play", self.autoplay)):
            ttk.Checkbutton(top, text=text, variable=var, takefocus=0, command=self.fill if var is not self.autoplay else None).pack(side="left", padx=8)
        self.progress = ttk.Label(top, text="")
        self.progress.pack(side="right")

        body = ttk.Frame(root, padding=(6, 0))
        body.pack(fill="both", expand=True)

        left = ttk.Frame(body)
        left.pack(side="left", fill="y")
        ttk.Label(left, text="Mario cues", font=("Segoe UI", 12, "bold")).pack(anchor="w")
        self.cue_tree = ttk.Treeview(left, columns=("cue", "n"), show="headings", selectmode="browse", height=26)
        self.cue_tree.heading("cue", text="cue")
        self.cue_tree.heading("n", text="picks")
        self.cue_tree.column("cue", width=250)
        self.cue_tree.column("n", width=50, anchor="center")
        self.cue_tree.pack(fill="y", expand=True)
        for fid, label, *_ in FAMILIES:
            self.cue_tree.insert("", "end", iid=fid, values=(label, ""))
        self.cue_tree.bind("<<TreeviewSelect>>", self.on_cue)

        mid = ttk.Frame(body, padding=(10, 0))
        mid.pack(side="left", fill="both", expand=True)
        self.heading = ttk.Label(mid, text="", font=("Segoe UI", 15, "bold"))
        self.heading.pack(anchor="w")
        self.mario_box = ttk.LabelFrame(mid, text="Mario's variants (click to hear, m = next)", padding=4)
        self.mario_box.pack(fill="x", pady=4)
        self.canvas = tk.Canvas(mid, height=110, bg="#101418", highlightthickness=0)
        self.canvas.pack(fill="x", pady=4)
        cols = ("tick", "clip", "secs", "fit", "disc", "bucket", "also")
        self.tree = ttk.Treeview(mid, columns=cols, show="headings", selectmode="browse")
        for c, w, t in (("tick", 36, ""), ("clip", 80, "clip"), ("secs", 55, "secs"), ("fit", 60, "length vs Mario"),
                        ("disc", 70, "disc"), ("bucket", 150, "your bucket"), ("also", 250, "also ticked for")):
            self.tree.heading(c, text=t)
            self.tree.column(c, width=w, anchor="center" if c in ("tick", "secs", "fit") else "w")
        sb = ttk.Scrollbar(mid, command=self.tree.yview)
        self.tree.configure(yscrollcommand=sb.set)
        self.tree.pack(side="left", fill="both", expand=True)
        sb.pack(side="left", fill="y")
        self.tree.tag_configure("ticked", background="#d6f5df")
        self.tree.tag_configure("rejected", foreground="#999")
        self.tree.bind("<<TreeviewSelect>>", self.on_clip)

        right = ttk.Frame(body, padding=(8, 0))
        right.pack(side="left", fill="y")
        for text, cmd in (("Play clip (Space)", self.play), ("Show 40 more", self.show_more), ("Mario then clip (b)", self.ab), ("Next Mario variant (m)", self.play_mario),
                          ("Tick for this cue (Enter)", self.toggle_tick), ("Reject (r)", self.reject),
                          ("No good match (n)", self.toggle_none), ("Stop", self.stop)):
            ttk.Button(right, text=text, command=cmd, takefocus=0, width=24).pack(pady=2)
        ttk.Label(right, text="Note for this cue:").pack(anchor="w", pady=(14, 0))
        self.note = tk.StringVar()
        self.note_entry = ttk.Entry(right, textvariable=self.note, width=30)
        self.note_entry.pack()
        self.note_entry.bind("<FocusOut>", lambda e: self.save_note())
        self.note_entry.bind("<Return>", lambda e: (self.save_note(), self.root.focus_set()))
        self.status = ttk.Label(right, text="", wraplength=230, foreground="#555")
        self.status.pack(anchor="w", pady=10)

        root.bind("<Key>", self.on_key)
        root.bind("<Tab>", lambda e: self.step_cue(1) or "break")
        root.bind("<Shift-Tab>", lambda e: self.step_cue(-1) or "break")
        root.protocol("WM_DELETE_WINDOW", self.close)
        self.refresh_cues()
        self.cue_tree.selection_set(self.fam)

    # ---- cues
    def family(self, fid=None):
        return next(f for f in FAMILIES if f[0] == (fid or self.fam))

    def variants(self, fid=None):
        return self.picks.mario_names(self.family(fid)[2])

    def refresh_cues(self):
        done = 0
        for fid, label, *_ in FAMILIES:
            c = self.picks.cue(fid)
            n = len(c["picks"])
            done += 1 if n else 0
            mark = str(n) if n else ("none" if c["none"] else "")
            self.cue_tree.item(fid, values=(label, mark))
        self.progress.configure(text="%d of %d cues have picks" % (done, len(FAMILIES)))

    def on_cue(self, _):
        sel = self.cue_tree.selection()
        if not sel:
            return
        self.save_note()
        self.fam = sel[0]
        self.variant = 0
        self.cap = LIST_STEP
        fid, label, prefixes, _ = self.family()
        self.heading.configure(text=label)
        for w in self.mario_box.winfo_children():
            w.destroy()
        vs = self.variants()
        for i, name in enumerate(vs):
            ttk.Button(self.mario_box, text="%s  %.2fs" % (name, self.dur.get(name, 0)), takefocus=0,
                       command=lambda n=name: self.play_path(self.mario[n])).grid(row=i // 3, column=i % 3, padx=2, pady=1, sticky="w")
        self.note.set(self.picks.cue(fid)["note"])
        self.cur = None
        self.fill()

    def step_cue(self, d):
        ids = [f[0] for f in FAMILIES]
        self.cue_tree.selection_set(ids[(ids.index(self.fam) + d) % len(ids)])

    # ---- candidates
    def score(self, clip):
        """(length fit as |ln ratio| to the nearest Mario variant, bucket nudge); lower is better."""
        ds = [self.dur[n] for n in self.variants() if self.dur.get(n, 0) > 0]
        fit = min((abs(math.log(max(clip.seconds, 0.03) / d)) for d in ds), default=9.0)
        likely = self.family()[3]
        nudge = 0.0 if self.bucket_of.get(clip.name) in likely else NOT_LIKELY
        return fit, nudge

    def fill(self):
        fid = self.fam
        c = self.picks.cue(fid)
        view = self.view.get()
        rows = []
        for clip in self.clips:
            fit, nudge = self.score(clip)
            n = clip.name
            if view == "Ticked":
                ok = n in c["picks"]
            elif view == "Rejected":
                ok = n in c["rejects"]
            elif view == "Everything":
                ok = True
            else:  # Best fits
                ok = n not in c["rejects"]
                if ok and not self.any_len.get() and fit > FIT_LIMIT:
                    ok = n in c["picks"]
                if ok and not self.every_bucket.get() and nudge > 0:
                    ok = n in c["picks"]
            if ok:
                rows.append((fit + nudge, fit, n))
        rows.sort()
        total = len(rows)
        if view == "Best fits" and total > self.cap:
            ticked = set(c["picks"])
            rows = [r for i, r in enumerate(rows) if i < self.cap or r[2] in ticked]
        self.tree.delete(*self.tree.get_children())
        for _, fit, n in rows:
            self.tree.insert("", "end", iid=n, values=self.row(n, fit), tags=self.tags(n))
        keep = self.cur if self.cur in {r[2] for r in rows} else (rows[0][2] if rows else None)
        if keep:
            self.tree.selection_set(keep)
            self.tree.see(keep)
        more = " (%d more fit)" % (total - len(rows)) if total > len(rows) else ""
        self.status.configure(text="%d clips listed%s" % (len(rows), more))

    def tags(self, n):
        c = self.picks.cue(self.fam)
        return ("ticked",) if n in c["picks"] else (("rejected",) if n in c["rejects"] else ())

    def row(self, n, fit=None):
        clip = self.by_name[n]
        if fit is None:
            fit = self.score(clip)[0]
        ratio = math.exp(fit)
        others = [self.family(f)[1].split(" (")[0] for f in self.picks.cues
                  if f != self.fam and n in self.picks.cues[f]["picks"]]
        tick = "✔" if n in self.picks.cue(self.fam)["picks"] else ("x" if n in self.picks.cue(self.fam)["rejects"] else "")
        return (tick, n, "%.2f" % clip.seconds, "x%.2f" % ratio, clip.disc or "?",
                self.buckets.get(self.bucket_of.get(n), ""), ", ".join(others))

    def on_clip(self, _):
        sel = self.tree.selection()
        if not sel:
            return
        self.cur = sel[0]
        self.draw(self.by_name[self.cur])
        if self.autoplay.get():
            self.play()

    def draw(self, clip):
        self.canvas.delete("all")
        w = max(100, self.canvas.winfo_width())
        h = int(self.canvas["height"])
        mid = h // 2
        self.canvas.create_line(0, mid, w, mid, fill="#2a323c")
        for x, (lo, hi) in enumerate(clip.peaks(w)):
            self.canvas.create_line(x, mid - hi * (mid - 5), x, mid - lo * (mid - 5) + 1, fill="#6fd0a8")

    # ---- picking
    def update_row(self, n):
        if n in self.tree.get_children():
            self.tree.item(n, values=self.row(n), tags=self.tags(n))

    def show_more(self):
        self.cap += LIST_STEP
        self.fill()

    def toggle_tick(self):
        if not self.cur:
            return
        c = self.picks.cue(self.fam)
        if self.cur in c["picks"]:
            c["picks"].remove(self.cur)
        else:
            c["picks"].append(self.cur)
            if self.cur in c["rejects"]:
                c["rejects"].remove(self.cur)
            c["none"] = False
        self.picks.save()
        self.update_row(self.cur)
        self.refresh_cues()
        self.status.configure(text="%s: %d ticked" % (self.family()[1], len(c["picks"])))

    def reject(self):
        if not self.cur:
            return
        c = self.picks.cue(self.fam)
        n = self.cur
        if n in c["picks"]:
            c["picks"].remove(n)
        if n not in c["rejects"]:
            c["rejects"].append(n)
        self.picks.save()
        kids = list(self.tree.get_children())
        i = kids.index(n) if n in kids else 0
        self.refresh_cues()
        if self.view.get() == "Best fits":
            self.tree.delete(n)
            kids = list(self.tree.get_children())
            if kids:
                self.tree.selection_set(kids[min(i, len(kids) - 1)])
        else:
            self.update_row(n)

    def toggle_none(self):
        c = self.picks.cue(self.fam)
        c["none"] = not c["none"]
        self.picks.save()
        self.refresh_cues()
        self.status.configure(text="no good match: %s" % ("yes" if c["none"] else "no"))

    def save_note(self):
        c = self.picks.cues.get(self.fam)
        if c is not None and c["note"] != self.note.get().strip():
            c["note"] = self.note.get().strip()
            self.picks.save()

    # ---- sound
    def play_path(self, path):
        if winsound:
            winsound.PlaySound(path, winsound.SND_FILENAME | winsound.SND_ASYNC)

    def play(self):
        if self.cur:
            self.play_path(self.by_name[self.cur].path)

    def stop(self):
        if self.after_id:
            self.root.after_cancel(self.after_id)
            self.after_id = None
        if winsound:
            winsound.PlaySound(None, winsound.SND_PURGE)

    def play_mario(self, delay_for=None):
        vs = self.variants()
        if not vs:
            return 0.0
        name = vs[self.variant % len(vs)]
        self.variant += 1
        self.play_path(self.mario[name])
        self.status.configure(text="Mario: %s (%.2fs)" % (name, self.dur.get(name, 0)))
        return self.dur.get(name, 0.0)

    def ab(self):
        """Mario's next variant, then the selected Luigi clip, to judge them back to back."""
        if not self.cur:
            return
        self.stop()
        d = self.play_mario()
        self.after_id = self.root.after(int((d + 0.3) * 1000), self.play)

    def on_key(self, e):
        if isinstance(self.root.focus_get(), (tk.Entry, ttk.Entry)):
            return
        k, ch = e.keysym, (e.char or "").lower()
        if k == "space":
            self.play()
        elif k == "Return" or ch == "p":
            self.toggle_tick()
        elif ch == "r" or k == "Delete":
            self.reject()
        elif ch == "b":
            self.ab()
        elif ch == "m":
            self.play_mario()
        elif ch == "n":
            self.toggle_none()

    def close(self):
        self.save_note()
        self.stop()
        self.root.destroy()


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--clips", help="folder of Luigi WAVs (default: found under 'iso backups')")
    ap.add_argument("--table", default=os.path.join(S.ROOT, "output", "lm-rip-match.txt"))
    ap.add_argument("--mario", default=os.path.join(S.ROOT, "output", "mario-refs"))
    ap.add_argument("--buckets", default=os.path.join(S.ROOT, "output", "luigi-voice-buckets.json"),
                    help="the sorter's file; its buckets only reorder the candidates")
    ap.add_argument("--state", default=os.path.join(S.ROOT, "output", "luigi-cue-picks.json"))
    ap.add_argument("--map", default=os.path.join(S.ROOT, "output", "luigi-voice-map-draft.json"))
    ap.add_argument("--selftest", action="store_true")
    a = ap.parse_args()
    clips_dir = a.clips or S.find_clips_dir()
    if not clips_dir or not os.path.isdir(clips_dir):
        sys.exit("No Luigi WAV folder found; pass --clips DIR")
    disc = S.read_table(a.table)
    paths = sorted(glob.glob(os.path.join(clips_dir, "*.wav")), key=lambda p: S.natural_key(os.path.basename(p)))
    clips = [S.Clip(os.path.splitext(os.path.basename(p))[0], p, disc.get(os.path.splitext(os.path.basename(p))[0])) for p in paths]
    mario = S.read_mario_refs(a.mario)
    if not mario:
        sys.exit("No Mario reference WAVs in %s (see luigi_voice_sorter --mario)" % a.mario)
    buckets, bucket_of = {}, {}
    if os.path.isfile(a.buckets):
        b = json.load(open(a.buckets, encoding="utf-8"))
        buckets = {x["id"]: x["label"] for x in b.get("buckets", [])}
        bucket_of = b.get("assign", {})

    def mario_names(prefixes):
        out = []
        for p in prefixes:
            out += sorted((n for n in mario if n == p or n.startswith(p + "_")), key=S.natural_key)
        return out

    picks = Picks(a.state, a.map, clips, mario_names)
    root = tk.Tk()
    app = Picker(root, clips, picks, buckets, bucket_of, mario)
    if a.selftest:
        root.update()
        app.autoplay.set(False)
        first = app.tree.get_children()[0]
        app.tree.selection_set(first)
        root.update()
        app.toggle_tick()
        second = app.tree.get_children()[1]
        app.tree.selection_set(second)
        root.update()
        app.reject()
        app.step_cue(1)
        root.update()
        app.tree.selection_set(app.tree.get_children()[0])
        root.update()
        app.toggle_tick()
        m = json.load(open(a.map, encoding="utf-8"))["voice_map"]
        assert m["jump_small"]["picks"][0]["clip"] == first, m["jump_small"]
        assert m["jump_mid"]["picks"], m["jump_mid"]
        print("selftest ok: %d clips, %d cues; first jump_small pick %s %s; map %s"
              % (len(clips), len(FAMILIES), first, m["jump_small"]["picks"][0]["disc"], a.map))
        root.destroy()
        return
    print("%d clips, %d Mario cues; picks %s; draft map %s" % (len(clips), len(mario), a.state, a.map))
    root.mainloop()


if __name__ == "__main__":
    main()
