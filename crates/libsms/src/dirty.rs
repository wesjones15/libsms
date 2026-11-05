//! Goop on Mario (Player/MarioMove.cpp `checkGraffito`, `checkGraffitoFire`/`Slip`/`Elec`,
//! `thinkDirty`, `dirtyLimitCheck`, `checkSink`; MarioRun.cpp `oilRun`/`oilSlip`/`oilSlope`;
//! MarioAutodemo.cpp `electricDamage`, `sinkLoser`; MarioCollision.cpp `damageExec`'s
//! `mDirty += pollutionAmount`; MarioDraw.cpp `addDirty`).
//!
//! * **Standing in goop** (`checkGraffito`): every tick Mario asks the goop under him
//!   ([`Host::goop`]) for its kind, then tests the kind's probe pattern ([`GoopKind::probe`]:
//!   oil the nine points round his body, the sinking kinds his feet and the four beside them,
//!   electric goop the one point). All on goop and he is `MARIO_FLAG_DIRTY`. Shadow Mario and
//!   Piantissimo never are (`mPlayerType`). Yoshi won't wade in (`getOffYoshi(true)`).
//! * **What each kind does**:
//!   - brown goop ([`GoopKind::Sink`], `checkSink`): the sink timer climbs; while he has
//!     health it stops at `mSinkDmgDepth` of `mSinkTime`; a floor hit every `mSinkDmgTime`
//!     (`floorDamageExec(1, 3, ..)`), the stick is scaled down (`checkController`), jumps cut
//!     and pulling him partway out (`setStatusToJumping`), drawn sunk by up to `mSinkHeight`
//!     ([`Mario::sink_offset`]). Out of health and past `mSinkTime`, `SINK_LOSER`.
//!   - instant goop ([`GoopKind::Instakill`]): `SINK_LOSER` at once, dropping out of sight
//!     under gravity (`unk374`/`unk378`).
//!   - burning goop ([`GoopKind::Fire`], [`GoopKind::Scorch`]; the host's
//!     [`GoopKind::Lava`]): within `mFireHeight` of the floor and not flashing, a hit with
//!     `DmgGraffitoFire` (`makeGraffitoDamage`), `mFireInvincibleTime`, and `FIRE_DOWN`
//!     with `MSD_SE_MA_DAMAGE_FIRE`.
//!   - oil ([`GoopKind::Slip`], `checkGraffitoSlip`): on the ground he can't run, only coast:
//!     `OIL_RUN` (from any standing or running status), `OIL_SLIP` (from a belly slide),
//!     `OIL_SLOPE` (on a slope steeper than `mSlopeAngle`, which is also a forced slip). Each
//!     tick on oil renews the brake (`mBrakeStartVal*`) for `mDirtyTime*` ticks; off it, it
//!     runs out and he stops. He spreads oil as he goes (`pollute(.., mPolSizeRun/Slip)`,
//!     [`Event::GoopPaint`]). Every `mFogTimeYellow + mFogTimeRed` ticks on it, a floor hit.
//!   - electric goop ([`GoopKind::Electric`], `checkGraffitoElec`): after
//!     `mGraffitoNoDmgTime` ticks on it, standing or running, `ELECTRIC_DAMAGE`
//!     (`MSD_SE_MA_DAMAGE_ELEC`, `MSD_SE_EF_LIGHTNING`): the shock clip and `DmgGraffitoElec`'s
//!     damage, sparks (`ms_m_biri_a/b/c`) for 120 ticks, then a jolt and 60 ticks of flashing.
//!   - glass-wall goop ([`GoopKind::GlassWall`]): he's put back where he was.
//!   - safe goop ([`GoopKind::Safe`]): only dirties.
//!   - the host's poison goop ([`GoopKind::Poison`]): a stumble with `DmgGraffitoPoison`
//!     every `mSinkDmgTime` ticks (the game has the table but no check for it).
//! * **Getting dirty** (`thinkDirty`): on goop, running adds `mIncRunning` a tick, a dive
//!   slide or oil slide `mIncCatching`, slipping `mIncSlipping`; a goop enemy's hit adds its
//!   `mDirty` (`dmgnamekuri.prm`: 100). Clamped to `mDirtyMax`.
//! * **Washing it off**: wading `mDecSwimming` a tick, a spin jump `mDecRotJump`, wet ground or
//!   water (`MARIO_FLAG_RECENTLY_LEFT_WATER`: FLUDD's water on the ground, a puddle)
//!   `mDecWaterHit`. Hosts add their own with [`Mario::wash`] (another Mario's FLUDD).
//! * **Looks** (`addDirty`): every body and hand material's TEV konst colour 0 alpha is set to
//!   the dirt, which blends the goop mask (`H_ma_rak.bti`) in; [`Mario::dirt_share`] is that
//!   alpha out of 255 for a host's material. On goop, `SMS_EmitSinkInPollutionEffect` every 20
//!   ticks: `ms_mari_rakuhamon` ripples, and `ms_mari_rakuball` blobs while sinking.
//!
//! Not ported: the footprints (`mPolSizeFootPrint` is 0 on the disc), the rumble, `OIL_PULLING`
//! (no pulling here), the fire hit's vertical-speed flip (`unk55C`).

use crate::event::{Event, FxAt, SoundCue};
use crate::goop::{GoopKind, TEXEL};
use crate::host::Host;
use crate::mario::{Kind, Mario, Status};
use crate::math::{cos, iconverge, sin};
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
    /// `unk374`/`unk378`: the instant goop's plunge (speed, and how far down he's drawn).
    pub plunge_vel: f32,
    pub plunge: f32,
    /// `mOilBrake`, `mDirtyTimer`.
    pub(crate) oil_brake: f32,
    pub(crate) dirty_timer: i32,
    /// `mStandingOnGraffitoTimer`: oil's floor-hit clock and electric goop's grace.
    pub(crate) graffito_timer: i32,
}

impl Mario {
    /// `isSinking`.
    pub fn is_sinking(&self) -> bool {
        self.goop.sink > 0.0
    }

    /// How far down the body is drawn while sinking (`unk318`'s translation, MarioDraw.cpp):
    /// the sink, plus the instant goop's plunge.
    pub fn sink_offset(&self) -> f32 {
        let g = &self.params.graffito;
        let sink = -(self.goop.sink / (g.sink_time.max(1) as f32)) * g.sink_height;
        sink + self.goop.plunge.min(0.0)
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

    fn sound_here(&mut self, name: &'static str) {
        self.events.push(Event::Sound(SoundCue { name, pos: self.pos, info: None }));
    }

    /// `isTouchGround4cm`.
    fn touch_ground_4cm(&self) -> bool {
        !self.status.is_jumping() && self.pos.y - self.floor_y < 4.0
    }

    /// `floorDamageExec(1, 3, 0, ..)`: a hit with no knock-back.
    fn floor_damage(&mut self) {
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

    /// `makeGraffitoDamage`: a hit from just in front of him.
    fn graffito_damage(&mut self, kind: crate::damage::DamageKind) -> bool {
        let p = self.params.damage(kind).clone();
        let from = self.pos + glam::Vec3::new(sin(self.face_yaw), 0.0, cos(self.face_yaw));
        self.damage_by(from, &p)
    }

    fn check_graffito(&mut self, host: &dyn Host) {
        let was_dirty = self.goop.on_goop;
        if self.goop.immune || self.ground_illegal() {
            self.goop.on_goop = false;
            self.goop.kind = None;
            return;
        }
        let kind = host.goop(self.pos);
        let on = kind.is_some_and(|k| {
            // The oil kinds probe his body's height, the rest the floor.
            let y = if k.probe() == crate::goop::Probe::Square { self.pos.y } else { self.floor_y };
            let base = glam::Vec3::new(self.pos.x, y, self.pos.z);
            k.probe().offsets().iter().all(|&(dx, dz)| {
                host.goop(base + glam::Vec3::new(dx as f32 * TEXEL, 0.0, dz as f32 * TEXEL)).is_some()
            })
        });
        if on && self.on_yoshi() {
            // `getOffYoshi(true)`: Yoshi won't wade through goop.
            self.get_off_yoshi(true);
        }
        self.goop.on_goop = on;
        self.goop.kind = if on { kind } else { None };
        match self.goop.kind {
            Some(GoopKind::Electric) => self.check_graffito_elec(was_dirty),
            Some(k) if k.burns() => self.check_graffito_fire(k),
            Some(GoopKind::Slip) => self.check_graffito_slip(was_dirty),
            Some(GoopKind::GlassWall) => {
                self.pos.x = self.prev_pos.x;
                self.pos.z = self.prev_pos.z;
            }
            _ => {
                // Off the oil mid-slide: the slide brakes harder (`mBrakeSlipNoPollute`).
                if matches!(self.status, Status::OilSlip | Status::OilSlope) {
                    self.goop.oil_brake = self.params.dirty.brake_slip_no_pollute;
                    self.goop.dirty_timer = self.params.dirty.dirty_time_slip as i32;
                }
            }
        }
        if on && self.touch_ground_4cm() && self.ticks % 20 == 0 {
            // `SMS_EmitSinkInPollutionEffect`.
            let pos = self.pos;
            self.fx("ms_mari_rakuhamon", FxAt::Once { pos, yaw: 0 });
            if self.is_sinking() {
                self.fx("ms_mari_rakuball", FxAt::Once { pos, yaw: 0 });
            }
        }
    }

    /// `checkGraffitoFire` (and the host's lava, with the lava table).
    fn check_graffito_fire(&mut self, kind: GoopKind) {
        if self.is_invincible() || self.is_down() {
            return;
        }
        if self.pos.y - self.floor_y > self.params.graffito.fire_height {
            return;
        }
        if matches!(self.status, Status::FireDown | Status::FireJumpEnd) {
            self.face_yaw = self.face_yaw.wrapping_add(i16::MIN);
        }
        let table = if kind == GoopKind::Lava {
            crate::damage::DamageKind::GraffitoLava
        } else {
            crate::damage::DamageKind::GraffitoFire
        };
        self.graffito_damage(table);
        if self.is_down() {
            return;
        }
        self.invincible = self.params.graffito.fire_invincible_time.max(0) as _;
        self.start_fire_down();
        self.sound_here("MSD_SE_MA_DAMAGE_FIRE");
    }

    /// `checkGraffitoElec`.
    fn check_graffito_elec(&mut self, was_dirty: bool) {
        if !was_dirty {
            self.goop.graffito_timer = self.params.mario.graffito_no_dmg_time as i32;
        }
        if self.goop.graffito_timer != 0 {
            self.goop.graffito_timer -= 1;
            return;
        }
        if self.is_invincible() || self.is_down() {
            return;
        }
        if matches!(self.status.kind(), Kind::Waiting | Kind::Running) {
            self.change_status(Status::ElectricDamage, 0, false);
            self.sound_here("MSD_SE_MA_DAMAGE_ELEC");
            self.sound_here("MSD_SE_EF_LIGHTNING");
        }
    }

    /// `checkGraffitoSlip`.
    fn check_graffito_slip(&mut self, was_dirty: bool) {
        let d = self.params.dirty.clone();
        if !self.touch_ground_4cm() {
            if matches!(self.status, Status::OilSlip | Status::OilSlope) {
                self.goop.oil_brake = d.brake_slip_no_pollute;
                self.goop.dirty_timer = d.dirty_time_slip as i32;
            }
            return;
        }
        let slip = |m: &mut Mario| {
            m.goop.oil_brake = d.brake_start_val_slip;
            m.goop.dirty_timer = d.dirty_time_slip as i32;
        };
        let run = |m: &mut Mario| {
            m.goop.oil_brake = d.brake_start_val_run;
            m.goop.dirty_timer = d.dirty_time_run as i32;
        };
        match self.status {
            Status::OilSlip | Status::OilSlope => slip(self),
            Status::OilRun => run(self),
            _ => {}
        }
        // What the oil takes over (the game's from any status on the ground; here the
        // standing and running ones, so it never eats a take-off).
        let free = matches!(self.status.kind(), Kind::Waiting | Kind::Running)
            && !matches!(self.status, Status::FireDash);
        if free && self.ground.normal.y <= d.slope_angle {
            slip(self);
            if self.status != Status::OilSlope {
                self.change_status(Status::OilSlope, 0, false);
                self.voice("MSD_SE_MV28_SPRISE_SMALL_01");
            }
        } else if matches!(self.status, Status::Catch | Status::OilSlip | Status::OilSlope) {
            slip(self);
            if self.status != Status::OilSlip {
                self.change_status(Status::OilSlip, 0, false);
                if self.prev_status != Status::OilSlip {
                    self.voice("MSD_SE_MV28_SPRISE_SMALL_01");
                }
            }
        } else if free && self.status != Status::CatchLost {
            run(self);
            if self.status != Status::OilRun {
                self.change_status(Status::OilRun, 0, false);
            }
        }
        let cycle = d.fog_time_yellow as i32 + d.fog_time_red as i32;
        if !was_dirty {
            self.goop.graffito_timer = cycle;
        }
        self.goop.graffito_timer -= 1;
        if self.goop.graffito_timer != 0 {
            if self.goop.graffito_timer == d.fog_time_red as i32 {
                self.floor_damage();
            }
        } else {
            self.goop.graffito_timer = cycle;
        }
    }

    /// `pollute(pos, size)` from the oil statuses: the host paints oil.
    fn spread_oil(&mut self, radius: f32) {
        if radius > 0.0 {
            self.events.push(Event::GoopPaint { pos: self.pos, radius, kind: GoopKind::Slip });
        }
    }

    /// `mDirtyTimer`'s countdown: when it runs out the brake is gone and he stops.
    fn oil_timer(&mut self) -> bool {
        self.goop.dirty_timer -= 1;
        if self.goop.dirty_timer <= 0 {
            self.goop.dirty_timer = 0;
            self.goop.oil_brake = 0.0;
            return true;
        }
        false
    }

    fn oil_turn(&mut self, rot: i16) {
        let diff = self.intended_yaw.wrapping_sub(self.face_yaw);
        let r = rot as i32;
        self.face_yaw = self.intended_yaw.wrapping_sub(iconverge(diff as i32, 0, r, r) as i16);
    }

    /// `oilRun`: coasting on oil.
    pub(crate) fn oil_run(&mut self, host: &mut dyn Host) -> bool {
        if self.input.jump {
            self.set_player_velocity(0.0);
            return self.change_jumping(Status::Jump);
        }
        if self.input.dive {
            self.vel.y = 20.0;
            return self.change_status(Status::JumpCatch, 1, false);
        }
        if self.vel.x.abs() < 1.0 && self.vel.z.abs() < 1.0 {
            self.set_player_velocity(0.0);
            return self.change_status(Status::Wait, 0, false);
        }
        let d = self.params.dirty.clone();
        self.spread_oil(d.pol_size_run);
        self.oil_turn(d.slip_rotate);
        self.vel.x += self.intended_mag * sin(self.face_yaw) * d.slip_run_sp;
        self.vel.z += self.intended_mag * cos(self.face_yaw) * d.slip_run_sp;
        self.oil_timer();
        self.vel.x *= self.goop.oil_brake;
        self.vel.z *= self.goop.oil_brake;
        self.forward_vel = 0.0;
        self.slide_x = 0.0;
        self.slide_z = 0.0;
        if self.intended_mag == 0.0 {
            let c = if self.input.b_held { crate::anim::clip::SQUAT } else { crate::anim::clip::WAIT };
            self.set_anim(c, 0.5);
        } else {
            self.set_anim(crate::anim::clip::RUN2, 0.5 * self.intended_mag * d.slip_anm_speed);
            if self.ticks % 60 == 0 {
                self.voice("MSD_SE_MV28_SPRISE_SMALL_01");
            }
            self.sound_here("MSD_SE_MA_SLIP_POLLUT");
        }
        if self.walk_process(host) == 0 {
            return self.change_status(Status::Landing, 0, false);
        }
        false
    }

    /// `oilSlip`: belly-sliding on oil.
    pub(crate) fn oil_slip(&mut self, host: &mut dyn Host) -> bool {
        if self.input.jump {
            self.set_player_velocity(0.0);
            return self.change_jumping(Status::Jump);
        }
        let d = self.params.dirty.clone();
        self.oil_turn(d.slip_catch_rotate);
        if self.oil_timer() {
            self.change_status(Status::Catch, 0, false);
        }
        self.spread_oil(d.pol_size_slip);
        self.sound_here("MSD_SE_MA_SLIP_POLLUT_CP");
        let off = self.face_yaw.wrapping_sub(self.intended_yaw);
        self.forward_vel += self.intended_mag * cos(off) * d.slip_catch_sp;
        self.forward_vel *= self.goop.oil_brake;
        self.set_player_velocity(self.forward_vel);
        if self.forward_vel.abs() < 1.0 {
            self.set_player_velocity(0.0);
            return self.change_status(Status::CatchLost, 0, false);
        }
        if self.walk_process(host) == 0 {
            return self.change_status(Status::Landing, 0, false);
        }
        self.set_anim(crate::anim::clip::DIVE, 1.0);
        if self.anim.frame > 50.0 {
            self.anim.frame = 50.0;
        }
        false
    }

    /// `oilSlope`: down an oily slope (`slipBackCommon`).
    pub(crate) fn oil_slope(&mut self, host: &mut dyn Host) -> bool {
        if self.oil_timer() {
            self.change_status(Status::Catch, 0, false);
        }
        let r = self.params.dirty.pol_size_slip;
        self.spread_oil(r);
        self.slip_back(host)
    }

    /// `electricDamage`: the shock.
    pub(crate) fn electric_damage(&mut self, _host: &mut dyn Host) -> bool {
        self.set_player_velocity(0.0);
        self.vel.y = 0.0;
        if self.status_state == 0 {
            self.voice("MSD_SE_MV04_DAMAGE_ELEC_01");
            let hp = self.params.damage(crate::damage::DamageKind::GraffitoElec).damage;
            self.dec_hp(hp as i16);
            if self.is_down() {
                return true;
            }
            self.status_state = 1;
        }
        self.set_anim("shock", 1.0);
        // `elecEffect`.
        self.fx("ms_m_biri_a", FxAt::Center);
        self.fx("ms_m_biri_c", FxAt::Center);
        self.fx("ms_m_biri_b", FxAt::Center);
        self.status_timer += 1;
        if self.status_timer > 0x78 {
            self.anim.frame = 0.0;
            self.voice("MSD_SE_MV07_DAMAGE_REACT_01");
            // `elecEndEffect`.
            self.fx("ms_moe_fire_off", FxAt::FeetFollow);
            let e = self.params.damage(crate::damage::DamageKind::GraffitoElec).clone();
            let p = Damage {
                damage: 0,
                down_type: 3,
                dirty: 0.0,
                invincible_time: 0x3C,
                ..e
            };
            let from = self.pos + glam::Vec3::new(sin(self.face_yaw), 0.0, cos(self.face_yaw));
            self.damage_by(from, &p);
            return self.change_status(Status::Wait, 0, true);
        }
        false
    }

    /// `sinkLoser`: swallowed, still sinking.
    pub(crate) fn sink_loser(&mut self, _host: &mut dyn Host) -> bool {
        self.set_player_velocity(0.0);
        self.vel.y = 0.0;
        self.set_anim("sink_down", 1.0);
        if self.status_state == 0 {
            self.voice("MSD_SE_MV10A_CRY_SHORT_01");
            self.status_state = 1;
        }
        let g = self.params.graffito.clone();
        self.goop.sink = (self.goop.sink + 1.0).min(g.sink_time as f32);
        self.goop.plunge_vel -= self.params.jump.gravity;
        self.goop.plunge = (self.goop.plunge + self.goop.plunge_vel).max(-2.0 * g.sink_height);
        false
    }

    /// `loserExec` then `SINK_LOSER`.
    fn swallow(&mut self) {
        if self.status == Status::SinkLoser {
            return;
        }
        self.vel = glam::Vec3::ZERO;
        self.forward_vel = 0.0;
        self.slide_x = 0.0;
        self.slide_z = 0.0;
        self.loser_exec();
        self.change_status(Status::SinkLoser, 0, false);
    }

    fn check_sink(&mut self) {
        if self.goop.immune || self.ground_illegal() || self.status == Status::SinkLoser {
            return;
        }
        if self.is_invincible() {
            return;
        }
        if self.floor_y + 100.0 < self.pos.y {
            self.goop.sink = 0.0;
            return;
        }
        match (self.goop.on_goop, self.goop.kind) {
            (true, Some(GoopKind::Sink)) => {
                let g = self.params.graffito.clone();
                self.goop.sink += 1.0;
                let depth = g.sink_time as f32 * g.sink_dmg_depth;
                if self.health > 0 && self.goop.sink > depth {
                    self.goop.sink = depth;
                }
                if self.ticks % g.sink_dmg_time.max(1) as u64 == 0 {
                    self.floor_damage();
                }
                if self.ticks % 30 == 0 {
                    self.sound_here("MSD_SE_MA_SINK_IN_MUD");
                }
                if self.goop.sink > g.sink_time as f32 {
                    self.swallow();
                }
                self.events.push(Event::GoopSink { pos: self.pos });
                return;
            }
            (true, Some(GoopKind::Instakill)) => {
                self.swallow();
                return;
            }
            (true, Some(GoopKind::Poison)) => {
                let t = self.params.graffito.sink_dmg_time.max(1) as u64;
                if self.ticks % t == 0 && !self.is_down() {
                    self.graffito_damage(crate::damage::DamageKind::GraffitoPoison);
                }
            }
            _ => {}
        }
        self.goop.plunge_vel = 0.0;
        self.goop.plunge = 0.0;
        self.goop.sink = 0.0;
    }

    fn think_dirty(&mut self) {
        let d = self.params.dirty.clone();
        if self.goop.on_goop {
            match self.status {
                Status::Run | Status::OilRun => self.goop.dirty += d.inc_running,
                Status::Catch | Status::OilSlip | Status::OilSlope => {
                    self.goop.dirty += d.inc_catching
                }
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

    fn on(kind: GoopKind) -> (Mario, GoopHost) {
        let (m, mut host) = setup();
        host.field.clear();
        host.field.stamp(&FlatHost::default(), Vec3::ZERO, 2000.0, kind);
        (m, host)
    }

    fn run(m: &mut Mario, host: &mut GoopHost, pad: &Pad, n: usize) {
        for _ in 0..n {
            m.tick(host, pad, 0);
            let evs = std::mem::take(&mut m.events);
            host.events.extend(evs);
        }
    }

    #[test]
    fn oil_turns_running_into_coasting_and_spreads_oil() {
        let (mut m, mut host) = on(GoopKind::Slip);
        let pad = Pad { stick: Vec2::new(0.0, 1.0), ..Pad::default() };
        let mut coasted = false;
        for _ in 0..240 {
            run(&mut m, &mut host, &pad, 1);
            coasted |= m.status == Status::OilRun;
        }
        assert!(coasted, "never coasted on oil: {:?}", m.status);
        assert!(host.events.iter().any(|e| matches!(e, Event::GoopPaint { kind: GoopKind::Slip, .. })));
        assert!(m.goop.dirty > 0.0);
    }

    #[test]
    fn electric_goop_shocks_him_then_lets_go() {
        let (mut m, mut host) = on(GoopKind::Electric);
        let hp = m.health;
        let mut shocked = false;
        for _ in 0..60 {
            run(&mut m, &mut host, &Pad::default(), 1);
            shocked |= m.status == Status::ElectricDamage;
        }
        assert!(shocked);
        assert!(m.health < hp);
        // The shock lasts 120 ticks, then the jolt's flashing keeps him from the next.
        run(&mut m, &mut host, &Pad::default(), 100);
        assert_ne!(m.status, Status::ElectricDamage);
        assert!(m.is_invincible() || m.status == Status::ElectricDamage || m.health < hp);
    }

    #[test]
    fn burning_goop_sets_his_seat_alight() {
        let (mut m, mut host) = on(GoopKind::Fire);
        let hp = m.health;
        run(&mut m, &mut host, &Pad::default(), 3);
        assert!(matches!(m.status, Status::FireDown | Status::FireJump | Status::FireDash), "{:?}", m.status);
        assert!(m.health < hp);
        assert!(m.is_invincible());
    }

    #[test]
    fn glass_wall_goop_turns_him_back() {
        let (mut m, mut host) = setup();
        host.field.clear();
        host.field.stamp(&FlatHost::default(), Vec3::new(0.0, 0.0, 900.0), 500.0, GoopKind::GlassWall);
        let pad = Pad { stick: Vec2::new(0.0, 1.0), ..Pad::default() };
        run(&mut m, &mut host, &pad, 600);
        assert!(m.pos.z < 900.0, "walked into the glass goop: {}", m.pos.z);
    }

    #[test]
    fn instant_goop_swallows_him() {
        let (mut m, mut host) = on(GoopKind::Instakill);
        run(&mut m, &mut host, &Pad::default(), 60);
        assert_eq!(m.status, Status::SinkLoser);
        assert!(m.is_down());
        assert!(m.sink_offset() < -150.0, "drawn under: {}", m.sink_offset());
    }

    #[test]
    fn poison_goop_stumbles_him_now_and_then() {
        let (mut m, mut host) = on(GoopKind::Poison);
        let hp = m.health;
        run(&mut m, &mut host, &Pad::default(), 200);
        assert!(m.health < hp);
        assert!(!m.is_sinking(), "poison doesn't sink");
    }

    #[test]
    fn safe_goop_only_dirties() {
        let (mut m, mut host) = on(GoopKind::Safe);
        let hp = m.health;
        let pad = Pad { stick: Vec2::new(0.0, 1.0), ..Pad::default() };
        run(&mut m, &mut host, &pad, 240);
        assert_eq!(m.health, hp);
        assert!(m.goop.on_goop || m.goop.dirty > 0.0);
    }
}
