//! Logical channels: `TChannel` and its update callbacks (JASChannel.cpp), the channel lists
//! (`TChannelMgr`, JASChannelMgr.cpp; `ChGlobal`, JASChGlobal.cpp), the queue for notes waiting
//! for a DSP voice (`DSPQueue`, JASChAllocQueue.cpp) and the bank side of a note
//! (`BankMgr::noteOn` / `gateOn`, JASBankMgr.cpp; `TBasicInst` / `TDrumSet::getParam`).

use std::sync::Arc;

use libsms_assets::sound::bank::{self, Program};

use crate::calc::{DAC_RATE, c5base, sinf_dolby2, sinf_t};
use crate::engine::*;
use crate::osc::{Osc, presets};

/// `Driver::calc_sw_table`: for each calc type, how the sound, effect and channel parts
/// combine (0 none, 1 add, 2 weighted by the pan power).
const CALC_SW: [[u8; 3]; 27] = [
    [0, 0, 0],
    [0, 0, 1],
    [0, 0, 1],
    [0, 1, 0],
    [0, 1, 1],
    [0, 1, 2],
    [0, 1, 0],
    [0, 2, 1],
    [0, 2, 2],
    [1, 0, 0],
    [1, 0, 1],
    [1, 0, 2],
    [1, 1, 0],
    [1, 1, 1],
    [1, 1, 2],
    [1, 2, 0],
    [1, 2, 1],
    [1, 2, 2],
    [1, 0, 0],
    [2, 0, 1],
    [2, 0, 2],
    [2, 1, 0],
    [2, 1, 1],
    [2, 1, 2],
    [2, 2, 0],
    [2, 2, 1],
    [2, 2, 2],
];

/// `calcEffect` / `calcPan`: `center` is 0.5 for pan, 0 for the rest.
fn calc_mix(m: &PanMatrix, power: &PanMatrix, kind: u8, center: f32) -> f32 {
    let sw = CALC_SW[(kind as usize).min(26)];
    let part = |how: u8, v: f32, p: f32| match how {
        1 => v - center,
        2 => (v - center) * p,
        _ => 0.0,
    };
    part(sw[0], m.sound, power.sound)
        + part(sw[1], m.effect, power.effect)
        + part(sw[2], m.channel, power.channel)
        + center
}

/// `TInstParam`: what an instrument's region gives a note.
#[derive(Debug, Clone)]
struct InstParam {
    wave_id: u16,
    oscs: Vec<Option<Arc<Osc>>>,
    volume: f32,
    pitch: f32,
    effect_volume: f32,
    effect_pitch: f32,
    pan: f32,
    fxmix: f32,
    dolby: f32,
    effect_pan: f32,
    effect_fxmix: f32,
    effect_dolby: f32,
    fixed_pitch: bool,
    release: u16,
}

impl Default for InstParam {
    fn default() -> Self {
        InstParam {
            wave_id: 0,
            oscs: Vec::new(),
            volume: 1.0,
            pitch: 1.0,
            effect_volume: 1.0,
            effect_pitch: 1.0,
            pan: 0.5,
            fxmix: 0.0,
            dolby: 0.0,
            effect_pan: 0.5,
            effect_fxmix: 0.0,
            effect_dolby: 0.0,
            fixed_pitch: false,
            release: 0,
        }
    }
}

impl InstParam {
    /// `TInstEffect::getY` results applied by target.
    fn apply_effect(&mut self, target: u8, y: f32) {
        match target {
            0 => self.effect_volume *= y,
            1 => self.effect_pitch *= y,
            2 => self.effect_pan = y,
            3 => self.effect_fxmix = y,
            4 => self.effect_dolby = y,
            _ => {}
        }
    }
}

/// `TInstSense::getY`
fn sense_y(s: &bank::Sense, key: i32, vel: i32) -> f32 {
    let input = match s.source {
        1 => vel,
        2 => key,
        _ => 0,
    } as f32;
    let c = s.center as f32;
    if s.center == 0x7F || s.center == 0 {
        s.low + input * (s.high - s.low) / 127.0
    } else if input < c {
        s.low + (1.0 - s.low) * (input / c)
    } else {
        (s.high - 1.0) * ((input - c) / (127.0 - c)) + 1.0
    }
}

impl Engine {
    // ------------------------------------------------------------ lists

    pub(crate) fn add_list_head(&mut self, m: usize, c: usize, l: u8) {
        self.mgrs[m].lists[l as usize].push_front(c);
        self.channels[c].list = Some((m, l));
    }

    pub(crate) fn add_list_tail(&mut self, m: usize, c: usize, l: u8) {
        self.mgrs[m].lists[l as usize].push_back(c);
        self.channels[c].list = Some((m, l));
    }

    pub(crate) fn list_head(&mut self, m: usize, l: u8) -> Option<usize> {
        let c = self.mgrs[m].lists[l as usize].pop_front()?;
        self.channels[c].list = None;
        Some(c)
    }

    /// `cutList`: takes a channel out of whatever list it's in. False if it wasn't in one.
    pub(crate) fn cut_list(&mut self, c: usize) -> bool {
        let Some((m, l)) = self.channels[c].list.take() else {
            return false;
        };
        let list = &mut self.mgrs[m].lists[l as usize];
        match list.iter().position(|&x| x == c) {
            Some(i) => {
                list.remove(i);
                true
            }
            None => false,
        }
    }

    /// `ChGlobal::alloc`: moves up to `n` free channels from the global pool to `m`.
    fn global_alloc(&mut self, m: usize, n: u32) -> u32 {
        let mut i = 0;
        while i < n {
            let Some(c) = self.list_head(GLOBAL, 0) else {
                break;
            };
            self.add_list_head(m, c, 0);
            self.channels[c].mgr = m;
            self.channel_init(c);
            i += 1;
        }
        self.mgrs[m].managed += i;
        self.mgrs[GLOBAL].managed -= i;
        i
    }

    /// `ChGlobal::release`
    fn global_release(&mut self, c: usize) {
        let m = self.channels[c].mgr;
        self.add_list_head(GLOBAL, c, 0);
        self.mgrs[m].managed = self.mgrs[m].managed.saturating_sub(1);
        self.mgrs[GLOBAL].managed += 1;
        self.channels[c].mgr = GLOBAL;
    }

    /// `ChGlobal::releaseAll` / `TChannelMgr::receiveAllChannels`: every channel of `src` moves
    /// to `dst`; playing and releasing ones become orphans that only get volume updates.
    fn move_all_channels(&mut self, dst: usize, src: usize) {
        for l in 0..4u8 {
            while let Some(c) = self.list_head(src, l) {
                let to = if l == 3 && self.delete_queue(c) { 0 } else { l };
                self.add_list_head(dst, c, to);
                self.channels[c].mgr = dst;
                if l == 1 || l == 2 {
                    self.channels[c].orphan = true;
                }
            }
        }
        self.mgrs[dst].managed += self.mgrs[src].managed;
        self.mgrs[src].managed = 0;
    }

    pub(crate) fn release_all_to_global(&mut self, m: usize) {
        self.move_all_channels(GLOBAL, m);
    }

    pub(crate) fn receive_all_channels(&mut self, dst: usize, src: usize) {
        self.move_all_channels(dst, src);
        self.mgrs[dst].borrowed += self.mgrs[src].borrowed;
        self.mgrs[src].borrowed = 0;
    }

    /// `TChannelMgr::initAllocChannel`
    pub(crate) fn init_alloc_channel(&mut self, m: usize, n: u32) {
        if self.mgrs[m].managed != 0 {
            self.release_all_to_global(m);
        }
        self.mgrs[m].init();
        self.global_alloc(m, n);
        self.mgrs[m].steal = n != 0;
    }

    /// `TChannelMgr::stopAll` + `stopAllRelease` (`TTrack::flushAll`).
    pub(crate) fn flush_all(&mut self, m: usize) {
        let playing: Vec<usize> = self.mgrs[m].lists[1].iter().copied().collect();
        for c in playing {
            self.channel_stop(c, 0);
        }
        self.stop_all_release(m);
    }

    /// `stopAllRelease`: force-stops the envelopes of releasing channels.
    pub(crate) fn stop_all_release(&mut self, m: usize) {
        let releasing: Vec<usize> = self.mgrs[m].lists[2].iter().copied().collect();
        for c in releasing {
            for i in 0..4 {
                self.force_stop_osc(c, i);
            }
        }
    }

    /// `getLogicalChannel`: a free channel of `m` (borrowing one from the global pool, and
    /// stealing a playing one if the manager allows it).
    fn logical_channel(&mut self, m: usize, limit_key: u32) -> Option<usize> {
        let c = match self.list_head(m, 0) {
            Some(c) => c,
            None => {
                if self.global_alloc(m, 1) == 0 {
                    return None;
                }
                self.mgrs[m].borrowed += 1;
                let c = self.list_head(m, 0)?;
                if self.mgrs[m].steal {
                    let victim = self.list_head(m, 2).or_else(|| self.list_head(m, 1));
                    if let Some(v) = victim {
                        self.force_stop_osc(v, 0);
                        self.add_list_head(m, v, 3);
                        if let Some(vo) = self.channels[v].voice {
                            self.force_stop_voice(vo);
                        }
                    }
                }
                c
            }
        };
        self.channel_init(c);
        let ch = &mut self.channels[c];
        ch.limit_key = limit_key;
        self.channel_set_pan_power(c, 1.0, 1.0, 1.0);
        Some(c)
    }

    // ------------------------------------------------------------ TChannel

    /// `TChannel::init`
    pub(crate) fn channel_init(&mut self, c: usize) {
        let m = self.mgrs[self.channels[c].mgr].clone();
        let ch = &mut self.channels[c];
        ch.logical_cb = false;
        ch.sweeping = false;
        ch.length = 0;
        ch.countdown = 0;
        ch.source = Source::None;
        ch.orphan = false;
        // A track that pans through the Dolby auto-mixer (bus 0xFFFF) gets the stereo mixer's buses
        // now, not at its first mixer update: `playLogicalChannel` connects the voice's buses from
        // these, and left unreplaced the first bus (the left output) connected to nothing, so
        // Mario's voices, footsteps and FLUDD came out of the right ear only.
        ch.bus = if m.bus[0] == 0xFFFF { STEREO_BUS } else { m.bus };
        ch.priority = m.priority;
        ch.priority_time = m.priority_time;
        ch.calc = m.calc;
        for i in 0..4 {
            ch.osc_src[i] = OscSrc::None;
            ch.oscs[i].init();
        }
        ch.paused = false;
        ch.generation = ch.generation.wrapping_add(1);
        if ch.generation == 0 {
            ch.generation = 1;
        }
    }

    /// The `Osc_` a channel oscillator follows, by value.
    fn osc_of(&self, c: usize, i: usize) -> Option<Osc> {
        match &self.channels[c].osc_src[i] {
            OscSrc::None => None,
            OscSrc::Shared(o) => Some((**o).clone()),
            OscSrc::Track(t, k) => Some(self.tracks[*t].osc_data[*k].clone()),
        }
    }

    fn is_osc(&self, c: usize, i: usize) -> bool {
        !matches!(self.channels[c].osc_src[i], OscSrc::None)
    }

    /// `copyOsc`
    pub(crate) fn channel_osc(&self, c: usize, i: usize) -> Option<Osc> {
        self.osc_of(c, i)
    }

    /// `setOscInit`
    fn set_osc_init(&mut self, c: usize, i: usize, src: OscSrc) {
        self.channels[c].osc_src[i] = src;
        let o = self.osc_of(c, i);
        self.channels[c].oscs[i].init_start(o.as_ref());
    }

    /// `overwriteOsc`
    pub(crate) fn channel_overwrite_osc(&mut self, c: usize, i: usize, src: OscSrc) {
        self.set_osc_init(c, i, src);
        let v = self.bank_osc_to_ofs(c, i);
        self.effect_osc(c, i, v);
    }

    /// `forceStopOsc`
    fn force_stop_osc(&mut self, c: usize, i: usize) -> bool {
        self.is_osc(c, i) && self.channels[c].oscs[i].force_stop()
    }

    /// `releaseOsc`
    fn release_osc(&mut self, c: usize, i: usize) -> bool {
        match self.osc_of(c, i) {
            Some(o) => self.channels[c].oscs[i].release(&o),
            None => false,
        }
    }

    /// `bankOscToOfs`: advances oscillator `i` one update.
    fn bank_osc_to_ofs(&mut self, c: usize, i: usize) -> f32 {
        match self.osc_of(c, i) {
            Some(o) => self.channels[c].oscs[i].offset(Some(&o)),
            None => 1.0,
        }
    }

    /// `effectOsc`
    fn effect_osc(&mut self, c: usize, i: usize, v: f32) {
        let Some(target) = self.osc_of(c, i).map(|o| o.target) else {
            return;
        };
        let ch = &mut self.channels[c];
        match target {
            1 => ch.osc_pitch *= v,
            0 => ch.osc_volume *= v,
            2 => ch.mix[0].effect = v,
            3 => ch.mix[1].effect = v,
            4 => ch.mix[2].effect = v,
            _ => {}
        }
    }

    /// `setPanPower`
    pub(crate) fn channel_set_pan_power(&mut self, c: usize, a: f32, b: f32, cc: f32) {
        let mut sum = a + b + cc;
        if sum == 0.0 {
            sum = 1.0;
        }
        self.channels[c].pan_power = PanMatrix {
            sound: a / sum,
            effect: b / sum,
            channel: cc / sum,
        };
    }

    /// `setKeySweepTarget`
    pub(crate) fn channel_set_key_sweep(&mut self, c: usize, key: u8, time: u32) {
        let ch = &mut self.channels[c];
        let k = match ch.source {
            Source::Osc(_) | Source::None => key as i32,
            Source::Wave(_) => key as i32 + 0x3C - ch.wave_key as i32,
        };
        let v = c5base(k.clamp(0, 0x7F)) * ch.base_pitch;
        if time == 0 {
            ch.pitch = v;
            ch.sweeping = false;
            return;
        }
        ch.sweep_target = v;
        ch.sweep_time = time.min(0xFFFF) as u16;
        ch.sweeping = true;
    }

    /// `extraUpdate`: one step of a key sweep.
    fn extra_update(&mut self, c: usize) -> bool {
        let ch = &mut self.channels[c];
        if ch.sweep_time == 0 {
            return false;
        }
        let mut d = ch.sweep_target - ch.pitch;
        if ch.sweep_time <= 1 {
            ch.sweep_time = 1;
        }
        d /= ch.sweep_time as f32;
        ch.pitch += d;
        ch.sweep_time -= 1;
        if ch.sweep_time == 0 {
            ch.sweeping = false;
        }
        true
    }

    /// `checkLogicalChannel`
    fn has_source(&self, c: usize) -> bool {
        !matches!(self.channels[c].source, Source::None)
    }

    /// `resetInitialVolume`
    pub(crate) fn channel_reset_initial_volume(&mut self, c: usize) {
        if self.channels[c].voice.is_none() || !self.has_source(c) {
            return;
        }
        self.update_effector_param(c);
        self.update_jc_to_dsp_init(c);
    }

    /// `TChannel::play`
    fn channel_play(&mut self, c: usize, len: u32) -> bool {
        let len = if len == 0 { -1 } else { len as i32 };
        let m = self.channels[c].mgr;
        let ch = &mut self.channels[c];
        ch.length = len;
        ch.countdown = len;
        ch.logical_cb = true;
        let v = self.alloc_voice(c);
        self.channels[c].voice = v;
        if v.is_none() && self.has_source(c) {
            if !self.enqueue(c) {
                self.add_list_tail(m, c, 0);
                return false;
            }
            self.add_list_tail(m, c, 3);
            return true;
        }
        if !self.play_logical_channel(c) {
            if let Some(v) = self.channels[c].voice.take() {
                self.free_voice(v, c);
            }
            self.add_list_tail(m, c, 0);
            return false;
        }
        self.add_list_tail(m, c, 1);
        true
    }

    /// `TChannel::stop`
    pub(crate) fn channel_stop(&mut self, c: usize, release: u16) {
        if self.channels[c].voice.is_none() {
            self.update_logical(c, 6);
        } else {
            if release != 0 {
                self.channels[c].oscs[0].release_direct(release);
            }
            self.update_logical(c, 0);
        }
    }

    /// `stopLogicalChannel`: frees the channel's DSP voice.
    fn stop_logical_channel(&mut self, c: usize) -> bool {
        let Some(v) = self.channels[c].voice.take() else {
            return false;
        };
        self.voices[v].callback = false;
        self.voices[v].cb_interval = 0;
        self.voices[v].enabled = false;
        self.free_voice(v, c);
        true
    }

    /// `playLogicalChannel`: starts the DSP voice.
    pub(crate) fn play_logical_channel(&mut self, c: usize) -> bool {
        let Some(v) = self.channels[c].voice else {
            return false;
        };
        if !self.has_source(c) {
            return false;
        }
        self.voices[v].callback = true;
        self.voices[v].cb_interval = 1;
        let source = self.channels[c].source.clone();
        self.voice_set_source(v, &source);
        for i in 0..6 {
            // `setBusConnect` with output mode 1 (stereo): bus 8 plays on bus 0xB.
            let mut hi = (self.channels[c].bus[i] >> 8) as u8;
            if hi == 8 {
                hi = 0xB;
            }
            self.voices[v].mix[i].bus = bus_id(hi);
        }
        self.channels[c].play_mgr = Some(self.channels[c].mgr);
        for i in 0..4 {
            if self.is_osc(c, i) {
                let o = self.bank_osc_to_ofs(c, i);
                self.effect_osc(c, i, o);
            }
        }
        self.update_effector_param(c);
        self.update_jc_to_dsp_init(c);
        let ch = &self.channels[c];
        let vo = &mut self.voices[v];
        vo.priority = (ch.priority & 0xFF) as u8;
        vo.priority_time = ch.priority_time;
        // `playStart`
        vo.age = 0;
        vo.pos = 0.0;
        vo.osc_phase = 0.0;
        vo.enabled = true;
        vo.done = false;
        true
    }

    /// `updateEffectorParam`: the channel's final volume, pan, fx and pitch.
    pub(crate) fn update_effector_param(&mut self, c: usize) {
        let m = self.channels[c].mgr;
        if self.channels[c].play_mgr == Some(m) {
            let mg = &self.mgrs[m];
            let ch = &mut self.channels[c];
            ch.mgr_pitch = mg.pitch;
            ch.mgr_volume = mg.volume;
            ch.mix[0].channel = mg.pan;
            ch.mix[1].channel = mg.fxmix;
            ch.mix[2].channel = mg.dolby;
            ch.calc = mg.calc;
        }
        let ch = &mut self.channels[c];
        // Output mode 1 (stereo).
        let pan = if ch.calc[0] == 0 {
            0.5
        } else {
            calc_mix(&ch.mix[0], &ch.pan_power, ch.calc[0], 0.5)
        };
        let fxmix = calc_mix(&ch.mix[1], &ch.pan_power, ch.calc[1], 0.0);
        let volume = ch.mgr_volume * (ch.volume * ch.osc_volume);
        let (pan, fxmix, dolby) = (pan.clamp(0.0, 1.0), fxmix.clamp(0.0, 1.0), 0.0);
        let pitch = 4096.0 * (ch.mgr_pitch * (ch.pitch * ch.osc_pitch));
        ch.dsp_pitch = pitch.clamp(0.0, 65535.0) as u16;
        if ch.bus[0] == 0xFFFF {
            // The Dolby auto-mixer: no surround here, so the stereo mixer's curve.
            ch.bus = STEREO_BUS;
        }
        // `updateMixer`
        let inv = |v: f32| 1.0 - v;
        for i in 0..6 {
            let cfg = ch.bus[i];
            if cfg >> 8 == 0 {
                ch.mixer[i] = 0;
                continue;
            }
            let scale = |l: u16| match l {
                1 => pan,
                2 => fxmix,
                3 => dolby,
                5 => inv(pan),
                6 => inv(fxmix),
                7 => inv(dolby),
                _ => 1.0,
            };
            let mut vol = volume;
            let l0 = (cfg >> 4) & 0xF;
            let l1 = cfg & 0xF;
            if l0 != 0 {
                vol *= sinf_t(scale(l0));
            }
            if l1 != 0 {
                vol *= if l1 == 3 || l1 == 7 {
                    sinf_dolby2(scale(l1))
                } else {
                    sinf_t(scale(l1))
                };
            }
            ch.mixer[i] = (vol.clamp(0.0, 1.0) * MIXER_LEVEL) as i16;
        }
    }

    /// `__UpdateJcToDSP`
    fn update_jc_to_dsp(&mut self, c: usize) {
        let Some(v) = self.channels[c].voice else {
            return;
        };
        let ch = &self.channels[c];
        let vo = &mut self.voices[v];
        if !vo.end_requested {
            for i in 0..6 {
                vo.mix[i].target = ch.mixer[i];
            }
        }
        vo.pitch = ch.dsp_pitch.min(0x7FFF);
        vo.pause = ch.paused;
    }

    /// `__UpdateJcToDSPInit`: volumes start at their targets.
    fn update_jc_to_dsp_init(&mut self, c: usize) {
        let Some(v) = self.channels[c].voice else {
            return;
        };
        let ch = &self.channels[c];
        let vo = &mut self.voices[v];
        for i in 0..6 {
            vo.mix[i].target = ch.mixer[i];
            vo.mix[i].current = ch.mixer[i];
        }
        vo.pitch = ch.dsp_pitch.min(0x7FFF);
        vo.pause = ch.paused;
    }

    /// `updatecallLogicalChannel`: 0 = release the note, 1 = the voice finished, 2 = the
    /// envelope finished, 6 = drop a note still waiting for a voice.
    pub(crate) fn update_logical(&mut self, c: usize, param: u32) -> bool {
        let m = self.channels[c].mgr;
        if param == 0 {
            for i in 0..4 {
                self.release_osc(c, i);
            }
            if let Some(v) = self.channels[c].voice {
                self.voices[v].priority = ((self.channels[c].priority >> 8) & 0xFF) as u8;
            }
            if self.cut_list(c) {
                self.add_list_tail(m, c, 2);
            }
            self.channels[c].length = -1;
            return false;
        }
        if matches!(param, 1 | 2 | 6) {
            if self.mgrs[m].borrowed != 0 {
                if self.cut_list(c) {
                    self.mgrs[m].borrowed -= 1;
                    self.global_release(c);
                }
            } else if self.cut_list(c) {
                self.add_list_head(m, c, 0);
            }
            if param != 6 {
                self.stop_logical_channel(c);
            } else {
                self.delete_queue(c);
            }
            let ch = &mut self.channels[c];
            ch.key = 0xFF;
            ch.length = -1;
            ch.logical_cb = false;
        }
        false
    }

    /// `updatecallDSPChannel`: the per-voice update. Returns the next callback interval.
    pub(crate) fn update_dsp_channel(&mut self, v: usize, param: u32) -> u16 {
        let Some(c) = self.voices[v].sign.filter(|_| self.voices[v].callback) else {
            self.voices[v].callback = false;
            self.voices[v].priority = 0;
            return 0;
        };
        let m = self.channels[c].mgr;
        if self.channels[c].voice != Some(v) {
            // A stale voice: detach it.
            self.voices[v].sign = None;
            self.voices[v].priority = 0;
            self.voices[v].callback = false;
            return 0;
        }
        match param {
            2 => {
                if self.channels[c].logical_cb {
                    self.update_logical(c, 1);
                } else {
                    self.stop_logical_channel(c);
                    if self.cut_list(c) {
                        self.add_list_head(m, c, 0);
                    }
                }
                return 0;
            }
            4 => {
                let p = ((self.channels[c].priority >> 16) & 0xFF) as u8;
                if p < self.voices[v].priority {
                    self.voices[v].priority = p;
                }
                return 0;
            }
            _ => {}
        }
        let mut param = param;
        if param == 3 {
            self.force_stop_osc(c, 0);
            if !self.cut_list(c) {
                return 1;
            }
            self.add_list_head(m, c, 3);
            param = 0;
        }
        if param == 0 {
            let ch = &mut self.channels[c];
            ch.osc_pitch = 1.0;
            ch.osc_volume = 1.0;
            ch.mix[0].effect = 0.5;
            ch.mix[1].effect = 0.0;
            ch.mix[2].effect = 0.0;
            let mut n = 0;
            for i in 0..4 {
                if !self.is_osc(c, i) {
                    continue;
                }
                let o = self.bank_osc_to_ofs(c, i);
                self.effect_osc(c, i, o);
                if i == 0 && self.channels[c].oscs[0].state == 0 {
                    // The envelope ended.
                    if !self.channels[c].logical_cb {
                        if !self.stop_logical_channel(c) {
                            self.voices[v].enabled = false;
                        }
                        if self.cut_list(c) {
                            self.add_list_head(m, c, 0);
                        }
                        return 0;
                    }
                    self.update_logical(c, 2);
                    return 0;
                }
                n += 1;
            }
            if n != 0 {
                self.update_effector_param(c);
                self.channels[c].dirty = 1;
            }
            if self.channels[c].sweeping && self.extra_update(c) {
                self.channels[c].dirty += 1;
            }
            if !self.channels[c].logical_cb {
                return 1;
            }
            let ch = &mut self.channels[c];
            if ch.countdown > 0 {
                ch.countdown -= 1;
            }
        }
        if self.channels[c].countdown == 0 {
            self.update_logical(c, 0);
            self.channels[c].countdown = self.channels[c].length;
        }
        if self.channels[c].dirty != 0 {
            self.update_jc_to_dsp(c);
            self.channels[c].dirty = 0;
        }
        1
    }

    // ------------------------------------------------------------ DSPQueue

    /// `DSPQueue::enQueue`: waits for a voice, stealing a lower-priority one if possible.
    fn enqueue(&mut self, c: usize) -> bool {
        if self.queue.len() >= QUEUE {
            return false;
        }
        if !self.break_lower((self.channels[c].priority & 0xFF) as u8) {
            return false;
        }
        self.queue.push_back(Some(c));
        true
    }

    /// `DSPQueue::deQueue`: gives waiting notes free voices (`one` = at most one).
    pub(crate) fn dequeue(&mut self, one: bool) {
        while let Some(&front) = self.queue.front() {
            match front {
                Some(c) => {
                    let Some(v) = self.alloc_voice(c) else {
                        return;
                    };
                    self.channels[c].voice = Some(v);
                    self.play_logical_channel(c);
                    let m = self.channels[c].mgr;
                    if self.cut_list(c) {
                        self.add_list_tail(m, c, 1);
                    }
                    self.queue.pop_front();
                    if one {
                        break;
                    }
                }
                None => {
                    self.queue.pop_front();
                }
            }
        }
    }

    /// `DSPQueue::deleteQueue`
    pub(crate) fn delete_queue(&mut self, c: usize) -> bool {
        match self.queue.iter_mut().find(|e| **e == Some(c)) {
            Some(e) => {
                *e = None;
                true
            }
            None => false,
        }
    }

    /// `DSPQueue::checkQueue`: waiting notes time out by their length.
    pub(crate) fn check_queue(&mut self) {
        self.dequeue(false);
        for i in 0..self.queue.len() {
            let Some(c) = self.queue[i] else {
                continue;
            };
            if self.channels[c].length > 0 {
                self.channels[c].length -= 1;
            }
            if self.channels[c].length == 0 {
                self.update_logical(c, 6);
                self.queue[i] = None;
            }
        }
    }

    // ------------------------------------------------------------ BankMgr

    /// `TBasicInst::getParam` / `TDrumSet::getParam`.
    fn inst_param(&mut self, bank: usize, prog: u8, key: i32, vel: i32) -> Option<InstParam> {
        let data = self.data.clone();
        let ib = &data.init.banks.get(bank)?.bank;
        let mut p = InstParam::default();
        match ib.programs.get(prog as usize)?.as_ref()? {
            Program::Inst(inst) => {
                p.oscs = self.banks[bank].oscs[prog as usize].to_vec();
                p.volume *= inst.volume;
                p.pitch *= inst.pitch;
                for r in inst.rands.iter().flatten() {
                    let y = r.base + r.width * self.inst_rng.sfloat_1();
                    p.apply_effect(r.target, y);
                }
                for s in inst.sensors.iter().flatten() {
                    p.apply_effect(s.target, sense_y(s, key, vel));
                }
                let region = inst.key_region(key.clamp(0, 255) as u8)?;
                let vm = bank::vmap_for(&region.vels, vel.clamp(0, 255) as u8)?;
                p.volume *= vm.volume;
                p.pitch *= vm.pitch;
                p.wave_id = vm.wave_id;
            }
            Program::Perc(perc) => {
                if !(0..128).contains(&key) {
                    return None;
                }
                let pk = perc.keys[key as usize].as_ref()?;
                p.fixed_pitch = true;
                p.volume *= pk.volume;
                p.pitch *= pk.pitch;
                p.pan = perc.pan[key as usize];
                p.release = perc.release[key as usize];
                p.oscs = vec![Some(Arc::new(presets::perc()))];
                for r in pk.rands.iter().flatten() {
                    let y = r.base + r.width * self.inst_rng.sfloat_1();
                    p.apply_effect(r.target, y);
                }
                let vm = bank::vmap_for(&pk.vels, vel.clamp(0, 255) as u8)?;
                p.volume *= vm.volume;
                p.pitch *= vm.pitch;
                p.wave_id = vm.wave_id;
            }
        }
        Some(p)
    }

    /// `BankMgr::noteOn`
    pub(crate) fn bank_note_on(
        &mut self,
        m: usize,
        bank: usize,
        prog: i32,
        key: i32,
        vel: i32,
        len: u32,
    ) -> Option<usize> {
        if prog > 0xEF {
            return self.note_on_osc(m, (prog - 0xF0) as u32, key, vel, len);
        }
        let p = self.inst_param(bank, prog as u8, key, vel)?;
        let wb = self.banks.get(bank)?.wave_bank;
        let wave_bank = &self.data.init.wave_banks.get(wb)?.bank;
        let prefer = if wb == 2 { self.scene_group } else { None };
        let at = wave_bank.find(p.wave_id, prefer)?;
        let wave = *wave_bank.wave(at)?;
        let c = self.logical_channel(m, (bank as u32) << 8 | prog as u32)?;
        let ch = &mut self.channels[c];
        ch.source = Source::Wave((wb, at.0, at.1));
        ch.wave_key = wave.key;
        ch.velocity = vel as u8;
        ch.key = key as u8;
        ch.base_pitch = p.pitch * (wave.sample_rate / DAC_RATE);
        ch.pitch = ch.base_pitch * p.effect_pitch;
        if !p.fixed_pitch {
            ch.pitch *= c5base((key + 0x3C - wave.key as i32).clamp(0, 0x7F));
        }
        ch.inst_volume = p.volume;
        let v = ch.velocity as f32 / 127.0;
        ch.volume = ch.inst_volume * (v * v) * p.effect_volume;
        ch.mix[0].sound = p.pan.clamp(0.0, 1.0);
        ch.mix[1].sound = p.fxmix.clamp(0.0, 1.0);
        ch.mix[2].sound = p.dolby;
        ch.mix[0].effect = p.effect_pan;
        ch.mix[1].effect = p.effect_fxmix;
        ch.mix[2].effect = p.effect_dolby;
        ch.osc_pitch = 1.0;
        ch.osc_volume = 1.0;
        for (i, o) in p.oscs.iter().enumerate().take(4) {
            let src = o.clone().map_or(OscSrc::None, OscSrc::Shared);
            self.set_osc_init(c, i, src);
        }
        self.channels[c].oscs[0].release_direct(p.release);
        if !self.channel_play(c, len) {
            return None;
        }
        Some(c)
    }

    /// `noteOnOsc`: a synthesized waveform note.
    fn note_on_osc(&mut self, m: usize, kind: u32, key: i32, vel: i32, len: u32) -> Option<usize> {
        let c = self.logical_channel(m, 0)?;
        let ch = &mut self.channels[c];
        ch.source = Source::Osc(kind);
        ch.velocity = vel as u8;
        ch.key = key as u8;
        ch.base_pitch = 16736.016 / DAC_RATE;
        ch.pitch = ch.base_pitch * c5base(key.clamp(0, 127));
        ch.inst_volume = 1.0;
        let v = ch.velocity as f32 / 127.0;
        ch.volume = v * v;
        ch.mix[0] = PanMatrix {
            sound: 0.5,
            effect: 0.5,
            channel: ch.mix[0].channel,
        };
        ch.mix[1] = PanMatrix {
            sound: 0.0,
            effect: 0.0,
            channel: ch.mix[1].channel,
        };
        ch.mix[2] = PanMatrix {
            sound: 0.0,
            effect: 0.0,
            channel: ch.mix[2].channel,
        };
        ch.osc_pitch = 1.0;
        ch.osc_volume = 1.0;
        self.set_osc_init(c, 0, OscSrc::Shared(Arc::new(presets::osc_env())));
        self.channels[c].limit_key = 0;
        if !self.channel_play(c, len) {
            return None;
        }
        Some(c)
    }

    /// `BankMgr::gateOn`: a tied note changes the playing channel's key and velocity.
    pub(crate) fn bank_gate_on(&mut self, c: usize, key: i32, vel: u8, len: i32) {
        let ch = &mut self.channels[c];
        if ch.length != -1 {
            return;
        }
        ch.length = len;
        ch.countdown = len;
        let k = match ch.source {
            Source::Osc(_) => key,
            _ => key + 0x3C - ch.wave_key as i32,
        };
        ch.velocity = vel;
        ch.key = key as u8;
        ch.pitch = ch.base_pitch * c5base(k.clamp(0, 0x7F));
        let v = vel as f32 / 127.0;
        ch.volume = ch.inst_volume * (v * v);
    }
}

/// The buses a voice mixes into with the stereo mixer: the left and right outputs scaled by the pan's
/// inverse and the pan, and the two echo sends.
const STEREO_BUS: [u16; 6] = [0x150, 0x210, 0x352, 0x412, 0, 0];

/// `setBusConnect`'s `connect_table`: bus number to DSP buffer id.
fn bus_id(hi: u8) -> u16 {
    const TABLE: [u16; 12] = [
        0x0000, 0x0D00, 0x0D60, 0x0DC0, 0x0E20, 0x0E80, 0x0EE0, 0x0CA0, 0x0F40, 0x0FA0, 0x0B00,
        0x09A0,
    ];
    TABLE.get(hi as usize).copied().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calc_add_all_sums_the_offsets() {
        let m = PanMatrix {
            sound: 0.6,
            effect: 0.5,
            channel: 0.7,
        };
        let p = PanMatrix {
            sound: 1.0,
            effect: 1.0,
            channel: 1.0,
        };
        // 13 = add all: 0.5 + 0.1 + 0 + 0.2
        assert!((calc_mix(&m, &p, 13, 0.5) - 0.8).abs() < 1e-6);
        // 1 = channel only
        assert!((calc_mix(&m, &p, 1, 0.5) - 0.7).abs() < 1e-6);
    }
}
