//! DSP voices: allocation and priority stealing (`TDSPChannel`, JASDSPChannel.cpp), the voice
//! parameters (`DSPInterface::DSPBuffer`, JASDSPInterface.cpp) and the mixer the DSP microcode
//! runs, which isn't in the decomp.
//!
//! Sunshine's microcode is the "Zelda" family: each voice mixes into up to six DSP buffers named
//! by id (`setBusConnect`). With the default bus setup a voice feeds 0x0D00/0x0D60 (front left
//! and right) and 0x0DC0/0x0E20 (sends to echo lines 0 and 1). The four echo lines
//! (`FXBuffer`, configured from the init data's FX scene) delay their send by
//! `frames * 80` samples, run it through an 8-tap FIR and add it to two destination buffers
//! at `volume / 32768`. Voices resample by `pitch / 4096` with linear interpolation and ramp
//! their mixer volumes across each subframe; the sum is scaled by the output level
//! (`DsetMixerLevel(outputGainUp = 5.0)`, 4.12 fixed point).
//!
//! The microcode's per-voice scaling isn't known. Taking mixer volumes as 1.15 fixed point
//! puts sequenced music about 8 dB above the title stream, which the game plays at unity
//! (`mixInterleaveTrack`), so the voice sum is halved (1.16) to match.

use crate::calc::SUBFRAME;
use crate::engine::*;

/// `DsetMixerLevel(5.0)`, with voices at 1.16 fixed point (see the module notes).
const OUTPUT_LEVEL: f32 = 5.0 * 0.5;

/// DSP buffer ids the stereo output and the echo lines use, in `SEND_TABLE` order.
const BUFFERS: [u16; 12] = [
    0x0D00, 0x0D60, 0x0DC0, 0x0E20, 0x0E80, 0x0EE0, 0x0CA0, 0x0F40, 0x0FA0, 0x0B00, 0x09A0, 0x0000,
];

fn buffer_index(id: u16) -> Option<usize> {
    BUFFERS.iter().position(|&b| b == id && id != 0)
}

impl Engine {
    /// `TDSPChannel::alloc`: the first free voice.
    pub(crate) fn alloc_voice(&mut self, c: usize) -> Option<usize> {
        let v = self.voices.iter().position(|v| v.status == 1)?;
        // `allocate` + `DSPBuffer::allocInit`
        let vo = &mut self.voices[v];
        *vo = Voice {
            status: 0,
            sign: Some(c),
            priority: 1,
            ..Voice::default()
        };
        Some(v)
    }

    /// `TDSPChannel::free`
    pub(crate) fn free_voice(&mut self, v: usize, c: usize) {
        let vo = &mut self.voices[v];
        if vo.sign != Some(c) {
            return;
        }
        vo.status = 1;
        vo.priority = 0;
        vo.callback = false;
        vo.sign = None;
    }

    /// `TDSPChannel::forceStop`: the DSP fades the voice out over the next subframe.
    pub(crate) fn force_stop_voice(&mut self, v: usize) -> bool {
        let vo = &mut self.voices[v];
        if vo.status != 0 || !vo.enabled {
            return false;
        }
        vo.end_requested = true;
        vo.status = 2;
        true
    }

    /// `getLower`: a free voice, else the lowest-priority one with a callback (oldest on ties).
    fn lower_voice(&self) -> usize {
        let (mut best, mut idx, mut age) = (0xFFu8, 0usize, 0u32);
        for (i, v) in self.voices.iter().enumerate() {
            if v.status == 2 {
                continue;
            }
            if v.status == 1 {
                return i;
            }
            if v.callback && v.priority <= best && (v.priority != best || v.age >= age) {
                age = v.age;
                idx = i;
                best = v.priority;
            }
        }
        idx
    }

    /// `breakLower`: stops a voice of at most `prio` so a waiting note can have it.
    pub(crate) fn break_lower(&mut self, prio: u8) -> bool {
        let v = self.lower_voice();
        if self.voices[v].priority > prio || self.voices[v].status == 1 {
            return false;
        }
        if self.voices[v].callback {
            self.voices[v].cb_interval = self.update_dsp_channel(v, 3);
        }
        self.force_stop_voice(v);
        true
    }

    /// `DSPBuffer::setWaveInfo` / `setOscInfo`.
    pub(crate) fn voice_set_source(&mut self, v: usize, s: &Source) {
        let vo = &mut self.voices[v];
        vo.source = s.clone();
        vo.pcm = None;
        if let Source::Wave(key) = s
            && let Some(w) = self.data.wave(*key)
        {
            vo.looping = w.looping;
            vo.loop_start = w.loop_start;
            vo.end = w.end;
            vo.pcm = self.data.pcm(*key).ok();
        }
    }

    /// `TDSPChannel::updateAll`: once per subframe, before mixing.
    pub fn update_voices(&mut self) {
        for v in 0..VOICES {
            if self.voices[v].status == 1 {
                continue;
            }
            if self.voices[v].done {
                if self.voices[v].callback {
                    self.voices[v].cb_interval = self.update_dsp_channel(v, 2);
                }
                // `replyFinishRequest`
                self.voices[v].done = false;
                self.voices[v].enabled = false;
            }
            if !self.voices[v].end_requested {
                self.voices[v].age += 1;
                if self.voices[v].age == self.voices[v].priority_time as u32
                    && self.voices[v].callback
                {
                    self.update_dsp_channel(v, 4);
                }
            }
            if self.voices[v].callback {
                let vo = &mut self.voices[v];
                vo.cb_interval = vo.cb_interval.wrapping_sub(1);
                if vo.cb_interval == 0 {
                    self.voices[v].cb_interval = self.update_dsp_channel(v, 0);
                    if self.voices[v].cb_interval == 0 {
                        self.voices[v].done = false;
                        self.voices[v].enabled = false;
                        self.dequeue(true);
                    }
                }
            }
        }
        self.check_queue();
    }

    /// Configures echo line `i` (`FXBuffer::setFXLine`).
    pub fn set_fx_line(
        &mut self,
        i: usize,
        enabled: bool,
        frames: usize,
        out: [(u16, i16); 2],
        fir: [i16; 8],
    ) {
        let f = &mut self.fx[i];
        f.enabled = enabled && frames > 0;
        f.delay = vec![0; frames * SUBFRAME];
        f.pos = 0;
        f.out = out;
        f.fir = fir;
        f.history = [0; 8];
    }

    /// Mixes one subframe into `out` (interleaved stereo, 80 frames, -1..1).
    pub fn render_subframe(&mut self, out: &mut [f32; SUBFRAME * 2]) {
        let mut bufs = [[0f32; SUBFRAME]; BUFFERS.len()];
        for vo in self.voices.iter_mut() {
            if !vo.enabled || vo.done {
                continue;
            }
            let fade_out = vo.end_requested;
            let step = vo.pitch as f64 / 4096.0;
            let mut samples = [0f32; SUBFRAME];
            let mut finished = false;
            if !vo.pause {
                for s in samples.iter_mut() {
                    *s = match (&vo.source, &vo.pcm) {
                        (Source::Wave(_), Some(pcm)) => {
                            let end = (vo.end as f64).min(pcm.len() as f64);
                            if vo.pos >= end {
                                if vo.looping && (vo.loop_start as f64) < end {
                                    let span = end - vo.loop_start as f64;
                                    vo.pos = vo.loop_start as f64 + (vo.pos - end) % span;
                                } else {
                                    finished = true;
                                    break;
                                }
                            }
                            let i = vo.pos as usize;
                            let f = (vo.pos - i as f64) as f32;
                            let a = pcm[i] as f32;
                            let next = if i + 1 < end as usize {
                                pcm[i + 1]
                            } else if vo.looping {
                                pcm.get(vo.loop_start as usize).copied().unwrap_or(0)
                            } else {
                                0
                            } as f32;
                            vo.pos += step;
                            a + (next - a) * f
                        }
                        (Source::Osc(kind), _) => {
                            // 32 steps per cycle at the base pitch (`noteOnOsc`).
                            let ph = vo.osc_phase.fract() as f32;
                            vo.osc_phase += step / 32.0;
                            let s = match kind {
                                0 => (ph * std::f32::consts::TAU).sin(),
                                1 => {
                                    if ph < 0.5 {
                                        1.0
                                    } else {
                                        -1.0
                                    }
                                }
                                2 => 2.0 * ph - 1.0,
                                _ => 1.0 - 4.0 * (ph - 0.5).abs(),
                            };
                            s * 16384.0
                        }
                        _ => 0.0,
                    };
                }
            }
            for m in vo.mix.iter_mut() {
                let target = if fade_out { 0 } else { m.target };
                if let Some(b) = buffer_index(m.bus) {
                    let (from, to) = (m.current as f32, target as f32);
                    for (n, s) in samples.iter().enumerate() {
                        let k = n as f32 / SUBFRAME as f32;
                        bufs[b][n] += s * (from + (to - from) * k) / 32768.0;
                    }
                }
                m.current = target;
            }
            if finished || fade_out {
                vo.done = true;
            }
        }
        // Echo lines: the delayed send, filtered, into their destinations; then this frame's
        // send goes into the delay.
        for i in 0..self.fx.len() {
            if !self.fx[i].enabled {
                continue;
            }
            let src = 2 + i;
            let f = &mut self.fx[i];
            let n = f.delay.len();
            let mut window = [0f32; SUBFRAME + 8];
            for (k, w) in window.iter_mut().enumerate().take(8) {
                *w = f.history[k] as f32;
            }
            for k in 0..SUBFRAME {
                window[8 + k] = f.delay[(f.pos + k) % n] as f32;
            }
            for k in 0..8 {
                f.history[k] = window[SUBFRAME + k] as i16;
            }
            let mut filtered = [0f32; SUBFRAME];
            for (k, y) in filtered.iter_mut().enumerate() {
                let acc: f32 = (0..8).map(|j| window[k + j] * f.fir[j] as f32).sum();
                *y = (acc / 32768.0).clamp(-32768.0, 32767.0);
            }
            let send = bufs[src];
            for (dst, vol) in f.out {
                if vol == 0 {
                    continue;
                }
                if let Some(b) = buffer_index(dst) {
                    for k in 0..SUBFRAME {
                        bufs[b][k] += filtered[k] * vol as f32 / 32768.0;
                    }
                }
            }
            let f = &mut self.fx[i];
            for (k, s) in send.iter().enumerate() {
                f.delay[(f.pos + k) % n] = s.clamp(-32768.0, 32767.0) as i16;
            }
            f.pos = (f.pos + SUBFRAME) % n;
        }
        for k in 0..SUBFRAME {
            out[k * 2] = (bufs[0][k] * OUTPUT_LEVEL / 32768.0 * self.output_gain).clamp(-1.0, 1.0);
            out[k * 2 + 1] =
                (bufs[1][k] * OUTPUT_LEVEL / 32768.0 * self.output_gain).clamp(-1.0, 1.0);
        }
    }

    /// One subframe of the whole system: sequences, channel updates, then the mix
    /// (`DSPBuf::updateDSP` then the DSP's frame).
    pub fn subframe(&mut self, h: &mut dyn SyncHandler, out: &mut [f32; SUBFRAME * 2]) {
        let running = self.running.clone();
        let mut keep = Vec::with_capacity(running.len());
        for t in running {
            if self.root_callback(t, h) {
                keep.push(t);
            }
        }
        // Roots started during this subframe stay registered.
        let added: Vec<usize> = self
            .running
            .iter()
            .copied()
            .filter(|t| {
                !keep.contains(t) && self.tracks[*t].state == 1 && self.roots.contains(&Some(*t))
            })
            .collect();
        keep.extend(added);
        self.running = keep;
        self.update_voices();
        self.render_subframe(out);
    }
}
