#!/usr/bin/env python
"""Sort Luigi's Mansion voice clips into buckets, to build the voice map for the Luigi skin.

Loads a folder of Luigi WAVs (a labelled rip: Luigi1.wav ... Luigi304.wav), plays each, and
files it under a bucket with a click or a hotkey. Every change is saved at once, so you can
stop and carry on later. Add finer buckets whenever you like; to split a bucket, pick it in
"Show", then move its clips into the new buckets (second pass).

    python libsms/tools/luigi_voice_sorter.py [--clips DIR] [--table rip-match.txt] [--state out.json]

Defaults look under "iso backups" for the rip, `output/lm-rip-match.txt` for where each clip
sits on the disc (from `rip_match`), and write `output/luigi-voice-buckets.json`.

Keys: 1-9, 0 and the letters shown beside each bucket file the clip there; Space replays;
Left/Right (or Up/Down in the list) move; Backspace takes the clip out of its bucket.
Standard library only (tkinter, winsound for playback on Windows). Reads and writes nothing
else; the rip stays where it is.
"""
import argparse
import glob
import json
import os
import re
import sys
import tempfile
import wave
import tkinter as tk
from tkinter import messagebox, simpledialog, ttk

try:
    import winsound
except ImportError:  # not Windows: the clips still list and sort, they just don't play
    winsound = None

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import mario_voice_info  # what each Mario cue is for (decomp call sites)

# Buckets to start with, named after what the Mario cues need. Add or rename freely.
DEFAULT_BUCKETS = [
    ("jump_small", "Jump - small (hup)"),
    ("jump_mid", "Jump - mid"),
    ("jump_large", "Jump - large"),
    ("jump_special", "Jump - special / spin"),
    ("jump_huge", "Jump - huge / triple"),
    ("dive", "Dive / belly slide"),
    ("effort", "Effort / small action"),
    ("damage_small", "Damage - small"),
    ("damage_big", "Damage - big"),
    ("land_react", "Landing / jump reaction"),
    ("down", "Knocked down / dead"),
    ("scared", "Scared / surprised"),
    ("whimper", "Whimper / cry"),
    ("laugh", "Laugh / happy"),
    ("call", "Calling / talking / hum"),
    ("other_useful", "Other - useful noise"),
    ("junk", "Junk / unusable"),
]
# Which of Mario's cues (name prefixes of the clips in the Mario folder) stand beside each
# starting bucket as its reference; a bucket you add has none until you pick one by hand.
MARIO_REF = {
    "jump_small": ["MV21_JUMP_SMALL", "MA_VO_JUMP_SMALL"],
    "jump_mid": ["MV22_JUMP_MID", "MA_VO_JUMP_MID"],
    "jump_large": ["MV23_JUMP_LARGE", "MA_VO_JUMP_BIG"],
    "jump_special": ["MV24_JUMP_SPECIAL", "MA_VO_JUMP_SPECIAL"],
    "jump_huge": ["MV25A_JUMP_HUGE"],
    "dive": ["MV13_ACTION_SMALL"],
    "effort": ["MV15_EXERT_INST", "MV17_EXERT_REACT", "MV15_REACT_SMALL"],
    "damage_small": ["MV03_DAMAGE_LITLE", "MV02_DAMAGE_MID", "MA_VO_DAMAGE_SMALL"],
    "damage_big": ["MV01_DAMAGE_BIG", "MA_VO_DAMAGE_BIG"],
    "land_react": ["MV26_JUMP_REACT", "MA_VO_LAND_LOW", "MA_VO_LAND_HI"],
    "down": ["MV08A_DOWN", "MA_VO_DEAD", "MV08B_DOWN_REACT"],
    "scared": ["MV27_SPRISE", "MV28_SPRISE_SMALL", "MV30_FRIGHT"],
    "whimper": ["MV10A_CRY_SHORT", "MV10B_CRY_JUMP"],
    "call": ["MV_BYEBYE", "MV_CHAO"],
}

# Hotkeys handed out in order; the rest are reserved for navigation.
KEY_POOL = list("1234567890qwertyuiopasdfghjklzcvbnm")
RESERVED = {" ", "x", "m"}  # x: next Mario reference of the last bucket; m: same, picked cue


def natural_key(name):
    return [int(p) if p.isdigit() else p for p in re.split(r"(\d+)", name)]


class Clip:
    def __init__(self, name, path, disc):
        self.name, self.path, self.disc = name, path, disc
        self.seconds = 0.0
        self._wave = None
        try:
            with wave.open(path) as w:
                self.seconds = w.getnframes() / float(w.getframerate())
        except (wave.Error, EOFError):
            pass

    def peaks(self, width):
        """`width` (low, high) pairs in -1..1 for the waveform view."""
        if self._wave is None:
            try:
                with wave.open(self.path) as w:
                    data = w.readframes(w.getnframes())
                    ch, sw = w.getnchannels(), w.getsampwidth()
                if sw != 2:
                    raise wave.Error("not 16-bit")
                import array

                a = array.array("h")
                a.frombytes(data)
                self._wave = a[::ch]
            except (wave.Error, EOFError):
                self._wave = []
        n = len(self._wave)
        if n == 0:
            return [(0.0, 0.0)] * width
        out = []
        for i in range(width):
            seg = self._wave[n * i // width : max(n * i // width + 1, n * (i + 1) // width)]
            out.append((min(seg) / 32768.0, max(seg) / 32768.0))
        return out


def read_table(path):
    """`rip_match` table: clip name -> 'b2_11' (bank and wave index on the disc)."""
    disc = {}
    if path and os.path.isfile(path):
        for line in open(path, encoding="utf-8").read().splitlines()[1:]:
            f = line.split()
            if len(f) >= 4:
                disc[f[0]] = "b%s_%s" % (f[1], f[3])
    return disc


def read_mario_refs(folder):
    """cue name -> wav path for Mario's own cues (rendered from the Sunshine disc)."""
    refs = {}
    if folder and os.path.isdir(folder):
        for p in glob.glob(os.path.join(folder, "*.wav")):
            refs[os.path.splitext(os.path.basename(p))[0]] = p
    return refs


def find_clips_dir():
    hits = glob.glob(os.path.join(ROOT, "iso backups", "*Luigi*Voice*", "**", "Luigi1.wav"), recursive=True)
    return os.path.dirname(hits[0]) if hits else None


class State:
    """The buckets and who is in them; saved after every change."""

    def __init__(self, path):
        self.path = path
        self.buckets = []  # {"id", "label", "key"}
        self.assign = {}  # clip name -> bucket id
        self.notes = {}
        if os.path.isfile(path):
            d = json.load(open(path, encoding="utf-8"))
            self.buckets = d.get("buckets", [])
            self.assign = d.get("assign", {})
            self.notes = d.get("notes", {})
        if not self.buckets:
            for bid, label in DEFAULT_BUCKETS:
                self.add_bucket(label, bid, save=False)
        self.save()

    def next_key(self):
        used = {b["key"] for b in self.buckets}
        return next((k for k in KEY_POOL if k not in used and k not in RESERVED), "")

    def add_bucket(self, label, bid=None, save=True):
        bid = bid or re.sub(r"[^a-z0-9]+", "_", label.lower()).strip("_") or "bucket"
        base, n = bid, 2
        while any(b["id"] == bid for b in self.buckets):
            bid, n = "%s_%d" % (base, n), n + 1
        b = {"id": bid, "label": label, "key": self.next_key()}
        self.buckets.append(b)
        if save:
            self.save()
        return b

    def label(self, bid):
        return next((b["label"] for b in self.buckets if b["id"] == bid), "")

    def count(self, bid):
        return sum(1 for v in self.assign.values() if v == bid)

    def save(self):
        data = {"version": 1, "buckets": self.buckets, "assign": self.assign, "notes": self.notes}
        fd, tmp = tempfile.mkstemp(dir=os.path.dirname(self.path) or ".", suffix=".tmp")
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(data, f, indent=1)
        os.replace(tmp, self.path)


class App:
    def __init__(self, root, clips, state, mario=None):
        self.root, self.clips, self.state = root, clips, state
        self.mario = mario or {}
        self.mario_next = {}  # bucket id -> which of its Mario variants plays next
        self.last_ref = None  # (bucket id, or None for the picker) the x key repeats
        self.by_name = {c.name: c for c in clips}
        self.current = None
        self.visible = []
        root.title("Luigi voice sorter")
        root.geometry("1180x720")

        top = ttk.Frame(root, padding=6)
        top.pack(fill="x")
        ttk.Label(top, text="Show:").pack(side="left")
        self.show = tk.StringVar(value="Unsorted")
        self.show_box = ttk.Combobox(top, textvariable=self.show, state="readonly", width=34)
        self.show_box.pack(side="left", padx=4)
        self.show_box.bind("<<ComboboxSelected>>", lambda e: self.refresh())
        ttk.Label(top, text="Order:").pack(side="left", padx=(12, 0))
        self.order = tk.StringVar(value="clip number")
        order = ttk.Combobox(top, textvariable=self.order, state="readonly", width=14, values=["clip number", "disc wave"])
        order.pack(side="left", padx=4)
        order.bind("<<ComboboxSelected>>", lambda e: self.refresh())
        self.autoplay = tk.BooleanVar(value=True)
        self.advance = tk.BooleanVar(value=True)
        ttk.Checkbutton(top, text="Auto-play", variable=self.autoplay, takefocus=0).pack(side="left", padx=(12, 0))
        ttk.Checkbutton(top, text="Advance after filing", variable=self.advance, takefocus=0).pack(side="left", padx=6)
        self.progress = ttk.Label(top, text="")
        self.progress.pack(side="right")

        body = ttk.Frame(root, padding=(6, 0))
        body.pack(fill="both", expand=True)

        left = ttk.Frame(body)
        left.pack(side="left", fill="both", expand=True)
        cols = ("clip", "secs", "disc", "bucket")
        self.tree = ttk.Treeview(left, columns=cols, show="headings", selectmode="browse")
        for c, w in zip(cols, (90, 55, 70, 170)):
            self.tree.heading(c, text=c)
            self.tree.column(c, width=w, anchor="w")
        sb = ttk.Scrollbar(left, command=self.tree.yview)
        self.tree.configure(yscrollcommand=sb.set)
        self.tree.pack(side="left", fill="both", expand=True)
        sb.pack(side="left", fill="y")
        self.tree.bind("<<TreeviewSelect>>", self.on_select)

        mid = ttk.Frame(body, padding=(10, 0))
        mid.pack(side="left", fill="both", expand=True)
        self.title = ttk.Label(mid, text="", font=("Segoe UI", 16, "bold"))
        self.title.pack(anchor="w")
        self.detail = ttk.Label(mid, text="")
        self.detail.pack(anchor="w")
        self.canvas = tk.Canvas(mid, height=170, bg="#101418", highlightthickness=0)
        self.canvas.pack(fill="x", pady=8)
        btns = ttk.Frame(mid)
        btns.pack(anchor="w")
        for text, cmd in (("Play (Space)", self.play), ("Stop", self.stop), ("< Prev", lambda: self.step(-1)), ("Next >", lambda: self.step(1)), ("Unfile (Backspace)", self.unfile)):
            ttk.Button(btns, text=text, command=cmd, takefocus=0).pack(side="left", padx=2)
        ttk.Label(mid, text="Note for this clip:").pack(anchor="w", pady=(12, 0))
        self.note = tk.StringVar()
        self.note_entry = ttk.Entry(mid, textvariable=self.note)
        self.note_entry.pack(fill="x")
        self.note_entry.bind("<FocusOut>", lambda e: self.save_note())
        self.note_entry.bind("<Return>", lambda e: (self.save_note(), self.root.focus_set()))
        self.status = ttk.Label(mid, text="", foreground="#555")
        self.status.pack(anchor="w", pady=(10, 0))
        self.cue_info = ttk.LabelFrame(mid, text="What Mario uses this cue for", padding=6)
        self.cue_info.pack(fill="x", pady=(10, 0))
        self.cue_text = ttk.Label(self.cue_info, text="Press a bucket's M button or pick a Mario cue.", wraplength=420, justify="left")
        self.cue_text.pack(anchor="w")

        right = ttk.Frame(body, padding=(6, 0))
        right.pack(side="left", fill="y")
        ttk.Label(right, text="Buckets", font=("Segoe UI", 12, "bold")).pack(anchor="w")
        self.bucket_frame = ttk.Frame(right)
        self.bucket_frame.pack(fill="y")
        pick = ttk.LabelFrame(right, text="Any Mario cue", padding=4)
        pick.pack(fill="x", pady=(8, 0))
        self.mario_pick = tk.StringVar()
        cues = sorted(self.mario, key=natural_key)
        box = ttk.Combobox(pick, textvariable=self.mario_pick, values=cues, state="readonly", width=30)
        box.pack(side="left")
        box.bind("<<ComboboxSelected>>", lambda e: self.play_picked())
        ttk.Button(pick, text="▶", width=3, command=self.play_picked, takefocus=0).pack(side="left", padx=2)
        ttk.Button(right, text="+ New bucket", command=self.new_bucket, takefocus=0).pack(fill="x", pady=(8, 0))
        ttk.Button(right, text="Rename selected-clip bucket", command=self.rename_bucket, takefocus=0).pack(fill="x", pady=2)
        ttk.Button(right, text="Write summary.txt", command=self.write_summary, takefocus=0).pack(fill="x", pady=2)

        root.bind("<Key>", self.on_key)
        root.bind("<Left>", lambda e: self.step(-1))
        root.bind("<Right>", lambda e: self.step(1))
        root.protocol("WM_DELETE_WINDOW", self.close)
        self.build_buckets()
        self.refresh()

    # ---- buckets
    def build_buckets(self):
        for w in self.bucket_frame.winfo_children():
            w.destroy()
        self.bucket_buttons = {}
        for b in self.state.buckets:
            row = ttk.Frame(self.bucket_frame)
            row.pack(fill="x", pady=1)
            cues = self.mario_cues(b["id"])
            ref = ttk.Button(row, text="▶ M" if cues else "  -  ", width=5, takefocus=0,
                             command=lambda bid=b["id"]: self.play_mario(bid))
            if not cues:
                ref.state(["disabled"])
            ref.pack(side="left", padx=(0, 2))
            btn = ttk.Button(row, command=lambda bid=b["id"]: self.file_to(bid), takefocus=0, width=34)
            btn.pack(side="left", fill="x", expand=True)
            self.bucket_buttons[b["id"]] = btn
        self.update_bucket_labels()
        self.show_box["values"] = ["All clips", "Unsorted"] + [b["label"] for b in self.state.buckets]

    def update_bucket_labels(self):
        for b in self.state.buckets:
            key = b["key"].upper() if b["key"] else "-"
            self.bucket_buttons[b["id"]].configure(text="[%s]  %s   (%d)" % (key, b["label"], self.state.count(b["id"])))

    def new_bucket(self):
        label = simpledialog.askstring("New bucket", "Name for the new bucket:", parent=self.root)
        if label and label.strip():
            self.state.add_bucket(label.strip())
            self.build_buckets()

    def rename_bucket(self):
        if not self.current or self.current not in self.state.assign:
            messagebox.showinfo("Rename", "Select a clip that is filed in the bucket to rename.", parent=self.root)
            return
        bid = self.state.assign[self.current]
        label = simpledialog.askstring("Rename bucket", "New name:", initialvalue=self.state.label(bid), parent=self.root)
        if label and label.strip():
            next(b for b in self.state.buckets if b["id"] == bid)["label"] = label.strip()
            self.state.save()
            self.build_buckets()
            self.refresh()

    # ---- list
    def refresh(self):
        pick = self.show.get()
        names = [c.name for c in self.clips]
        if pick == "Unsorted":
            names = [n for n in names if n not in self.state.assign]
        elif pick != "All clips":
            ids = {b["id"] for b in self.state.buckets if b["label"] == pick}
            names = [n for n in names if self.state.assign.get(n) in ids]
        if self.order.get() == "disc wave":
            names.sort(key=lambda n: natural_key(self.by_name[n].disc or "z"))
        else:
            names.sort(key=natural_key)
        self.visible = names
        self.tree.delete(*self.tree.get_children())
        for n in names:
            self.tree.insert("", "end", iid=n, values=self.row(n))
        total, done = len(self.clips), len(self.state.assign)
        self.progress.configure(text="%d / %d filed, %d left" % (done, total, total - done))
        self.update_bucket_labels()
        if names:
            target = self.current if self.current in names else names[0]
            self.tree.selection_set(target)
            self.tree.see(target)

    def row(self, n):
        c = self.by_name[n]
        return (n, "%.2f" % c.seconds, c.disc or "?", self.state.label(self.state.assign.get(n, "")))

    def on_select(self, _):
        sel = self.tree.selection()
        if not sel:
            return
        self.save_note()
        self.current = sel[0]
        c = self.by_name[self.current]
        self.title.configure(text=c.name)
        bid = self.state.assign.get(self.current)
        self.detail.configure(text="%.2f s   disc wave %s   filed: %s" % (c.seconds, c.disc or "unknown", self.state.label(bid) if bid else "not yet"))
        self.note.set(self.state.notes.get(self.current, ""))
        self.draw(c)
        if self.autoplay.get():
            self.play()

    def draw(self, c):
        self.canvas.delete("all")
        w = max(100, self.canvas.winfo_width())
        h = int(self.canvas["height"])
        mid = h // 2
        self.canvas.create_line(0, mid, w, mid, fill="#2a323c")
        for x, (lo, hi) in enumerate(c.peaks(w)):
            self.canvas.create_line(x, mid - hi * (mid - 6), x, mid - lo * (mid - 6) + 1, fill="#6fd0a8")

    def step(self, d):
        if not self.visible or self.current not in self.visible:
            return
        i = max(0, min(len(self.visible) - 1, self.visible.index(self.current) + d))
        self.tree.selection_set(self.visible[i])
        self.tree.see(self.visible[i])

    # ---- filing
    def file_to(self, bid):
        if not self.current:
            return
        n = self.current
        old = self.visible.index(n) if n in self.visible else 0
        self.state.assign[n] = bid
        self.save_note(save=False)
        self.state.save()
        self.status.configure(text="%s -> %s" % (n, self.state.label(bid)))
        pick = self.show.get()
        if pick == "Unsorted":
            # The clip leaves this list: stay at the same position (the next clip slides in).
            self.refresh()
            if self.visible:
                self.tree.selection_set(self.visible[min(old, len(self.visible) - 1)])
        else:
            self.tree.item(n, values=self.row(n))
            self.progress.configure(text="%d / %d filed, %d left" % (len(self.state.assign), len(self.clips), len(self.clips) - len(self.state.assign)))
            self.update_bucket_labels()
            if self.advance.get():
                self.step(1)

    def unfile(self):
        if self.current and self.current in self.state.assign:
            del self.state.assign[self.current]
            self.state.save()
            self.tree.item(self.current, values=self.row(self.current)) if self.current in self.visible else None
            self.update_bucket_labels()
            self.status.configure(text="%s unfiled" % self.current)

    def save_note(self, save=True):
        if self.current is None:
            return
        text = self.note.get().strip()
        if text:
            self.state.notes[self.current] = text
        else:
            self.state.notes.pop(self.current, None)
        if save:
            self.state.save()

    def on_key(self, e):
        if isinstance(self.root.focus_get(), (tk.Entry, ttk.Entry)):
            return
        if e.keysym == "BackSpace":
            return self.unfile()
        if e.keysym == "space":
            return self.play()
        if (e.char or "").lower() == "x":
            return self.repeat_mario()
        if (e.char or "").lower() == "m" and self.current:
            # m: Mario's reference for the bucket this clip is filed in.
            bid = self.state.assign.get(self.current)
            return self.play_mario(bid) if bid else None
        ch = (e.char or "").lower()
        for b in self.state.buckets:
            if b["key"] and b["key"] == ch:
                return self.file_to(b["id"])

    # ---- sound
    def play(self):
        if self.current and winsound:
            winsound.PlaySound(self.by_name[self.current].path, winsound.SND_FILENAME | winsound.SND_ASYNC)
        elif not winsound:
            self.status.configure(text="No sound player on this system (winsound is Windows-only).")

    def mario_cues(self, bid):
        """Mario's reference cues for a bucket, in cue order (every variant of each prefix)."""
        out = []
        for prefix in MARIO_REF.get(bid, []):
            out += sorted((n for n in self.mario if n == prefix or n.startswith(prefix + "_")), key=natural_key)
        return out

    def play_mario(self, bid):
        """Plays the bucket's next Mario variant, one per press, so every one gets heard."""
        cues = self.mario_cues(bid)
        if not cues or not winsound:
            return
        i = self.mario_next.get(bid, 0) % len(cues)
        self.mario_next[bid] = i + 1
        self.last_ref = bid
        winsound.PlaySound(self.mario[cues[i]], winsound.SND_FILENAME | winsound.SND_ASYNC)
        self.status.configure(text="Mario: %s  (%d of %d for %s)" % (cues[i], i + 1, len(cues), self.state.label(bid)))
        self.show_cue(cues[i])

    def show_cue(self, cue):
        title, use = mario_voice_info.describe(cue)
        self.cue_info.configure(text="%s  -  %s" % (cue, title) if title else cue)
        self.cue_text.configure(text=use or "No note for this cue yet.")

    def play_picked(self):
        cue = self.mario_pick.get()
        if cue in self.mario and winsound:
            self.last_ref = None
            winsound.PlaySound(self.mario[cue], winsound.SND_FILENAME | winsound.SND_ASYNC)
            self.status.configure(text="Mario: %s" % cue)
            self.show_cue(cue)

    def repeat_mario(self):
        """x: Mario's next reference for the last bucket you played one from."""
        if self.last_ref is None:
            self.play_picked()
        else:
            self.play_mario(self.last_ref)

    def stop(self):
        if winsound:
            winsound.PlaySound(None, winsound.SND_PURGE)

    def write_summary(self):
        out = os.path.splitext(self.state.path)[0] + "-summary.txt"
        lines = []
        for b in self.state.buckets:
            names = sorted((n for n, v in self.state.assign.items() if v == b["id"]), key=natural_key)
            lines.append("%s  (%d)" % (b["label"], len(names)))
            for n in names:
                note = self.state.notes.get(n, "")
                lines.append("    %-10s %-8s %s" % (n, self.by_name[n].disc or "?", note))
        left = [c.name for c in self.clips if c.name not in self.state.assign]
        lines.append("Unsorted (%d): %s" % (len(left), " ".join(left)))
        open(out, "w", encoding="utf-8").write("\n".join(lines) + "\n")
        self.status.configure(text="wrote %s" % out)

    def close(self):
        self.save_note()
        self.stop()
        self.root.destroy()


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--clips", help="folder of Luigi WAVs (default: found under 'iso backups')")
    ap.add_argument("--table", default=os.path.join(ROOT, "output", "lm-rip-match.txt"), help="rip_match table")
    ap.add_argument("--mario", default=os.path.join(ROOT, "output", "mario-refs"),
                    help="folder of Mario's cue WAVs, named by cue (MV21_JUMP_SMALL_01.wav)")
    ap.add_argument("--state", default=os.path.join(ROOT, "output", "luigi-voice-buckets.json"))
    ap.add_argument("--selftest", action="store_true", help="build the window, file a clip, save, quit")
    a = ap.parse_args()
    clips_dir = a.clips or find_clips_dir()
    if not clips_dir or not os.path.isdir(clips_dir):
        sys.exit("No Luigi WAV folder found; pass --clips DIR")
    disc = read_table(a.table)
    paths = sorted(glob.glob(os.path.join(clips_dir, "*.wav")), key=lambda p: natural_key(os.path.basename(p)))
    clips = [Clip(os.path.splitext(os.path.basename(p))[0], p, disc.get(os.path.splitext(os.path.basename(p))[0])) for p in paths]
    if not clips:
        sys.exit("No .wav files in %s" % clips_dir)
    os.makedirs(os.path.dirname(a.state), exist_ok=True)
    state = State(a.state)
    root = tk.Tk()
    mario = read_mario_refs(a.mario)
    app = App(root, clips, state, mario)
    if a.selftest:
        root.update()
        first = clips[0].name
        app.tree.selection_set(first)
        root.update()
        app.autoplay.set(False)
        app.file_to(state.buckets[0]["id"])
        app.write_summary()
        root.update()
        check = json.load(open(a.state, encoding="utf-8"))
        assert check["assign"].get(first) == state.buckets[0]["id"], check
        refs = {b["id"]: len(app.mario_cues(b["id"])) for b in state.buckets if app.mario_cues(b["id"])}
        print("selftest ok: %d clips, filed %s -> %s, state %s" % (len(clips), first, state.buckets[0]["id"], a.state))
        print("Mario references: %d clips, per bucket %s" % (len(mario), refs))
        root.destroy()
        return
    print("%d clips from %s; state %s" % (len(clips), clips_dir, a.state))
    root.mainloop()


if __name__ == "__main__":
    main()
