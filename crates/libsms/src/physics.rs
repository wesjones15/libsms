//! Ground and air movement (Player/MarioPhysics.cpp and the slope/slide/run math at the top of
//! Player/MarioRun.cpp, Player/MarioJump.cpp `doJumping`/`fallProcess`).
//!
//! `walk_process` and `jump_process` return the decomp's result codes:
//!
//! | walk | meaning                         | jump | meaning                               |
//! |------|---------------------------------|------|---------------------------------------|
//! | 0    | left the ground (walked off)    | 0    | still in the air                      |
//! | 1    | on the ground, moved            | 1    | landed                                |
//! | 2    | blocked (wall, ceiling, void)   | 2    | hit a wall head-on (within clash angle)|
//! |      |                                 | 3    | caught a ledge (`hangonCheck`)        |
//! |      |                                 | 4    | grabbed a fence from below (unused)   |

use glam::Vec3;

use crate::event::Event;
use crate::host::{Host, SurfaceFlags, WallHit};
use crate::mario::{HEIGHT, Mario, STEP_UP, Status};
use crate::math::{self, cos, diff, matan, sin};
use crate::params::Slip;

/// Which `SlipParam*.prm` set applies to the ground under Mario.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlipSet {
    Normal,
    All,
    Slider,
    Slope45,
    WaterGround,
    WaterSlope,
}

/// How much of a broken jump pad flight's sideways speed is left each 120 Hz tick (about a second's half-life).
pub(crate) const PAD_CARRY_BRAKE: f32 = 0.996;

impl Mario {
    pub(crate) fn set_player_velocity(&mut self, v: f32) {
        self.forward_vel = v;
        self.slide_x = v * sin(self.face_yaw);
        self.slide_z = v * cos(self.face_yaw);
        self.vel.x = self.slide_x;
        self.vel.z = self.slide_z;
    }

    // ---- surfaces and slipping ------------------------------------------------------------

    pub(crate) fn ground_illegal(&self) -> bool {
        self.ground.flags.has(SurfaceFlags::ILLEGAL)
    }

    /// `isForceSlip`.
    pub(crate) fn is_force_slip(&self) -> bool {
        self.ground.flags.has(SurfaceFlags::FORCE_SLIP)
            // Oil on a slope (`mPollutionTypeStandingOn == SLIP` and dirty).
            || (self.goop.on_goop
                && self.goop.kind == Some(crate::goop::GoopKind::Slip)
                && self.ground.normal.y < self.params.dirty.slope_angle)
            || self.ground.normal.y < self.params.mario.force_slip_angle
    }

    fn slip_set(&self) -> SlipSet {
        let g = &self.ground;
        if self.is_force_slip() {
            SlipSet::All
        } else if g.flags.has(SurfaceFlags::SLIDER) {
            SlipSet::Slider
        } else if g.flags.has(SurfaceFlags::SLIP_45) {
            SlipSet::Slope45
        } else if g.flags.has(SurfaceFlags::WET) {
            if g.normal.y > 0.99 {
                SlipSet::WaterGround
            } else {
                SlipSet::WaterSlope
            }
        } else {
            SlipSet::Normal
        }
    }

    pub(crate) fn slip_params(&self) -> &Slip {
        let p = &self.params;
        match self.slip_set() {
            SlipSet::Normal => &p.slip_normal,
            SlipSet::All => &p.slip_all,
            SlipSet::Slider => &p.slip_all_slider,
            SlipSet::Slope45 => &p.slip_45,
            SlipSet::WaterGround => &p.slip_water_ground,
            SlipSet::WaterSlope => &p.slip_water_slope,
        }
    }

    /// `isSlipStart`: is the ground steep enough that Mario starts sliding?
    pub(crate) fn is_slip_start(&self) -> bool {
        let g = &self.ground;
        if self.is_force_slip() || g.flags.has(SurfaceFlags::SLIDER) {
            return true;
        }
        if g.flags.has(SurfaceFlags::SLIP_45) && g.normal.y < 0.866_025_4 {
            return true;
        }
        if g.flags.has(SurfaceFlags::NO_SLIP) {
            return false;
        }
        g.normal.y < self.params.mario.slip_start
    }

    /// `isFrontSlip`: is the slope's downhill direction ahead of Mario?
    pub(crate) fn is_front_slip(&self, use_reverse: bool) -> bool {
        let mut angle = self.face_yaw as i32;
        if use_reverse && self.forward_vel < 0.0 {
            angle += 0x8000;
        }
        let d = (self.slope_angle as i32 - angle) as i16;
        d > -0x4000 && d < 0x4000
    }

    pub(crate) fn can_slip_jump(&self) -> bool {
        self.slip_params().jump_enable != 0
    }

    pub(crate) fn is_jump_miss(&self) -> bool {
        self.slip_params().miss_jump != 0
    }

    // ---- walls, ground and ceiling probes --------------------------------------------------

    /// `checkWallPlane`: pushes `pos` out of walls at height `y_off`, returns the nearest one.
    pub(crate) fn check_wall_plane(
        &self,
        host: &mut dyn Host,
        pos: &mut Vec3,
        y_off: f32,
        radius: f32,
    ) -> Option<WallHit> {
        let mut walls = Vec::new();
        let center = Vec3::new(pos.x, pos.y + y_off, pos.z);
        let pushed = host.walls(center, radius, 4, &mut walls);
        pos.x = pushed.x;
        pos.z = pushed.z;
        walls.first().copied()
    }

    /// `checkGroundAtWalking`.
    fn check_ground_at_walking(&mut self, host: &mut dyn Host, mut v: Vec3) -> i32 {
        let r = self.radius;
        self.check_wall_plane(host, &mut v, 30.0, 0.5 * r);
        let wall = self.check_wall_plane(host, &mut v, 60.0, r);

        let (floor_y, ground) = self.check_ground_plane(host, Vec3::new(v.x, v.y + 30.0, v.z));
        let (roof_y, _) = self.check_roof_plane(host, Vec3::new(v.x, self.pos.y + 80.0, v.z));
        self.wall = wall;

        if ground.flags.has(SurfaceFlags::ILLEGAL) {
            return 2;
        }
        if v.y + HEIGHT >= roof_y {
            return 2;
        }

        if v.y > floor_y + STEP_UP {
            // Walked off a ledge.
            self.pos = v;
            self.ground = ground;
            self.floor_y = floor_y;
            return 0;
        }

        if (self.pos.y - floor_y).abs() > 100_000.0 {
            self.pos = self.prev_pos;
        } else {
            self.pos = Vec3::new(v.x, floor_y, v.z);
            self.ground = ground;
            self.floor_y = floor_y;
        }

        if let Some(w) = wall
            && !w.flags.has(SurfaceFlags::MARIO_THROUGH)
        {
            let d = diff(matan(w.normal.z, w.normal.x), self.face_yaw);
            if (0x2aaa..=0x5555).contains(&d) || (-0x5555..=-0x2aaa).contains(&d) {
                return 1; // glancing: slides along the wall
            }
            return 3; // head-on or from behind
        }
        1
    }

    /// `walkProcess`: advance along the ground by a quarter of `vel` (scaled by the slope).
    pub(crate) fn walk_process(&mut self, host: &mut dyn Host) -> i32 {
        if self.ground_illegal() {
            return 2;
        }
        let ny = self.ground.normal.y * self.limit_ground_speed();
        let next = Vec3::new(
            self.pos.x + self.vel.x / 4.0 * ny,
            self.pos.y,
            self.pos.z + self.vel.z / 4.0 * ny,
        );
        let ret = self.check_ground_at_walking(host, next);
        if ret == 1 || ret == 3 {
            self.fall_peak_y = self.pos.y;
        }
        self.model_yaw = self.face_yaw;
        if ret == 3 { 2 } else { ret }
    }

    /// `waitProcess`: stand still on the floor.
    pub(crate) fn wait_process(&mut self, host: &mut dyn Host) -> i32 {
        self.set_player_velocity(0.0);
        if (self.pos.y - self.floor_y).abs() > 100_000.0 {
            self.pos = self.prev_pos;
            self.change_status(Status::RocketLanding, 0, false);
        } else {
            self.pos.y = self.floor_y;
        }
        self.model_yaw = self.face_yaw;
        self.walk_process(host)
    }

    /// `checkGroundAtJumping`: moves to `target`, resolving floor, ceiling and walls.
    fn check_ground_at_jumping(&mut self, host: &mut dyn Host, target: Vec3, arg: u32) -> i32 {
        let r = self.radius;
        let mut pos = target;
        let wall1 = self.check_wall_plane(host, &mut pos, 150.0, r);
        let wall2 = self.check_wall_plane(host, &mut pos, 30.0, r);

        let (fy, g) = self.check_ground_plane(host, Vec3::new(pos.x, pos.y + 30.0, pos.z));
        self.floor_y = fy;
        self.ground = g;
        let (ry, roof) = self.check_roof_plane(host, Vec3::new(pos.x, self.pos.y + 80.0, pos.z));
        self.roof_y = ry;
        self.roof = roof;

        let (mut ground_code, mut wall_code, mut roof_code) = (7, 7, 7);
        self.pos = pos;

        if self.ground_illegal() {
            self.pos = self.prev_pos;
            ground_code = 2;
        } else {
            let mut passable = self.ground.flags.has(SurfaceFlags::MARIO_THROUGH);
            if self.status == Status::HipDrop && self.ground.flags.has(SurfaceFlags::POUND_THROUGH)
            {
                passable = true;
            }
            if !passable && pos.y <= self.floor_y {
                self.pos.y = self.floor_y;
                ground_code = 1;
            }
        }

        if let Some(roof) = self.roof
            && !roof.flags.has(SurfaceFlags::MARIO_THROUGH)
            && HEIGHT + pos.y > self.roof_y
        {
            self.pos.y = self.roof_y - HEIGHT;
            self.set_player_velocity(0.0);
            if self.vel.y >= 0.0 {
                self.vel.y = 0.0;
                if arg & 2 != 0 {
                    roof_code = 0; // fence-hang is not implemented; masks wall code
                }
            }
        }

        self.wall = None;
        let passable = |w: &Option<WallHit>| match w {
            None => true,
            Some(w) => w.flags.has(SurfaceFlags::MARIO_THROUGH),
        };
        let (w1p, w2p) = (passable(&wall1), passable(&wall2));

        if arg & 1 != 0 && w1p && !w2p {
            // A ledge below head height: catch it if `hangonCheck` agrees.
            self.wall = wall2;
            wall_code = match wall2 {
                Some(w) if self.hangon_check(host, &w, target, pos) => 3,
                _ => 0,
            };
        } else if !w1p || !w2p {
            self.wall = if wall1.is_some() { wall1 } else { wall2 };
            if let Some(w) = self.wall {
                let d = diff(
                    matan(w.normal.z, w.normal.x),
                    self.face_yaw.wrapping_add(i16::MIN),
                );
                let max = self.params.jump.clash_angle;
                if -max < d && d < max {
                    wall_code = 2;
                }
            }
        }

        if ground_code != 7 {
            return ground_code;
        }
        if roof_code != 7 {
            return roof_code;
        }
        if wall_code != 7 {
            return wall_code;
        }
        0
    }

    /// `isFallCancel`: letting go of jump while rising fast cuts the jump short (host tuning:
    /// not below `hop_floor_y`, [`crate::Options::short_hop`]).
    fn is_fall_cancel(&self) -> bool {
        self.short_hop && !self.input.jump_held && self.vel.y > 20.0 && self.pos.y >= self.hop_floor_y
    }

    /// `fallProcess`: gravity and terminal velocity.
    pub(crate) fn fall_process(&mut self) {
        if let Some(gravity) = self.pad_gravity {
            self.vel.y -= gravity;
            self.pad_age = self.pad_age.saturating_add(1);
            return;
        }
        let j = &self.params.jump;
        if self.status == Status::JumpCatch && self.corkscrew > 0 {
            // jump_catch sets the camera-steered vertical speed; no gravity during the roll.
        } else if self.is_fall_cancel() {
            self.vel.y *= 0.75;
        } else if matches!(
            self.status,
            Status::LeftRotateJump | Status::RightRotateJump
        ) && self.vel.y < 0.0
        {
            self.vel.y -= j.spin_jump_gravity;
        } else {
            self.vel.y -= j.gravity;
        }
        if self.on_yoshi() || self.options.yoshi_skin {
            self.think_hold_out();
        }
        // Host extension (`Options::spin_pound_fast`): a spinning ground pound sets its own, faster fall speed.
        let spun_pound = self.status == Status::HipDrop && self.pound_spin && self.options.spin_pound_fast;
        if self.vel.y < -75.0 && !spun_pound {
            self.vel.y = -75.0;
        }
    }

    /// `jumpProcess`: one tick of air movement; returns the jump result code.
    pub(crate) fn jump_process(&mut self, host: &mut dyn Host, arg: u32) -> i32 {
        let speed = (self.vel.x * self.vel.x + self.vel.z * self.vel.z).sqrt();
        let mut max = self.params.jump.jumping_max;
        if self.corkscrew > 0 {
            max *= crate::air::CORKSCREW_CAP;
        }
        if self.pad_gravity.is_none() && speed > max {
            self.vel.x *= max / speed;
            self.vel.z *= max / speed;
        }

        let k = if self.pad_gravity.is_some() { 1.0 } else { self.limit_air_speed() };
        self.fall_peak_y = self.fall_peak_y.max(self.pos.y);
        let carry = self.pad_carry;
        let next = self.pos
            + Vec3::new(self.vel.x * k + carry.x, self.vel.y, self.vel.z * k + carry.z) * 0.25;
        let ret = self.check_ground_at_jumping(host, next, arg);
        if ret != 0 {
            self.pad_gravity = None;
            self.pad_carry = Vec3::ZERO;
        } else {
            self.pad_carry *= PAD_CARRY_BRAKE;
        }
        self.fall_peak_y = self.fall_peak_y.max(self.pos.y);
        if ret == 1 {
            // MarioJump.cpp jumpProcess/askStrongGroundTouch: sample contact before
            // fallProcess or the status handler changes the impact speed/move.
            let diving = self.status == Status::JumpCatch;
            let water_landing = host
                .water_level(self.pos)
                .is_some_and(|level| level.is_finite() && level > self.pos.y);
            host.event(Event::FallImpact {
                drop: (self.fall_peak_y - self.pos.y).max(0.0),
                pos: self.pos,
                diving,
                ground_pound: self.status == Status::HipDrop,
                cushioned: self.on_yoshi() || water_landing || (!diving && self.vel.y > -70.0),
            });
            self.fall_peak_y = self.pos.y;
        }

        if self.vel.y >= 0.0 {
            self.jump_peak_y = self.pos.y;
        }
        self.fall_process();
        self.model_yaw = self.face_yaw;
        ret
    }

    /// `playerRefrection`: bounce off the last wall hit.
    pub(crate) fn player_refrection(&mut self, reverse_speed: bool, host: &mut dyn Host) {
        if let Some(w) = self.wall {
            let ang = matan(w.normal.z, w.normal.x);
            self.face_yaw = ang.wrapping_sub(diff(self.face_yaw, ang));
        }
        if reverse_speed {
            self.set_player_velocity(-self.forward_vel);
        } else {
            self.face_yaw = self.face_yaw.wrapping_add(i16::MIN);
        }
        host.event(Event::WallBump);
    }

    // ---- slope, slide, run, air control ----------------------------------------------------

    /// `slopeProcess`: slope acceleration on the forward speed; also refreshes the velocity.
    pub(crate) fn slope_process(&mut self) {
        let g = &self.ground;
        let mag = (g.normal.x * g.normal.x + g.normal.z * g.normal.z).sqrt();
        let ang_diff = diff(self.slope_angle, self.face_yaw);
        let s = self.slip_params();
        let (up, down) = (s.slope_accele_up, s.slope_accele_down);
        if ang_diff > -0x4000 && ang_diff < 0x4000 {
            self.forward_vel += up * mag;
        } else {
            self.forward_vel = -(down * mag - self.forward_vel);
        }
        let max = self.params.mario.running_max;
        if self.forward_vel > max {
            self.forward_vel = max;
        }
        self.slide_dir = self.face_yaw;
        self.slide_x = self.forward_vel * sin(self.face_yaw);
        self.slide_z = self.forward_vel * cos(self.face_yaw);
        self.vel = Vec3::new(self.slide_x, 0.0, self.slide_z);
    }

    fn change_angle_speed(&self) -> f32 {
        0.03125 * (self.slip_params().slide_angle_y_sp as f32 * self.forward_vel)
    }

    /// `slideProcess(baseAcc, friction)`: accelerates the slide velocity downhill.
    fn slide_process(&mut self, base_acc: f32, friction: f32) {
        let n = self.ground.normal;
        let dir_ang = matan(n.z, n.x);
        let mag = (n.x * n.x + n.z * n.z).sqrt();
        let ang_diff = diff(self.slope_angle, self.face_yaw);
        let s = self.slip_params();
        let (up, down) = (s.slide_accele_up, s.slide_accele_down);
        let acc = if ang_diff > -0x4000 && ang_diff < 0x4000 {
            up * mag + base_acc
        } else {
            down * mag + base_acc
        };

        self.slide_x += acc * sin(dir_ang);
        self.slide_z += acc * cos(dir_ang);
        self.slide_x *= friction;
        self.slide_z *= friction;
        self.slide_dir = matan(self.slide_z, self.slide_x);

        let mut turn: i32 = 0;
        if !(-0.1 < self.slide_x && self.slide_x < 0.1 && -0.1 < self.slide_z && self.slide_z < 0.1)
        {
            turn = diff(self.face_yaw, self.slide_dir) as i32;
            let step = self.change_angle_speed();
            if turn > 0 && turn <= 0x4000 {
                turn = ((turn as f32 - step) as i32).max(0);
            } else if turn > -0x4000 && turn < 0 {
                turn = ((turn as f32 + step) as i32).min(0);
            } else if turn > 0x4000 && turn < 0x8000 {
                turn = ((turn as f32 + step) as i32).min(0x8000);
            } else if turn > -0x8000 && turn < -0x4000 {
                turn = ((turn as f32 - step) as i32).max(-0x8000);
            }
            self.face_yaw = (self.slide_dir as i32 + turn) as i16;
        }

        self.vel = Vec3::new(self.slide_x, 0.0, self.slide_z);
        self.forward_vel = (self.slide_x * self.slide_x + self.slide_z * self.slide_z).sqrt();
        if self.forward_vel > 100.0 {
            self.slide_x = 100.0 * self.slide_x / self.forward_vel;
            self.slide_z = 100.0 * self.slide_z / self.forward_vel;
        }
        if !(-0x4000..=0x4000).contains(&turn) {
            self.forward_vel *= -1.0;
        }
    }

    /// `doSliding`: steer and brake a slide; returns true once Mario has stopped.
    pub(crate) fn do_sliding(&mut self, stop_threshold: f32) -> bool {
        let rel = diff(self.intended_yaw, self.slide_dir);
        let (sn, mut cs) = (sin(rel), cos(rel));
        if cs < 0.0 && self.forward_vel >= 0.0 {
            cs *= 0.01 * (0.5 * self.forward_vel) + 0.5;
        }

        let slip_friction = if self.on_yoshi() && self.slip_set() == SlipSet::Normal {
            self.params.slip_yoshi.slip_friction
        } else if self.slip_set() == SlipSet::Normal && self.status == Status::Catch {
            // A belly slide over FLUDD's water glides (`mWasOnWaterSlip`), and in water drags
            // (`mInWaterSlip`).
            // A host that lets shallow water wet the ground has its wet slide glide through it.
            let glides = self.status_state == 1;
            if self.in_water && !(glides && self.options.shallow_water_wets) {
                self.params.mario.in_water_slip
            } else if glides {
                self.params.mario.was_on_water_slip
            } else {
                self.params.slip_normal.slip_friction
            }
        } else {
            self.slip_params().slip_friction
        };
        let stick = self.intended_mag * 0.03125;
        let mult = 0.02 * self.limit_slide_push(stick, cs) + slip_friction;
        let old_mag = (self.slide_x * self.slide_x + self.slide_z * self.slide_z).sqrt();

        let sm = self.slip_params().stick_slide_mult;
        self.slide_x += sn * (self.slide_z * stick) * sm;
        self.slide_z = -((sn * (self.slide_x * stick) * sm) - self.slide_z);

        let new_mag = (self.slide_x * self.slide_x + self.slide_z * self.slide_z).sqrt();
        if old_mag > 0.0 && new_mag > 0.0 {
            self.slide_x = self.slide_x * old_mag / new_mag;
            self.slide_z = self.slide_z * old_mag / new_mag;
        }

        self.slide_process(0.0, mult);

        let slider = self
            .ground
            .flags
            .has(SurfaceFlags::FORCE_SLIP | SurfaceFlags::SLIDER);
        if !slider && self.forward_vel * self.forward_vel < stop_threshold * stop_threshold {
            self.set_player_velocity(0.0);
            self.input.slip = false;
            return true;
        }
        false
    }

    /// `doRunning`: accelerate toward the stick, turn toward it, apply slope.
    pub(crate) fn do_running(&mut self) {
        let r = self.params.run.clone();
        let y = self.params.yoshi.clone();
        let mut sp = self.intended_mag.min(r.max_speed);
        if self.on_yoshi() {
            sp *= y.run_yoshi_mult;
        }

        // Host extension: `Options::traction` scales each change (1 is the game's).
        let t = self.options.traction;
        if self.forward_vel <= 0.0 {
            self.forward_vel += r.vel_minus_brake * t;
        } else if self.forward_vel <= sp {
            self.forward_vel += (r.add_base - self.forward_vel * r.add_vel_div) * t;
        } else if self.ground.normal.y >= r.dec_start_nrm_y {
            // Both brakes apply, riding or not (`doRunning`, MarioRun.cpp, matching).
            self.forward_vel -= r.dec_brake * t;
            self.forward_vel -= y.dec_brake * t;
        }
        if self.forward_vel < 0.0 {
            self.forward_vel = 0.0;
        }

        let m = &self.params.mario;
        let (lo, hi) = if self.is_upper_pumping_style() {
            (m.pumping_rot_sp_min, m.pumping_rot_sp_max)
        } else {
            (m.running_rot_sp_min, m.running_rot_sp_max)
        };
        let mut rot_sp = (0.03125 * (self.forward_vel * (hi - lo) as f32) + lo as f32) as i16;
        if self.on_yoshi() {
            rot_sp = (rot_sp as f32 * y.rot_yoshi_mult) as i16;
        }
        if self.fludd_emitting {
            rot_sp = r.dash_rot_sp;
        }

        self.face_yaw = math::turn_toward(self.face_yaw, self.intended_yaw, rot_sp as i32);
        self.slope_process();
    }

    /// `doJumping`: steer in the air.
    pub(crate) fn do_jumping(&mut self) {
        let j = self.params.jump.clone();
        let mut side_vel = 0.0;
        self.forward_vel *= j.jump_speed_brake;

        if self.input.stick {
            let mut mag = self.intended_mag;
            let ang = diff(self.intended_yaw, self.face_yaw);

            if self.status == Status::Rocket
                && self.fludd_emitting
                && self
                    .fludd
                    .as_ref()
                    .is_some_and(|f| f.can_spray(&self.params))
            {
                mag *= 2.5;
            }
            if self.status == Status::WallJump
                && self.vel.y > 0.0
                && !(-0x4000..=0x4000).contains(&ang)
            {
                mag = 0.0;
            }

            // Fluttering steers less (`mHoldOutAccCtrlF/B`, `mHoldOutSldCtrl`).
            let (acc, slide) = if self.fluttering() {
                let y = &self.params.yoshi;
                let acc = if ang > -0x4000 && ang < 0x4000 {
                    y.hold_out_acc_ctrl_f
                } else {
                    y.hold_out_acc_ctrl_b
                };
                (acc, y.hold_out_sld_ctrl)
            } else {
                (j.jump_accel_control, j.jump_slide_control)
            };
            self.forward_vel += mag * cos(ang) * acc;
            side_vel = mag * sin(ang) * slide;
        }

        if self.forward_vel > 32.0 {
            self.forward_vel -= 0.2;
        }
        if self.forward_vel < -16.0 {
            self.forward_vel += 0.4;
        }
        // Host extension (`Options::turbo_air_thrust`): the held Turbo pushes in the air, after the
        // brakes above so they cannot eat it. It ramps in with the dash wind-up (`dash_speed` to its 32
        // ceiling, about half a second, as long as a jump) and stops at the dash's `mDashMax`.
        if self.options.turbo_air_thrust {
            let cap = self.params.mario.dash_max;
            if self.forward_vel < cap {
                self.forward_vel = (self.forward_vel + self.turbo_thrust()).min(cap);
            }
        }

        let f = self.face_yaw;
        self.slide_x = self.forward_vel * sin(f) + side_vel * sin(f.wrapping_add(0x4000));
        self.slide_z = self.forward_vel * cos(f) + side_vel * cos(f.wrapping_add(0x4000));
        self.vel.x = self.slide_x;
        self.vel.z = self.slide_z;
    }

    /// Host extension: pushes him along his travel by `amount`, never past `cap` (a belly slide keeps its speed
    /// in `slide_x`/`slide_z`, not `forward_vel`; every other status moves `forward_vel`).
    pub(crate) fn push_along(&mut self, amount: f32, cap: f32) {
        if amount == 0.0 {
            return;
        }
        if self.status != Status::Catch {
            if self.forward_vel < cap {
                self.forward_vel = (self.forward_vel + amount).min(cap);
            }
            return;
        }
        let mag = (self.slide_x * self.slide_x + self.slide_z * self.slide_z).sqrt();
        let (dx, dz) = if mag > 1e-3 {
            (self.slide_x / mag, self.slide_z / mag)
        } else {
            (sin(self.face_yaw), cos(self.face_yaw))
        };
        let add = amount.min((cap - mag).max(0.0));
        self.slide_x += dx * add;
        self.slide_z += dz * add;
    }

    /// Host extension: this tick's forward push from a held Turbo with no feet on the ground (a jump, a dive,
    /// a belly slide, a corkscrew), `TURBO_AIR_ACCEL` ramped in with the dash wind-up (`dash_speed` to 32).
    /// 0 when the Turbo isn't held.
    pub(crate) fn turbo_thrust(&self) -> f32 {
        if !self.turbo_held || self.on_yoshi() {
            return 0.0;
        }
        crate::mario::TURBO_AIR_ACCEL * (self.dash_speed / 32.0).clamp(0.0, 1.0)
    }

    /// `doSlipJumping`: steering for slip-fall.
    pub(crate) fn do_slip_jumping(&mut self) {
        let j = self.params.jump.clone();
        self.forward_vel *= j.jump_speed_brake;
        if self.input.stick {
            let ang = diff(self.intended_yaw, self.face_yaw) as u16 as i32;
            let ang = ang as u16 as i16;
            let inc = 0.03125 * self.intended_mag;
            self.forward_vel += inc * cos(ang) * j.jump_accel_control;
            self.face_yaw = self
                .face_yaw
                .wrapping_add((inc * sin(ang) * j.jump_slide_control) as i16);
        }
        if self.forward_vel > 32.0 {
            self.forward_vel -= 0.2;
        }
        if self.forward_vel < -16.0 {
            self.forward_vel += 0.4;
        }
        self.slide_x = self.forward_vel * sin(self.face_yaw);
        self.slide_z = self.forward_vel * cos(self.face_yaw);
        self.vel.x = self.slide_x;
        self.vel.z = self.slide_z;
    }

    /// `askStrongGroundTouch`: did this landing hit hard enough for the heavy effect?
    pub(crate) fn ask_strong_ground_touch(&self) -> bool {
        !self.on_yoshi()
            && self.fall_start_y - self.pos.y > self.params.mario.damage_fall_height
            && self.vel.y <= -70.0
    }

    pub(crate) fn is_upper_pumping_style(&self) -> bool {
        self.fludd.is_some()
            && matches!(
                self.upper,
                crate::mario::UpperState::Pumping | crate::mario::UpperState::HoldingPump
            )
    }
}
