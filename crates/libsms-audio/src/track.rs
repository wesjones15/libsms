//! Sequence tracks: `TTrack` (JASTrack.cpp), its helpers (JASSeqCtrl.cpp, JASTrackPort.cpp,
//! JASTrackInterrupt.cpp, JASRegisterParam.cpp, JASNoteMgr.cpp, JASTrackMgr.cpp) and the BMS
//! interpreter (`TSeqParser`, JASSeqParser.cpp).
//!
//! A sequence is a tree of tracks. Each tick a track runs bytecode until it waits: 0x00-0x7F
//! play notes, 0x80/0x88 wait, other 0x8X and 0xF9 stop voices, 0x90 glides a timed parameter,
//! 0xA0 does register arithmetic, 0xB0 runs a command with register arguments, 0xC0+ are
//! commands. Root tracks tick `tickRate` times per DSP subframe.

use std::sync::Arc;

use crate::calc::{DAC_RATE, SUBFRAME, SUBFRAMES, extend8to16, pitch_to_cent};
use crate::engine::*;
use crate::osc::{Osc, Table, presets};

/// `Arglist` (JASSeqParser.cpp), indexed by command - 0xC0: (count, 2-bit types: 0 = u8,
/// 1 = u16, 2 = u24, 3 = register). Sunshine drops command 0xD3 and adds 0xF3 (`VolumeMode`)
/// compared with Pikmin's JAudio.
const ARGLIST: [(u8, u16); 64] = [
    (0, 0),
    (2, 0x0008), // OpenTrack
    (2, 0x0008), // OpenTrackBros
    (1, 0x0002), // (no handler)
    (0, 0),      // Call
    (0, 0),
    (1, 0),      // Ret
    (1, 0x0002), // (no handler)
    (0, 0),      // Jmp
    (1, 0x0001), // LoopS
    (0, 0),      // LoopE
    (2, 0),      // ReadPort
    (2, 0x000C), // WritePort
    (1, 0),      // CheckPortImport
    (1, 0),      // CheckPortExport
    (1, 0x0003), // Wait (register)
    (2, 0x0005), // ConnectName
    (2, 0x000C), // ParentWritePort
    (2, 0x000C), // ChildWritePort
    (0, 0),
    (1, 0),      // SetLastNote
    (1, 0),      // TimeRelate
    (1, 0),      // SimpleOsc
    (2, 0x0008), // SimpleEnv
    (5, 0x0155), // SimpleADSR
    (1, 0),      // Transpose
    (1, 0),      // CloseTrack
    (1, 0),      // OutSwitch
    (1, 0x0001), // UpdateSync
    (2, 0x0004), // BusConnect
    (1, 0),      // PauseStatus
    (2, 0x0008), // SetInterrupt
    (1, 0),      // DisInterrupt
    (0, 0),      // ClrI
    (0, 0),      // SetI
    (0, 0),      // RetI
    (2, 0x0004), // IntTimer
    (0, 0),      // ConnectOpen
    (0, 0),      // ConnectClose
    (1, 0x0001), // SyncCPU
    (0, 0),      // FlushAll
    (0, 0),      // FlushRelease
    (1, 0x0002), // Wait (u24)
    (5, 0),      // PanPowSet
    (4, 0x0055), // IIRSet
    (1, 0x0002), // FIRSet
    (1, 0x0002), // EXTSet
    (3, 0),      // PanSwSet
    (1, 0),      // OscRoute
    (1, 0),      // IIRCutOff
    (3, 0x0028), // OscFull
    (1, 0),      // VolumeMode
    (0, 0),
    (0, 0),
    (0, 0),
    (0, 0),
    (0, 0),
    (0, 0),
    (1, 0x0001), // CheckWave
    (0, 0),      // Printf
    (0, 0),      // Nop
    (1, 0x0001), // Tempo
    (1, 0x0001), // TimeBase
    (0, 0),      // Finish
];

/// `TTrack::sOscTable`: oscillator target -> update flag.
const OSC_FLAGS: [u32; 5] = [
    update::VOLUME,
    update::PITCH,
    update::PAN,
    update::FXMIX,
    update::DOLBY,
];

/// `Player::CUTOFF_TO_IIR_TABLE` (JASPlayer_impl.cpp): each column runs linearly between its
/// first and last rows (row 0 = 0x0F5C, 0x0A3D, 0x4665, 0x3999; row 127 = 0x7FFF, 0, 0, 0).
/// The table's own rounding differs from this by at most one unit.
fn cutoff_to_iir(i: u8) -> [i16; 4] {
    let i = i.min(127) as f32 / 127.0;
    let lerp = |a: f32, b: f32| (a + (b - a) * i).round() as i16;
    [
        lerp(3932.0, 32767.0),
        lerp(2621.0, 0.0),
        lerp(18021.0, 0.0),
        lerp(14745.0, 0.0),
    ]
}

/// `TTrack::panCalc`
pub fn pan_calc(own: f32, other: f32, ratio: f32, kind: u8) -> f32 {
    match kind {
        0 => own,
        1 => other,
        _ => own * (1.0 - ratio) + other * ratio,
    }
}

/// Reads an envelope table out of sequence data: triplets up to a jump, sustain or stop.
fn table_at(data: &[u8], ofs: u32) -> Table {
    let mut t = Vec::new();
    let mut at = ofs as usize;
    while let Some(b) = data.get(at..at + 6) {
        let e = [
            i16::from_be_bytes([b[0], b[1]]),
            i16::from_be_bytes([b[2], b[3]]),
            i16::from_be_bytes([b[4], b[5]]),
        ];
        t.push(e);
        at += 6;
        if (0xD..=0xF).contains(&e[0]) || t.len() >= 64 {
            break;
        }
    }
    t.into()
}

impl Engine {
    // ------------------------------------------------------------ TSeqCtrl

    pub(crate) fn rb(&mut self, t: usize) -> u8 {
        let s = &mut self.tracks[t].seq;
        let b = s.byte_at(s.pos);
        s.pos = s.pos.wrapping_add(1);
        b
    }

    fn r16(&mut self, t: usize) -> u16 {
        (self.rb(t) as u16) << 8 | self.rb(t) as u16
    }

    fn r24(&mut self, t: usize) -> u32 {
        (self.rb(t) as u32) << 16 | (self.rb(t) as u32) << 8 | self.rb(t) as u32
    }

    fn get_n(&self, t: usize, at: u32, n: u32) -> u32 {
        (0..n).fold(0, |v, i| v << 8 | self.tracks[t].seq.byte_at(at + i) as u32)
    }

    fn seq_data(&self, t: usize) -> Arc<[u8]> {
        self.tracks[t]
            .seq
            .data
            .clone()
            .unwrap_or_else(|| Arc::from(&[][..]))
    }

    // ------------------------------------------------------------ TIntrMgr

    /// `TIntrMgr::request` (`TTrack::setInterrupt`).
    pub fn set_interrupt(&mut self, t: usize, n: u32) {
        let i = &mut self.tracks[t].intr;
        if i.mask & (1 << n) != 0 {
            i.pending |= 1 << n;
        }
    }

    /// `tryInterrupt`
    fn try_interrupt(&mut self, t: usize) -> bool {
        let tr = &mut self.tracks[t];
        if tr.seq.intr_return.is_some() || !tr.intr.enabled {
            return false;
        }
        let r = tr.intr.mask & tr.intr.pending;
        if r == 0 {
            return false;
        }
        let i = r.trailing_zeros();
        tr.intr.pending &= !(1 << i);
        let vector = tr.intr.vectors[i as usize];
        // `callIntr`
        tr.seq.intr_return = Some(tr.seq.pos);
        tr.seq.pos = vector;
        tr.seq.saved_wait = tr.seq.wait;
        tr.seq.wait = 0;
        true
    }

    /// `timerProcess`
    fn timer_process(&mut self, t: usize) {
        let i = &mut self.tracks[t].intr;
        if i.timer == 0 {
            return;
        }
        i.timer -= 1;
        if i.timer != 0 {
            return;
        }
        self.set_interrupt(t, 6);
        let i = &mut self.tracks[t].intr;
        if i.timer_count != 0 {
            i.timer_count -= 1;
            if i.timer_count != 0 {
                i.timer = i.timer_reload;
            }
        } else {
            i.timer = i.timer_reload;
        }
    }

    // ------------------------------------------------------------ registers

    /// `readReg32`
    fn read_reg32(&self, t: usize, reg: u8) -> u32 {
        match reg {
            0x28..=0x2B => self.address(t, reg - 0x28),
            0x23 => (self.read_reg(t, 4) as u32) << 16 | self.read_reg(t, 5) as u32,
            _ => self.read_reg(t, reg) as u32,
        }
    }

    fn address(&self, t: usize, i: u8) -> u32 {
        let r = &self.tracks[t].regs;
        (r[16 + i as usize * 2] as u32) << 16 | r[17 + i as usize * 2] as u32
    }

    fn set_address(&mut self, t: usize, i: u8, v: u32) {
        let r = &mut self.tracks[t].regs;
        r[16 + i as usize * 2] = (v >> 16) as u16;
        r[17 + i as usize * 2] = v as u16;
    }

    /// `exchangeRegisterValue`: registers below 0x40, ports above.
    pub(crate) fn exchange(&self, t: usize, reg: u8) -> u32 {
        if reg < 0x40 {
            self.read_reg32(t, reg)
        } else {
            self.tracks[t].port.value[((reg - 0x40) & 0xF) as usize] as u32
        }
    }

    /// `readRegDirect`
    pub fn read_reg(&self, t: usize, reg: u8) -> u16 {
        let tr = &self.tracks[t];
        match reg {
            32 => tr.regs[6] >> 8,
            33 => tr.regs[6] & 0xFF,
            34 => self.read_reg(t, 0) << 8 | self.read_reg(t, 1),
            44 => (0..16).rev().fold(0, |v, i| {
                v << 1 | tr.children[i].is_some_and(|c| self.tracks[c].state != 0) as u16
            }),
            45 => (0..8).rev().fold(0, |v, i| {
                let free = match self.note_channel(t, i) {
                    None => true,
                    Some(c) => self.channels[c].key == 0xFF,
                };
                v << 1 | free as u16
            }),
            48 => {
                let s = &tr.seq;
                if s.depth == 0 {
                    0
                } else {
                    s.loop_timers[s.depth as usize - 1]
                }
            }
            _ => tr.regs.get(reg as usize).copied().unwrap_or(0),
        }
    }

    /// `writeRegDirect`
    pub fn write_reg(&mut self, t: usize, reg: u8, value: u16) {
        let (mut slot, mut value, flag) = match reg {
            0..=2 => {
                let v = value & 0xFF;
                (reg, v, extend8to16(v as u8))
            }
            32 | 33 => return,
            34 => {
                let top = value >> 8;
                self.tracks[t].regs[0] = top;
                self.tracks[t].regs[3] = extend8to16(top as u8);
                (1, value & 0xFF, value)
            }
            _ => (reg, value, value),
        };
        if slot as usize >= self.tracks[t].regs.len() {
            slot = 3;
            value = flag;
        }
        self.tracks[t].regs[slot as usize] = value;
        self.tracks[t].regs[3] = flag;
    }

    /// `TNoteMgr::getChannel`: the channel a voice slot holds, if it's still that note.
    pub(crate) fn note_channel(&self, t: usize, i: usize) -> Option<usize> {
        let (c, generation) = self.tracks[t].notes.channels.get(i).copied().flatten()?;
        (self.channels[c].generation == generation).then_some(c)
    }

    // ------------------------------------------------------------ ports

    /// `routeTrack`: a descendant by route (`route >> 28` levels, a nibble each).
    pub fn route_track(&self, t: usize, mut route: u32) -> Option<usize> {
        let mut found = t;
        for _ in 0..route >> 28 {
            found = self.tracks[found].children[(route & 0xF) as usize]?;
            route >>= 4;
        }
        Some(found)
    }

    /// `writePortApp`: port `(addr >> 16) & 0xFF` of the track at route `addr`.
    pub fn write_port_app(&mut self, t: usize, addr: u32, value: u16) -> bool {
        let Some(f) = self.route_track(t, addr) else {
            return false;
        };
        let p = ((addr >> 16) & 0xF) as usize;
        self.write_port_direct(f, p, value);
        true
    }

    /// `readPortApp`
    pub fn read_port_app(&mut self, t: usize, addr: u32) -> Option<u16> {
        let f = self.route_track(t, addr)?;
        let p = ((addr >> 16) & 0xF) as usize;
        let port = &mut self.tracks[f].port;
        port.export[p] = false;
        Some(port.value[p])
    }

    /// `writePortAppDirect`
    pub fn write_port_direct(&mut self, t: usize, p: usize, value: u16) {
        let port = &mut self.tracks[t].port;
        port.import[p] = true;
        port.value[p] = value;
        if p <= 1 {
            self.set_interrupt(t, if p == 0 { 3 } else { 4 });
        }
    }

    /// `readPortAppDirect`
    pub fn read_port_direct(&mut self, t: usize, p: usize) -> u16 {
        let port = &mut self.tracks[t].port;
        port.export[p] = false;
        port.value[p]
    }

    // ------------------------------------------------------------ notes

    /// `TTrack::noteOn`
    fn track_note_on(&mut self, t: usize, voice: u8, key: i32, vel: i32, len: i32) -> i32 {
        let tr = &self.tracks[t];
        if tr.mute && tr.pause_status & 0x40 != 0 {
            return -1;
        }
        if self.note_channel(t, voice as usize).is_some() {
            self.track_note_off(t, voice, 0);
        }
        let parent = self.tracks[t].parent;
        // Find a manager with a free channel: our own, else the parent's (the C re-checks
        // the direct parent while walking up the chain).
        let mut mgr = t;
        let mut walk = parent;
        while self.mgrs[mgr].managed == 0 || self.mgrs[mgr].lists[0].is_empty() {
            let Some(w) = walk else {
                mgr = t;
                break;
            };
            mgr = parent.unwrap_or(t);
            walk = self.tracks[w].parent;
        }
        if self.tracks[t].open_mode == 4 {
            let Some(p) = parent else {
                return -1;
            };
            if mgr != p {
                if let Some(c) = self.list_head(mgr, 0) {
                    self.mgrs[mgr].managed -= 1;
                    self.add_list_head(t, c, 0);
                    self.mgrs[p].managed += 1;
                    self.channels[c].mgr = p;
                }
                mgr = p;
            }
        } else if mgr != t {
            if let Some(c) = self.list_head(mgr, 0) {
                self.mgrs[mgr].managed -= 1;
                self.add_list_head(t, c, 0);
                self.mgrs[t].managed += 1;
                self.channels[c].mgr = t;
            }
            mgr = t;
        }
        let reg = self.read_reg(t, 6);
        let Some(bank) = self.data.init.bank_physical((reg >> 8) & 0xFF) else {
            return -1;
        };
        let Some(c) = self.bank_note_on(mgr, bank, reg as u8 as i32, key, vel, len as u32) else {
            return -1;
        };
        self.tracks[t].notes.channels[voice as usize] = Some((c, self.channels[c].generation));
        let r = &self.tracks[t].regs;
        let (a, b, cc) = (r[8] as f32, r[9] as f32, r[10] as f32);
        self.channel_set_pan_power(c, a, b, cc);
        for i in 0..2 {
            let mut m = self.tracks[t].osc_mode[i];
            if m == 0xF || m == 0xE {
                continue;
            }
            if m >= 8 {
                m -= 8;
                if let Some(o) = self.channel_osc(c, m as usize) {
                    self.tracks[t].osc_data[i] = o;
                }
            } else if m >= 4 {
                m -= 4;
                let rel = self.tracks[t].osc_data[i].rel.clone();
                if let Some(o) = self.channel_osc(c, m as usize) {
                    self.tracks[t].osc_data[i] = o;
                    self.tracks[t].osc_data[i].rel = rel;
                }
            }
            if (m as usize) < 4 {
                self.channel_overwrite_osc(c, m as usize, OscSrc::Track(t, i));
            }
        }
        if self.update_sync_mode == 0 {
            self.update_track(t, update::VOLUME | update::PITCH | update::PAN);
        }
        self.channel_reset_initial_volume(c);
        0
    }

    /// `TTrack::noteOff`
    pub(crate) fn track_note_off(&mut self, t: usize, voice: u8, release: u16) -> bool {
        let Some(c) = self.note_channel(t, voice as usize) else {
            return false;
        };
        self.channel_stop(c, release);
        self.tracks[t].notes.channels[voice as usize] = None;
        true
    }

    /// `TTrack::gateOn`
    fn track_gate_on(&mut self, t: usize, voice: u8, key: i32, vel: i32, len: i32) -> i32 {
        let Some(c) = self.note_channel(t, voice as usize) else {
            return -1;
        };
        self.bank_gate_on(c, key, vel as u8, len);
        0
    }

    /// `checkNoteStop`
    fn check_note_stop(&self, t: usize, i: usize) -> bool {
        match self.note_channel(t, i) {
            None => true,
            Some(c) => self.channels[c].key == 0xFF,
        }
    }

    // ------------------------------------------------------------ oscillators

    /// `oscSetupFull`
    fn osc_setup_full(&mut self, t: usize, p: u8, ads: u32, rel: u32) {
        let i = ((p & 0x10) >> 4) as usize;
        let target = p & 0xF;
        let data = self.seq_data(t);
        let o = &mut self.tracks[t].osc_data[i];
        if p & 0x80 != 0 {
            *o = presets::envelope();
            o.target = target;
            if target == 1 {
                o.vertex = 1.0;
            }
        }
        if p & 0x40 != 0 {
            o.ads = Some(table_at(&data, ads));
        }
        if p & 0x20 != 0 {
            o.rel = Some(table_at(&data, rel));
        }
    }

    /// `oscSetupSimpleEnv`
    fn osc_setup_simple_env(&mut self, t: usize, kind: u8, ofs: u32) {
        let data = self.seq_data(t);
        match kind {
            0 => {
                self.tracks[t].osc_data[0] = Osc {
                    ads: Some(table_at(&data, ofs)),
                    ..presets::envelope()
                }
            }
            1 => self.tracks[t].osc_data[0].rel = Some(table_at(&data, ofs)),
            _ => {}
        }
    }

    /// `oscSetupSimple`
    fn osc_setup_simple(&mut self, t: usize, kind: u8) {
        match kind {
            0 => self.tracks[t].osc_data[1] = presets::vibrato(),
            1 => self.tracks[t].osc_data[0] = presets::tremolo(),
            2 => self.tracks[t].osc_data[1] = presets::tremolo(),
            _ => {}
        }
    }

    /// `oscUpdateParam`
    fn osc_update_param(&mut self, t: usize, i: usize, v: f32) {
        let o = &mut self.tracks[t].osc_data;
        match i {
            6 => o[0].width = v,
            7 => o[0].rate = v,
            8 => o[0].vertex = v,
            9 => o[1].width = v,
            10 => o[1].rate = v,
            11 => o[1].vertex = v,
            _ => {}
        }
    }

    // ------------------------------------------------------------ mixing

    /// `updateTrackAll`
    pub(crate) fn update_track_all(&mut self, t: usize) {
        let tr = &self.tracks[t];
        if tr.open_mode == 4 {
            return;
        }
        let parent_power = tr.regs[11] as f32 / 32767.0;
        let mgr = &mut self.mgrs[t];
        mgr.priority = tr.regs[13] as u32 | 0x10000;
        mgr.priority_time = 0;
        let (front, back) = pan_delays(tr.timed[timed::PAN_DELAY].current);
        mgr.delay_max = 0x10;
        mgr.delays[0] = front;
        mgr.delays[1] = back;
        let mut volume = tr.timed[timed::VOLUME].current;
        if tr.volume_mode == 0 {
            volume *= volume;
        }
        if tr.mute {
            volume = 0.0;
        }
        let mut pitch = pitch_to_cent(tr.timed[timed::PITCH].current, tr.regs[7] as f32);
        let mut pan = tr.timed[timed::PAN].current;
        let mut fxmix = tr.timed[timed::FXMIX].current;
        let mut dolby = tr.timed[timed::DOLBY].current;
        let o = &self.outer[t];
        if o.check(update::VOLUME) {
            volume *= o.volume;
        }
        if o.check(update::PITCH) {
            pitch *= o.pitch;
        }
        if o.check(update::FXMIX) {
            fxmix = pan_calc(fxmix, o.fxmix, parent_power, tr.pan_switch_ext[1]);
        }
        if o.check(update::DOLBY) {
            dolby = pan_calc(dolby, o.dolby, parent_power, tr.pan_switch_ext[2]);
        }
        if o.check(update::PAN) {
            pan = pan_calc(pan, o.pan, parent_power, tr.pan_switch_parent[0]);
        }
        match tr.parent.filter(|_| tr.open_mode & 1 == 0) {
            None => {
                let m = &mut self.mgrs[t];
                (m.volume, m.pitch, m.pan, m.fxmix, m.dolby) = (volume, pitch, pan, fxmix, dolby);
            }
            Some(p) => {
                let pm = self.mgrs[p].clone();
                let track_power = tr.regs[12] as f32 / 32767.0;
                let sw = tr.pan_switch_parent;
                let fir = o.check(update::FIR).then_some(o.fir);
                let iir: [i16; 4] =
                    std::array::from_fn(|i| (tr.timed[timed::IIR0 + i].current * 32767.0) as i16);
                let dist = (tr.timed[timed::DIST_FILTER].current * 32767.0) as i16;
                let m = &mut self.mgrs[t];
                m.volume = pm.volume * volume;
                m.pitch = pm.pitch * pitch;
                m.pan = pan_calc(pan, pm.pan, track_power, sw[0]);
                m.fxmix = pan_calc(fxmix, pm.fxmix, track_power, sw[1]);
                m.dolby = pan_calc(dolby, pm.dolby, track_power, sw[2]);
                if let Some(f) = fir {
                    m.fir = f;
                    m.filter_mode = 8;
                }
                m.iir = iir;
                m.filter_mode |= 0x20;
                m.dist_filter = dist;
            }
        }
    }

    /// `updateTrack`
    pub fn update_track(&mut self, t: usize, flags: u32) {
        let tr = &self.tracks[t];
        if tr.open_mode == 4 {
            return;
        }
        let parent_power = tr.regs[11] as f32 / 32767.0;
        if flags & update::PAN_DELAY != 0 {
            let (front, back) = pan_delays(tr.timed[timed::PAN_DELAY].current);
            self.mgrs[t].delays[0] = front;
            self.mgrs[t].delays[1] = back;
        }
        if flags & update::TEMPO != 0 && self.tracks[t].parent.is_none() {
            self.update_tempo(t);
        }
        let tr = &self.tracks[t];
        let o = self.outer[t];
        let (mut volume, mut pitch, mut pan, mut fxmix, mut dolby) = (0.0, 1.0, 0.5, 0.0, 0.0);
        if flags & update::VOLUME != 0 {
            volume = tr.timed[timed::VOLUME].current;
            if tr.volume_mode == 0 {
                volume *= volume;
            }
            if tr.mute {
                volume = 0.0;
            }
            if o.check(update::VOLUME) {
                volume *= o.volume;
            }
            if tr.paused && tr.pause_status & 1 != 0 {
                volume *= tr.timed[timed::PAUSE_VOLUME].current;
            }
        }
        if flags & update::PITCH != 0 {
            pitch = pitch_to_cent(tr.timed[timed::PITCH].current, tr.regs[7] as f32);
            if o.check(update::PITCH) {
                pitch *= o.pitch;
            }
        }
        if flags & update::PAN != 0 {
            pan = tr.timed[timed::PAN].current;
            if o.check(update::PAN) {
                pan = pan_calc(pan, o.pan, parent_power, tr.pan_switch_ext[0]);
            }
        }
        if flags & update::FXMIX != 0 {
            fxmix = tr.timed[timed::FXMIX].current;
            if o.check(update::FXMIX) {
                fxmix = pan_calc(fxmix, o.fxmix, parent_power, tr.pan_switch_ext[1]);
            }
        }
        if flags & update::DOLBY != 0 {
            dolby = tr.timed[timed::DOLBY].current;
            if o.check(update::DOLBY) {
                dolby = pan_calc(dolby, o.dolby, parent_power, tr.pan_switch_ext[2]);
            }
        }
        if flags & update::IIRS != 0 {
            let iir: [i16; 4] =
                std::array::from_fn(|i| (tr.timed[timed::IIR0 + i].current * 32767.0) as i16);
            self.mgrs[t].iir = iir;
            self.mgrs[t].filter_mode |= 0x20;
        }
        if flags & update::FIR != 0 && o.check(update::FIR) {
            let m = &mut self.mgrs[t];
            m.fir = o.fir;
            m.filter_mode = (m.filter_mode & 0x20) + 8;
        }
        if flags & update::DIST_FILTER != 0 {
            self.mgrs[t].dist_filter =
                (self.tracks[t].timed[timed::DIST_FILTER].current * 32767.0) as i16;
        }
        for i in 0..2 {
            if self.tracks[t].osc_mode[i] != 0xE {
                continue;
            }
            let od = self.tracks[t].osc_data[i].clone();
            let off = self.tracks[t].oscs[i].offset_no_count(Some(&od));
            match od.target {
                1 => pitch *= off,
                0 => volume *= off,
                2 => pan *= off,
                3 => fxmix *= off,
                4 => dolby *= off,
                _ => {}
            }
        }
        let tr = &self.tracks[t];
        match tr.parent.filter(|_| tr.open_mode & 1 == 0) {
            None => {
                let m = &mut self.mgrs[t];
                if flags & update::VOLUME != 0 {
                    m.volume = volume;
                }
                if flags & update::PITCH != 0 {
                    m.pitch = pitch;
                }
                if flags & update::PAN != 0 {
                    m.pan = pan;
                }
                if flags & update::FXMIX != 0 {
                    m.fxmix = fxmix;
                }
                if flags & update::DOLBY != 0 {
                    m.dolby = dolby;
                }
            }
            Some(p) => {
                let (pv, pp, ppan, pfx, pdol) = {
                    let m = &self.mgrs[p];
                    (m.volume, m.pitch, m.pan, m.fxmix, m.dolby)
                };
                let power = tr.regs[12] as f32 / 32767.0;
                let sw = tr.pan_switch_parent;
                let m = &mut self.mgrs[t];
                if flags & update::VOLUME != 0 {
                    m.volume = pv * volume;
                }
                if flags & update::PITCH != 0 {
                    m.pitch = pp * pitch;
                }
                if flags & update::PAN != 0 {
                    m.pan = pan_calc(pan, ppan, power, sw[0]);
                }
                if flags & update::FXMIX != 0 {
                    m.fxmix = pan_calc(fxmix, pfx, power, sw[1]);
                }
                if flags & update::DOLBY != 0 {
                    m.dolby = pan_calc(dolby, pdol, power, sw[2]);
                }
            }
        }
    }

    /// `updateTempo`
    pub(crate) fn update_tempo(&mut self, t: usize) {
        match self.tracks[t].parent {
            None => {
                let tr = &self.tracks[t];
                let mut rate =
                    tr.time_base as f32 * tr.tempo as f32 / DAC_RATE * SUBFRAME as f32 / 60.0;
                let o = &self.outer[t];
                if o.check(update::TEMPO) {
                    rate *= o.tempo;
                }
                self.tracks[t].tick_rate = rate;
            }
            Some(p) => {
                self.tracks[t].tick_rate = self.tracks[p].tick_rate;
                self.tracks[t].time_base = self.tracks[p].time_base;
            }
        }
        for i in 0..16 {
            if let Some(c) = self.tracks[t].children[i]
                && self.tracks[c].state != 0
            {
                self.update_tempo(c);
            }
        }
    }

    /// `updateSeq`
    pub(crate) fn update_seq(&mut self, t: usize, flags: u32, recurse: bool) {
        let mut f = flags | self.tracks[t].pending_update;
        f |= self.outer[t].update as u32;
        self.outer[t].update = 0;
        self.tracks[t].pending_update = 0;
        if f != 0 {
            self.update_track(t, f);
        }
        for i in 0..16 {
            let Some(c) = self.tracks[t].children[i] else {
                continue;
            };
            if self.tracks[c].state == 0 {
                continue;
            }
            if recurse {
                self.update_seq(c, f, true);
            } else {
                self.tracks[c].pending_update |= f;
            }
        }
    }

    /// `incSelfOsc`
    fn inc_self_osc(&mut self, t: usize) {
        for i in 0..2 {
            if self.tracks[t].osc_mode[i] == 0xE {
                let od = self.tracks[t].osc_data[i].clone();
                self.tracks[t].oscs[i].offset(Some(&od));
            }
        }
        for i in 0..16 {
            if let Some(c) = self.tracks[t].children[i] {
                self.inc_self_osc(c);
            }
        }
    }

    // ------------------------------------------------------------ the main loop

    /// `TTrack::mainProc`: one tick of a track and its children. -1 closes the track.
    fn main_proc(&mut self, t: usize, h: &mut dyn SyncHandler) -> i32 {
        if let Some(p) = self.tracks[t].parent
            && self.tracks[t].own_tempo
        {
            let ratio = (self.tracks[t].tempo as f32 / self.tracks[p].tempo.max(1) as f32).min(1.0);
            self.tracks[t].tick_counter += ratio;
            if self.tracks[t].tick_counter < 1.0 {
                return 0;
            }
            self.tracks[t].tick_counter -= 1.0;
        }
        if let Some(p) = self.tracks[t].parent
            && self.mgrs[t].managed != 0
            && let Some(c) = self.list_head(t, 0)
        {
            self.add_list_head(p, c, 0);
            self.channels[c].mgr = p;
            self.mgrs[t].managed -= 1;
            self.mgrs[p].managed += 1;
        }
        let pt = self.tracks[t]
            .parent
            .map_or(0, |p| self.tracks[p].transpose_total);
        let tr = &mut self.tracks[t];
        tr.transpose_total = tr.transpose.wrapping_add(pt);
        self.set_interrupt(t, 7);
        self.timer_process(t);

        let mut bail = false;
        let mut guard = 0;
        loop {
            self.try_interrupt(t);
            let tr = &self.tracks[t];
            if tr.paused && tr.pause_status & 2 != 0 {
                bail = true;
                break;
            }
            if self.tracks[t].seq.wait == -1 {
                if !self.check_note_stop(t, 0) {
                    break;
                }
                self.tracks[t].seq.wait = 0;
            }
            if self.tracks[t].seq.wait > 0 {
                // `waitCountDown`
                let s = &mut self.tracks[t].seq;
                s.wait -= 1;
                if s.wait != 0 {
                    break;
                }
                // `TNoteMgr::endProcess`
                let n = &mut self.tracks[t].notes;
                if n.base_time != u32::MAX && n.connect == 0 {
                    n.channels[0] = None;
                }
            }
            let r = self.parse(t, h);
            if r == -1 {
                return -1;
            }
            guard += 1;
            if r != -2 || guard > 64 {
                break;
            }
        }
        let mut flags = 0u32;
        if !bail {
            for i in 0..timed::COUNT {
                let m = &mut self.tracks[t].timed[i];
                if m.time > 0.0 {
                    m.current += m.step;
                    m.time -= 1.0;
                    let v = m.current;
                    if i <= timed::DIST_FILTER || i >= timed::OSC1_VERTEX {
                        flags |= 1 << i;
                    } else {
                        self.osc_update_param(t, i, v);
                    }
                }
            }
            for i in 0..2 {
                if self.tracks[t].osc_mode[i] == 0xE {
                    let target = self.tracks[t].osc_data[i].target as usize;
                    flags |= OSC_FLAGS.get(target).copied().unwrap_or(0);
                }
            }
        }
        self.tracks[t].pending_update |= flags;
        self.update_seq(t, 0, false);
        for i in 0..16 {
            let Some(c) = self.tracks[t].children[i] else {
                continue;
            };
            if self.tracks[c].state == 0 {
                continue;
            }
            if self.main_proc(c, h) == -1 {
                self.close_track(c);
                self.tracks[t].children[i] = None;
            }
        }
        0
    }

    /// `seqTimeToDspTime`: ticks x gate% to channel updates.
    fn seq_time_to_dsp(&self, t: usize, ticks: i32, gate: u8) -> i32 {
        let tr = &self.tracks[t];
        let mut r = ticks as f32 * gate as f32 / 100.0;
        if tr.time_relate {
            r /= tr.tick_rate;
        } else {
            // `gOutputRate` is 0: scaled by subframes / 10.
            r = r * 120.0 / tr.time_base.max(1) as f32 * SUBFRAMES as f32 / 10.0;
        }
        r as i32
    }

    /// `writeTimeParam` (0x90-0x9F): glide timed parameter `n` to a value over a time.
    fn write_time_param(&mut self, t: usize, op: u8) {
        let n = self.rb(t) as usize;
        let value: i16 = match op & 0xC {
            0 => {
                let r = self.rb(t);
                self.read_reg(t, r) as i16
            }
            4 => self.rb(t) as i16,
            8 => {
                let b = self.rb(t) as u16;
                (if b & 0x80 != 0 {
                    b << 8
                } else {
                    b << 8 | b << 1
                }) as i16
            }
            _ => self.r16(t) as i16,
        };
        let time: i32 = match op & 3 {
            0 => -1,
            1 => {
                let r = self.rb(t);
                self.read_reg(t, r) as i32
            }
            2 => self.rb(t) as i32,
            _ => self.r16(t) as i32,
        };
        let Some(m) = self.tracks[t].timed.get_mut(n) else {
            return;
        };
        m.target = value as f32 / 32768.0;
        if time <= 0 {
            m.current = m.target;
            m.step = 0.0;
            m.time = 1.0;
        } else {
            m.step = (m.target - m.current) / time as f32;
            m.time = time as f32;
        }
    }

    /// `writeRegParam` (0xA0-0xAF): register arithmetic.
    fn write_reg_param(&mut self, t: usize, mut op: u8) {
        let mut src = (op & 0xC) as u32;
        let mut alu = (op & 3) as u32;
        let mut tbl_kind = 0u32;
        if op & 0xF == 0xB {
            src = 0;
            alu = 0xB;
        }
        if op & 0xF == 0xA {
            alu = 10;
            op = self.rb(t);
            src = (op & 0xC) as u32;
            tbl_kind = ((op >> 4) & 0xF) as u32 + 4;
        }
        if op & 0xF == 0x9 {
            alu = self.rb(t) as u32;
            src = alu & 0xC;
            alu &= 0xF0;
            if src == 8 {
                src = 0x10;
            }
        }
        let mut dst = self.rb(t);
        let mut base = 0u32;
        if alu == 10 {
            let r = self.rb(t);
            base = self.read_reg32(t, r);
        }
        let mut v: i16 = match src {
            0 => {
                let r = self.rb(t);
                self.read_reg(t, r) as i16
            }
            4 => self.rb(t) as i16,
            8 => {
                let b = self.rb(t) as u16;
                (if b & 0x80 != 0 {
                    b << 8
                } else {
                    b << 8 | b << 1
                }) as i16
            }
            0xC => self.r16(t) as i16,
            _ => -1,
        };
        let cur = self.read_reg(t, dst) as i16;
        match alu {
            0 => {}
            1 => {
                if src == 4 {
                    v = extend8to16(v as u8) as i16;
                }
                v = cur.wrapping_add(v);
            }
            2 => {
                let product = (cur as u16 as u32).wrapping_mul(v as i32 as u32);
                self.write_reg(t, 4, (product >> 16) as u16);
                self.write_reg(t, 5, product as u16);
                return;
            }
            3 => {
                self.tracks[t].regs[3] = cur.wrapping_sub(v) as u16;
                return;
            }
            0xB => v = cur.wrapping_sub(v),
            0x10 | 0x20 => {
                if src == 4 {
                    v = extend8to16(v as u8) as i16;
                }
                v = if alu == 0x10 {
                    // Logical shift.
                    if v < 0 {
                        ((cur as u16) >> (-v as u32).min(15)) as i16
                    } else {
                        ((cur as u16) << (v as u32).min(15)) as i16
                    }
                } else if v < 0 {
                    cur >> (-v as u32).min(15)
                } else {
                    cur << (v as u32).min(15)
                };
            }
            0x30 => v &= cur,
            0x40 => v |= cur,
            0x50 => v ^= cur,
            0x60 => v = cur.wrapping_neg(),
            0x90 => {
                let r = self.rng.s32() as u32;
                v = (r % (v as u16).max(1) as u32) as i16;
            }
            10 => {
                let r = self.load_tbl(t, base, v as u16 as u32, tbl_kind);
                base = r;
                v = r as u16 as i16;
            }
            _ => {}
        }
        let flag: u16;
        match dst {
            0..=2 => {
                v = v as u8 as i16;
                flag = extend8to16(v as u8);
            }
            0x21 => {
                v = ((v as u8 as u16) | (self.read_reg(t, 32) << 8)) as i16;
                dst = 6;
                flag = v as u16;
            }
            0x20 => {
                v = ((v as u16) << 8 | self.read_reg(t, 33) & 0xFF) as i16;
                dst = 6;
                flag = v as u16;
            }
            0x2E => {
                v = ((self.tracks[t].regs[13] & 0xFF00) | v as u8 as u16) as i16;
                dst = 0xD;
                flag = v as u16;
            }
            0x2F => {
                v = ((v as u16) << 8 | self.tracks[t].regs[13] & 0xFF) as i16;
                dst = 0xD;
                flag = v as u16;
            }
            0x22 => {
                self.write_reg(t, 0, (v as u16) >> 8);
                v &= 0xFF;
                dst = 1;
                flag = v as u16;
            }
            0x28..=0x2B => {
                self.set_address(t, dst - 0x28, base);
                return;
            }
            _ => flag = v as u16,
        }
        if let Some(r) = self.tracks[t].regs.get_mut(dst as usize) {
            *r = v as u16;
        }
        self.tracks[t].regs[3] = flag;
        let tr = &mut self.tracks[t];
        if dst == 6 {
            for m in &mut tr.osc_mode {
                if *m != 0xE {
                    *m = 0xF;
                }
            }
        }
        if dst == 7 {
            tr.pending_update |= update::PITCH;
        }
        if dst == 0xD {
            let p = tr.regs[13] as u32 | 0x10000;
            self.mgrs[t].priority = p;
            self.mgrs[t].priority_time = 0;
        }
    }

    /// `loadTbl`
    fn load_tbl(&self, t: usize, base: u32, i: u32, kind: u32) -> u32 {
        match kind {
            4 => self.get_n(t, base + i, 1),
            5 => self.get_n(t, base + 2 * i, 2),
            6 => self.get_n(t, base + 3 * i, 3),
            7 => self.get_n(t, base + 4 * i, 4),
            8 => self.get_n(t, base + i, 4),
            _ => 0,
        }
    }

    // ------------------------------------------------------------ lifetime

    /// `setSeqData` (`JAISystemInterface::setSeqData` with a fresh track): loads a sequence
    /// into a new root. Returns the root handle.
    pub fn set_seq_data(&mut self, data: Arc<[u8]>) -> Option<usize> {
        let t = self.new_track()?;
        let Some(handle) = self.roots.iter().position(Option::is_none) else {
            self.back_track(t);
            self.tracks[t].state = 0;
            return None;
        };
        self.roots[handle] = Some(t);
        self.tracks[t].route = handle as u32;
        self.tracks[t].open_mode = 3;
        self.tracks[t].seq.data = Some(data);
        self.init_track(t, 0, None);
        self.init_alloc_channel(t, 0);
        self.tracks[t].tick_counter = 0.0;
        self.tracks[t].tick_rate = 1.0;
        self.update_track_all(t);
        self.tracks[t].state = 2;
        Some(handle)
    }

    /// `startSeq`: registers the root's subframe callback.
    pub fn start_seq(&mut self, handle: usize) -> bool {
        let Some(t) = self.root(handle) else {
            return false;
        };
        if self.tracks[t].state != 2 {
            return false;
        }
        self.tracks[t].state = 1;
        if !self.running.contains(&t) {
            self.running.push(t);
        }
        true
    }

    /// `stopSeq`: the root stops at its next tick.
    pub fn stop_seq(&mut self, handle: usize) -> bool {
        let Some(t) = self.root(handle) else {
            return false;
        };
        match self.tracks[t].state {
            0 => {}
            2 => {
                self.tracks[t].state = 0;
                if self.tracks[t].inner_memory {
                    self.back_track(t);
                }
                self.dealloc_root(t);
            }
            _ => self.tracks[t].state = 3,
        }
        true
    }

    fn dealloc_root(&mut self, t: usize) {
        for r in &mut self.roots {
            if *r == Some(t) {
                *r = None;
            }
        }
    }

    /// `stopSeqMain`
    fn stop_seq_main(&mut self, t: usize) {
        self.update_seq(t, 0, true);
        self.close_track(t);
        self.dealloc_root(t);
    }

    /// `allNoteOff`
    fn all_note_off(&mut self, t: usize) {
        let release = if self.tracks[t].parent.is_none() {
            10
        } else {
            0
        };
        for i in 0..8 {
            self.track_note_off(t, i, release);
        }
        self.tracks[t].notes = NoteMgr::default();
    }

    /// `closeTrack`
    pub(crate) fn close_track(&mut self, t: usize) {
        if self.tracks[t].state == 0 {
            return;
        }
        self.all_note_off(t);
        self.tracks[t].state = 0;
        if self.tracks[t].inner_memory {
            self.back_track(t);
        }
        for i in 0..16 {
            if let Some(c) = self.tracks[t].children[i].take() {
                self.close_track(c);
            }
        }
        self.tracks[t].mute = false;
        // `releaseChannelAll`
        match self.tracks[t].parent {
            Some(p) => self.receive_all_channels(p, t),
            None => self.release_all_to_global(t),
        }
        if self.tracks[t].connected {
            for e in &mut self.registry {
                if e.1 == Some(t) {
                    e.1 = None;
                }
            }
            while self.registry.last().is_some_and(|e| e.1.is_none()) {
                self.registry.pop();
            }
            self.tracks[t].connected = false;
        }
    }

    /// `muteTrack`
    pub fn mute_track(&mut self, t: usize, mute: bool) {
        self.tracks[t].mute = mute;
        self.tracks[t].pending_update |= update::VOLUME;
        if mute && self.tracks[t].pause_status & 0x20 != 0 {
            for i in 0..8 {
                if let Some(c) = self.note_channel(t, i) {
                    self.channel_stop(c, 10);
                    self.tracks[t].notes.channels[i] = None;
                }
            }
        }
        for i in 0..16 {
            if let Some(c) = self.tracks[t].children[i] {
                self.mute_track(c, mute);
            }
        }
    }

    /// `pauseTrack`
    pub fn pause_track(&mut self, t: usize, recursive: bool) {
        self.tracks[t].paused = true;
        let status = self.tracks[t].pause_status;
        if status & 1 != 0 {
            self.tracks[t].pending_update |= update::VOLUME;
        }
        if status & 4 != 0 {
            for i in 0..8 {
                if let Some(c) = self.note_channel(t, i) {
                    self.channel_stop(c, 10);
                    self.tracks[t].notes.channels[i] = None;
                }
            }
        }
        if status & 8 != 0 {
            for i in 0..8 {
                if let Some(c) = self.note_channel(t, i) {
                    self.channels[c].paused = true;
                }
            }
        }
        self.set_interrupt(t, 0);
        if recursive {
            for i in 0..16 {
                if let Some(c) = self.tracks[t].children[i]
                    && self.tracks[c].state != 0
                {
                    self.pause_track(c, true);
                }
            }
        }
    }

    /// `unPauseTrack`
    pub fn unpause_track(&mut self, t: usize, recursive: bool) {
        self.tracks[t].paused = false;
        self.tracks[t].pending_update |= update::VOLUME;
        for i in 0..8 {
            if let Some(c) = self.note_channel(t, i) {
                self.channels[c].paused = false;
            }
        }
        self.set_interrupt(t, 1);
        if recursive {
            for i in 0..16 {
                if let Some(c) = self.tracks[t].children[i]
                    && self.tracks[c].state != 0
                {
                    self.unpause_track(c, true);
                }
            }
        }
    }

    /// `initTrack`
    fn init_track(&mut self, t: usize, pos: u32, parent: Option<usize>) {
        let p = parent.map(|p| self.tracks[p].clone());
        let tr = &mut self.tracks[t];
        // `TSeqCtrl::init`
        tr.seq.pos = pos;
        tr.seq.wait = 0;
        tr.seq.depth = 0;
        tr.seq.stack = [0; 8];
        tr.seq.loop_timers = [0; 8];
        tr.seq.saved_wait = 0;
        tr.seq.intr_return = None;
        match &p {
            None => {
                tr.tempo = 0x78;
                tr.time_base = 0x30;
                tr.time_relate = true;
                tr.paused = false;
                tr.pause_status = 10;
                tr.volume_mode = 0;
                tr.mute = false;
            }
            Some(p) => {
                tr.tempo = p.tempo;
                tr.own_tempo = false;
                tr.tick_rate = p.tick_rate;
                tr.time_base = p.time_base;
                tr.time_relate = p.time_relate;
                tr.paused = p.paused;
                tr.pause_status = p.pause_status;
                tr.volume_mode = p.volume_mode;
                tr.mute = p.mute;
            }
        }
        tr.notes = NoteMgr::default();
        tr.state = 1;
        tr.parent = parent;
        // `TIntrMgr::init`
        tr.intr = IntrMgr {
            enabled: true,
            ..Default::default()
        };
        // `initTimed`
        for (i, m) in tr.timed.iter_mut().enumerate() {
            let v = match i {
                1 | 2 | 4 | 5 | 13 | 14 | 15 | 17 => 0.0,
                3 | 16 => 0.5,
                _ => 1.0,
            };
            *m = Move {
                current: v,
                target: v,
                time: 0.0,
                step: 0.0,
            };
        }
        match &p {
            Some(p) if tr.open_mode & 2 == 0 => {
                // `TRegisterParam::inherit`
                let mut regs = [0u16; 24];
                regs[6] = p.regs[6];
                regs[7] = p.regs[7];
                regs[8..=13].copy_from_slice(&p.regs[8..=13]);
                tr.regs = regs;
                tr.pan_switch_ext = p.pan_switch_ext;
                tr.pan_switch_parent = p.pan_switch_parent;
            }
            _ => {
                // `TRegisterParam::init`
                let mut regs = [0u16; 24];
                regs[6] = 0xF0;
                regs[7] = 0x0C;
                regs[8..=12].copy_from_slice(&[0, 1, 1, 0x7FFF, 0x4000]);
                regs[13] = 0x40;
                tr.regs = regs;
                tr.pan_switch_ext = [0; 3];
                tr.pan_switch_parent = [0; 3];
            }
        }
        let calc = match (&p, tr.open_mode & 2 == 0) {
            (Some(_), true) => parent.map(|p| self.mgrs[p].calc).unwrap_or([0x1A; 3]),
            _ => [0x1A; 3],
        };
        let tr = &mut self.tracks[t];
        tr.children = [None; 16];
        tr.osc_data = [presets::envelope(), presets::vibrato()];
        tr.osc_mode = [0xF, 0xE];
        let od = tr.osc_data[1].clone();
        tr.oscs[1].init_start(Some(&od));
        tr.transpose = 0;
        tr.transpose_total = 0;
        tr.port = TrackPort::default();
        tr.connected = false;
        self.mgrs[t].calc = calc;
    }

    /// `startTrack`
    fn start_track(&mut self, t: usize, parent: usize, idx: u8, mode: u8, pos: u32) {
        let pr = self.tracks[parent].route;
        self.tracks[t].route =
            ((pr << 4) | idx as u32) & 0x0FFF_FFFF | ((pr & 0xF000_0000) + 0x1000_0000);
        self.tracks[t].connect_name = 0;
        self.tracks[t].open_mode = mode;
        self.tracks[t].seq.data = self.tracks[parent].seq.data.clone();
        self.init_track(t, pos, Some(parent));
        self.outer[t].switch = 0;
        self.outer[t].update = 0;
        self.init_alloc_channel(t, 0);
        self.update_track_all(t);
    }

    /// `openTrack`
    fn open_track(&mut self, t: usize, idx: u8) -> Option<usize> {
        if let Some(old) = self.tracks[t].children[idx as usize] {
            self.close_track(old);
        }
        let n = self.new_track()?;
        self.tracks[t].children[idx as usize] = Some(n);
        Some(n)
    }

    /// `rootCallback`: one subframe of a running root. Returns false when it's finished.
    pub(crate) fn root_callback(&mut self, t: usize, h: &mut dyn SyncHandler) -> bool {
        match self.tracks[t].state {
            0 => return false,
            3 => {
                self.stop_seq_main(t);
                return false;
            }
            _ => {}
        }
        self.tracks[t].tick_counter += self.tracks[t].tick_rate;
        if self.tracks[t].tick_counter < 1.0 {
            self.update_seq(t, 0, true);
        } else {
            while self.tracks[t].tick_counter >= 1.0 {
                self.tracks[t].tick_counter -= 1.0;
                if self.main_proc(t, h) == -1 {
                    self.stop_seq_main(t);
                    return false;
                }
            }
        }
        self.inc_self_osc(t);
        true
    }

    // ------------------------------------------------------------ TSeqParser

    /// `conditionCheck`
    fn condition(&self, t: usize, flag: u8) -> bool {
        let v = self.read_reg(t, 3);
        match flag & 0xF {
            0 => true,
            1 => v == 0,
            2 => v != 0,
            3 => v == 1,
            4 => v >= 0x8000,
            5 => v < 0x8000,
            _ => false,
        }
    }

    /// `TSeqParser::mainProc`: runs bytecode until a wait (0), an interrupt return (-2) or the
    /// end of the track (-1).
    fn parse(&mut self, t: usize, h: &mut dyn SyncHandler) -> i32 {
        for _ in 0..10_000 {
            let op = self.rb(t);
            let r = if op & 0x80 == 0 {
                self.cmd_note_on(t, op)
            } else if op & 0xF0 == 0x80 && op & 7 == 0 {
                // `cmdWait(flag)`: 0x80 = u8, 0x88 = u16.
                let n = if op == 0x80 { 1 } else { 2 };
                let mut w = 0i32;
                for _ in 0..n {
                    w = w << 8 | self.rb(t) as i32;
                }
                self.tracks[t].seq.wait = w;
                (w != 0) as i32
            } else if op & 0xF0 == 0x80 || op == 0xF9 {
                self.cmd_note_off(t, op)
            } else {
                match op & 0xF0 {
                    0x90 => {
                        self.write_time_param(t, op & 0xF);
                        0
                    }
                    0xA0 => {
                        self.write_reg_param(t, op & 0xF);
                        0
                    }
                    0xB0 => self.reg_cmd(t, op & 8 != 0, (op & 7) as u32, h),
                    _ => self.cmd(t, op, 0, h),
                }
            };
            match r {
                0 => continue,
                1 => return 0,
                2 => return -2,
                3 => return -1,
                _ => continue,
            }
        }
        // A runaway loop with no waits; treat it as finished.
        -1
    }

    /// `RegCmd_Process`
    fn reg_cmd(&mut self, t: usize, reg: bool, n: u32, h: &mut dyn SyncHandler) -> i32 {
        let mut op = self.rb(t);
        if reg {
            op = self.exchange(t, op) as u8;
        }
        let mut types = 0u16;
        if !reg || n != 0 {
            let mut b = self.rb(t);
            let mut mask = 3u16;
            for _ in 0..=n {
                if b & 0x80 != 0 {
                    types |= mask;
                }
                b <<= 1;
                mask <<= 2;
            }
        }
        self.cmd(t, op, types, h)
    }

    /// `Cmd_Process`: reads the arguments `Arglist` describes, then runs the command.
    fn cmd(&mut self, t: usize, op: u8, extra: u16, h: &mut dyn SyncHandler) -> i32 {
        if op < 0xC0 {
            return 0;
        }
        let (count, types) = ARGLIST[(op - 0xC0) as usize];
        let mut types = types | extra;
        let mut a = [0u32; 8];
        for slot in a.iter_mut().take(count as usize) {
            *slot = match types & 3 {
                0 => self.rb(t) as u32,
                1 => self.r16(t) as u32,
                2 => self.r24(t),
                _ => {
                    let r = self.rb(t);
                    self.exchange(t, r)
                }
            };
            types >>= 2;
        }
        match op - 0xC0 {
            1 | 2 => {
                let owner = if op == 0xC1 {
                    Some(t)
                } else {
                    self.tracks[t].parent
                };
                let Some(owner) = owner else {
                    return 0;
                };
                let idx = (a[0] & 0xF) as u8;
                let mode = if a[0] & 0x20 != 0 {
                    4
                } else {
                    ((a[0] >> 6) & 3) as u8
                };
                if let Some(c) = self.open_track(owner, idx) {
                    self.start_track(c, owner, idx, mode, a[1]);
                }
            }
            4 => {
                let flag = self.rb(t);
                let ofs = self.jump_target(t, flag, false);
                if let Some(ofs) = ofs
                    && self.condition(t, flag)
                {
                    let s = &mut self.tracks[t].seq;
                    if (s.depth as usize) < s.stack.len() {
                        s.stack[s.depth as usize] = s.pos;
                        s.depth += 1;
                        s.pos = ofs;
                    }
                }
            }
            6 => {
                if self.condition(t, a[0] as u8) {
                    let s = &mut self.tracks[t].seq;
                    if s.depth == 0 {
                        return 3;
                    }
                    s.depth -= 1;
                    s.pos = s.stack[s.depth as usize];
                }
            }
            8 => {
                let flag = self.rb(t);
                let ofs = self.jump_target(t, flag, true);
                if let Some(ofs) = ofs
                    && self.condition(t, flag)
                {
                    self.tracks[t].seq.pos = ofs;
                }
            }
            9 => {
                let s = &mut self.tracks[t].seq;
                if (s.depth as usize) < s.stack.len() {
                    s.stack[s.depth as usize] = s.pos;
                    s.loop_timers[s.depth as usize] = a[0] as u16;
                    s.depth += 1;
                }
            }
            10 => {
                // `loopE`
                let s = &mut self.tracks[t].seq;
                if s.depth != 0 {
                    let d = s.depth as usize - 1;
                    let n = s.loop_timers[d].saturating_sub(1);
                    if n == 0 {
                        s.depth -= 1;
                    } else {
                        s.loop_timers[d] = n;
                        s.pos = s.stack[d];
                    }
                }
            }
            11 => {
                let p = (a[0] & 0xF) as usize;
                let port = &mut self.tracks[t].port;
                port.import[p] = false;
                let v = port.value[p];
                self.write_reg(t, a[1] as u8, v);
            }
            12 => {
                let port = &mut self.tracks[t].port;
                let p = (a[0] & 0xF) as usize;
                port.export[p] = true;
                port.value[p] = a[1] as u16;
            }
            13 => {
                let v = self.tracks[t].port.import[(a[0] & 0xF) as usize] as u16;
                self.write_reg(t, 3, v);
            }
            14 => {
                let v = self.tracks[t].port.export[(a[0] & 0xF) as usize] as u16;
                self.write_reg(t, 3, v);
            }
            15 | 42 => {
                self.tracks[t].seq.wait = a[0] as i32;
                return (a[0] != 0) as i32;
            }
            16 => self.tracks[t].connect_name = a[0] << 16 | a[1],
            17 => {
                if let Some(p) = self.tracks[t].parent {
                    self.write_port_direct(p, (a[0] & 0xF) as usize, a[1] as u16);
                }
            }
            18 => {
                if let Some(c) = self.tracks[t].children[((a[0] >> 4) & 0xF) as usize] {
                    self.write_port_direct(c, (a[0] & 0xF) as usize, a[1] as u16);
                }
            }
            20 => {
                let k = (a[0] as u8).wrapping_add(self.tracks[t].transpose_total as u8);
                self.tracks[t].notes.last_note = k;
            }
            21 => self.tracks[t].time_relate = a[0] != 0,
            22 => self.osc_setup_simple(t, a[0] as u8),
            23 => self.osc_setup_simple_env(t, a[0] as u8, a[1]),
            24 => {
                let tr = &mut self.tracks[t];
                tr.ads_table[0][1] = a[0] as i16;
                tr.ads_table[1][1] = a[1] as i16;
                tr.ads_table[2][1] = a[2] as i16;
                tr.ads_table[2][2] = a[3] as i16;
                tr.rel_table[0][1] = a[4] as i16;
                tr.osc_data[0] = Osc {
                    ads: Some(tr.ads_table.as_slice().into()),
                    rel: Some(tr.rel_table.as_slice().into()),
                    ..presets::adsr()
                };
            }
            25 => {
                let tr = &mut self.tracks[t];
                tr.transpose = a[0] as u8 as i8;
                let pt = tr.parent;
                let pt = pt.map_or(0, |p| self.tracks[p].transpose);
                let tr = &mut self.tracks[t];
                tr.transpose_total = tr.transpose.wrapping_add(pt);
            }
            26 => {
                let i = (a[0] & 0xF) as usize;
                if let Some(c) = self.tracks[t].children[i] {
                    self.close_track(c);
                    self.tracks[t].children[i] = None;
                }
            }
            27 => {
                self.outer[t].switch = a[0] as u16;
                self.outer[t].update = 0xFFFF;
            }
            28 => self.update_track(t, a[0]),
            29 => {
                if a[0] < 6 {
                    self.mgrs[t].bus[a[0] as usize] = a[1] as u16;
                }
            }
            30 => self.tracks[t].pause_status = a[0] as u8,
            31 => {
                let i = &mut self.tracks[t].intr;
                let n = (a[0] & 7) as usize;
                i.mask |= 1 << n;
                i.vectors[n] = a[1];
            }
            32 => self.tracks[t].intr.mask &= !(1 << (a[0] & 7)),
            33 => {
                self.tracks[t].intr.enabled = true;
                self.tracks[t].seq.intr_return = None;
            }
            34 => self.tracks[t].intr.enabled = false,
            35 => {
                self.tracks[t].intr.enabled = true;
                let s = &mut self.tracks[t].seq;
                if let Some(r) = s.intr_return.take() {
                    s.wait = s.saved_wait;
                    s.pos = r;
                }
                return 2;
            }
            36 => {
                let i = &mut self.tracks[t].intr;
                i.timer_count = a[0] as u8;
                i.timer = a[1];
                i.timer_reload = a[1];
            }
            37 => {
                let name = self.tracks[t].connect_name;
                if !self.registry.iter().any(|e| e.0 == name) {
                    if let Some(e) = self.registry.iter_mut().find(|e| e.1.is_none()) {
                        *e = (name, Some(t));
                    } else if self.registry.len() < 32 {
                        self.registry.push((name, Some(t)));
                    }
                }
                self.tracks[t].connected = true;
            }
            38 => {
                if self.tracks[t].connected {
                    for e in &mut self.registry {
                        if e.1 == Some(t) {
                            e.1 = None;
                        }
                    }
                    self.tracks[t].connected = false;
                }
            }
            39 => {
                let v = h.sync(self, t, a[0] as u16);
                self.write_reg(t, 3, v);
            }
            40 => self.flush_all(t),
            41 => self.stop_all_release(t),
            43 => {
                let r = &mut self.tracks[t].regs;
                r[8] = a[0] as u16;
                r[9] = a[1] as u16;
                r[10] = a[2] as u16;
                r[11] = (a[3] as f32 * 327.67) as u16;
                r[12] = (a[4] as f32 * 327.67) as u16;
            }
            44 => {
                for i in 0..4 {
                    let m = &mut self.tracks[t].timed[timed::IIR0 + i];
                    m.target = a[i] as u16 as i16 as f32 / 32768.0;
                    m.current = m.target;
                    m.step = 0.0;
                    m.time = 1.0;
                }
            }
            45 => {
                let fir: [i16; 8] =
                    std::array::from_fn(|i| self.get_n(t, a[0] + 2 * i as u32, 2) as i16);
                let o = &mut self.outer[t];
                o.update |= update::FIR as u16;
                o.switch |= update::FIR as u16;
                o.fir = fir;
            }
            46 => {
                // `EXTSet` points the outer parameters at a buffer in the sequence; a fresh
                // buffer has every switch off.
                self.outer[t].switch = 0;
                self.outer[t].update = 0;
            }
            47 => {
                const CALC: [u8; 7] = [0, 0, 0, 1, 1, 2, 2];
                const PARENT: [u8; 7] = [0, 1, 2, 0, 2, 0, 2];
                for i in 0..3 {
                    let k = ((a[i] >> 5) as usize).min(6);
                    self.tracks[t].pan_switch_ext[i] = CALC[k];
                    self.tracks[t].pan_switch_parent[i] = PARENT[k];
                    self.mgrs[t].calc[i] = (a[i] & 0x1F) as u8;
                    self.tracks[t].pending_update |= update::PAN;
                }
            }
            48 => {
                let i = ((a[0] >> 4) & 0xF) as usize;
                let v = (a[0] & 0xF) as u8;
                if i < 2 {
                    self.tracks[t].osc_mode[i] = v;
                    if v == 0xE {
                        let od = self.tracks[t].osc_data[i].clone();
                        self.tracks[t].oscs[i].init_start(Some(&od));
                    }
                }
            }
            49 => {
                let row = cutoff_to_iir(a[0] as u8);
                for (i, v) in row.into_iter().enumerate() {
                    let m = &mut self.tracks[t].timed[timed::IIR0 + i];
                    m.target = v as f32 / 32767.0;
                    m.current = m.target;
                    m.step = 0.0;
                    m.time = 1.0;
                }
            }
            50 => self.osc_setup_full(t, a[0] as u8, a[1], a[2]),
            51 => self.tracks[t].volume_mode = a[0] as u8,
            58 => self.write_reg(t, 3, 0),
            59 => self.skip_printf(t),
            61 => {
                self.tracks[t].tempo = a[0] as u16;
                if self.tracks[t].parent.is_none() {
                    self.update_tempo(t);
                } else {
                    self.tracks[t].own_tempo = true;
                }
            }
            62 => {
                self.tracks[t].time_base = a[0] as u16;
                if self.tracks[t].parent.is_none() {
                    self.update_tempo(t);
                }
            }
            63 => {
                let mut flags = 0;
                for i in 0..timed::COUNT {
                    let m = &mut self.tracks[t].timed[i];
                    if m.time > 0.0 {
                        m.current += m.step;
                        m.time -= 1.0;
                        let v = m.current;
                        if i <= timed::DIST_FILTER || i >= timed::OSC1_VERTEX {
                            flags |= 1 << i;
                        } else {
                            self.osc_update_param(t, i, v);
                        }
                    }
                }
                self.update_seq(t, flags, true);
                return 3;
            }
            _ => {}
        }
        0
    }

    /// The target of `cmdCall` / `cmdJmp`: a u24, a register, or an entry of a u24 table.
    fn jump_target(&mut self, t: usize, flag: u8, jmp: bool) -> Option<u32> {
        if flag & 0x80 == 0 {
            return Some(self.r24(t));
        }
        let c = self.rb(t);
        if flag & 0x40 != 0 {
            let idx = self.read_reg(t, c) as u32;
            let base = if flag & 0x20 != 0 {
                let r = self.rb(t);
                self.read_reg(t, r) as u32
            } else {
                self.r24(t)
            };
            return Some(self.get_n(t, base + idx * 3, 3));
        }
        if jmp {
            Some(self.read_reg32(t, c))
        } else {
            Some(self.read_reg(t, c) as u32)
        }
    }

    /// `cmdPrintf`: consumes the format string and its register arguments.
    fn skip_printf(&mut self, t: usize) {
        let mut args = 0;
        for _ in 0..128 {
            let c = self.rb(t);
            if c == 0 {
                break;
            }
            if c == b'\\' {
                if self.rb(t) == 0 {
                    break;
                }
                continue;
            }
            if c == b'%' {
                if self.rb(t) == 0 {
                    break;
                }
                args += 1;
            }
        }
        for _ in 0..args {
            self.rb(t);
        }
    }

    /// `cmdNoteOff` (0x81-0x8F except 0x88, and 0xF9).
    fn cmd_note_off(&mut self, t: usize, mut op: u8) -> i32 {
        if op == 0xF9 {
            let b = self.rb(t);
            let v = self.exchange(t, b & 7) as u8;
            if v > 7 || v == 0 {
                if b & 0x80 != 0 {
                    self.rb(t);
                }
                return 0;
            }
            op = v + 0x80;
            if b & 0x80 != 0 {
                op |= 8;
            }
        }
        let mut voice = op & 0xF;
        let mut release = 0u16;
        if voice & 8 != 0 {
            voice -= 8;
            let r = self.rb(t) as u16;
            release = if r > 100 { (r - 98) * 20 } else { r };
        }
        self.track_note_off(t, voice, release);
        0
    }

    /// `cmdNoteOn` (0x00-0x7F).
    fn cmd_note_on(&mut self, t: usize, note: u8) -> i32 {
        let tt = self.tracks[t].transpose_total as u8;
        let mut key = note.wrapping_add(tt);
        let flags = self.rb(t);
        if flags & 0x80 != 0 {
            key = (self.exchange(t, key) as u8).wrapping_add(tt);
        }
        let connect = (flags >> 5) & 3;
        let length_bytes = (flags >> 3) & 3;
        let voice_bits = flags & 7;
        let mut sweep_to = 0u8;
        if connect & 2 != 0 {
            sweep_to = key;
            key = self.tracks[t].notes.last_note;
        }
        let mut vel = self.rb(t);
        if vel >= 0x80 {
            vel = self.exchange(t, vel - 0x80) as u8;
        }
        let (voice, gate, ticks): (u8, u8, u32);
        if voice_bits == 0 {
            voice = 0;
            let mut g = self.rb(t);
            if g >= 0x80 {
                g = self.exchange(t, g - 0x80) as u8;
            }
            gate = g;
            let mut d = 0u32;
            for _ in 0..length_bytes {
                d = d << 8 | self.rb(t) as u32;
            }
            if length_bytes == 1 && d >= 0x80 {
                d = self.exchange(t, (d - 0x80) as u8);
            }
            ticks = d;
        } else {
            let mut v = voice_bits;
            if length_bytes != 0 {
                v = self.exchange(t, v - 1) as u8;
            }
            voice = v;
            ticks = u32::MAX;
            gate = 100;
        }
        if voice >= 8 {
            return 0;
        }
        self.tracks[t].notes.connect = connect;
        let tr = &self.tracks[t];
        let paused_notes = tr.paused && tr.pause_status & 0x10 != 0;
        let mut len = ticks as i32;
        if tr.notes.before_tie {
            if connect & 1 != 0 {
                len = -1;
            }
            if len != -1 {
                len = self.seq_time_to_dsp(t, len, gate);
            }
            if !paused_notes {
                self.track_gate_on(t, voice, key as i32, vel as i32, len);
            }
        } else {
            if len != -1 {
                len = self.seq_time_to_dsp(t, len, gate);
            }
            if connect & 1 != 0 {
                len = -1;
            }
            if !paused_notes {
                self.track_note_on(t, voice, key as i32, vel as i32, len);
            }
        }
        self.tracks[t].notes.base_time = ticks;
        self.tracks[t].notes.before_tie = connect & 1 != 0;
        if connect & 2 != 0 {
            if len == -1 {
                len = self.seq_time_to_dsp(t, ticks as i32, gate);
            }
            if let Some(c) = self.note_channel(t, 0) {
                let target = sweep_to.wrapping_add(self.tracks[t].transpose_total as u8);
                self.channel_set_key_sweep(c, target, len.max(0) as u32);
            }
            key = sweep_to;
        }
        self.tracks[t].notes.last_note = key;
        if ticks == u32::MAX {
            return 0;
        }
        self.tracks[t].seq.wait = if ticks != 0 { ticks as i32 } else { -1 };
        1
    }
}

/// `unk110 * 128` as an s8: positive delays the back channels, negative the front.
fn pan_delays(v: f32) -> (u8, u8) {
    let d = (v * 128.0).clamp(-128.0, 127.0) as i8;
    if d < 0 {
        ((-(d as i16)) as u8, 0)
    } else {
        (0, d as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cutoff_table_matches_known_rows() {
        // Rows of `CUTOFF_TO_IIR_TABLE`.
        let rows: [(u8, [i16; 4]); 7] = [
            (0, [0x0F5C, 0x0A3D, 0x4665, 0x3999]),
            (1, [0x103F, 0x0A28, 0x45D7, 0x3925]),
            (2, [0x1122, 0x0A14, 0x454A, 0x38B0]),
            (21, [0x21FC, 0x088B, 0x3AC1, 0x3012]),
            (64, [0x481F, 0x0514, 0x22EB, 0x1C92]),
            (126, [0x7F1B, 0x0014, 0x008D, 0x0074]),
            (127, [0x7FFF, 0, 0, 0]),
        ];
        for (i, want) in rows {
            let got = cutoff_to_iir(i);
            for k in 0..4 {
                assert!(
                    (got[k] - want[k]).abs() <= 1,
                    "row {i}: {got:?} vs {want:?}"
                );
            }
        }
    }
}
