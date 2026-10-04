//! Angle and interpolation helpers with the decomp's conventions.
//!
//! Angles are `i16` (65536 = a full turn, wrapping arithmetic). A facing angle `a` points
//! along `(sin a, 0, cos a)`, and the angle of an XZ vector is `atan2(x, z)`
//! (MarioUtil/MathUtil.cpp `matan(z, x)`, JMASSin/JMASCos).

use std::f32::consts::PI;

/// Binary angle: 0x4000 = 90 degrees.
pub type Angle = i16;

const TO_RAD: f32 = PI / 32768.0;
const FROM_RAD: f32 = 32768.0 / PI;

pub fn sin(a: Angle) -> f32 {
    (a as f32 * TO_RAD).sin()
}

pub fn cos(a: Angle) -> f32 {
    (a as f32 * TO_RAD).cos()
}

/// Angle of the XZ vector `(x, z)`: `matan(z, x)` in the decomp.
pub fn matan(z: f32, x: f32) -> Angle {
    to_angle(x.atan2(z) * FROM_RAD)
}

/// Converts a (possibly out-of-range) float angle to `Angle` with wrapping.
pub fn to_angle(a: f32) -> Angle {
    (a.round() as i64) as i16
}

/// `a - b` with wrap (the decomp's `(s16)(a - b)`).
pub fn diff(a: Angle, b: Angle) -> Angle {
    a.wrapping_sub(b)
}

/// MathUtil `IConverge`: step `value` toward `target` by `inc` (below) or `dec` (above).
pub fn iconverge(value: i32, target: i32, inc: i32, dec: i32) -> i32 {
    let mut r = value;
    if r < target {
        r += inc;
        if r > target {
            r = target;
        }
    } else {
        r -= dec;
        if r < target {
            r = target;
        }
    }
    r
}

/// MathUtil `FConverge`.
pub fn fconverge(value: f32, target: f32, inc: f32, dec: f32) -> f32 {
    let mut r = value;
    if r < target {
        r += inc;
        if r > target {
            r = target;
        }
    } else {
        r -= dec;
        if r < target {
            r = target;
        }
    }
    r
}

/// The idiom `target - IConverge((s16)(target - current), 0, step, step)`: turn `current`
/// toward `target` by at most `step` per call.
pub fn turn_toward(current: Angle, target: Angle, step: i32) -> Angle {
    let d = diff(target, current) as i32;
    (target as i32 - iconverge(d, 0, step, step)) as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matan_matches_facing_convention() {
        assert_eq!(matan(1.0, 0.0), 0);
        assert_eq!(matan(0.0, 1.0), 0x4000);
        assert_eq!(matan(-1.0, 0.0), i16::MIN);
        assert_eq!(matan(0.0, -1.0), -0x4000);
        assert!((sin(0x4000) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn turn_toward_takes_the_short_way() {
        assert_eq!(turn_toward(0, 1000, 100), 100);
        assert_eq!(turn_toward(0, -1000, 100), -100);
        // across the wrap seam
        assert_eq!(
            turn_toward(0x7f00u16 as i16, 0x8100u16 as i16, 0x40),
            0x7f40u16 as i16
        );
        assert_eq!(turn_toward(5, 5, 100), 5);
    }
}
