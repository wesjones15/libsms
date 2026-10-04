//! Controller input and the per-tick "intent" bits the state machine reads.
//!
//! The decomp folds the pad into `mInput` bits in `TMario::checkController`
//! (Player/MarioMove.cpp). We keep those semantics but name the bits.

use glam::Vec2;

use crate::math::{self, Angle};
use crate::params::Controller;

/// Raw pad state for one tick. Hosts fill this from whatever device they have.
/// Edges ("pressed this tick") are derived by the sim by comparing with the previous tick.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Pad {
    /// Main stick on the unit disc (`JUTGamePad::CStick`: raw/54, clamped to length 1);
    /// +x right, +y up (away from the camera). See [`Pad::stick_from_gc_raw`].
    pub stick: Vec2,
    /// Jump (GameCube A).
    pub a: bool,
    /// Dive in the air, and a held modifier on the ground (GameCube B).
    pub b: bool,
    /// Left trigger fully clicked: ground pound in the air, crouch with FLUDD (GameCube L).
    pub l: bool,
    /// Right trigger fully clicked: crouch with FLUDD (GameCube R, digital).
    pub r: bool,
    /// Left trigger analog, raw 0..=255.
    pub l_analog: u8,
    /// Right trigger analog, raw 0..=255. Drives the spray pressure.
    pub r_analog: u8,
    /// Switch FLUDD nozzle (GameCube X).
    pub x: bool,
}

impl Pad {
    /// Converts a GameCube main-stick reading (`s8` per axis, about +-72 at the rim) to
    /// [`Pad::stick`] the way the SDK does: divide by 54 and clamp to the unit disc.
    pub fn stick_from_gc_raw(x: i8, y: i8) -> Vec2 {
        let v = Vec2::new(x as f32, y as f32) / 54.0;
        if v.length() > 1.0 { v.normalize() } else { v }
    }
}

/// What the state machine sees this tick (decomp `mInput` bits).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Input {
    /// 0x1: stick pushed past the dead zone (`mIntendedMag > 0`).
    pub stick: bool,
    /// 0x2: jump button pressed this tick.
    pub jump: bool,
    /// 0x4: standing more than 100 units above the floor.
    pub airborne: bool,
    /// 0x8: the ground under Mario is steep enough to slip on.
    pub slip: bool,
    /// 0x20: neither stick nor jump.
    pub idle: bool,
    /// 0x80: jump button held.
    pub jump_held: bool,
    /// 0x100: R trigger pressed this tick (only with FLUDD and not mid-dash).
    pub r_pressed: bool,
    /// 0x200: R or L held (only with FLUDD and not mid-dash).
    pub lr_held: bool,
    /// 0x2000: B pressed this tick.
    pub b_pressed: bool,
    /// 0x4000: B held.
    pub b_held: bool,
    /// 0x8000: dive/ground-pound trigger: B pressed, or L pressed while in a jumping status.
    pub dive: bool,
    /// L pressed this tick (`checkFrameMeaning(MEANING_L)`).
    pub l_pressed: bool,
}

/// `TMario::getLRLevel`: maps a raw 0..=255 trigger value to a 0..=1 level with a dead zone,
/// a soft first stage up to `middle_level`, and a linear second stage.
pub fn lr_level(raw: u8, c: &Controller) -> f32 {
    let (zero, middle, max) = (
        c.analog_lr_to_zero_val,
        c.analog_lr_to_middle_val,
        c.analog_lr_to_max_val,
    );
    let mid_lvl = c.analog_lr_middle_level;
    if raw < zero {
        0.0
    } else if raw < middle {
        mid_lvl * (raw - zero) as f32 / (middle - zero) as f32
    } else if raw < max {
        mid_lvl + (1.0 - mid_lvl) * (raw - middle) as f32 / (max - middle) as f32
    } else {
        1.0
    }
}

/// Stick processing from `checkController`: returns `(magnitude, yaw)` where magnitude is
/// `32 * (dist/64)^2` in `[0, 32]` and yaw is the intended facing (stick direction plus camera).
/// `fallback_yaw` is returned when the stick is neutral.
pub fn stick_intent(
    stick: Vec2,
    camera_yaw: Angle,
    fallback_yaw: Angle,
    c: &Controller,
) -> (f32, Angle) {
    // (s16)(128 * pos): truncation toward zero.
    let h = (128.0 * stick.x) as i16;
    let v = (128.0 * stick.y) as i16;

    // Dead zone: ignore |v| <= 7, subtract 6 outside it.
    let dz = |s: i16| -> f32 {
        if s < -7 {
            (s + 6) as f32
        } else if s > 7 {
            (s - 6) as f32
        } else {
            0.0
        }
    };
    let (mut sh, mut sv) = (dz(h), dz(v));

    let mut dist = (sh * sh + sv * sv).sqrt();
    for _ in 0..c.length_mult_times.max(0) {
        dist *= c.length_mult;
    }
    if dist > 64.0 {
        sh *= 64.0 / dist;
        sv *= 64.0 / dist;
        dist = 64.0;
    }

    let norm = dist / 64.0;
    let mag = 32.0 * (norm * norm);
    let yaw = if mag > 0.0 {
        math::matan(-sv, sh).wrapping_add(camera_yaw)
    } else {
        fallback_yaw
    };
    (mag, yaw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_stick_has_no_magnitude() {
        let c = Controller::default();
        let (mag, yaw) = stick_intent(Vec2::ZERO, 123, 77, &c);
        assert_eq!(mag, 0.0);
        assert_eq!(yaw, 77);
        // inside the dead zone
        let (mag, _) = stick_intent(Vec2::new(0.05, 0.0), 0, 0, &c);
        assert_eq!(mag, 0.0);
    }

    #[test]
    fn full_stick_up_faces_away_from_camera() {
        let c = Controller::default();
        let (mag, yaw) = stick_intent(Vec2::new(0.0, 1.0), 0x1000, 0, &c);
        assert!(mag > 0.0 && mag <= 32.0, "{mag}");
        assert_eq!(yaw, 0x1000i16.wrapping_add(i16::MIN));
    }

    #[test]
    fn magnitude_is_quadratic_in_stick() {
        let c = Controller {
            length_mult_times: 0,
            ..Controller::default()
        };
        let (quarter, _) = stick_intent(Vec2::new(0.0, 0.25), 0, 0, &c);
        let (half, _) = stick_intent(Vec2::new(0.0, 0.5), 0, 0, &c);
        // magnitude = 32 * (dist/64)^2: doubling the deflection roughly quadruples it
        assert!(quarter < 0.3 * half, "{quarter} vs {half}");
        // the 64-unit clamp saturates at about half deflection when length_mult is 1
        let (full, _) = stick_intent(Vec2::new(0.0, 1.0), 0, 0, &c);
        assert_eq!(full, 32.0);
    }

    #[test]
    fn gc_raw_stick_clamps_to_the_unit_disc() {
        assert_eq!(Pad::stick_from_gc_raw(0, 0), Vec2::ZERO);
        let rim = Pad::stick_from_gc_raw(72, 72);
        assert!((rim.length() - 1.0).abs() < 1e-5);
        assert_eq!(Pad::stick_from_gc_raw(27, 0), Vec2::new(0.5, 0.0));
    }

    #[test]
    fn trigger_levels() {
        let c = Controller::default();
        assert_eq!(lr_level(0, &c), 0.0);
        assert_eq!(lr_level(255, &c), 1.0);
        let mid = lr_level(c.analog_lr_to_middle_val, &c);
        assert!((mid - c.analog_lr_middle_level).abs() < 1e-6);
        assert!(lr_level(100, &c) > mid);
    }
}
