//! Sound modulation by a game value (`JALSeModVolFunk` / `JALSeModPitFunk`, JALModSe.cpp):
//! `startSoundActorWithInfo` passes a number (nozzle pressure, a bounce speed...) that a
//! per-sound curve turns into volume or pitch, and that can gate the sound off entirely.
//! The registrations are `MSoundSE`'s (MSound/MSoundSE.cpp); only the ones Mario and FLUDD
//! use are listed.

/// `JALCalc::CurveSign`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Curve {
    Negative,
    Positive,
    Linear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Volume,
    Pitch,
}

/// One `JALSystem::append` registration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModFunc {
    pub sound: &'static str,
    pub target: Target,
    /// Input range (`unk18`).
    pub x: (f32, f32),
    /// Curve exponent (`unk28`).
    pub exp: f32,
    /// Output range (`unk20`).
    pub y: (f32, f32),
    pub curve: Curve,
    /// Bit 0: gate below the input range; bit 1: gate above it (`unk2D`).
    pub gate: u8,
}

/// The Mario and FLUDD entries of `MSoundSE`'s table.
pub const MODS: &[ModFunc] = &[
    // "放水音": the water sound's pitch follows the nozzle output.
    ModFunc {
        sound: "MSD_SE_PO_WATER_HI",
        target: Target::Pitch,
        x: (0.01, 1.0),
        exp: 6.0,
        y: (0.3, 1.0),
        curve: Curve::Linear,
        gate: 1,
    },
    // "ノーマルノズルのエアノイズ": the Spray's air noise, by trigger pressure.
    ModFunc {
        sound: "MSD_SE_PO_NORMAL_NOZZLE_IMI",
        target: Target::Pitch,
        x: (0.01, 0.56),
        exp: 5.0,
        y: (0.25, 1.5),
        curve: Curve::Positive,
        gate: 1,
    },
    // "マリオしりもち音": Mario's bump, by speed.
    ModFunc {
        sound: "MSD_SE_MA_BOUND",
        target: Target::Volume,
        x: (0.01, 50.0),
        exp: 10.0,
        y: (0.2, 1.0),
        curve: Curve::Positive,
        gate: 1,
    },
];

/// `JALCalc::linearTransform`.
fn linear(x: f32, x0: f32, x1: f32, y0: f32, y1: f32, clamp: bool) -> f32 {
    let r = (x - x0) * ((y1 - y0) / (x1 - x0)) + y0;
    if !clamp {
        return r;
    }
    let (lo, hi) = if y0 < y1 { (y0, y1) } else { (y1, y0) };
    r.clamp(lo, hi)
}

impl ModFunc {
    /// `JALCalc::getParamByExp`.
    pub fn value(&self, x: f32) -> f32 {
        let (x0, x1) = self.x;
        let (y0, y1) = self.y;
        let v = match self.curve {
            Curve::Positive => {
                let p = linear(x, x0, x1, 0.0, self.exp, false).exp();
                linear(p, 1.0, self.exp.exp(), y0, y1, false)
            }
            Curve::Negative => {
                let p = linear(x, x0, x1, self.exp, 0.0, false).exp();
                linear(p, self.exp.exp(), 1.0, y0, y1, false)
            }
            Curve::Linear => linear(x, x0, x1, y0, y1, true),
        };
        v.clamp(y0.min(y1), y1.max(y0))
    }

    /// `gateCheckDyna`: true silences the sound.
    pub fn gated(&self, x: f32) -> bool {
        let (lo, hi) = if self.x.1 >= self.x.0 {
            (self.x.0, self.x.1)
        } else {
            (self.x.1, self.x.0)
        };
        (x > hi && self.gate & 2 != 0) || (x < lo && self.gate & 1 != 0)
    }
}

/// The registrations for a sound name.
pub fn mods_for(name: &str) -> impl Iterator<Item = &'static ModFunc> + '_ {
    MODS.iter().filter(move |m| m.sound == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn water_pitch_follows_output_and_gates_at_zero() {
        let m = mods_for("MSD_SE_PO_WATER_HI").next().unwrap();
        assert!((m.value(1.0) - 1.0).abs() < 1e-6);
        assert!((m.value(0.01) - 0.3).abs() < 1e-6);
        assert!(m.gated(0.0));
        assert!(!m.gated(0.5));
    }

    #[test]
    fn positive_curve_spans_its_range() {
        let m = mods_for("MSD_SE_PO_NORMAL_NOZZLE_IMI").next().unwrap();
        assert!((m.value(0.01) - 0.25).abs() < 1e-4);
        assert!((m.value(0.56) - 1.5).abs() < 1e-4);
        let mid = m.value(0.3);
        assert!(mid > 0.25 && mid < 0.875, "{mid}");
    }
}
