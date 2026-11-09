//! Airborne statuses (Player/MarioJump.cpp).

use glam::Vec3;

use crate::anim::{ClipName, clip};
use crate::event::Event;
use crate::host::{Host, SurfaceFlags};
use crate::mario::{HEIGHT, Mario, Status};
use crate::math::{cos, diff, fconverge, sin, to_angle};
use crate::params::NozzleKind;

/// 120 Hz ticks after a jump pad throws him before a move can break the flight, so a button held through the launch (a B or R from before) doesn't.
pub(crate) const PAD_ACTION_GRACE: u32 = 12;

/// The Turbo corkscrew lasts as long as the stick keeps circling, and this many ticks (a fifth of a second, the length of the stick history)
/// past the last full circle's reading, so a slow circle doesn't drop out between turns.
/// Host extension: the most the stick moves a ground pound sideways in its fall (decomp units per 30 Hz
/// frame; a run is 45 to 80).
const POUND_AIM_SPEED: f32 = 12.0;
const CORKSCREW_GRACE: u16 = 24;
/// There is no cap on either corkscrew (Turbo or plain dive): it goes on as long as the stick circles.
/// A corkscrew that has gone on this long (1 s) is in flames and hits with the super blow.
pub(crate) const CORKSCREW_SUPER_TICKS: u16 = 120;
/// A burning corkscrew's landing slide keeps its speed and its fire this many ticks (2 s).
const BLAZE_TICKS: u16 = 240;
/// The corkscrew's flight speed is the Turbo jump's, floored here, plus this: well past a plain dive's.
const CORKSCREW_MIN_SPEED: f32 = 60.0;
const CORKSCREW_EXTRA_SPEED: f32 = 36.0;
/// The air-speed cap (`jumping_max`) is this much higher for the corkscrew.
pub(crate) const CORKSCREW_CAP: f32 = 1.5;

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
                let from_rocket = self.status == RocketLanding;
                // Host extension: out of the rocket's climb the stick turns him as well as pushing him.
                if from_rocket && self.input.stick {
                    let rot = self.params.hover.rot_sp as i32;
                    self.face_yaw = crate::math::turn_toward(self.face_yaw, self.intended_yaw, rot);
                    self.model_yaw = self.face_yaw;
                }
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
                // Host extension: a full stick circle spins him in the air from here too, out of the
                // rocket's climb included (the spray he still holds must not start the rocket again).
                if self.consider_jump_rotate() {
                    self.rocket_spin = from_rocket;
                    if from_rocket {
                        self.rocket_spin_boost();
                    }
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
                // Host extension: a full stick circle out of the Turbo's thrust is the corkscrew dive.
                if self.turbo_held && self.consider_corkscrew() {
                    return true;
                }
                if !self.turbo_held && (self.check_back_trig() || self.consider_jump_rotate()) {
                    return true;
                }
                self.jumping_basic(host, Run, "broad_jump", 1);
                self.fludd_emitting = self.turbo_held;
                false
            }
            RotateBroadJump => {
                if self.options.redive_after_slide && self.input.b_pressed && self.check_back_trig() {
                    return true;
                }
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
            FireDown => self.fire_downing(host),
            FireJump => self.fire_jumping(host),
            FireLand => self.fire_landing(host),
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
        // Host extension: with the Turbo held in the air the circle is the corkscrew, not a plain spin
        // (which would have the Turbo's water flying round him).
        if self.turbo_held && self.status != Status::JumpCatch && self.consider_corkscrew() {
            return true;
        }
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

    /// Host extension: a spin out of the Rocket's climb doubles the climb. The rise still to come is
    /// `v^2 / 8g` (a tick moves a quarter of `v`); the boost makes it the height climbed so far plus twice that, so
    /// the apex, measured from where the Rocket started, is twice what it would have been.
    fn rocket_spin_boost(&mut self) {
        let g = self.params.jump.gravity.max(0.1);
        let climbed = (self.pos.y - self.rocket_start_y).max(0.0);
        let v = self.vel.y.max(0.0);
        let rest = v * v / (8.0 * g);
        self.vel.y = (8.0 * g * (climbed + 2.0 * rest)).sqrt().max(self.vel.y);
        self.rocket_super = true;
    }

    /// Host extension: out of the Turbo's thrust a full stick circle throws him forward head first in a
    /// dive that rolls about its own length (a corkscrew), then carries on as the dive.
    fn consider_corkscrew(&mut self) -> bool {
        if self.on_yoshi() || self.check_stick_rotate().is_none() {
            return false;
        }
        self.break_pad_flight();
        // On top of the Turbo's speed, not instead of it. (Entering the dive caps its speed at 48, so the
        // corkscrew sets its own after, and holds it.)
        let speed = self.forward_vel.max(CORKSCREW_MIN_SPEED) + CORKSCREW_EXTRA_SPEED;
        self.fludd_emitting = false;
        self.change_status(Status::JumpCatch, 0, false);
        self.corkscrew = CORKSCREW_GRACE;
        self.corkscrew_camera_start = self.flight_camera.map(|a| a.pitch);
        self.corkscrew_age = 0;
        self.corkscrew_used = true;
        self.begin_corkscrew_hold();
        // One call-out of the spin jump's, not the dive's grunt; the animation's own cues are muted.
        self.events.retain(|e| !matches!(e, Event::Jump(crate::event::JumpKind::Dive)));
        self.events.push(Event::Sound(crate::event::SoundCue {
            name: "MSD_SE_MV24_JUMP_SPECIAL_01",
            pos: self.pos,
            info: None,
        }));
        self.corkscrew_speed = speed;
        self.wet_dive = true;
        self.set_player_velocity(speed);
        self.vel.y = 0.0;
        true
    }

    /// `checkBackTrig`: B dives, L ground-pounds.
    fn check_back_trig(&mut self) -> bool {
        if self.air_throw() {
            return false;
        }
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
        // Host extension: a use the host granted this press (`Limits::air_uses`) may start from the fall out of an
        // earlier Rocket or Hover too, which the game's own check refuses.
        let granted = self.limit_granted();
        // Host extension (`Options::aim_rocket`): a charge that fired as a precision slug doesn't launch him.
        if self.rocket_precision {
            return false;
        }
        if self.status == Status::Rocket
            || (self.status == Status::RocketLanding && !granted)
            || self.air_rocket_used
        {
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
        // Host extension: a full stick circle spins him out of the rocket's thrust, as out of any jump
        // (not the Hover nozzle, whose steering is the stick).
        if nozzle != NozzleKind::Hover && self.consider_jump_rotate() {
            self.rocket_spin = true;
            self.rocket_spin_boost();
            return true;
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
        // Host extension: the stick turns him in the rocket's thrust too, as the Hover's does.
        if nozzle != NozzleKind::Hover && self.input.stick {
            let rot = self.params.hover.rot_sp as i32;
            self.face_yaw = crate::math::turn_toward(self.face_yaw, self.intended_yaw, rot);
            self.model_yaw = self.face_yaw;
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
        if self.check_back_trig() || (!self.rocket_spin && self.rocket_check()) {
            return true;
        }
        self.set_anim(clip::SPIN, 1.0);
        self.emit_blur_spin_jump();
        // Host extension: the flaming rocket spin, while it rises, is steered hard by the stick (the
        // Rocket's own 2.5x air control) and turns him toward it, so the climb can be aimed.
        let steer = self.rocket_super && self.vel.y > 0.0 && self.input.stick;
        let mag = self.intended_mag;
        if steer {
            let rot = self.params.hover.rot_sp as i32;
            self.face_yaw = crate::math::turn_toward(self.face_yaw, self.intended_yaw, rot);
            self.intended_mag = mag * 2.5;
        }
        self.jumping_basic(host, Status::SecJumpSlip, clip::SPIN, 0);
        self.intended_mag = mag;
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
        // Host extension: a full stick circle in a plain dive is the corkscrew too, at the dive's own speed.
        if self.corkscrew == 0 && !self.corkscrew_used && !self.on_yoshi() && self.check_stick_rotate().is_some() {
            self.corkscrew = CORKSCREW_GRACE;
            self.corkscrew_camera_start = self.flight_camera.map(|a| a.pitch);
            self.corkscrew_age = 0;
            self.corkscrew_used = true;
            self.begin_corkscrew_hold();
            self.corkscrew_speed = self.forward_vel;
            self.vel.y = 0.0;
            self.events.push(Event::Sound(crate::event::SoundCue {
                name: "MSD_SE_MV24_JUMP_SPECIAL_01",
                pos: self.pos,
                info: None,
            }));
        }
        if self.corkscrew > 0 {
            // The spin jump's own motion laid flat: tipped head first along the dive, spinning about
            // his length at the spin jump's rate (4096 a tick), then the plain dive.
            // It goes on at the same height while the stick keeps turning about its circle; once it
            // stops, the dive.
            self.corkscrew_age = self.corkscrew_age.saturating_add(1);
            if self.corkscrew_age >= CORKSCREW_SUPER_TICKS {
                self.corkscrew_super = true;
            }
            // Host extension: a slow circle holds it as long as it keeps turning
            // (`Options::corkscrew_hold_degrees` a refresh); the game's whole-circle test would
            // ask for a fast one every tick.
            if self.corkscrew_turning() {
                self.corkscrew = CORKSCREW_GRACE;
            } else {
                self.corkscrew -= 1;
            }
            self.set_anim(clip::SPIN, 1.0);
            self.emit_blur_spin_jump();
            self.model_yaw = self.face_yaw;
            self.model_pitch = 0x4000;
            self.model_roll = self.model_roll.wrapping_add(0x1000);
            // Host extension (both Turbo and ordinary dive): camera yaw steers the flight;
            // pitch is a small change relative to the camera at the start of this roll.
            // Keep the stick circle for sustaining the roll, rather than steering in circles.
            // Host extension: a flaming corkscrew (the super one) is fully steerable. It turns and
            // pitches much harder, and more so the faster it flies (the Turbo's is quicker than a plain
            // dive's, and its wider arcs would otherwise feel sluggish), so it can be aimed at a target.
            let (turn_k, pitch_gain, pitch_max, pitch_ease) = if self.corkscrew_super {
                let k = (self.corkscrew_speed / self.params.jump.jump_jump_catch_sp.max(1.0)).clamp(1.0, 3.0);
                (2.0 * k, 1.0, 55.0_f32.to_radians(), (0.15 * k).min(0.3))
            } else {
                (1.0, 0.35, 15.0_f32.to_radians(), 0.08)
            };
            let target_pitch = if let Some(camera) = self.flight_camera {
                let start = *self.corkscrew_camera_start.get_or_insert(camera.pitch);
                let delta = camera.pitch.wrapping_sub(start) as f32 * std::f32::consts::PI / 32768.0;
                let rot = (self.params.hover.rot_sp as f32 * turn_k) as i32;
                self.face_yaw = crate::math::turn_toward(self.face_yaw, camera.yaw, rot);
                (delta * pitch_gain).clamp(-pitch_max, pitch_max)
            } else { 0.0 };
            self.corkscrew_pitch += (target_pitch - self.corkscrew_pitch) * pitch_ease;
            if self.flight_camera.is_none() && self.input.stick {
                let rot = ((self.params.hover.rot_sp as f32 * turn_k) as i32) / 2;
                self.face_yaw = crate::math::turn_toward(self.face_yaw, self.intended_yaw, rot);
            }
            self.model_yaw = self.face_yaw;
            self.model_pitch = 0x4000 - (self.corkscrew_pitch * 32768.0 / std::f32::consts::PI) as i16;
            // Host extension (`Options::slide_dive_nozzles`): an active Turbo keeps speeding the corkscrew up,
            // to the same cap `jump_process` limits its flight to.
            if self.options.slide_dive_nozzles {
                let cap = self.params.jump.jumping_max * CORKSCREW_CAP;
                if self.corkscrew_speed < cap {
                    self.corkscrew_speed = (self.corkscrew_speed + self.turbo_thrust()).min(cap);
                }
            }
            self.set_player_velocity(self.corkscrew_speed * self.corkscrew_pitch.cos());
            self.vel.y = self.corkscrew_speed * self.corkscrew_pitch.sin();
        } else {
            self.model_pitch = 0;
            self.model_roll = 0;
            self.set_anim(clip::DIVE, 1.0);
            self.do_jumping();
        }
        // After a corkscrew, spin and dive both throw the wet slide's spray, and the landing is a wet slide.
        if self.wet_dive {
            self.front_slip_effect();
        }
        match self.jump_process(host, 0) {
            1 => {
                host.event(Event::Land {
                    strong: self.ask_strong_ground_touch(),
                });
                let burning = self.corkscrew_super;
                self.change_status(Status::Catch, 0, false);
                if burning {
                    // Lands in a blazing slide: the flight speed and the fire go on a while.
                    self.blaze = BLAZE_TICKS;
                }
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
        // Host extension: a full stick circle spins the pound (the host widens and strengthens its landing).
        if !self.pound_spin && self.check_stick_rotate().is_some() {
            self.pound_spin = true;
            self.pound_spin_age = 0;
        }
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
                    self.pound_flame_clock = 0.0;
                    self.status_state = 2;
                }
                self.vel.y = 0.0;
                match self.jump_process(host, 0) {
                    1 => return self.change_status(Status::JumpEnd, 0, false),
                    // Host extension: a wall doesn't bump him out of a ground pound; he stays in it.
                    2 => {
                        self.set_player_velocity(0.0);
                    }
                    _ => {}
                }
            }
            _ => {
                self.set_anim(clip::HIP_FALL, 1.0);
                self.status_timer += 1;
                // Host extension (`Options::spin_pound_fast`): a spun pound falls faster, flaming or not, and
                // its flame clock runs as much faster so the flame still starts at the same height.
                let spun = self.options.spin_pound_fast && self.pound_spin;
                let (regular, flaming) = (j.hip_attack_speed_y, j.super_hip_attack_speed_y);
                let regular_spun = (regular * crate::mario::SPIN_POUND_REGULAR_MULT).min(flaming * 1.1);
                self.pound_flame_clock += if spun { regular_spun / regular } else { 1.0 };
                if self.pound_flame_clock > j.super_hip_attack_ct as f32 {
                    self.status_state = 3;
                }
                self.vel.y = match (self.status_state == 2, spun) {
                    (true, false) => regular,
                    (true, true) => regular_spun,
                    (false, false) => flaming,
                    (false, true) => flaming * crate::mario::SPIN_POUND_FLAME_MULT,
                };
                // Host extension: the stick nudges the fall along (up to POUND_AIM_SPEED, a fraction of a
                // run), turning him toward it, so a pound can be aimed a little in motion.
                if self.input.stick {
                    let rot = (self.params.hover.rot_sp as i32) / 2;
                    self.face_yaw = crate::math::turn_toward(self.face_yaw, self.intended_yaw, rot);
                    self.set_player_velocity(POUND_AIM_SPEED * (self.intended_mag / 32.0).clamp(0.0, 1.0));
                } else {
                    self.set_player_velocity(0.0);
                }
                // `emitBlurHipDrop`: the fall's smear (the super pound's flames are the view's,
                // from `pounding`).
                if self.status_state == 2 {
                    self.fx("ms_m_blur3", crate::event::FxAt::Center);
                }
                match self.jump_process(host, 0) {
                    1 => {
                        let drop = (self.jump_peak_y - self.pos.y).max(0.0);
                        self.last_pound_drop = drop;
                        self.last_pound_spin = self.pound_spin;
                        host.event(Event::GroundPound {
                            super_: self.status_state == 3,
                            spin: self.pound_spin,
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
                        self.set_player_velocity(0.0);
                    }
                    _ => {}
                }
            }
        }
        self.model_yaw = if self.pound_spin {
            // The spin jump's swirl (`emitBlurSpinJump`) around him every tick of the spinning fall.
            self.emit_blur_spin_jump();
            self.pound_spin_age = self.pound_spin_age.wrapping_add(1);
            self.face_yaw.wrapping_add((self.pound_spin_age as i32 * 4096) as i16)
        } else {
            self.face_yaw
        };
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

    /// `fireDowning`: thrown up by a fire hit. Mario yells, steers a little, and bounces twice
    /// (0.4 of the fall speed back up) before he lands for good.
    fn fire_downing(&mut self, host: &mut dyn Host) -> bool {
        // The yell comes on the second tick (`mStatusTimer == 1` before the increment).
        if self.status_timer == 1 {
            self.voice(if self.guarded_fire() && self.fire_voice_alt {
                "MSD_SE_MV05_DAMAGE_FIRE_02"
            } else { "MSD_SE_MV05_DAMAGE_FIRE_01" });
        }
        self.status_timer += 1;
        if self.input.stick {
            let angle = self.intended_yaw.wrapping_sub(self.face_yaw);
            let inc = 0.03125 * self.intended_mag * self.params.jump.fire_down_control;
            self.forward_vel += inc * cos(angle);
            self.face_yaw = self
                .face_yaw
                .wrapping_add(to_angle(1024.0 * inc * sin(angle)));
            if self.forward_vel < 0.0 {
                self.face_yaw = self.face_yaw.wrapping_add(i16::MIN);
                self.forward_vel = -self.forward_vel;
            }
            if self.forward_vel > 32.0 {
                self.forward_vel -= 2.0;
            }
        } else {
            self.forward_vel = fconverge(self.forward_vel, 0.0, 0.35, 0.35);
        }
        self.slide_x = self.forward_vel * sin(self.face_yaw);
        self.slide_z = self.forward_vel * cos(self.face_yaw);
        self.vel.x = self.slide_x;
        self.vel.z = self.slide_z;
        match self.jump_process(host, 0) {
            1 => {
                if self.status_state < 2 && self.vel.y < 0.0 {
                    self.vel.y = 0.4 * -self.vel.y;
                    self.set_player_velocity(0.5 * self.forward_vel);
                    self.status_state += 1;
                } else {
                    // The game lies down (`FIRE_JUMP_END`); the host's option gets up running.
                    let to = if self.options.fire_panic_run {
                        Status::FireDash
                    } else {
                        self.voice("MSD_SE_MV07_DAMAGE_REACT_01");
                        Status::FireJumpEnd
                    };
                    self.change_status(to, 0, false);
                }
            }
            2 => self.player_refrection(false, host),
            _ => {}
        }
        self.set_anim("firejmp", 1.0);
        self.fire_flicker();
        false
    }

    /// `fireJumping`: a hop out of the panicked run, which goes on when he lands.
    fn fire_jumping(&mut self, host: &mut dyn Host) -> bool {
        if self.status_timer == 1 && !self.guarded_fire() {
            self.voice("MSD_SE_MV05_DAMAGE_FIRE_01");
        }
        self.status_timer += 1;
        self.set_player_velocity(self.forward_vel);
        if self.jump_process(host, 0) == 1 {
            self.change_status(Status::FireDash, 0, false);
        }
        // `mStatusArg == 0` plays the plain jump clip; the dash's hop (arg 0) does.
        let c = if self.status_arg == 0 { clip::JUMP } else { "firejmp" };
        self.set_anim(c, 1.0);
        self.fire_flicker();
        false
    }

    /// `fireLanding`: off an edge in the panicked run; lands back in it.
    fn fire_landing(&mut self, host: &mut dyn Host) -> bool {
        self.set_player_velocity(self.forward_vel);
        if self.jump_process(host, 0) == 1 {
            self.change_status(Status::FireDash, 0, false);
        }
        self.set_anim(clip::LAND, 1.0);
        self.fire_flicker();
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
        // Host extension to MarioJump.cpp::catchStop: accept a fresh B press during recovery.
        if self.options.redive_after_slide && self.input.b_pressed && self.check_back_trig() {
            return true;
        }
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
