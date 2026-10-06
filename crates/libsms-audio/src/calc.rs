//! JASystem's numeric helpers: rates, the pitch tables, the pan curves and the random
//! generators.

/// `Kernel::gDacRate` (JASRate.cpp): the DSP's output rate.
pub const DAC_RATE: f32 = 32028.5;
/// Samples per DSP subframe. Each subframe runs the sequencers and channel updates once
/// (`DSPBuf::updateDSP`, JASDSPBuf.cpp).
pub const SUBFRAME: usize = 80;
/// `gSubFrames`: subframes per audio frame (`gFrameSamples` = 0x230).
pub const SUBFRAMES: usize = 7;

/// `Driver::C5BASE_PITCHTABLE` (JASDriverTables.cpp): equal temperament around key 60.
pub fn c5base(key: i32) -> f32 {
    2f32.powf((key.clamp(0, 127) - 60) as f32 / 12.0)
}

/// `Player::pitchToCent` (JASPlayer_impl.cpp): a track's pitch parameter, scaled by the
/// bend range register, to a frequency ratio. `s_key_table[n]` is 1.00091^n.
pub fn pitch_to_cent(pitch: f32, range: f32) -> f32 {
    let v = pitch * 4.0 * range;
    let mut whole = v as i16 as i32;
    let mut frac = v - whole as f32;
    if v < 0.0 && frac != 0.0 {
        frac += 1.0;
        whole -= 1;
    }
    if frac >= 1.0 {
        frac -= 1.0;
        whole += 1;
    }
    let fine = ((frac * 64.0) as u16).min(63);
    1.00091f32.powi(fine as i32) * c5base(whole + 0x3C)
}

/// `Calc::sinfT` (JASCalc.cpp): a 256-step quarter sine for equal-power panning.
pub fn sinf_t(x: f32) -> f32 {
    let i = ((x * 256.0) as i32).clamp(0, 256);
    (std::f32::consts::FRAC_PI_2 * i as f32 / 256.0).sin()
}

/// `Calc::sinfDolby2`: the surround curve.
pub fn sinf_dolby2(x: f32) -> f32 {
    let i = ((x * 256.0) as i32).clamp(0, 256);
    (std::f32::consts::FRAC_PI_2 * (0.32612 + 0.34776 * i as f32 / 256.0)).sin()
}

/// `Player::extend8to16`
pub fn extend8to16(v: u8) -> u16 {
    if v & 0x80 != 0 {
        v as u16 + 0xFF00
    } else {
        v as u16
    }
}

/// `Player::getRandomS32` (JASPlayer_impl.cpp), used by the `RANDOM` register op.
#[derive(Debug, Clone)]
pub struct PlayerRandom {
    v0: i32,
    v1: i32,
}

impl Default for PlayerRandom {
    fn default() -> Self {
        PlayerRandom {
            v0: 0x1000,
            v1: 0x5555,
        }
    }
}

impl PlayerRandom {
    pub fn s32(&mut self) -> i32 {
        let r = self
            .v1
            .wrapping_mul(-0x6789_ABCE)
            .wrapping_add(1)
            .wrapping_add(self.v0.wrapping_mul(0x1357_9BDE) >> 4);
        self.v0 = self.v1;
        self.v1 = r;
        r
    }
}

/// `JMath::TRandomFast` (JMath.hpp): the LCG behind `TInstRand` and the JAI layer.
#[derive(Debug, Clone, Default)]
pub struct RandomFast {
    pub value: u32,
}

impl RandomFast {
    pub fn get(&mut self) -> u32 {
        self.value = self.value.wrapping_mul(0x19660D).wrapping_add(0x3C6E_F35F);
        self.value
    }

    /// 0..1
    pub fn ufloat_1(&mut self) -> f32 {
        f32::from_bits((self.get() >> 9) | 0x3F80_0000) - 1.0
    }

    /// (-1, 1)
    pub fn sfloat_1(&mut self) -> f32 {
        2.0 * self.ufloat_1() - 0.9999999
    }

    pub fn uint8(&mut self, n: u8) -> u8 {
        (n as f32 * self.ufloat_1()) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pitch_tables() {
        assert_eq!(c5base(60), 1.0);
        assert!((c5base(0) - 0.03125).abs() < 1e-6);
        assert!((pitch_to_cent(0.0, 12.0) - 1.0).abs() < 1e-6);
        // 1/48 of a full-scale bend at range 12 is one semitone.
        assert!((pitch_to_cent(1.0 / 48.0, 12.0) - c5base(61)).abs() < 1e-5);
        // `s_key_table[63]`
        assert!((1.00091f32.powi(63) - 1.058_981).abs() < 1e-4);
    }

    #[test]
    fn pan_curves() {
        assert_eq!(sinf_t(0.0), 0.0);
        assert!((sinf_t(1.0) - 1.0).abs() < 1e-6);
        assert!((sinf_t(0.5) - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3);
    }
}
