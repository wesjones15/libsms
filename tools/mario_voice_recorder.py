#!/usr/bin/env python
"""Record your own Mario voice (and footsteps and other sounds) to make a no-asset sound pack.

Lists every Mario voice cue the game has, with what it is used for (mario_voice_info.py), plus
the non-voice sounds a pack needs (footsteps, body impacts, ...). Pick one, press Record, make
the sound, press Stop; it plays back, and you keep it or try again. Several takes per cue are
fine (the game picks voice variants at random); the numbered clips in a cue's family (_01 _02 ...)
each get their own slot.

    python libsms/tools/mario_voice_recorder.py [--out DIR] [--mario DIR]

Writes `output/mario-voice-pack/<CUE>.wav` (16-bit mono 44.1 kHz) and `pack.json` listing the cues
you recorded, each saved at once. `--mario` points at the folder of Mario's own cue WAVs (to hear
the original beside your take, and to learn the cue list from); without it the built-in list is used.
The pack is your own voice: nothing from the disc goes into it. Windows only (recording uses the
system's winmm MCI); standard library only.

Keys: R record / stop, Space play your take, O play Mario's original, Delete remove the take,
Right/Down next cue, Left/Up previous, N next cue that has no take.
"""
import argparse
import ctypes
import glob
import json
import os
import re
import sys
import tempfile
import tkinter as tk
import wave
from tkinter import messagebox, ttk

try:
    import winsound
except ImportError:
    winsound = None

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import mario_voice_info  # noqa: E402

ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))


def natural_key(name):
    return [int(p) if p.isdigit() else p for p in re.split(r"(\d+)", name)]


def builtin_cues():
    """Voice cue family names (prefixes) with how many numbered variants the game has, when the
    disc's references are not at hand: one slot per family, which is enough for a first pack."""
    return [c[0] for c in mario_voice_info.CUES]


class Recorder:
    """Records from the default microphone with winmm MCI (no extra packages)."""

    def __init__(self):
        self.mci = getattr(getattr(ctypes, "windll", None), "winmm", None)
        self.live = False

    def _send(self, cmd):
        buf = ctypes.create_unicode_buffer(256)
        err = self.mci.mciSendStringW(cmd, buf, 255, 0)
        if err:
            msg = ctypes.create_unicode_buffer(256)
            self.mci.mciGetErrorStringW(err, msg, 255)
            raise RuntimeError("%s -> %s" % (cmd, msg.value))
        return buf.value

    def available(self):
        return self.mci is not None

    def start(self):
        self._send("close mvrec") if self.live else None
        self._send("open new type waveaudio alias mvrec")
        self.live = True
        self._send("set mvrec time format ms bitspersample 16 channels 1 samplespersec 44100 alignment 2 bytespersec 88200")
        self._send("record mvrec")

    def stop(self, path):
        """Stops and saves to `path`; returns the clip's length in seconds."""
        self._send("stop mvrec")
        self._send('save mvrec "%s" wait' % path)
        self._send("close mvrec")
        self.live = False
        return clip_seconds(path)

    def abort(self):
        if self.live:
            try:
                self._send("close mvrec")
            except RuntimeError:
                pass
            self.live = False


def clip_seconds(path):
    try:
        with wave.open(path) as w:
            return w.getnframes() / float(w.getframerate())
    except (wave.Error, EOFError, OSError):
        return 0.0


def trim_silence(path, floor=600, pad=0.04):
    """Cuts leading/trailing near-silence from a 16-bit mono take, in place; keeps `pad` s."""
    import array

    with wave.open(path) as w:
        params, data = w.getparams(), w.readframes(w.getnframes())
    if params.sampwidth != 2:
        return
    a = array.array("h")
    a.frombytes(data)
    n = len(a)
    first = next((i for i in range(n) if abs(a[i]) > floor), None)
    if first is None:
        return
    last = next(i for i in range(n - 1, -1, -1) if abs(a[i]) > floor)
    keep = int(pad * params.framerate)
    lo, hi = max(0, first - keep), min(n, last + keep + 1)
    with wave.open(path, "wb") as w:
        w.setparams(params._replace(nframes=hi - lo))
        w.writeframes(a[lo:hi].tobytes())


def read_refs(folder):
    refs = {}
    if folder and os.path.isdir(folder):
        for p in glob.glob(os.path.join(folder, "*.wav")):
            refs[os.path.splitext(os.path.basename(p))[0]] = p
    return refs


class App:
    def __init__(self, root, slots, refs, out):
        self.root, self.refs, self.out = root, refs, out
        os.makedirs(out, exist_ok=True)
        self.slots = slots  # [(name, kind)] kind: "voice" | "sfx"
        self.rec = Recorder()
        self.recording = False
        self.current = None
        root.title("Mario voice recorder")
        root.geometry("1100x640")

        top = ttk.Frame(root, padding=6)
        top.pack(fill="x")
        ttk.Label(top, text="Show:").pack(side="left")
        self.show = tk.StringVar(value="All")
        box = ttk.Combobox(top, textvariable=self.show, state="readonly", width=22, values=["All", "Voices", "Footsteps and effects", "Not recorded yet", "Recorded"])
        box.pack(side="left", padx=4)
        box.bind("<<ComboboxSelected>>", lambda e: self.refresh())
        self.progress = ttk.Label(top, text="")
        self.progress.pack(side="right")

        body = ttk.Frame(root, padding=(6, 0))
        body.pack(fill="both", expand=True)
        left = ttk.Frame(body)
        left.pack(side="left", fill="both", expand=True)
        cols = ("cue", "kind", "take")
        self.tree = ttk.Treeview(left, columns=cols, show="headings", selectmode="browse")
        for c, w in zip(cols, (230, 70, 70)):
            self.tree.heading(c, text=c)
            self.tree.column(c, width=w, anchor="w")
        sb = ttk.Scrollbar(left, command=self.tree.yview)
        self.tree.configure(yscrollcommand=sb.set)
        self.tree.pack(side="left", fill="both", expand=True)
        sb.pack(side="left", fill="y")
        self.tree.bind("<<TreeviewSelect>>", self.on_select)

        mid = ttk.Frame(body, padding=(12, 0))
        mid.pack(side="left", fill="both", expand=True)
        self.title = ttk.Label(mid, text="", font=("Segoe UI", 16, "bold"))
        self.title.pack(anchor="w")
        self.what = ttk.Label(mid, text="", wraplength=470, justify="left")
        self.what.pack(anchor="w", pady=(4, 8))
        self.canvas = tk.Canvas(mid, height=120, bg="#101418", highlightthickness=0)
        self.canvas.pack(fill="x", pady=4)
        self.status = ttk.Label(mid, text="", foreground="#555", wraplength=470, justify="left")
        self.status.pack(anchor="w", pady=4)
        btns = ttk.Frame(mid)
        btns.pack(anchor="w", pady=6)
        self.rec_btn = ttk.Button(btns, text="Record (R)", command=self.toggle, takefocus=0)
        self.rec_btn.pack(side="left", padx=2)
        for text, cmd in (("Play mine (Space)", self.play_mine), ("Play Mario's (O)", self.play_orig), ("Delete take", self.delete), ("Next unrecorded (N)", self.next_missing)):
            ttk.Button(btns, text=text, command=cmd, takefocus=0).pack(side="left", padx=2)
        self.trim = tk.BooleanVar(value=True)
        ttk.Checkbutton(mid, text="Trim silence from the ends after recording", variable=self.trim, takefocus=0).pack(anchor="w")
        if not self.rec.available():
            self.status.configure(text="Recording needs Windows (winmm); you can still browse the cues.")

        root.bind("<Key>", self.on_key)
        root.bind("<Down>", lambda e: self.step(1))
        root.bind("<Up>", lambda e: self.step(-1))
        root.bind("<Right>", lambda e: self.step(1))
        root.bind("<Left>", lambda e: self.step(-1))
        root.protocol("WM_DELETE_WINDOW", self.close)
        self.refresh()

    # ---- files
    def path(self, name):
        return os.path.join(self.out, name + ".wav")

    def has(self, name):
        return os.path.isfile(self.path(name))

    def write_pack(self):
        names = [n for n, _ in self.slots if self.has(n)]
        data = {"version": 1, "voice": [n for n in names if kind_of(n, self.slots) == "voice"],
                "sfx": [n for n in names if kind_of(n, self.slots) == "sfx"]}
        fd, tmp = tempfile.mkstemp(dir=self.out, suffix=".tmp")
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(data, f, indent=1)
        os.replace(tmp, os.path.join(self.out, "pack.json"))

    # ---- list
    def visible(self):
        pick = self.show.get()
        out = []
        for n, k in self.slots:
            if pick == "Voices" and k != "voice":
                continue
            if pick == "Footsteps and effects" and k != "sfx":
                continue
            if pick == "Not recorded yet" and self.has(n):
                continue
            if pick == "Recorded" and not self.has(n):
                continue
            out.append(n)
        return out

    def refresh(self):
        keep = self.current
        names = self.visible()
        self.tree.delete(*self.tree.get_children())
        for n in names:
            self.tree.insert("", "end", iid=n, values=self.row(n))
        done = sum(1 for n, _ in self.slots if self.has(n))
        self.progress.configure(text="%d / %d recorded" % (done, len(self.slots)))
        if names:
            target = keep if keep in names else names[0]
            self.tree.selection_set(target)
            self.tree.see(target)

    def row(self, n):
        k = kind_of(n, self.slots)
        take = "%.2f s" % clip_seconds(self.path(n)) if self.has(n) else ""
        return (n, "voice" if k == "voice" else "effect", take)

    def on_select(self, _):
        sel = self.tree.selection()
        if not sel or self.recording:
            return
        self.current = sel[0]
        title, use = mario_voice_info.describe(self.current)
        self.title.configure(text=self.current)
        self.what.configure(text=("%s\n\n%s" % (title, use)) if title else "No note for this cue.")
        self.draw()

    def draw(self):
        c = self.canvas
        c.delete("all")
        w = max(100, c.winfo_width())
        h = int(c["height"])
        mid = h // 2
        c.create_line(0, mid, w, mid, fill="#2a323c")
        if not self.current or not self.has(self.current):
            return
        try:
            import array

            with wave.open(self.path(self.current)) as f:
                data = f.readframes(f.getnframes())
            a = array.array("h")
            a.frombytes(data)
        except (wave.Error, EOFError, OSError):
            return
        n = len(a)
        for x in range(w):
            seg = a[n * x // w : max(n * x // w + 1, n * (x + 1) // w)]
            if seg:
                c.create_line(x, mid - max(seg) / 32768.0 * (mid - 4), x, mid - min(seg) / 32768.0 * (mid - 4) + 1, fill="#e0b060")

    def step(self, d):
        names = self.visible()
        if not names or self.current not in names or self.recording:
            return
        i = max(0, min(len(names) - 1, names.index(self.current) + d))
        self.tree.selection_set(names[i])
        self.tree.see(names[i])

    def next_missing(self):
        if self.recording:
            return
        names = [n for n, _ in self.slots]
        start = names.index(self.current) + 1 if self.current in names else 0
        for n in names[start:] + names[:start]:
            if not self.has(n):
                if n not in self.visible():
                    self.show.set("All")
                    self.refresh()
                self.tree.selection_set(n)
                self.tree.see(n)
                return
        self.status.configure(text="Every cue has a take.")

    # ---- recording
    def toggle(self):
        if not self.current:
            return
        if not self.recording:
            if not self.rec.available():
                return
            self.stop_playback()
            try:
                self.rec.start()
            except RuntimeError as e:
                self.status.configure(text="Could not start recording: %s" % e)
                return
            self.recording = True
            self.rec_btn.configure(text="Stop (R)")
            self.status.configure(text="Recording %s ... press R to stop." % self.current)
            return
        self.recording = False
        self.rec_btn.configure(text="Record (R)")
        tmp = self.path(self.current) + ".new.wav"
        try:
            secs = self.rec.stop(tmp)
        except RuntimeError as e:
            self.status.configure(text="Could not save the take: %s" % e)
            return
        if self.trim.get():
            trim_silence(tmp)
        if clip_seconds(tmp) < 0.05:
            os.remove(tmp)
            self.status.configure(text="Heard nothing: check the microphone and try again.")
            return
        os.replace(tmp, self.path(self.current))
        self.write_pack()
        self.tree.item(self.current, values=self.row(self.current))
        self.draw()
        self.progress.configure(text="%d / %d recorded" % (sum(1 for n, _ in self.slots if self.has(n)), len(self.slots)))
        self.status.configure(text="Saved %s (%.2f s). Space plays it back; R records another take over it." % (self.current, clip_seconds(self.path(self.current))))
        self.play_mine()

    def play_mine(self):
        if self.current and self.has(self.current) and winsound and not self.recording:
            winsound.PlaySound(self.path(self.current), winsound.SND_FILENAME | winsound.SND_ASYNC)

    def play_orig(self):
        ref = self.refs.get(self.current)
        if ref and winsound and not self.recording:
            winsound.PlaySound(ref, winsound.SND_FILENAME | winsound.SND_ASYNC)
        elif not ref:
            self.status.configure(text="No original WAV for this cue (point --mario at the folder of Mario's cues).")

    def stop_playback(self):
        if winsound:
            winsound.PlaySound(None, winsound.SND_PURGE)

    def delete(self):
        if self.current and self.has(self.current) and messagebox.askyesno("Delete take", "Delete your take of %s?" % self.current, parent=self.root):
            os.remove(self.path(self.current))
            self.write_pack()
            self.refresh()
            self.draw()

    def on_key(self, e):
        ch = (e.char or "").lower()
        if e.keysym == "space":
            return self.play_mine()
        if e.keysym == "Delete":
            return self.delete()
        if ch == "r":
            return self.toggle()
        if ch == "o":
            return self.play_orig()
        if ch == "n":
            return self.next_missing()

    def close(self):
        self.rec.abort()
        self.stop_playback()
        self.root.destroy()


def kind_of(name, slots):
    return next((k for n, k in slots if n == name), "voice")


def build_slots(refs):
    """One slot per Mario voice clip on the disc reference folder (each numbered variant), or per
    cue family without it; then one per non-voice sound the animations key."""
    voices = sorted(refs, key=natural_key) if refs else builtin_cues()
    return [(n, "voice") for n in voices] + [(n, "sfx") for n, _, _ in mario_voice_info.SFX]


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", default=os.path.join(ROOT, "output", "mario-voice-pack"))
    ap.add_argument("--mario", default=os.path.join(ROOT, "output", "mario-refs"), help="folder of Mario's cue WAVs")
    ap.add_argument("--selftest", action="store_true", help="build the window, check the lists, quit")
    a = ap.parse_args()
    refs = read_refs(a.mario)
    slots = build_slots(refs)
    root = tk.Tk()
    app = App(root, slots, refs, a.out)
    if a.selftest:
        root.update()
        silent = [n for n, k in slots if k == "voice" and not mario_voice_info.describe(n)[0]]
        assert not silent, silent
        print("selftest ok: %d slots (%d voice, %d effects), %d references, recording %s" % (
            len(slots), sum(1 for _, k in slots if k == "voice"), sum(1 for _, k in slots if k == "sfx"), len(refs),
            "available" if app.rec.available() else "unavailable"))
        root.destroy()
        return
    root.mainloop()


if __name__ == "__main__":
    main()
