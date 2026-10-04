//! Fixed-step clock.
//!
//! `TMarDirector::direct` (System/MarDirectorDirect.cpp) converts each rendered frame into
//! `600 / vsync` time units (vsync = `SMSGetVSyncTimesPerSec()` = 30 NTSC / 25 PAL) and burns
//! them in 5-unit simulation ticks. That is 4 ticks per frame at 30 fps, i.e. **120 ticks per
//! second in both regions**. Mario's physics constants are per tick (`pos += 0.25 * vel`),
//! so we run exactly 120 Hz and ignore the region.

pub const TICK_HZ: u32 = 120;
pub const TICK_DT: f32 = 1.0 / TICK_HZ as f32;

/// Upper bound on ticks per `accumulate` call so a long host stall can't spiral.
const MAX_STEPS_PER_FRAME: u32 = 16;

#[derive(Debug, Default, Clone)]
pub struct FixedClock {
    acc: f32,
}

impl FixedClock {
    /// Adds `dt` seconds and returns how many ticks are now due. Negative or NaN `dt` counts as 0.
    pub fn accumulate(&mut self, dt: f32) -> u32 {
        if dt.is_finite() && dt > 0.0 {
            self.acc += dt;
        }
        let mut steps = 0;
        while self.acc >= TICK_DT && steps < MAX_STEPS_PER_FRAME {
            self.acc -= TICK_DT;
            steps += 1;
        }
        if steps == MAX_STEPS_PER_FRAME {
            self.acc = self.acc.min(TICK_DT);
        }
        steps
    }

    /// Fraction of the way to the next tick, for render interpolation.
    pub fn alpha(&self) -> f32 {
        (self.acc / TICK_DT).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thirty_fps_host_runs_four_ticks_a_frame() {
        let mut c = FixedClock::default();
        let ticks: u32 = (0..30).map(|_| c.accumulate(1.0 / 30.0)).sum();
        assert!((119..=120).contains(&ticks), "{ticks}");
    }

    #[test]
    fn stall_is_capped() {
        let mut c = FixedClock::default();
        assert_eq!(c.accumulate(5.0), MAX_STEPS_PER_FRAME);
    }

    #[test]
    fn bad_dt_is_ignored() {
        let mut c = FixedClock::default();
        assert_eq!(c.accumulate(-1.0), 0);
        assert_eq!(c.accumulate(f32::NAN), 0);
    }
}
