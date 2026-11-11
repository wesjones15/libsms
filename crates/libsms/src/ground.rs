//! Waiting and running statuses (Player/MarioWait.cpp, Player/MarioRun.cpp).
//!
//! Handlers return `true` when they changed status and the state machine should run the new
//! status's handler in the same tick (the decomp's `BOOL` return).

use glam::Vec3;

use crate::anim::{ClipName, clip};
use crate::event::Event;
use crate::host::Host;
use crate::mario::{Mario, Status};
use crate::math::{self, cos, diff, fconverge, matan, sin};
use crate::params::NozzleKind;

/// A turbo sled flies off a crest when its recent climb is at least this fast (`vel.y` units; about 9 degrees at a
/// sled's speed), with at most `SLED_MAX_LIFT` (host extension).
const SLED_MIN_LIFT: f32 = 12.0;
const SLED_MAX_LIFT: f32 = 90.0;
/// Clip frame from which a host may let Mario out of a knock-down ([`Options::knockdown_cancel`]).
const KNOCKDOWN_CANCEL_FRAME: f32 = 12.0;
/// Slide speed under which it may.
const KNOCKDOWN_CANCEL_SPEED: f32 = 4.0;

/// `TMario::JumpSlipRecord`: how long the post-landing window lasts and what A does in it.
struct JumpSlipRec {
    max_timer: i32,
    end: Status,
    jump: Status,
}

impl Mario {
    /// `setAnimation`: returns the current frame.
    pub(crate) fn set_anim(&mut self, c: ClipName, rate: f32) -> f32 {
        // Riding, Mario's clip is Yoshi's (and the leaning one while spitting).
        let c = if self.on_yoshi() {
            let spitting = self
                .fludd
                .as_ref()
                .is_some_and(|f| f.can_spray(&self.params));
            if spitting {
                "yo_run"
            } else {
                crate::yoshi::mario_clip(self.yoshi.clip)
            }
        } else {
            c
        };
        self.anim.set(c, rate, &self.anims);
        self.anim.frame
    }

    // ---- shared transitions ---------------------------------------------------------------

    /// `considerRotateJumpStart`: a full stick circle starts a spin jump.
    pub(crate) fn consider_rotate_jump_start(&mut self) -> bool {
        match self.check_stick_rotate() {
            Some(2) => {
                self.change_status(Status::RightRotateJump, 0, false);
                true
            }
            Some(3) => {
                self.change_status(Status::LeftRotateJump, 0, false);
                true
            }
            _ => false,
        }
    }

    /// `considerRotateStart`: a full stick circle while spraying starts the ground spin.
    pub(crate) fn consider_rotate_start(&mut self) -> bool {
        let spraying = self
            .fludd
            .as_ref()
            .is_some_and(|f| f.can_spray(&self.params));
        if spraying {
            match self.check_stick_rotate() {
                Some(d) if d > 2 => return self.change_status(Status::RotateL, 0, false),
                Some(_) => return self.change_status(Status::RotateR, 0, false),
                None => {}
            }
        }
        false
    }

    /// `setMissJumping`: jumping off a slope too steep to jump from.
    fn set_miss_jumping(&mut self) {
        self.model_yaw = self.face_yaw;
        if self.forward_vel > 0.0 {
            let slope = self.slope_angle.wrapping_add(i16::MIN);
            let angle = diff(self.face_yaw, slope);
            let x = self.forward_vel * sin(angle);
            let z = self.forward_vel * cos(angle) * 0.75;
            self.set_player_velocity((x * x + z * z).sqrt());
            self.face_yaw = slope.wrapping_add(matan(z, x));
        }
        self.change_status(Status::MissJump, 0, false);
    }

    /// `changePlayerJumping`.
    pub(crate) fn change_jumping(&mut self, st: Status) -> bool {
        if self.is_jump_miss() {
            self.set_miss_jumping();
        } else {
            if self.consider_rotate_jump_start() {
                return true;
            }
            self.change_status(st, 0, false);
        }
        true
    }

    /// `changePlayerTriJump`: a plain jump (the chain is decided by the landing windows).
    pub(crate) fn change_tri_jump(&mut self) -> bool {
        self.change_jumping(Status::Jump)
    }

    /// `checkAllMotions`.
    fn check_all_motions(&mut self) -> bool {
        if self.input.jump {
            if self.consider_rotate_jump_start() {
                return true;
            }
            return self.change_status(Status::Jump, 0, false);
        }
        if self.input.airborne {
            return self.change_status(Status::Landing, 0, false);
        }
        if self.input.stick {
            return self.change_status(Status::Run, 0, false);
        }
        if self.input.slip {
            return self.change_status(Status::Slip, 0, false);
        }
        false
    }

    fn any_motion(&self) -> bool {
        let i = &self.input;
        i.stick || i.jump || i.airborne || i.slip
    }

    /// `stopCommon`: stand still, play `c`, go to `next` when it ends.
    pub(crate) fn stop_common(&mut self, host: &mut dyn Host, c: ClipName, next: Status) {
        self.wait_process(host);
        self.set_anim(c, 1.0);
        let ends = if self.on_yoshi() {
            let y = &self.yoshi.ctrl;
            y.frame + y.rate + 0.1 >= y.end
        } else {
            self.anim.at_last_frame()
        };
        if ends {
            self.change_status(next, 0, false);
        }
    }

    fn jump_end_common(&mut self, host: &mut dyn Host, c: ClipName, next: Status) -> bool {
        self.wait_process(host);
        self.set_anim(c, 1.0);
        if self.anim.at_last_frame() {
            return self.change_status(next, 0, false);
        }
        false
    }

    fn jump_end_events(&mut self, jump: Option<Status>) -> bool {
        if self.input.jump {
            return match jump {
                None => self.change_tri_jump(),
                Some(s) => self.change_jumping(s),
            };
        }
        if self.any_motion() {
            return self.check_all_motions();
        }
        false
    }

    // ---- waiting ------------------------------------------------------------------------------

    pub(crate) fn wait_main(&mut self, host: &mut dyn Host) -> bool {
        use Status::*;
        // `waitMain`: B with something in hand puts it down (or throws a throwing kind).
        if self.check_throw_start(host, false) {
            return true;
        }
        match self.status {
            Wait => self.waiting(host),
            Squat => self.squating(host),
            SquatStandup => self.squat_standup(host),
            JumpEnd => {
                if self.jump_end_events(None) {
                    return true;
                }
                self.jump_end_common(host, clip::JMPED, Wait);
                false
            }
            SecJumpEnd => {
                if self.jump_end_events(None) {
                    return true;
                }
                self.jump_end_common(host, clip::JUMP2_END, Wait);
                false
            }
            LandEnd => {
                if self.jump_end_events(None) {
                    return true;
                }
                self.jump_end_common(host, clip::LAND_END, Wait);
                false
            }
            UltraJumpEnd => {
                if self.jump_end_events(Some(Jump)) {
                    return true;
                }
                self.jump_end_common(host, clip::LAND_END, Wait);
                false
            }
            FireJumpEnd => {
                // `fireJumpEnd`: no jump or dive button while he gets up.
                self.input.jump = false;
                self.input.b_pressed = false;
                if self.jump_end_events(None) {
                    return true;
                }
                self.jump_end_common(host, "fjpend", Wait);
                false
            }
            UTurnJumpEnd => {
                if self.jump_end_events(None) {
                    return true;
                }
                self.jump_end_common(host, clip::UTURN_END, Wait);
                self.model_yaw = self.model_yaw.wrapping_add(i16::MIN);
                false
            }
            BroadJumpEnd => {
                self.input.b_pressed = false;
                if self.jump_end_events(Some(Jump)) {
                    return if self.status == Run {
                        self.change_status(SquatStandup, 0, false)
                    } else {
                        true
                    };
                }
                self.jump_end_common(host, clip::SQUAT, SquatStandup);
                false
            }
            HipAttackEnd => {
                self.status_state = 1;
                // Host tuning: A leaps back up (`Options::pound_jump`); the game ignores it.
                if self.options.pound_jump > 0.0 && self.input.jump {
                    self.pound_rebound = Some(self.last_pound_drop);
                    return self.change_status(UltraJump, 0, false);
                }
                if self.input.airborne {
                    self.change_status(Landing, 0, false)
                } else if self.input.slip {
                    self.change_status(SlipFore, 0, false)
                } else {
                    self.jump_end_common(host, clip::HIP_END, SlipEnd);
                    false
                }
            }
            BrakeEnd => {
                if self.any_motion() {
                    return self.check_all_motions();
                }
                self.stop_common(host, clip::BREND, Wait);
                false
            }
            SlipEnd => {
                if self.any_motion() {
                    return self.check_all_motions();
                }
                self.stop_common(host, clip::SLIP_END, Wait);
                false
            }
            _ => false,
        }
    }

    /// `waitingCommonEvents`.
    fn waiting_common_events(&mut self) -> bool {
        // B with nothing in hand reaches out (`mInput & 0x10000`, whose source isn't in the
        // decomp's `checkController`; taken as a B press).
        if self.held.is_none() && self.input.b_pressed {
            return self.change_status(Status::TakePose, 0, false);
        }
        if self.input.jump {
            if self.consider_rotate_jump_start() {
                return true;
            }
            return self.change_jumping(Status::Jump);
        }
        if self.input.airborne {
            return self.change_status(Status::Landing, 0, false);
        }
        if self.input.slip {
            return self.change_status(Status::Slip, 0, false);
        }
        if self.input.stick {
            let rot = self.params.mario.waiting_rot_sp as i32;
            self.face_yaw = math::turn_toward(self.face_yaw, self.intended_yaw, rot);
            if self.intended_mag > self.params.controller.start_to_walk_level {
                self.emit_smoke(self.face_yaw);
                return self.change_status(Status::Run, 0, false);
            }
        }
        if self.can_squat() {
            self.forward_vel = 0.0;
            return self.change_status(Status::Squat, 0, false);
        }
        if self.rocket_check() {
            self.hover_target_y = self.floor_y + self.params.water_gun.hover_height;
            return self.change_status(Status::Rocket, 0, false);
        }
        self.consider_rotate_start()
    }

    fn waiting(&mut self, host: &mut dyn Host) -> bool {
        if self.waiting_common_events() {
            return true;
        }
        let c = if self.intended_mag == 0.0 {
            clip::WAIT
        } else {
            clip::PIVOT
        };
        self.set_anim(c, 1.0);
        // Host aim without a crouch (a lock-on, not the L button): a standing Mario with no
        // stick turns to face it, so FLUDD sprays at the target. Not in the game.
        if self.intended_mag == 0.0 && self.options.turn_to_aim {
            if let Some(a) = self.aim {
                let d = diff(a.yaw, self.face_yaw);
                self.face_yaw = self.face_yaw.wrapping_add(d / 2);
            }
        }
        self.wait_process(host);
        false
    }

    /// `canSquat`: disabled — the squat stance never triggers. (The left trigger's air
    /// ground pound is a separate path and is unaffected.)
    pub(crate) fn can_squat(&self) -> bool {
        false
    }

    fn squating(&mut self, host: &mut dyn Host) -> bool {
        if self.input.airborne {
            return self.change_status(Status::Landing, 0, false);
        }
        if self.input.slip {
            return self.change_status(Status::Slip, 0, false);
        }
        if !self.input.b_held && !self.input.lr_held {
            return self.change_status(Status::SquatStandup, 0, false);
        }
        let Some(f) = self.fludd.as_ref() else {
            return self.change_status(Status::SquatStandup, 0, false);
        };
        let (nozzle, rocket_type) = (f.nozzle, f.nozzle_params(&self.params).rocket_type);
        let emitting = f.can_spray(&self.params);

        if self.input.jump && self.pad.r && nozzle == NozzleKind::Spray {
            return self.change_status(Status::BackJump, 0, false);
        }
        if rocket_type == 1 && emitting && !self.hover_gun_now() {
            self.hover_target_y = self.floor_y + self.params.water_gun.hover_height;
            return self.change_status(Status::Rocket, 0, false);
        }

        if let Some(a) = self.aim {
            // The host's L-button camera: Mario turns to where it looks (half the way a tick),
            // and the stick side-steps.
            let d = diff(a.yaw, self.face_yaw);
            self.face_yaw = self.face_yaw.wrapping_add(d / 2);
        }
        if self.pad.l {
            // Side-step while crouched (getSideWalkValues).
            let d = diff(self.intended_yaw, self.face_yaw);
            let side = self.intended_mag * sin(d) * self.params.run.pumping_slide_sp;
            if side == 0.0 {
                self.set_anim(clip::SQUAT, 1.0);
            } else {
                self.set_anim(clip::SQUAT, side.abs() * 0.5);
            }
            self.pos.x += side * cos(self.face_yaw);
            self.pos.z -= side * sin(self.face_yaw);
        } else if self.pad.r && self.aim.is_none() {
            // Turn on the spot with the stick (aiming).
            let h = self.pad.stick.x;
            let abs_h = h.abs();
            let c = &self.params.controller;
            let (mid, mid_val) = (c.squat_rot_mid_analog, c.squat_rot_mid_value);
            let mut rot = if abs_h < mid {
                mid_val * (abs_h / mid)
            } else {
                mid_val + (1.0 - mid_val) * ((abs_h - mid) / (1.0 - mid))
            };
            if h < 0.0 {
                rot = -rot;
            }
            let step = (rot * -(self.params.mario.waiting_rot_sp as f32)) as i16;
            self.face_yaw = self.face_yaw.wrapping_add(step);
            self.set_anim(clip::SQUAT, 1.0);
        }
        self.wait_process(host);
        false
    }

    fn squat_standup(&mut self, host: &mut dyn Host) -> bool {
        if self.input.airborne {
            return self.change_status(Status::Landing, 0, false);
        }
        if self.input.slip {
            return self.change_status(Status::Slip, 0, false);
        }
        if self.input.jump {
            return self.change_status(Status::Jump, 0, false);
        }
        if self.input.stick {
            return self.change_status(Status::Run, 0, false);
        }
        self.wait_process(host);
        self.set_anim(clip::SQUAT_END, 1.0);
        if self.anim.at_last_frame() {
            self.change_status(Status::Wait, 0, false);
        }
        false
    }

    // ---- running ------------------------------------------------------------------------------

    pub(crate) fn move_main(&mut self, host: &mut dyn Host) -> bool {
        use Status::*;
        match self.status {
            Run => self.running(host),
            RotateL | RotateR => self.rotating(host),
            Turn => self.turning(host),
            TurnEnd => self.turn_end(host),
            Brake => self.braking(host),
            WalkEnd => self.walk_end(host),
            SlipFore => self.slip_fore(host),
            SlipBack => self.slip_back(host),
            Catch => self.catching(host),
            OilRun => self.oil_run(host),
            OilSlip => self.oil_slip(host),
            OilSlope => self.oil_slope(host),
            JumpSlip => self.jump_slip(
                host,
                JumpSlipRec {
                    max_timer: 0x10,
                    end: JumpEnd,
                    jump: SecondJump,
                },
                clip::JMPED,
            ),
            LandSlip => self.jump_slip(
                host,
                JumpSlipRec {
                    max_timer: 0x10,
                    end: LandEnd,
                    jump: SecondJump,
                },
                clip::LAND_END,
            ),
            SecJumpSlip => self.jump_slip(
                host,
                JumpSlipRec {
                    max_timer: 0x10,
                    end: SecJumpEnd,
                    jump: UltraJump,
                },
                clip::JUMP2_END,
            ),
            UTurnJumpSlip => {
                let rec = JumpSlipRec {
                    max_timer: 0x10,
                    end: UTurnJumpEnd,
                    jump: Jump,
                };
                if self.jump_slip_events(&rec) {
                    return true;
                }
                if self.jump_slip_common(host, clip::UTURN_END) != 2 {
                    self.model_yaw = self.model_yaw.wrapping_add(i16::MIN);
                }
                false
            }
            UltraJumpSlip => self.jump_slip(
                host,
                JumpSlipRec {
                    max_timer: 4,
                    end: UltraJumpEnd,
                    jump: Jump,
                },
                clip::ROLL,
            ),
            BroadJumpSlip => {
                if !self.input.b_held {
                    self.input.jump = false;
                }
                self.jump_slip(
                    host,
                    JumpSlipRec {
                        max_timer: 0x18,
                        end: BroadJumpEnd,
                        jump: BroadJump,
                    },
                    clip::SQUAT,
                )
            }
            BackDown => self.downing(host, "bkdwn", 86.0),
            ForeDown => self.downing(host, clip::SLIDE_FORE, 42.0),
            ShortBackDown => self.downing(host, "sdown", 88.0),
            ShortForeDown => self.downing(host, "shfdn", 80.0),
            SafeBackDown => self.downing(host, "sfbdn", 200.0),
            SafeForeDown => self.downing(host, "sffdn", 100.0),
            CatchDown => self.downing(host, "sldwn", 128.0),
            FireDash => self.fire_dashing(host),
            Slip => false,
            _ => false,
        }
    }

    /// `isRunningSlipStart`.
    fn is_running_slip_start(&self) -> bool {
        self.input.slip && (self.forward_vel <= 0.1 || self.is_front_slip(false))
    }

    /// `isRunningTurnning`: stick is more than ~100 degrees off the facing.
    fn is_running_turning(&self) -> bool {
        if self.is_upper_pumping_style() || self.on_yoshi() {
            return false;
        }
        let d = diff(self.intended_yaw, self.face_yaw);
        !(-0x471C..=0x471C).contains(&d)
    }

    fn running(&mut self, host: &mut dyn Host) -> bool {
        self.status_timer += 1;
        if self.check_throw_start(host, true) {
            return true;
        }

        if self.is_running_slip_start() {
            return self.change_status(Status::Slip, 0, false);
        }

        let dash_max = self.params.mario.dash_max;
        if self.fludd_emitting && self.input.jump && self.forward_vel > dash_max - 1.0 {
            return self.change_jumping(Status::BroadJump);
        }
        if self.input.jump {
            return self.change_tri_jump();
        }
        if self.input.dive {
            if self.forward_vel > self.params.run.do_jump_catch_sp && self.intended_mag > 0.75 {
                self.vel.y = 20.0;
                return self.change_status(Status::JumpCatch, 1, false);
            }
            if self.held.is_none() {
                self.change_status(Status::TakePose, 0, false);
            }
        }
        if self.input.idle {
            if self.status_timer > 0xF0
                && self.forward_vel >= 16.0
                && self.ground.normal.y <= 0.173_648_18
            {
                return self.change_status(Status::Brake, 0, false);
            }
            return self.change_status(Status::WalkEnd, 0, false);
        }
        if self.is_running_turning() && self.forward_vel >= self.params.run.turn_need_sp {
            // The skid's dust, kicked up behind.
            let back = self.face_yaw.wrapping_add(i16::MIN);
            self.fx_once("ms_mariwalk1_a", back);
            self.fx_once("ms_mariwalk1_c", back);
            self.fx_once("ms_mariwalk1_b", back);
            return self.change_status(Status::Turn, 0, false);
        }
        if self.can_squat() {
            self.set_player_velocity(0.0);
            return self.change_status(Status::Squat, 0, false);
        }
        if self.rocket_check() {
            self.hover_target_y = self.floor_y + self.params.water_gun.hover_height;
            return self.change_status(Status::Rocket, 0, false);
        }

        self.status_state = 0;
        self.do_running();

        match self.walk_process(host) {
            0 => {
                self.change_status(Status::Landing, 0, false);
                self.set_anim(clip::LAND, 1.0);
            }
            1 => self.do_running_animation(),
            2 if self.on_yoshi() => self.set_player_velocity(0.0),
            2 => {
                if self.forward_vel > self.params.mario.clash_speed {
                    self.bump_stars();
                    host.event(Event::WallBump);
                    return self.change_status(Status::JumpShortBackDown, 0, false);
                }
                if self.input.jump {
                    // Wall kick: jump pressed while pushing into a wall that reaches
                    // `jump_wall_height` (isTouchedOneWall at y + mJumpWallHeight).
                    let c = Vec3::new(
                        self.pos.x,
                        self.pos.y + self.params.mario.jump_wall_height,
                        self.pos.z,
                    );
                    let mut hits = Vec::new();
                    host.walls(c, 50.0, 1, &mut hits);
                    if !hits.is_empty() {
                        self.vel.y = 52.0;
                        self.face_yaw = self.face_yaw.wrapping_add(i16::MIN);
                        self.set_player_velocity(50.0);
                        return self.change_status(Status::WallJump, 0, false);
                    }
                }
                self.set_anim(clip::PUSH, 1.0);
                self.dash_timer = 0;
                self.fludd_emitting = false;
            }
            _ => {}
        }
        self.check_descent(host);

        if self.fludd_emitting {
            let v = self.params.mario.dash_max;
            self.set_player_velocity(v);
            // `startSoundActor(MSD_SE_PO_SPREAD)`, every tick of the dash (a held sound).
            self.cue("MSD_SE_PO_SPREAD");
            // `TMarioEffect::perform`: `ms_watboost_a` and `_b` bound to the emit matrix while it lasts.
            self.fx("ms_watboost_a", crate::event::FxAt::Muzzle(0));
            self.fx("ms_watboost_b", crate::event::FxAt::Muzzle(0));
        }
        false
    }

    /// `fireDashing`: running about in a panic. Mario can't stop: he runs at about 32, turns
    /// toward the stick a little each tick, and hops (A) or tumbles off edges, until the panic
    /// wears off after 0xA0 ticks or he reaches water.
    fn fire_dashing(&mut self, host: &mut dyn Host) -> bool {
        // The host panic uses one alternating fire voice per ignition, from fire_downing.
        if self.input.jump {
            return self.change_status(Status::FireJump, 0, false);
        }
        let timer = self.status_timer;
        self.status_timer += 1;
        if timer > 0xA0 || self.in_water {
            return self.change_status(Status::Run, 0, false);
        }
        self.forward_vel = self.forward_vel.clamp(8.0, 48.0);
        self.forward_vel = fconverge(self.forward_vel, 32.0, 4.0, 1.0);
        if self.input.stick {
            self.face_yaw = math::turn_toward(self.face_yaw, self.intended_yaw, 0x600);
        }
        self.slope_process();
        if self.walk_process(host) == 0 {
            self.change_status(Status::FireLand, 0, false);
        }
        self.set_anim("firejmp", 0.1 * (0.5 * self.forward_vel));
        self.fire_flicker();
        false
    }

    /// `doRunningAnimation`: picks idle-step/walk-run blend/turbo dash by speed.
    fn do_running_animation(&mut self) {
        let sp = self.intended_mag.max(self.forward_vel).max(4.0);
        let dash_max = self.params.mario.dash_max;
        if self.forward_vel >= dash_max - 1.0 {
            self.set_anim(clip::TURBO_DASH, 1.0);
        } else if sp < self.params.run.soft_to_walk {
            self.set_anim(clip::SSTEP, (sp.max(0.1)) * 0.15);
        } else {
            self.set_anim(clip::RUN2, sp * 0.055 + 1.0);
        }
    }

    fn rotating(&mut self, host: &mut dyn Host) -> bool {
        if self.input.jump {
            let to = if self.status == Status::RotateL {
                Status::RightRotateJump
            } else {
                Status::LeftRotateJump
            };
            return self.change_status(to, 0, false);
        }
        self.set_anim(clip::SPIN, 1.0);
        // `rotating`: the water-shot effects and the swirl, every tick of the spin.
        self.emit_rotate_shoot_effect();
        self.emit_blur_spin_jump();
        self.status_timer += 1;
        if self.status_timer > 0x78 {
            return self.change_status(Status::Wait, 0, false);
        }
        self.do_running();
        if self.walk_process(host) == 0 {
            self.change_status(Status::Landing, 0, false);
        }
        let t = (self.status_timer * 4096) as i16;
        self.model_yaw = if self.status == Status::RotateL {
            t
        } else {
            t.wrapping_neg()
        };
        false
    }

    fn turning(&mut self, host: &mut dyn Host) -> bool {
        if self.input.slip {
            return self.change_status(Status::Slip, 0, false);
        }
        if self.input.jump {
            return self.change_jumping(Status::UTurnJump);
        }
        if self.consider_rotate_start() {
            return true;
        }
        if self.input.idle {
            return self.change_status(Status::Brake, 0, false);
        }
        if !self.is_running_turning() {
            return self.change_status(Status::Run, 0, false);
        }

        self.forward_vel = fconverge(self.forward_vel, 0.0, 4.0, 4.0);
        let zeroed = self.forward_vel == 0.0;
        self.slope_process();
        if zeroed {
            self.face_yaw = self.intended_yaw;
            self.set_player_velocity(8.0);
            return self.change_status(Status::TurnEnd, 0, false);
        }
        if self.walk_process(host) == 0 {
            self.change_status(Status::Landing, 0, false);
        }
        if self.forward_vel >= 18.0 {
            self.set_anim(clip::TURN, 1.0);
        } else {
            self.set_anim(clip::TRNED, 1.0);
            if self.anim.at_last_frame() {
                let v = self.forward_vel;
                self.face_yaw = self.intended_yaw;
                self.set_player_velocity(if v > 0.0 { -v } else { 8.0 });
                self.change_status(Status::Run, 0, false);
            }
        }
        false
    }

    fn turn_end(&mut self, host: &mut dyn Host) -> bool {
        if self.input.slip {
            return self.change_status(Status::Slip, 0, false);
        }
        if self.input.jump {
            return self.change_jumping(Status::UTurnJump);
        }
        if self.consider_rotate_start() {
            return true;
        }
        self.do_running();
        self.set_anim(clip::TRNED, 1.0);
        if self.walk_process(host) == 0 {
            self.change_status(Status::Landing, 0, false);
        }
        if self.anim.at_last_frame() {
            self.change_status(Status::Run, 0, false);
        }
        self.model_yaw = self.model_yaw.wrapping_add(i16::MIN);
        false
    }

    fn braking(&mut self, host: &mut dyn Host) -> bool {
        if self.any_motion() {
            return self.check_all_motions();
        }
        self.forward_vel = fconverge(self.forward_vel, 0.0, 4.0, 4.0);
        let zeroed = self.forward_vel == 0.0;
        self.slope_process();
        if zeroed {
            return self.change_status(Status::BrakeEnd, 0, false);
        }
        match self.walk_process(host) {
            0 => {
                self.change_status(Status::Landing, 0, false);
            }
            2 => {
                if self.forward_vel > 16.0 {
                    self.player_refrection(true, host);
                    self.change_status(Status::ShortBackDown, 0, false);
                } else {
                    self.set_player_velocity(0.0);
                    self.change_status(Status::BrakeEnd, 0, false);
                }
            }
            _ => {}
        }
        self.set_anim(clip::BRAKE, 1.0);
        false
    }

    fn walk_end(&mut self, host: &mut dyn Host) -> bool {
        if self.is_running_slip_start() {
            return self.change_status(Status::Slip, 0, false);
        }
        if self.input.jump {
            return self.change_tri_jump();
        }
        if self.input.stick {
            return self.change_status(Status::Run, 0, false);
        }
        if self.input.dive && self.held.is_none() {
            return self.change_status(Status::TakePose, 0, false);
        }
        if self.consider_rotate_start() {
            return true;
        }
        self.forward_vel = fconverge(self.forward_vel, 0.0, 1.0, 1.0);
        let zeroed = self.forward_vel == 0.0;
        self.set_player_velocity(self.forward_vel);
        if zeroed {
            return self.change_status(Status::Wait, 0, false);
        }
        match self.walk_process(host) {
            0 => {
                self.change_status(Status::Landing, 0, false);
            }
            2 => self.set_player_velocity(0.0),
            _ => {}
        }
        let rate = (0.25 * self.forward_vel).max(0.1);
        self.set_anim(clip::RUN1, rate);
        false
    }

    // ---- slipping -----------------------------------------------------------------------------

    /// `slippingBasic`.
    fn slipping_basic(
        &mut self,
        host: &mut dyn Host,
        on_stop: Status,
        on_fall: Status,
        anim: ClipName,
    ) {
        if self.input.jump && self.can_slip_jump() {
            self.change_status(Status::Jump, 0, false);
            return;
        }
        if self.input.dive {
            self.vel.y = 20.0;
            self.change_status(Status::JumpCatch, 1, false);
            return;
        }
        match self.walk_process(host) {
            0 => {
                self.change_status(on_fall, 0, false);
            }
            1 => {
                self.set_anim(anim, 1.0);
                self.front_slip_effect();
            }
            2 if self.on_yoshi() => self.set_player_velocity(0.0),
            2 => {
                if self.forward_vel.abs() > self.params.mario.clash_speed {
                    self.bump_stars();
                    host.event(Event::WallBump);
                }
                if self.is_slip_start() {
                    if let Some(w) = self.wall {
                        let wall_ang = matan(w.normal.z, w.normal.x);
                        let mag = ((self.slide_x * self.slide_x + self.slide_z * self.slide_z)
                            .sqrt()
                            * 0.9)
                            .max(4.0);
                        self.slide_dir = wall_ang
                            .wrapping_sub(diff(self.slide_dir, wall_ang))
                            .wrapping_add(i16::MIN);
                        self.slide_x = mag * sin(self.slide_dir);
                        self.slide_z = mag * cos(self.slide_dir);
                        self.vel.x = self.slide_x;
                        self.vel.z = self.slide_z;
                    }
                } else if self.forward_vel > 16.0 {
                    self.player_refrection(true, host);
                    self.change_status(Status::CatchDown, 0, false);
                } else {
                    self.set_player_velocity(0.0);
                    self.change_status(on_stop, 0, false);
                }
            }
            _ => {}
        }
    }

    /// `slipFore`/`slipForeCommon`: sliding downhill facing downhill.
    fn slip_fore(&mut self, host: &mut dyn Host) -> bool {
        if self.status_timer > 20 && self.can_slip_jump() {
            if self.input.jump {
                return self.change_jumping(Status::Jump);
            }
        } else {
            self.status_timer += 1;
        }
        let stop = self.slip_params().slide_stop_normal;
        if self.do_sliding(stop) {
            return self.change_status(Status::SlipEnd, 0, false);
        }
        self.slipping_basic(host, Status::SlipEnd, Status::SlipFall, clip::SLIP);
        false
    }

    /// `slipBack`/`slipBackCommon`: sliding downhill facing uphill.
    pub(crate) fn slip_back(&mut self, host: &mut dyn Host) -> bool {
        if self.status_timer > 20 {
            if !self.input.slip && self.input.jump && self.can_slip_jump() {
                return self.change_status(Status::CatchStop, 0, false);
            }
        } else {
            self.status_timer += 1;
        }
        let stop = self.slip_params().slide_stop_normal;
        if self.do_sliding(stop) {
            return self.change_status(Status::CatchLost, 0, false);
        }
        self.slipping_basic(host, Status::CatchLost, Status::Landing, "slpbk");
        false
    }

    /// Host extension (`Options::slide_dive_nozzles`): a Turbo belly slide is a sled. The ground it climbs gives him
    /// an upward speed (`sled_vy`, the smoothed rise per tick in `vel.y` units); when the ground flattens, falls
    /// away or ends under him at a crest or lip, he flies off it with that speed as a dive, keeping the sled's
    /// speed (up to the corkscrew's flight cap), and lands back in a slide. `y0` and `speed0` are the height
    /// and speed before this tick's slide step.
    fn sled_launch(&mut self, y0: f32, speed0: f32) {
        if !(self.options.slide_dive_nozzles && self.turbo_held) || self.on_yoshi() {
            self.sled_vy = 0.0;
            return;
        }
        let rise = (self.pos.y - y0) * 4.0;
        let sliding = self.status == Status::Catch;
        let left_ground = self.status == Status::Landing;
        let crest = sliding && self.sled_vy > SLED_MIN_LIFT && rise < self.sled_vy * 0.5;
        if crest || (left_ground && self.sled_vy > 0.0) {
            self.vel.y = self.sled_vy.min(SLED_MAX_LIFT);
            self.sled_vy = 0.0;
            self.change_status(Status::JumpCatch, 1, false);
            self.forward_vel = speed0;
            self.set_player_velocity(speed0);
            self.sled_air = true;
        } else if sliding {
            self.sled_vy = 0.5 * self.sled_vy + 0.5 * rise;
        } else {
            self.sled_vy = 0.0;
        }
    }

    fn catching(&mut self, host: &mut dyn Host) -> bool {
        if !self.input.slip && self.input.jump {
            if self.forward_vel > self.params.mario.clash_speed {
                return self.change_status(Status::RotateBroadJump, 0, false);
            }
            return self.change_status(Status::CatchStop, 0, false);
        }
        // FLUDD's water on the ground: the belly slide glides (`do_sliding`). Hosts may cap
        // its duration (`WetSlide::timed`) or supply their own availability rules.
        self.status_state = self.limit_wet_slide() as u32;
        if self.blaze > 0 {
            // A burning corkscrew's slide holds its flight speed, then turns into the plain wet slide.
            self.blaze -= 1;
            self.set_player_velocity(self.corkscrew_speed);
        }
        self.rocket_boost_step();
        // Host extension (`Options::slide_dive_nozzles`): a held Turbo pushes the slide along too.
        if self.options.slide_dive_nozzles {
            let (thrust, cap) = (self.turbo_thrust(), self.params.mario.dash_max);
            self.push_along(thrust, cap);
        }
        let stop = self.slip_params().slide_stop_catch;
        if self.do_sliding(stop) {
            self.set_player_velocity(0.0);
            return self.change_status(Status::CatchLost, 0, false);
        }
        let (y0, speed0) = (self.pos.y, self.forward_vel);
        self.slipping_basic(host, Status::CatchLost, Status::Landing, clip::DIVE);
        self.sled_launch(y0, speed0);
        self.sound("MSD_SE_MA_SLIP", None);
        // The dive clip holds its belly-slide pose (frame 50) for as long as the slide lasts.
        if self.anim.frame > 50.0 {
            self.anim.frame = 50.0;
        }
        false
    }

    // ---- post-landing windows (the double/triple jump chain) -----------------------------------

    fn jump_slip_events(&mut self, rec: &JumpSlipRec) -> bool {
        self.status_timer += 1;
        if self.status_timer >= rec.max_timer {
            // Host tuning: the chain stays on offer a little longer (`Options::chain_grace_ticks`;
            // counted down from the next tick).
            let grace = self.options.chain_grace_ticks;
            if grace > 0 && matches!(rec.jump, Status::SecondJump | Status::UltraJump) {
                self.chain_grace = Some((rec.jump, grace + 1));
            }
            return self.change_status(rec.end, 0, false);
        }
        if self.input.jump {
            let j = &self.params.jump;
            if rec.jump == Status::SecondJump && self.forward_vel >= j.sec_jump_enable_sp {
                return self.change_jumping(Status::SecondJump);
            }
            if rec.jump == Status::UltraJump && self.forward_vel >= j.tri_jump_enable_sp {
                return self.change_jumping(Status::UltraJump);
            }
            return self.change_jumping(Status::Jump);
        }
        if self.input.dive {
            self.vel.y = 20.0;
            return self.change_status(Status::JumpCatch, 1, false);
        }
        if self.input.airborne {
            return self.change_status(Status::Landing, 0, false);
        }
        false
    }

    /// `jumpSlipCommon`: slide to a stop after landing.
    fn jump_slip_common(&mut self, host: &mut dyn Host, c: ClipName) -> i32 {
        if self.input.stick {
            self.slope_process();
            self.forward_vel *= 0.98;
            if self.forward_vel * self.forward_vel < 1.0 {
                self.set_player_velocity(0.0);
            }
        } else if self.forward_vel >= 16.0 {
            self.forward_vel = fconverge(self.forward_vel, 0.0, 4.0, 4.0);
            self.slope_process();
        } else {
            self.vel.y = 0.0;
        }
        let r = self.walk_process(host);
        match r {
            0 => {
                self.change_status(Status::Landing, 0, false);
            }
            2 => {
                self.set_anim(clip::PUSH, 1.0);
            }
            _ => {}
        }
        self.set_anim(c, 1.0);
        r
    }

    fn jump_slip(&mut self, host: &mut dyn Host, rec: JumpSlipRec, c: ClipName) -> bool {
        if self.jump_slip_events(&rec) {
            return true;
        }
        self.jump_slip_common(host, c);
        false
    }

    // ---- knock-downs --------------------------------------------------------------------------

    /// `downingCommon`: slide while the get-up clip is before `limit`, then stand.
    fn downing(&mut self, host: &mut dyn Host, c: ClipName, limit: f32) -> bool {
        if self.status_timer == 0 {
            self.status_timer += 1;
            self.bump_stars();
            host.event(Event::WallBump);
        }
        let frame = self.set_anim(c, 1.0);
        // Not in the game: the host lets him out of the get-up once the slide has spent itself.
        if self.options.knockdown_cancel
            && frame >= KNOCKDOWN_CANCEL_FRAME
            && self.forward_vel.abs() < KNOCKDOWN_CANCEL_SPEED
            && (self.input.stick
                || self.input.jump
                || self.input.b_pressed
                || self.input.r_pressed
                || self.input.l_pressed)
        {
            if self.input.jump {
                return self.change_jumping(Status::Jump);
            }
            self.change_status(Status::Wait, 0, false);
            return true;
        }
        if frame < limit {
            self.slope_process();
            self.forward_vel *= 0.96;
            if self.forward_vel * self.forward_vel < 1.0 {
                self.set_player_velocity(0.0);
            }
        } else {
            self.set_player_velocity(if self.forward_vel >= 0.0 { 0.1 } else { -0.1 });
        }
        if self.walk_process(host) == 0 {
            let to = if self.forward_vel >= 0.0 {
                Status::JumpShortForeDown
            } else {
                Status::JumpShortBackDown
            };
            self.change_status(to, self.status_arg, false);
        } else if self.anim.at_last_frame() {
            self.change_status(Status::Wait, 0, false);
        }
        false
    }

    /// Actions: only `CatchLost` (getting up after a belly slide).
    pub(crate) fn action_main(&mut self, host: &mut dyn Host) -> bool {
        match self.status {
            Status::Take => return self.taking(host),
            Status::TakePose => return self.take_pose(host),
            Status::Putting => return self.putting(host),
            Status::Pitching => return self.pitching(host),
            Status::CatchLost => {}
            _ => return false,
        }
        if self.input.jump {
            if self.consider_rotate_jump_start() {
                return true;
            }
            return self.change_jumping(Status::Jump);
        }
        if self.input.airborne {
            return self.change_status(Status::Landing, 0, false);
        }
        if self.input.slip {
            return self.change_status(Status::Slip, 0, false);
        }
        self.stop_common(host, "lost", Status::Wait);
        false
    }

    pub(crate) fn state_machine_upper(&mut self, pad: &crate::input::Pad) {
        self.upper_state_update(pad);
    }
}
