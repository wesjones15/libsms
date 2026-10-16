//! Airborne statuses (Player/MarioJump.cpp).

use glam::Vec3;

use crate::anim::{ClipName, clip};
use crate::event::Event;
use crate::host::{Host, SurfaceFlags};
use crate::mario::{HEIGHT, Mario, Status};
use crate::math::diff;
use crate::params::NozzleKind;

/// 120 Hz ticks after a jump pad throws him before a move can break the flight, so a button held through the launch (a B or R from before) doesn't.
pub(crate) const PAD_ACTION_GRACE: u32 = 12;

impl Mario {
    pub(crate) fn jump_main(&mut self, host: &mut dyn Host) -> bool {
        use Status::*;
        match self.status {
            YoshiThrown => {
                // Lands in `THROWN_DOWN`, which the sim does not have; the back-down is the
                // nearest lying-down status.
                self.jumping_basic(host, BackDown, "thrown", 0);
                false
            }
            ForceJump | Jump => {
                // A jump pad's flight can be broken by any move once it has got going (a dive, a
                // rocket or hover, the Turbo, a ground pound, a spin jump); the pad's sideways speed
                // then carries on beside the move (`pad_carry`).
                if self.pad_gravity.is_none() || self.pad_age >= PAD_ACTION_GRACE {
                    self.pad_turbo_break();
                    if self.jumping_common_events() {
                        return true;
                    }
                }
                let anim = if self.status == ForceJump {
                    if self.vel.y >= 0.0 {
                        clip::JUMP2_UP
                    } else {
                        clip::JUMP2_DOWN
                    }
                } else {
                    clip::JUMP
                };
                self.jumping_basic(host, JumpSlip, anim, 3);
                false
            }
            SecondJump => {
                if self.status_timer == 0 {
                    self.status_timer += 1;
                }
                let anim = if self.vel.y >= 0.0 {
                    clip::JUMP2_UP
                } else {
                    clip::JUMP2_DOWN
                };
                if self.jumping_common_events() {
                    return true;
                }
                self.jumping_basic(host, SecJumpSlip, anim, 3);
                false
            }
            UltraJump => {
                if self.status_timer == 0 {
                    self.status_timer += 1;
                }
                if self.check_back_trig() || self.rocket_check() {
                    return true;
                }
                self.jumping_basic(host, UltraJumpSlip, clip::ROLL, 0);
                false
            }
            BackJump => {
                if self.check_back_trig() || self.rocket_check() {
                    return true;
                }
                if self.status_state == 0 && self.anim.at_last_frame() {
                    self.status_state = 1;
                }
                let c = if self.status_state == 0 {
                    "jump_rolling"
                } else {
                    clip::LAND
                };
                self.jumping_basic(host, SecJumpSlip, c, 0);
                false
            }
            Landing | RocketLanding => {
                if self.vel.y < 0.0 {
                    self.status_timer += 1;
                    if self.status_timer > 240 {
                        self.status_timer = 240;
                        // `landing`: two seconds into a fall Mario flails (`ANIM_THROWN`) and
                        // cries out. The game requests the cry every tick from then on; the
                        // one that starts it is enough here.
                        if self.status_arg != 3 {
                            self.sound("MSD_SE_MV10B_CRY_JUMP_01", None);
                        }
                        self.status_arg = 3;
                    }
                }
                if self.check_back_trig() || self.rocket_check() {
                    return true;
                }
                let c = match self.status_arg {
                    0 => clip::LAND,
                    1 => "slpla",
                    _ => "thrown",
                };
                self.jumping_basic(host, LandSlip, c, 3);
                false
            }
            UTurnJump => {
                if self.jumping_common_events() || self.check_back_trig() {
                    return true;
                }
                if self.jumping_basic(host, UTurnJumpSlip, clip::UTURN_JUMP, 1) != 3 {
                    self.model_yaw = self.model_yaw.wrapping_add(i16::MIN);
                }
                false
            }
            WallJump => {
                if self.check_back_trig() || self.consider_jump_rotate() || self.rocket_check() {
                    return true;
                }
                self.jumping_basic(host, JumpSlip, clip::WALL_JUMP, 3);
                false
            }
            MissJump => self.miss_jumping(host),
            BroadJump => {
                // Host: once the Turbo is let go the dash is over (`broadJumping` keeps its flag
                // until he lands, which keeps the water-boost cone and sounds going), and he can
                // dive like out of any other nozzle's jump.
                if !self.turbo_held && self.check_back_trig() {
                    return true;
                }
                self.jumping_basic(host, Run, "broad_jump", 1);
                self.fludd_emitting = self.turbo_held;
                false
            }
            RotateBroadJump => {
                self.jumping_basic(host, Run, "roll_jump", 1);
                false
            }
            Rocket => self.rocketing(host),
            LeftRotateJump | RightRotateJump => self.rotate_jumping(host),
            JumpCatch => self.jump_catch(host),
            HipDrop => self.hip_attacking(host),
            Trample => {
                // `trample`.
                if self.jumping_common_events() {
                    return true;
                }
                let c = crate::collide::TRAMPLE_CLIPS[(self.status_arg as usize).min(2)];
                self.jumping_basic(host, SecJumpSlip, c, 3);
                false
            }
            JumpShortBackDown => self.jump_down(host, ShortBackDown, "bkdwn", -16.0),
            JumpShortForeDown => self.jump_down(host, ShortForeDown, clip::FORE_DOWN, 16.0),
            JumpForeDown => self.jump_down(host, ForeDown, clip::FORE_DOWN, 16.0),
            JumpBackDown => self.jump_down(host, BackDown, "bkdwn", -16.0),
            LandSafeDown => {
                let v = self.forward_vel;
                self.jump_down(host, LandSlip, clip::LAND, v)
            }
            WallSlide => self.stay_wall(host),
            CatchStop => self.catch_stop(host),
            SlipFall => self.slip_falling(host),
            _ => false,
        }
    }

    /// `jumpingBasic`: shared air step. Returns the `jump_process` result.
    fn jumping_basic(
        &mut self,
        host: &mut dyn Host,
        on_ground: Status,
        c: ClipName,
        process_arg: u32,
    ) -> i32 {
        if self.pad_gravity.is_none() { self.do_jumping(); }
        let r = self.jump_process(host, process_arg);
        match r {
            0 => {
                self.set_anim(c, 1.0);
            }
            1 => {
                let strong = self.ask_strong_ground_touch();
                host.event(Event::Land { strong });
                // `jumpingBasic`: a hard landing with FLUDD on (but the Hover) shakes him
                // with the heavy dust; then a spin jump's landing is heavy, any other light.
                if strong
                    && self
                        .fludd
                        .as_ref()
                        .is_some_and(|f| f.nozzle != crate::params::NozzleKind::Hover)
                {
                    self.strong_touch_down_effect();
                }
                let spun = matches!(
                    self.status,
                    Status::UTurnJump | Status::LeftRotateJump | Status::RightRotateJump
                );
                self.fall_start_y = self.pos.y;
                self.change_status(on_ground, 0, false);
                if spun {
                    self.strong_touch_down_effect();
                } else {
                    self.small_touch_down_effect();
                }
            }
            2 => {
                if self.forward_vel > self.params.mario.clash_speed {
                    self.bump_stars();
                    host.event(Event::WallBump);
                    self.change_status(Status::JumpShortBackDown, 0, false);
                    return r;
                }
                self.set_anim(c, 1.0);
                if self.on_yoshi() {
                    self.set_player_velocity(0.0);
                } else if self
                    .wall
                    .is_some_and(|w| w.flags.has(SurfaceFlags::NO_WALL_JUMP))
                {
                    self.change_status(Status::RocketLanding, 0, false);
                    self.set_player_velocity(0.0);
                } else if self.forward_vel > 16.0 && self.status != Status::RocketLanding {
                    self.player_refrection(false, host);
                    self.face_yaw = self.face_yaw.wrapping_add(i16::MIN);
                    if self.wall.is_some() {
                        self.change_status(Status::WallSlide, 0, false);
                    } else {
                        if self.vel.y > 0.0 {
                            self.vel.y = 0.0;
                        }
                        if self.forward_vel >= 38.0 {
                            self.change_status(Status::JumpShortBackDown, 0, false);
                        } else {
                            if self.forward_vel > 8.0 {
                                self.set_player_velocity(-8.0);
                            }
                            self.change_status(Status::LandSafeDown, 0, false);
                        }
                    }
                } else {
                    self.set_player_velocity(0.0);
                }
            }
            3 => {
                // The catch grunt (`TMario::soundProcess`, from a jumping status).
                self.events.push(Event::Sound(crate::event::SoundCue {
                    name: "MSD_SE_MV15_EXERT_INST_02",
                    pos: self.pos,
                    info: None,
                }));
                self.set_anim(crate::hang::clip::HANG, 1.0);
                self.change_status(Status::Hanging, 0, false);
            }
            _ => {}
        }
        r
    }

    /// A move breaks a jump pad's flight: normal air control and gravity are back, and the pad's
    /// sideways speed carries on (`pad_carry`) beside whatever the move does.
    pub(crate) fn break_pad_flight(&mut self) {
        if self.pad_gravity.take().is_some() {
            self.pad_carry = Vec3::new(self.pad_vel.x, 0.0, self.pad_vel.z);
        }
    }

    /// The Turbo held in a pad flight breaks it. The Turbo has no thrust of its own in the air and
    /// no status change, so this hands his air control back and keeps the pad's momentum.
    fn pad_turbo_break(&mut self) {
        if self.pad_gravity.is_some() && self.turbo_held {
            self.break_pad_flight();
        }
    }

    /// `considerJumpRotate`: switches straight to a spin jump mid-air (no take-off boost).
    fn consider_jump_rotate(&mut self) -> bool {
        match self.check_stick_rotate() {
            Some(2) => {
                self.break_pad_flight();
                self.status = Status::RightRotateJump;
                true
            }
            Some(3) => {
                self.break_pad_flight();
                self.status = Status::LeftRotateJump;
                true
            }
            _ => false,
        }
    }

    /// `checkBackTrig`: B dives, L ground-pounds.
    fn check_back_trig(&mut self) -> bool {
        if self.input.dive {
            if self.input.l_pressed {
                return self.change_status(Status::HipDrop, 0, false);
            }
            if self.on_yoshi() {
                return false;
            }
            let v = self.params.jump.jump_jump_catch_sp;
            self.set_player_velocity(v);
            return self.change_status(Status::JumpCatch, 0, false);
        }
        false
    }

    fn jumping_common_events(&mut self) -> bool {
        self.check_back_trig() || self.rocket_check() || self.consider_jump_rotate()
    }

    /// `rocketCheck`: with FLUDD firing a rocket-type nozzle, start rocketing.
    pub(crate) fn rocket_check(&mut self) -> bool {
        if matches!(self.status, Status::Rocket | Status::RocketLanding) {
            return false;
        }
        let ok = self.fludd.as_ref().is_some_and(|f| {
            f.nozzle_params(&self.params).rocket_type == 1
                && self.upper == crate::mario::UpperState::Pumping
                && f.can_spray(&self.params)
        });
        if ok {
            self.hover_target_y = self.pos.y + self.params.water_gun.hover_height;
            return self.change_status(Status::Rocket, 0, false);
        }
        false
    }

    /// `rocketing`: hover/rocket flight. The nozzle's recoil (see `fludd.rs`) supplies the thrust;
    /// the hover nozzle also steers and holds a target height.
    fn rocketing(&mut self, host: &mut dyn Host) -> bool {
        let state = self.fludd.as_ref().map(|f| {
            (
                f.nozzle,
                f.nozzle_params(&self.params).rocket_type,
                f.can_spray(&self.params),
            )
        });
        let Some((nozzle, rocket_type, can_spray)) = state else {
            return self.change_status(Status::RocketLanding, 0, false);
        };
        if rocket_type != 1 || self.upper != crate::mario::UpperState::Pumping || !can_spray {
            return self.change_status(Status::RocketLanding, 0, false);
        }

        if self.input.stick && nozzle == NozzleKind::Hover {
            let mag = self.intended_mag;
            let ang = diff(self.intended_yaw, self.face_yaw);
            let np = self.params.nozzle(NozzleKind::Hover).clone();
            let along = (ang > -0x1555 && ang < 0x1555) || !(-0x6AAA..=0x6AAA).contains(&ang);
            if along {
                let max = if (-0x4000..=0x4000).contains(&ang) {
                    np.side_angle_max_front
                } else {
                    np.side_angle_max_back
                };
                let tilt = (0.03125 * -mag * max as f32 * crate::math::cos(ang)) as i16;
                if let Some(f) = self.fludd.as_mut() {
                    f.tilt_target = [tilt, tilt];
                }
                self.forward_vel += mag * crate::math::cos(ang) * self.params.diving.accel_control;
            } else {
                let tilt =
                    (0.03125 * -mag * np.side_angle_max_side as f32 * crate::math::sin(ang)) as i16;
                if let Some(f) = self.fludd.as_mut() {
                    f.tilt_target = [tilt.wrapping_neg(), tilt];
                }
                let rot = self.params.hover.rot_sp as i32;
                self.face_yaw = crate::math::turn_toward(self.face_yaw, self.intended_yaw, rot);
            }
        } else if let Some(f) = self.fludd.as_mut() {
            f.tilt_target = [0, 0];
        }

        self.slide_x = self.forward_vel * crate::math::sin(self.face_yaw);
        self.slide_z = self.forward_vel * crate::math::cos(self.face_yaw);
        self.vel.x = self.slide_x;
        self.vel.z = self.slide_z;

        if nozzle == NozzleKind::Hover {
            self.vel.y = (self.hover_target_y - self.pos.y) * self.params.hover.accel_rate;
            self.forward_vel *= self.params.hover.brake;
        }

        self.jump_process(host, 2);

        if self.roof.is_some() && HEIGHT + self.pos.y > self.roof_y {
            self.pos.y = self.roof_y - HEIGHT;
        }
        self.set_anim(clip::LAND, 1.0);
        false
    }

    fn miss_jumping(&mut self, host: &mut dyn Host) -> bool {
        if self.input.dive {
            return self.change_status(Status::JumpCatch, 0, false);
        }
        self.set_player_velocity(0.98 * self.forward_vel);
        match self.jump_process(host, 0) {
            1 => {
                let next = if self.forward_vel < 0.0 {
                    Status::Slip
                } else {
                    Status::JumpSlip
                };
                self.change_status(next, 0, false);
            }
            2 => self.set_player_velocity(0.0),
            _ => {}
        }
        self.set_anim(clip::JUMP, 1.0);
        false
    }

    fn rotate_jumping(&mut self, host: &mut dyn Host) -> bool {
        if self.check_back_trig() || self.rocket_check() {
            return true;
        }
        self.set_anim(clip::SPIN, 1.0);
        self.emit_blur_spin_jump();
        self.jumping_basic(host, Status::SecJumpSlip, clip::SPIN, 0);
        self.status_timer += 1;
        let t = (self.status_timer * 4096) as i16;
        self.model_yaw = if self.status == Status::RightRotateJump {
            t
        } else {
            t.wrapping_neg()
        };
        false
    }

    /// `jumpCatch`: the belly-flop dive.
    fn jump_catch(&mut self, host: &mut dyn Host) -> bool {
        if self.input.dive && self.input.l_pressed {
            return self.change_status(Status::HipDrop, 0, false);
        }
        self.set_anim(clip::DIVE, 1.0);
        self.do_jumping();
        match self.jump_process(host, 0) {
            1 => {
                host.event(Event::Land {
                    strong: self.ask_strong_ground_touch(),
                });
                self.change_status(Status::Catch, 0, false);
            }
            2 => {
                self.player_refrection(true, host);
                if self.vel.y > 0.0 {
                    self.vel.y = 0.0;
                }
                // A dive into a wall: stars.
                self.bump_stars();
                self.change_status(Status::JumpShortBackDown, 0, false);
            }
            _ => {}
        }
        false
    }

    /// `hipAttacking`: ground pound (hover, then fall).
    fn hip_attacking(&mut self, host: &mut dyn Host) -> bool {
        let j = self.params.jump.clone();
        match self.status_state {
            0 | 1 => {
                self.status_state = 1;
                if self.floor_y > self.pos.y {
                    self.pos.y = 1.0 + self.floor_y;
                    self.change_status(Status::HipAttackEnd, 0, false);
                }
                if self.status_timer < 0x28 {
                    let lift = (0x28 - self.status_timer) as f32 * 0.5;
                    if HEIGHT + (self.pos.y + lift) < self.roof_y {
                        self.pos.y += lift * 0.25;
                        self.jump_peak_y = self.pos.y;
                    }
                }
                self.set_player_velocity(0.0);
                self.set_anim(clip::HIP_START, 1.0);
                self.status_timer += 1;
                if self.status_timer >= 60 {
                    self.status_timer = 0;
                    self.status_state = 2;
                }
                self.vel.y = 0.0;
                match self.jump_process(host, 0) {
                    1 => return self.change_status(Status::JumpEnd, 0, false),
                    2 => {
                        self.set_player_velocity(-16.0);
                        if self.vel.y > 0.0 {
                            self.vel.y = 0.0;
                        }
                        return self.change_status(Status::JumpShortBackDown, 0, false);
                    }
                    _ => {}
                }
            }
            _ => {
                self.set_anim(clip::HIP_FALL, 1.0);
                self.status_timer += 1;
                if self.status_timer > j.super_hip_attack_ct as i32 {
                    self.status_state = 3;
                }
                self.vel.y = if self.status_state == 2 {
                    j.hip_attack_speed_y
                } else {
                    j.super_hip_attack_speed_y
                };
                // `emitBlurHipDrop`: the fall's smear (the super pound's flames are the view's,
                // from `pounding`).
                if self.status_state == 2 {
                    self.fx("ms_m_blur3", crate::event::FxAt::Center);
                }
                match self.jump_process(host, 0) {
                    1 => {
                        let drop = (self.jump_peak_y - self.pos.y).max(0.0);
                        self.last_pound_drop = drop;
                        host.event(Event::GroundPound {
                            super_: self.status_state == 3,
                            drop,
                        });
                        let names: &[&'static str] = if self.status_state == 2 {
                            &["ms_hipdrop_c", "ms_hipdrop_b", "ms_hipdrop_a"]
                        } else {
                            &[
                                "ms_m_sphipd_hit_a",
                                "ms_m_sphipd_hit_b",
                                "ms_m_sphipd_hit_c",
                                "ms_m_sphipd_hit_d",
                            ]
                        };
                        for &n in names {
                            self.fx_once(n, 0);
                        }
                        self.change_status(Status::HipAttackEnd, 0, false);
                    }
                    2 => {
                        self.set_player_velocity(-16.0);
                        if self.vel.y > 0.0 {
                            self.vel.y = 0.0;
                        }
                        self.change_status(Status::JumpShortBackDown, 0, false);
                    }
                    _ => {}
                }
            }
        }
        self.model_yaw = self.face_yaw;
        false
    }

    /// `jumpDownCommon`: tumbling after a hard bump.
    fn jump_down(&mut self, host: &mut dyn Host, next: Status, c: ClipName, velocity: f32) -> bool {
        if self.status_timer == 0 {
            self.status_timer += 1;
        }
        self.set_player_velocity(velocity);
        match self.jump_process(host, 0) {
            0 => {
                self.set_anim(c, 1.0);
            }
            1 => {
                self.change_status(next, self.status_arg, false);
            }
            2 => {
                self.set_anim("bkdwn", 1.0);
                self.player_refrection(false, host);
                if self.vel.y > 0.0 {
                    self.vel.y = 0.0;
                }
                self.set_player_velocity(-velocity);
            }
            _ => {}
        }
        false
    }

    /// `stayWall`: sliding down a wall; jump kicks off it.
    fn stay_wall(&mut self, host: &mut dyn Host) -> bool {
        self.status_timer = (self.status_timer + 1).min(60);
        if self.input.jump {
            return self.start_jump_wall();
        }
        if self.check_back_trig() {
            return true;
        }
        if self.status_timer < 20 {
            self.status_timer += 1;
            self.vel = Vec3::ZERO;
        } else {
            self.vel.y = -(self.status_timer as f32) * 0.5;
        }
        if let Some(w) = self.wall {
            self.pos.x -= w.normal.x;
            self.pos.z -= w.normal.z;
        }
        if self.jump_process(host, 0) == 1 {
            self.face_yaw = self.face_yaw.wrapping_add(i16::MIN);
            return self.change_status(Status::Landing, 0, false);
        }
        if self.wall.is_none() {
            self.face_yaw = self.face_yaw.wrapping_add(i16::MIN);
            self.set_player_velocity(0.0);
            self.vel.y = 0.0;
            return self.change_status(Status::Landing, 0, false);
        }
        self.set_anim(clip::WALL_SLIDE, 1.0);
        // `wallSlipEffect`: dust off the wall while sliding down it fast.
        if self.vel.y < -10.0 {
            self.fx("ms_mariwallsl1", crate::event::FxAt::Feet);
        }
        false
    }

    /// `startJumpWall`.
    fn start_jump_wall(&mut self) -> bool {
        if let Some(w) = self.wall {
            let yaw = crate::math::matan(w.normal.z, w.normal.x).wrapping_add(i16::MIN);
            self.fx_once("ms_wallkick_a", yaw);
            self.fx_once("ms_wallkick_b", yaw);
        }
        self.vel.y = 52.0;
        self.face_yaw = self.face_yaw.wrapping_add(i16::MIN);
        if self.vel.y + (HEIGHT + self.pos.y) >= self.roof_y {
            self.vel.y = 1.0;
        }
        self.change_status(Status::WallJump, 0, false)
    }

    fn catch_stop(&mut self, host: &mut dyn Host) -> bool {
        if self.status_state == 0 {
            self.vel.y = 30.0;
            self.status_state = 1;
        }
        self.do_jumping();
        match self.jump_process(host, 0) {
            0 => {
                let c = if self.status_state == 1 {
                    clip::ROLL
                } else {
                    clip::LAND
                };
                self.set_anim(c, 1.0);
            }
            1 => {
                self.change_status(Status::LandEnd, 0, false);
            }
            2 => self.set_player_velocity(0.0),
            _ => {}
        }
        if self.status_state == 1 && self.anim.at_last_frame() {
            self.status_state = 2;
        }
        false
    }

    fn slip_falling(&mut self, host: &mut dyn Host) -> bool {
        self.status_timer += 1;
        if self.status_timer > 120 && self.pos.y - self.floor_y > 500.0 {
            return self.change_status(Status::Landing, 1, false);
        }
        self.do_slip_jumping();
        match self.jump_process(host, 0) {
            1 => {
                if self.status_state == 0 && self.vel.y < 0.0 && self.ground.normal.y >= 0.984_807_7
                {
                    self.vel.y = -self.vel.y / 2.0;
                    self.status_state = 1;
                } else {
                    self.change_status(Status::SlipFore, 0, false);
                }
            }
            2 => {
                if self.vel.y > 0.0 {
                    self.vel.y = 0.0;
                }
                self.change_status(Status::JumpShortBackDown, 0, false);
            }
            _ => {}
        }
        self.set_anim(clip::SLIP, 1.0);
        false
    }
}
