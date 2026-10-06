//! Envelopes and LFOs: `TOscillator` (JASOscillator.cpp) and the preset `Osc_`s
//! (JASPlayer_impl.cpp, JASBankMgr.cpp).
//!
//! A table is (mode, time, value) triplets. Mode 0 moves linearly to `value / 32768` over
//! `time` (1/600 s units), modes 1-3 follow the square, square-root and "sample cell" release
//! curves; 13 jumps to entry `value`, 14 sustains, 15 stops. The oscillator's output is
//! `vertex + phase * width`.

use std::sync::{Arc, LazyLock};

use libsms_assets::sound::bank;

use crate::calc::{DAC_RATE, SUBFRAME};

/// Flat (mode, time, value) triplets.
pub type Table = Arc<[[i16; 3]]>;

/// `TOscillator::Osc_`
#[derive(Debug, Clone, PartialEq)]
pub struct Osc {
    /// 0 volume, 1 pitch, 2 pan, 3 fx, 4 dolby.
    pub target: u8,
    pub rate: f32,
    pub ads: Option<Table>,
    pub rel: Option<Table>,
    pub width: f32,
    pub vertex: f32,
}

impl Osc {
    pub fn from_bank(o: &bank::Osc) -> Osc {
        Osc {
            target: o.target,
            rate: o.rate,
            ads: o.attack.as_ref().map(|t| t.as_slice().into()),
            rel: o.release.as_ref().map(|t| t.as_slice().into()),
            width: o.width,
            vertex: o.vertex,
        }
    }

    fn same_tables(&self) -> bool {
        match (&self.ads, &self.rel) {
            (Some(a), Some(r)) => Arc::ptr_eq(a, r),
            (None, None) => true,
            _ => false,
        }
    }
}

/// The preset oscillators and tables.
pub mod presets {
    use super::*;

    fn table(t: &[[i16; 3]]) -> Table {
        t.into()
    }

    /// `Player::sAdsTable`: a track's SimpleADSR template.
    pub const ADS_TABLE: [[i16; 3]; 4] = [[0, 0, 0x7FFF], [0, 0, 0x7FFF], [0, 0, 0], [0xE, 0, 0]];
    /// `Player::sRelTable`
    pub static REL_TABLE: LazyLock<Table> = LazyLock::new(|| table(&[[0, 0xA, 0], [0xF, 1, 0]]));
    static VIB_TABLE: LazyLock<Table> = LazyLock::new(|| {
        table(&[
            [0, 0, 0],
            [0, 0xC, 0x7FFF],
            [0, 0xC, 0],
            [0, 0xC, -0x4000],
            [0, 0xC, 0],
            [0xD, 0, 1],
        ])
    });
    static TRE_TABLE: LazyLock<Table> = LazyLock::new(|| {
        table(&[
            [0, 0, 0x7FFF],
            [0, 0x14, 0],
            [0, 0x14, -0x7FFF],
            [0, 0x14, 0],
            [0, 0x14, 0x7FFF],
            [0xD, 0, 1],
        ])
    });
    /// `BankMgr::OSC_RELEASE_TABLE`
    static OSC_RELEASE: LazyLock<Table> = LazyLock::new(|| table(&[[1, 0xA, 0], [0xF, 0, 0]]));

    /// `Player::sAdsrDef`
    pub fn adsr() -> Osc {
        Osc {
            target: 0,
            rate: 1.0,
            ads: None,
            rel: None,
            width: 1.0,
            vertex: 0.0,
        }
    }

    /// `Player::sEnvelopeDef`
    pub fn envelope() -> Osc {
        Osc {
            rel: Some(REL_TABLE.clone()),
            ..adsr()
        }
    }

    /// `Player::sVibratoDef`
    pub fn vibrato() -> Osc {
        Osc {
            target: 1,
            rate: 0.5,
            ads: Some(VIB_TABLE.clone()),
            rel: Some(VIB_TABLE.clone()),
            width: 0.0,
            vertex: 1.0,
        }
    }

    /// `Player::sTremoroDef`
    pub fn tremolo() -> Osc {
        Osc {
            target: 0,
            rate: 0.5,
            ads: Some(TRE_TABLE.clone()),
            rel: Some(TRE_TABLE.clone()),
            width: 0.0,
            vertex: 1.0,
        }
    }

    /// `BankMgr::OSC_ENV`: the envelope of oscillator (synth) notes.
    pub fn osc_env() -> Osc {
        Osc {
            rel: Some(OSC_RELEASE.clone()),
            ..adsr()
        }
    }

    /// The envelope `TDrumSet::getParam` gives every percussion note.
    pub fn perc() -> Osc {
        adsr()
    }
}

/// `Driver::FORCE_STOP_TABLE`
const FORCE_STOP: [[i16; 3]; 2] = [[0, 15, 0], [15, 0, 0]];

/// `relTableSquare`, `relTableSqRoot` and `relTableSampleCell`, indexed by curve mode 1-3.
#[allow(clippy::approx_constant)]
const CURVES: [[f32; 17]; 3] = [
    [
        1.0, 0.968_246, 0.935_414, 0.901_388, 0.866_025, 0.829_156, 0.790_569, 0.75, 0.707_107,
        0.661_438, 0.612_372, 0.559_017, 0.5, 0.433_013, 0.353_553, 0.25, 0.0,
    ],
    [
        1.0,
        0.878_906,
        0.765_625,
        0.660_156,
        0.5625,
        0.472_656,
        0.390_625,
        0.316_406,
        0.25,
        0.191_406,
        0.140_625,
        0.097_656_2,
        0.0625,
        0.035_156_2,
        0.015_625,
        0.003_906_25,
        0.0,
    ],
    [
        1.0,
        0.970_489,
        0.781_274,
        0.546_281,
        0.399_792,
        0.289_315,
        0.212_104,
        0.157_476,
        0.112_613,
        0.081_789_6,
        0.057_985_2,
        0.043_641_5,
        0.030_823_7,
        0.023_712_9,
        0.015_259_3,
        0.009_155_55,
        0.0,
    ],
];

/// Table time units (1/600 s) to channel updates (one per subframe).
fn to_updates(time: f32) -> f32 {
    time * (DAC_RATE / SUBFRAME as f32 / 600.0)
}

/// `TOscillator`: the running state. The `Osc` it follows is passed in on each call, since a
/// channel's oscillator can follow a track's live `mOscData`.
#[derive(Debug, Clone, Default)]
pub struct Oscillator {
    /// 0 stopped, 1 fresh, 2 attack/decay, 3 sustain, 4 release, 5 force stop, 6 direct release.
    pub state: u8,
    curve: u8,
    index: u16,
    release_rate: f32,
    pub phase: f32,
    target_phase: f32,
    change_rate: f32,
    direct_release: u16,
    initial_release_rate: f32,
}

impl Oscillator {
    /// `TOscillator::init`
    pub fn init(&mut self) {
        *self = Oscillator {
            state: 1,
            ..Default::default()
        };
    }

    /// `initStart`
    pub fn init_start(&mut self, osc: Option<&Osc>) {
        self.state = 2;
        self.direct_release = 0;
        let Some(o) = osc.filter(|o| o.ads.is_some()) else {
            self.phase = 0.0;
            return;
        };
        self.index = 0;
        self.release_rate = -o.rate;
        self.target_phase = 0.0;
    }

    /// `releaseDirect`
    pub fn release_direct(&mut self, release: u16) {
        self.direct_release = release;
    }

    /// `getOffset`: advances one update and returns the value.
    pub fn offset(&mut self, osc: Option<&Osc>) -> f32 {
        let Some(o) = osc else {
            self.phase = 1.0;
            return 1.0;
        };
        match self.state {
            0 => return 1.0,
            3 => return o.vertex + self.phase * o.width,
            1 => self.state = 2,
            _ => {}
        }
        let table: Option<&[[i16; 3]]> = match self.state {
            4 => o.rel.as_deref(),
            5 => Some(&FORCE_STOP),
            _ => o.ads.as_deref(),
        };
        if table.is_none() && self.state != 6 {
            self.phase = 1.0;
            return 1.0;
        }
        self.release_rate -= if self.state == 5 { 1.0 } else { o.rate };
        self.calc(o, table.unwrap_or(&[]))
    }

    /// `getOffsetNoCount`: the value without advancing.
    pub fn offset_no_count(&mut self, osc: Option<&Osc>) -> f32 {
        let Some(o) = osc else {
            self.phase = 1.0;
            return 1.0;
        };
        match self.state {
            0 => return 1.0,
            1 => self.state = 2,
            _ => {}
        }
        self.phase * o.width + o.vertex
    }

    /// `forceStop`
    pub fn force_stop(&mut self) -> bool {
        if self.state == 5 {
            return false;
        }
        self.index = 0;
        self.release_rate = 0.0;
        self.target_phase = self.phase;
        self.state = 5;
        true
    }

    /// `release`
    pub fn release(&mut self, o: &Osc) -> bool {
        if self.state == 5 {
            return false;
        }
        if !o.same_tables() {
            self.index = 0;
            self.release_rate = 0.0;
            self.target_phase = self.phase;
        }
        if o.rel.is_none() && self.direct_release == 0 {
            self.direct_release = 0x10;
        }
        if self.direct_release != 0 {
            self.state = 6;
            self.curve = ((self.direct_release >> 14) & 3) as u8;
            self.release_rate = to_updates((self.direct_release & 0x3FFF) as f32).max(1.0);
            self.initial_release_rate = self.release_rate;
            self.target_phase = 0.0;
            self.change_rate = if self.curve == 0 {
                (self.target_phase - self.phase) / self.release_rate
            } else {
                self.target_phase - self.phase
            };
        } else {
            self.state = 4;
        }
        true
    }

    /// `calc`
    fn calc(&mut self, o: &Osc, table: &[[i16; 3]]) -> f32 {
        let mut guard = 0;
        while self.release_rate <= 0.0 {
            self.phase = self.target_phase;
            if self.state == 6 {
                self.state = 0;
                break;
            }
            guard += 1;
            // Past the table's end (or a jump loop with no time) reads as a stop.
            let [mode, time, value] = match table.get(self.index as usize) {
                Some(&e) if guard < 256 => e,
                _ => [15, 0, 0],
            };
            match mode {
                13 => {
                    self.index = value as u16;
                    continue;
                }
                15 => {
                    self.state = 0;
                    break;
                }
                14 => {
                    self.state = 3;
                    return o.vertex + self.phase * o.width;
                }
                _ => {}
            }
            self.curve = mode as u8;
            if time == 0 {
                self.target_phase = value as f32 / 32768.0;
                self.index += 1;
                continue;
            }
            self.release_rate = to_updates(time as f32);
            self.initial_release_rate = self.release_rate;
            self.target_phase = value as f32 / 32768.0;
            self.change_rate = if self.curve == 0 {
                (self.target_phase - self.phase) / self.release_rate
            } else {
                self.target_phase - self.phase
            };
            self.index += 1;
        }
        if o.width == 0.0 {
            return o.vertex;
        }
        let p = if self.initial_release_rate == 0.0 {
            self.target_phase
        } else if self.curve == 0 || self.change_rate == 0.0 {
            self.target_phase - self.change_rate * self.release_rate
        } else if (1..=3).contains(&self.curve) {
            let t = &CURVES[self.curve as usize - 1];
            let ratio = self.release_rate / self.initial_release_rate;
            let f = if self.change_rate < 0.0 {
                16.0 * (1.0 - ratio)
            } else {
                16.0 * ratio
            };
            let (mut i, mut frac) = (f.max(0.0) as usize, f - (f.max(0.0) as usize) as f32);
            if i >= 16 {
                i = 15;
                frac = 1.0;
            }
            let v = (self.change_rate * (frac * (t[i + 1] - t[i]) + t[i])).abs();
            if self.change_rate < 0.0 {
                self.target_phase + v
            } else {
                self.target_phase - (self.change_rate - v)
            }
        } else {
            self.target_phase - self.change_rate * self.release_rate
        };
        self.phase = p;
        p * o.width + o.vertex
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_holds_then_releases_to_zero() {
        let o = presets::envelope();
        let mut s = Oscillator::default();
        s.init();
        s.init_start(Some(&o));
        // No attack table: full level.
        assert_eq!(s.offset(Some(&o)), 1.0);
        assert!(s.release(&o));
        assert_eq!(s.state, 4);
        // Falls over `sRelTable`'s 10 units, then stops (a stopped oscillator reads 1.0; the
        // channel frees itself when its envelope's state reaches 0).
        let mut vals = Vec::new();
        while s.state != 0 && vals.len() < 50 {
            vals.push(s.offset(Some(&o)));
        }
        assert!(vals.windows(2).all(|w| w[1] <= w[0]), "{vals:?}");
        assert!(vals[vals.len() - 2] < 0.2 && vals.len() < 12, "{vals:?}");
    }

    #[test]
    fn vibrato_loops() {
        let o = presets::vibrato();
        let mut s = Oscillator::default();
        s.init();
        s.init_start(Some(&o));
        let vals: Vec<f32> = (0..200).map(|_| s.offset(Some(&o))).collect();
        // Width 0 holds the vertex until a sequence sets a depth.
        assert!(vals.iter().all(|&v| v == 1.0));
        let deep = Osc { width: 0.1, ..o };
        let vals: Vec<f32> = (0..200).map(|_| s.offset(Some(&deep))).collect();
        let (lo, hi) = vals
            .iter()
            .fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
        assert!(hi > 1.09 && lo < 0.96, "{lo} {hi}");
        assert_ne!(s.state, 0);
    }

    #[test]
    fn direct_release_curve_ends_at_zero() {
        let o = presets::adsr();
        let mut s = Oscillator::default();
        s.init();
        s.init_start(Some(&o));
        s.phase = 1.0;
        s.release_direct(0x4000 | 30);
        s.release(&o);
        assert_eq!(s.state, 6);
        let mut n = 0;
        while s.state != 0 && n < 100 {
            s.offset(Some(&o));
            n += 1;
        }
        assert!(s.phase.abs() < 1e-4 && n > 10, "{} {n}", s.phase);
    }
}
