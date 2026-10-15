//! Game-agnostic reimplementation of Super Mario Sunshine's Mario locomotion and FLUDD, using
//! the doldecomp/sms decomp as a behavioral spec. See `docs/behavior-spec.md`.
//!
//! The sim never touches a game directly: everything goes through [`Host`]. Units are the
//! decomp's (about 1 unit = 1 cm, Mario is 160 tall); hosts convert. The sim runs at a fixed
//! 120 Hz ([`TICK_HZ`]).

pub mod anim;
pub mod assets;
pub mod clock;
mod collide;
pub mod damage;
pub mod egg;
pub mod event;
pub mod extract;
pub mod fludd;
pub mod fludd_anim;
pub mod host;
pub mod input;
pub mod limits;
pub mod mario;
pub mod math;
pub mod params;
pub mod prm;
pub mod skeleton;
pub mod tongue;
pub mod water;
pub mod yoshi;

mod air;
pub mod carry;
mod ground;
pub mod hang;
mod physics;
pub mod shine;

pub use anim::{AnimState, AnimTable, ClipInfo, LoopMode};
pub use assets::Assets;
pub use clock::{FixedClock, TICK_DT, TICK_HZ};
pub use damage::{Coin, DamageKind};
pub use egg::{Egg, EggLook, EggState};
pub use event::{EnemyHitKind, Event, FxAt, JumpKind, ParticleCue, SoundCue, WaterEmit};
pub use fludd::{Aim, Fludd, TriggerState};
pub use fludd_anim::FluddLook;
pub use glam::{Vec2, Vec3};
pub use host::{Enemy, FlatHost, Fruit, GroundHit, Host, RoofHit, SurfaceFlags, WallHit};
pub use input::{Input, Pad};
pub use limits::{Limits, WetSlide};
pub use mario::{Mario, Options, Status, UpperState};
pub use params::{NozzleKind, Params};
pub use skeleton::{Clip, ClipLibrary, MtxCalc, Skeleton, Srt, calc_joints};
pub use yoshi::{YoshiLook, YoshiState};

/// A Mario plus the fixed-step clock that drives him.
#[derive(Debug, Clone)]
pub struct Sim {
    pub mario: Mario,
    clock: FixedClock,
}

impl Sim {
    pub fn new(params: Params, anims: AnimTable, pos: Vec3, face_yaw: math::Angle) -> Sim {
        Sim {
            mario: Mario::new(params, anims, pos, face_yaw),
            clock: FixedClock::default(),
        }
    }

    /// Advances by `dt` seconds of real time, running as many 120 Hz ticks as are due with the
    /// same `pad` and camera. Returns the number of ticks run.
    ///
    /// `camera_yaw` is the facing angle of the camera as seen from Mario (pointing from Mario
    /// toward the camera): stick-up then moves Mario away from the camera.
    pub fn advance(
        &mut self,
        host: &mut dyn Host,
        pad: &Pad,
        camera_yaw: math::Angle,
        dt: f32,
    ) -> u32 {
        let n = self.clock.accumulate(dt);
        for _ in 0..n {
            self.mario.tick(host, pad, camera_yaw);
        }
        n
    }

    /// Like [`Self::advance`], with an observer after each tick. Hosts use this to sample
    /// animation cues without missing short clips between display frames.
    pub fn advance_observed<H: Host>(
        &mut self,
        host: &mut H,
        pad: &Pad,
        camera_yaw: math::Angle,
        dt: f32,
        mut after_tick: impl FnMut(&Mario, &mut H),
    ) -> u32 {
        let n = self.clock.accumulate(dt);
        for _ in 0..n {
            self.mario.tick(host, pad, camera_yaw);
            after_tick(&self.mario, host);
        }
        n
    }

    /// Fraction of the way to the next tick, for render interpolation.
    pub fn alpha(&self) -> f32 {
        self.clock.alpha()
    }
}
