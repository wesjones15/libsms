//! Goop on Mario (Player/MarioMove.cpp `checkGraffito`, `thinkDirty`, `dirtyLimitCheck`,
//! `checkSink`; MarioCollision.cpp `damageExec`'s `mDirty += pollutionAmount`; MarioDraw.cpp
//! `addDirty`).
//!
//! * **Standing in goop** (`checkGraffito`): every tick Mario asks the goop under him
//!   ([`Host::goop`]). Brown goop and the other "sinking" kinds count only when his feet and the
//!   points a texel around them are all goop (the game probes 32 units out); then he is
//!   `MARIO_FLAG_DIRTY`. Shadow Mario and Piantissimo never are (`mPlayerType` check).
//! * **Getting dirty** (`thinkDirty`): while on goop, running adds `mIncRunning` a tick, a dive
//!   slide `mIncCatching`, slipping `mIncSlipping`; a goop enemy's hit adds its `mDirty`
//!   (`dmgnamekuri.prm`: 100). Clamped to `mDirtyMax`.
//! * **Washing it off**: wading `mDecSwimming` a tick, a spin jump `mDecRotJump`, wet ground or
//!   water (`MARIO_FLAG_RECENTLY_LEFT_WATER`: FLUDD's water on the ground, a puddle)
//!   `mDecWaterHit`. Hosts add their own with [`Mario::wash`] (another Mario's FLUDD).
//! * **Sinking** (`checkSink`, brown goop only): each tick on it the sink timer climbs; while he
//!   has health it stops at `mSinkDmgDepth` of `mSinkTime`. He takes one floor hit every
//!   `mSinkDmgTime` ticks (`floorDamageExec(1, 3, ..)`: 1 damage, no knock-back). The stick is
//!   scaled down (`checkController`) and jumps are cut (`setStatusToJumping`), each jump also
//!   pulling him partway out. He is drawn sunk by up to `mSinkHeight` ([`Mario::sink_offset`]).
//!   Off the goop or more than 100 units over the floor, the timer resets.
//! * **Looks** (`addDirty`): every body and hand material's TEV konst colour 0 alpha is set to
//!   the dirt, which blends the goop mask (`H_ma_rak.bti`) in; [`Mario::dirt_share`] is that
//!   alpha out of 255 for a host's material.
//!
//! Not ported: the oil kinds' `OIL_*` statuses (Mario slides on [`GoopKind::Slip`] like any
//! goop here), fire and electric goop's own damage, `SINK_LOSER` (a host's health decides
//! that), the footprints (`mPolSizeFootPrint` is 0 on the disc, so they paint nothing).

use crate::event::{Event, SoundCue};
use crate::goop::{GoopKind, TEXEL};
use crate::host::Host;
use crate::mario::{Mario, Status};
use crate::params::Damage;

/// Goop on Mario.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GoopState {
    /// `mDirty`: 0 to `mDirtyMax`.
    pub dirty: f32,
    /// `mSinkTimer`.
    pub sink: f32,
    /// `MARIO_FLAG_DIRTY`: on goop this tick.
    pub on_goop: bool,
    /// `mPollutionTypeStandingOn` when on goop.
    pub kind: Option<GoopKind>,
    /// Shadow Mario and the like: goop never catches him.
    pub immune: bool,
}

impl Mario {
    /// `isSinking`.
    pub fn is_sinking(&self) -> bool {
        self.goop.sink > 0.0
    }

    /// How far down the body is drawn while sinking (`unk318`'s translation, MarioDraw.cpp).
    pub fn sink_offset(&self) -> f32 {
        let g = &self.params.graffito;
        -(self.goop.sink / (g.sink_time.max(1) as f32)) * g.sink_height
    }

    /// The dirt as the materials' konst alpha share (`addDirty`: `color.a = mDirty`).
    pub fn dirt_share(&self) -> f32 {
        (self.goop.dirty / 255.0).clamp(0.0, 1.0)
    }

    /// Adds goop to Mario (`damageExec`'s `pollutionAmount`, a goop enemy's hit).
    pub fn add_dirty(&mut self, amount: f32) {
        if self.goop.immune {
            return;
        }
        self.goop.dirty += amount;
        self.dirty_limit_check();
    }

    /// Washes `amount` of goop off (a host's water: another Mario's FLUDD, rain).
    pub fn wash(&mut self, amount: f32) {
        self.goop.dirty -= amount;
        self.dirty_limit_check();
    }

    /// `dirtyLimitCheck`.
    fn dirty_limit_check(&mut self) {
        self.goop.dirty = self.goop.dirty.clamp(0.0, self.params.dirty.dirty_max);
    }

    /// The share of the stick left while sinking (`checkController`).
    pub(crate) fn sink_move_scale(&self) -> f32 {
        if !self.is_sinking() {
            return 1.0;
        }
        let g = &self.params.graffito;
        let t = 1.0 - self.goop.sink / g.sink_time.max(1) as f32;
        (g.sink_move_max - g.sink_move_min) * t + g.sink_move_min
    }

    /// `setStatusToJumping`'s sinking part: the take-off is cut, and pulls him partway out.
    pub(crate) fn sink_jump(&mut self) {
        if !self.is_sinking() {
            return;
        }
        let g = self.params.graffito.clone();
        let time = g.sink_time.max(1) as f32;
        let t = 1.0 - self.goop.sink / time;
        let scale = (g.sink_jump_rate_max - g.sink_jump_rate_min) * t + g.sink_jump_rate_min;
        self.vel.y *= scale;
        self.forward_vel *= scale;
        self.goop.sink = (self.goop.sink - t * time * g.sink_recover).max(0.0);
    }

    /// `checkGraffito` + `checkSink` + `thinkDirty`, once a tick.
    pub(crate) fn think_goop(&mut self, host: &mut dyn Host) {
        self.check_graffito(host);
        self.check_sink();
        self.think_dirty();
    }

    fn check_graffito(&mut self, host: &dyn Host) {
        if self.goop.immune {
            self.goop.on_goop = false;
            self.goop.kind = None;
            return;
        }
        let feet = glam::Vec3::new(self.pos.x, self.floor_y, self.pos.z);
        let kind = host.goop(self.pos);
        let all_round = |host: &dyn Host| {
            (-1..=1).all(|dz| {
                (-1..=1).all(|dx| {
                    host.goop(feet + glam::Vec3::new(dx as f32 * TEXEL, 0.0, dz as f32 * TEXEL))
                        .is_some()
                })
            })
        };
        // Sink, fire and the glass wall test the feet on the floor; slip and the rest test his
        // body's point. Both want the eight around as well.
        self.goop.on_goop = kind.is_some() && all_round(host);
        self.goop.kind = if self.goop.on_goop { kind } else { None };
        if self.goop.on_goop && self.on_yoshi() {
            // `getOffYoshi(true)`: Yoshi won't wade through goop.
            self.get_off_yoshi(true);
        }
    }

    fn check_sink(&mut self) {
        if self.is_invincible() || self.goop.immune {
            return;
        }
        if self.floor_y + 100.0 < self.pos.y {
            self.goop.sink = 0.0;
            return;
        }
        if self.goop.on_goop && self.goop.kind == Some(GoopKind::Sink) {
            let g = self.params.graffito.clone();
            self.goop.sink += 1.0;
            let depth = g.sink_time as f32 * g.sink_dmg_depth;
            if self.health > 0 && self.goop.sink > depth {
                self.goop.sink = depth;
            }
            if self.ticks % g.sink_dmg_time.max(1) as u64 == 0 {
                // `floorDamageExec(1, 3, 0, mMotorReturn)`.
                let p = Damage {
                    damage: 1,
                    down_type: 3,
                    water_emit: 0,
                    min_speed: 0.0,
                    dirty: 0.0,
                    invincible_time: 0,
                    ..Damage::default()
                };
                self.damage_by(self.pos, &p);
            }
            if self.ticks % 30 == 0 {
                self.events.push(Event::Sound(SoundCue {
                    name: "MSD_SE_MA_SINK_IN_MUD",
                    pos: self.pos,
                    info: None,
                }));
            }
            self.events.push(Event::GoopSink { pos: self.pos });
            return;
        }
        self.goop.sink = 0.0;
    }

    fn think_dirty(&mut self) {
        let d = self.params.dirty.clone();
        if self.goop.on_goop {
            match self.status {
                Status::Run => self.goop.dirty += d.inc_running,
                Status::Catch => self.goop.dirty += d.inc_catching,
                Status::Slip | Status::SlipFore | Status::SlipBack => {
                    self.goop.dirty += d.inc_slipping
                }
                _ => {}
            }
        }
        if self.in_water() {
            self.goop.dirty -= d.dec_swimming;
        }
        if matches!(self.status, Status::LeftRotateJump | Status::RightRotateJump) {
            self.goop.dirty -= d.dec_rot_jump;
        }
        if self.wet_ground {
            self.goop.dirty -= d.dec_water_hit;
        }
        self.dirty_limit_check();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goop::GoopField;
    use crate::host::{FlatHost, GroundHit};
    use crate::input::Pad;
    use crate::{AnimTable, Params};
    use glam::{Vec2, Vec3};

    struct GoopHost {
        flat: FlatHost,
        field: GoopField,
        events: Vec<Event>,
    }

    impl Host for GoopHost {
        fn ground(&self, p: Vec3) -> Option<GroundHit> {
            self.flat.ground(p)
        }
        fn event(&mut self, e: Event) {
            self.events.push(e);
        }
        fn goop(&self, p: Vec3) -> Option<GoopKind> {
            self.field.kind_at(p)
        }
    }

    fn setup() -> (Mario, GoopHost) {
        let mut params = Params::default();
        // The disc's values (graffito.prm, dirty.prm) where the tests lean on them.
        params.graffito.sink_time = 240;
        params.graffito.sink_dmg_time = 90;
        params.graffito.sink_dmg_depth = 0.6;
        params.graffito.sink_move_min = 0.3;
        params.graffito.sink_move_max = 0.3;
        params.dirty.dirty_max = 150.0;
        params.dirty.inc_running = 0.15;
        let m = Mario::new(params, AnimTable::default(), Vec3::ZERO, 0);
        let mut field = GoopField::new();
        field.stamp(&FlatHost::default(), Vec3::ZERO, 2000.0, GoopKind::Sink);
        (m, GoopHost { flat: FlatHost::default(), field, events: Vec::new() })
    }

    #[test]
    fn standing_in_brown_goop_sinks_and_hurts() {
        let (mut m, mut host) = setup();
        let hp = m.health;
        for _ in 0..400 {
            m.tick(&mut host, &Pad::default(), 0);
        }
        assert!(m.goop.on_goop);
        assert!(m.is_sinking());
        // Stops at the damage depth while he has health.
        assert!((m.goop.sink - 240.0 * 0.6).abs() < 1.0, "sink {}", m.goop.sink);
        assert!(m.sink_offset() < -80.0);
        assert!(m.health < hp, "floor hits every 90 ticks");
        assert!(host.events.iter().any(|e| matches!(e, Event::GoopSink { .. })));
    }

    #[test]
    fn running_in_goop_gets_him_dirty_and_a_spin_or_water_washes_it() {
        let (mut m, mut host) = setup();
        let pad = Pad { stick: Vec2::new(0.0, 1.0), ..Pad::default() };
        for _ in 0..240 {
            m.tick(&mut host, &pad, 0);
        }
        assert!(m.goop.dirty > 0.0, "dirt {}", m.goop.dirty);
        // A goop enemy's hit tops him up to the cap.
        m.add_dirty(1000.0);
        assert_eq!(m.goop.dirty, 150.0);
        m.wash(50.0);
        assert_eq!(m.goop.dirty, 100.0);
        // Out of the goop on wet ground the water takes it off.
        host.field.clear();
        m.water_underfoot = true;
        for _ in 0..600 {
            m.tick(&mut host, &Pad::default(), 0);
        }
        assert_eq!(m.goop.dirty, 0.0);
        assert!(!m.is_sinking());
    }

    #[test]
    fn shadow_mario_is_never_caught() {
        let (mut m, mut host) = setup();
        m.goop.immune = true;
        m.add_dirty(100.0);
        for _ in 0..300 {
            m.tick(&mut host, &Pad::default(), 0);
        }
        assert!(!m.goop.on_goop && !m.is_sinking());
        assert_eq!(m.goop.dirty, 0.0);
    }

    #[test]
    fn sinking_slows_the_stick_and_cuts_jumps() {
        let (mut m, mut host) = setup();
        for _ in 0..200 {
            m.tick(&mut host, &Pad::default(), 0);
        }
        assert!((m.sink_move_scale() - 0.3).abs() < 1e-4);
        let sink = m.goop.sink;
        m.vel.y = 40.0;
        m.forward_vel = 10.0;
        m.sink_jump();
        assert!(m.vel.y < 40.0 && m.goop.sink < sink);
    }
}
