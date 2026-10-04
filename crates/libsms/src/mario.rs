//! Mario's per-tick state and the top-level tick (`TMario::playerControl`,
//! Player/MarioMove.cpp), plus status changes and the shared helpers.
//!
//! The state machine is split by decomp file: `ground.rs` (MarioWait/MarioRun), `air.rs`
//! (MarioJump), `physics.rs` (MarioPhysics and the slope/slide math in MarioRun), `fludd.rs`
//! (WaterGun, nozzles, MarioUpper).

use glam::Vec3;

use crate::anim::{AnimState, AnimTable, clip};
use crate::event::{Event, JumpKind};
use crate::fludd::Fludd;
use crate::host::{GroundHit, Host, RoofHit, SurfaceFlags, WallHit};
use crate::input::{Input, Pad, lr_level, stick_intent};
use crate::math::{self, Angle, cos, sin};
use crate::params::Params;

/// Floor height reported over a void (checkGround returns a very low plane).
pub const ILLEGAL_FLOOR: f32 = -2.0e5;
/// Mario's standing height (head clearance checks use `y + 160`).
pub const HEIGHT: f32 = 160.0;
/// Highest ledge he walks up onto in one step.
pub const STEP_UP: f32 = 100.0;
/// Wall collision radius (`unk15C`).
pub const WALL_RADIUS: f32 = 50.0;

/// Mario's statuses. Names follow `MARIO_STATUS_*` in Player/MarioStatus.hpp. Statuses this
/// crate does not implement yet (swimming, bars, wires, Yoshi, demos...) are absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {
    // Waiting (type 0x000)
    Wait,
    Squat,
    SquatStandup,
    JumpEnd,
    SecJumpEnd,
    LandEnd,
    UTurnJumpEnd,
    UltraJumpEnd,
    BroadJumpEnd,
    HipAttackEnd,
    BrakeEnd,
    SlipEnd,
    // Running (type 0x040)
    Run,
    RotateL,
    RotateR,
    Turn,
    TurnEnd,
    Brake,
    WalkEnd,
    /// Request only: becomes `SlipFore` or `SlipBack` depending on slope direction.
    Slip,
    SlipFore,
    SlipBack,
    Catch,
    JumpSlip,
    LandSlip,
    SecJumpSlip,
    UTurnJumpSlip,
    UltraJumpSlip,
    BroadJumpSlip,
    BackDown,
    ForeDown,
    ShortBackDown,
    ShortForeDown,
    CatchDown,
    // Jumping (type 0x080)
    Jump,
    SecondJump,
    UltraJump,
    BackJump,
    ForceJump,
    WallJump,
    MissJump,
    UTurnJump,
    BroadJump,
    RotateBroadJump,
    /// The "dive jump" (belly flop).
    JumpCatch,
    Rocket,
    Landing,
    RocketLanding,
    SlipFall,
    HipDrop,
    LeftRotateJump,
    RightRotateJump,
    CatchStop,
    WallSlide,
    LandSafeDown,
    JumpShortBackDown,
    JumpShortForeDown,
    JumpForeDown,
    JumpBackDown,
    // Actions (type 0x180)
    CatchLost,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Waiting,
    Running,
    Jumping,
    Action,
}

impl Status {
    pub fn kind(self) -> Kind {
        use Status::*;
        match self {
            Wait | Squat | SquatStandup | JumpEnd | SecJumpEnd | LandEnd | UTurnJumpEnd
            | UltraJumpEnd | BroadJumpEnd | HipAttackEnd | BrakeEnd | SlipEnd => Kind::Waiting,
            Run | RotateL | RotateR | Turn | TurnEnd | Brake | WalkEnd | Slip | SlipFore
            | SlipBack | Catch | JumpSlip | LandSlip | SecJumpSlip | UTurnJumpSlip
            | UltraJumpSlip | BroadJumpSlip | BackDown | ForeDown | ShortBackDown
            | ShortForeDown | CatchDown => Kind::Running,
            CatchLost => Kind::Action,
            _ => Kind::Jumping,
        }
    }

    /// `MARIO_STATUS_FLAG_JUMPING` (0x800): the airborne family.
    pub fn is_jumping(self) -> bool {
        self.kind() == Kind::Jumping
    }

    /// Statuses with decomp flag 0x2000000, which enable releasing the jump button to cut a
    /// rising jump short (`isFallCancel`).
    fn allows_short_hop(self) -> bool {
        use Status::*;
        matches!(
            self,
            Jump | SecondJump | WallJump | MissJump | RotateBroadJump | SlipFall
        )
    }
}

/// Upper-body state (`TMario::UPPER_STATE_*`), which decides when FLUDD may fire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UpperState {
    #[default]
    Idle,
    /// R held: nozzle active.
    Pumping,
    /// Just released: the pump stays up for a moment (`mPumpWaitTime`).
    HoldingPump,
}

/// Build-time options for behaviours that need states this crate does not have yet.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Options {
    /// Grab ledges while jumping (needs the hanging states, not implemented). When false,
    /// Mario bumps into ledges instead.
    pub ledge_grab: bool,
    /// Literal reading of `TNozzleBase::movement` (WaterGun.cpp): the continuous nozzles' output
    /// is `(pressed - gate)` where the gate ramps up to the pressed level at `mTriggerRate`
    /// per tick, so a held trigger fades to nothing after about 6 s. That function matches the
    /// binary 99.8%, but it contradicts how the game plays as far as we know, so the default
    /// is a steady output set by trigger pressure. Needs in-game verification; see the spec.
    pub spray_pressure_fades: bool,
}

#[derive(Debug, Clone)]
pub struct Mario {
    pub params: Params,
    pub anims: AnimTable,
    pub options: Options,

    pub pos: Vec3,
    pub prev_pos: Vec3,
    /// `mVel`: units per 30 Hz frame; position advances by a quarter of it each tick.
    pub vel: Vec3,
    pub face_yaw: Angle,
    /// Yaw the model is drawn at (differs from `face_yaw` in spins, turns, U-turn jumps).
    pub model_yaw: Angle,
    pub forward_vel: f32,
    pub(crate) slide_x: f32,
    pub(crate) slide_z: f32,
    /// Direction of slide travel (`unk9E`).
    pub(crate) slide_dir: Angle,

    pub status: Status,
    pub prev_status: Status,
    pub(crate) status_arg: u32,
    pub(crate) status_state: u32,
    pub(crate) status_timer: i32,

    pub intended_mag: f32,
    pub intended_yaw: Angle,
    pub input: Input,
    pub(crate) pad: Pad,
    pub(crate) pad_prev: Pad,
    pub(crate) events: Vec<Event>,
    pub(crate) camera_yaw: Angle,
    pub(crate) stick_hist: Vec<Angle>,

    pub(crate) ground: GroundHit,
    pub(crate) floor_y: f32,
    pub(crate) roof: Option<RoofHit>,
    pub(crate) roof_y: f32,
    pub(crate) wall: Option<WallHit>,
    pub(crate) slope_angle: Angle,
    pub(crate) radius: f32,
    /// Highest Y reached this jump (`unk104`).
    pub(crate) jump_peak_y: f32,
    /// Y at take-off, for the hard-landing check (`unk2A8.y`).
    pub(crate) fall_start_y: f32,
    pub(crate) short_hop: bool,

    pub anim: AnimState,

    pub fludd: Option<Fludd>,
    pub upper: UpperState,
    pub(crate) pump_cooldown: i32,
    pub(crate) dash_speed: f32,
    pub(crate) dash_timer: i32,
    pub(crate) fludd_emitting: bool,
    pub(crate) hover_target_y: f32,
    pub(crate) in_water: bool,

    pub ticks: u64,
}

impl Mario {
    pub fn new(params: Params, anims: AnimTable, pos: Vec3, face_yaw: Angle) -> Mario {
        let anim = AnimState::new(clip::WAIT, &anims);
        Mario {
            params,
            anims,
            options: Options::default(),
            pos,
            prev_pos: pos,
            vel: Vec3::ZERO,
            face_yaw,
            model_yaw: face_yaw,
            forward_vel: 0.0,
            slide_x: 0.0,
            slide_z: 0.0,
            slide_dir: face_yaw,
            status: Status::Wait,
            prev_status: Status::Wait,
            status_arg: 0,
            status_state: 0,
            status_timer: 0,
            intended_mag: 0.0,
            intended_yaw: face_yaw,
            input: Input::default(),
            pad: Pad::default(),
            pad_prev: Pad::default(),
            events: Vec::new(),
            camera_yaw: 0,
            stick_hist: Vec::new(),
            ground: GroundHit {
                height: pos.y,
                normal: Vec3::Y,
                flags: SurfaceFlags::NONE,
            },
            floor_y: pos.y,
            roof: None,
            roof_y: f32::MAX,
            wall: None,
            slope_angle: 0,
            radius: WALL_RADIUS,
            jump_peak_y: pos.y,
            fall_start_y: pos.y,
            short_hop: false,
            anim,
            fludd: None,
            upper: UpperState::Idle,
            pump_cooldown: 0,
            dash_speed: 0.0,
            dash_timer: 0,
            fludd_emitting: false,
            hover_target_y: pos.y,
            in_water: false,
            ticks: 0,
        }
    }

    /// Gives Mario FLUDD with a full Spray tank.
    pub fn give_fludd(&mut self) {
        self.fludd = Some(Fludd::new(&self.params));
    }

    pub fn has_fludd(&self) -> bool {
        self.fludd.is_some()
    }

    /// Current floor height under Mario (`mFloorPosition.y`).
    pub fn floor_y(&self) -> f32 {
        self.floor_y
    }

    pub fn on_ground(&self) -> bool {
        self.pos.y <= self.floor_y + 4.0
    }

    /// Speed over ground, signed by facing.
    pub fn speed(&self) -> f32 {
        self.forward_vel
    }

    // ---- the tick -----------------------------------------------------------------------

    /// One 120 Hz step (`TMario::playerControl`).
    pub fn tick(&mut self, host: &mut dyn Host, pad: &Pad, camera_yaw: Angle) {
        self.camera_yaw = camera_yaw;
        self.pad = *pad;
        self.prev_pos = self.pos;

        self.check_player_action(host, pad);
        self.state_machine(host);
        self.state_machine_upper(pad);
        self.think_height(host);
        self.gun_exec(host, pad);

        self.anim.advance();
        self.pad_prev = *pad;
        for e in self.events.drain(..) {
            host.event(e);
        }
        self.ticks += 1;
    }

    /// `checkPlayerAction`: controller, stick history, ground/roof/wall probes.
    fn check_player_action(&mut self, host: &mut dyn Host, pad: &Pad) {
        self.input = Input::default();
        self.check_controller(pad);
        self.make_history();
        self.check_current_plane(host);
        if !(self.input.stick || self.input.jump) {
            self.input.idle = true;
        }
    }

    /// `checkController` (the part that fills `mInput`).
    fn check_controller(&mut self, pad: &Pad) {
        let (mag, yaw) = stick_intent(
            pad.stick,
            self.camera_yaw,
            self.face_yaw,
            &self.params.controller,
        );
        self.intended_mag = mag;
        self.intended_yaw = yaw;

        let r_level = lr_level(pad.r_analog, &self.params.controller);
        self.turbo_dash_control(pad, r_level);

        let prev = self.pad_prev;
        let a_edge = pad.a && !prev.a;
        let b_edge = pad.b && !prev.b;
        let l_edge = pad.l && !prev.l;
        let r_edge = pad.r && !prev.r;

        let i = &mut self.input;
        i.stick = self.intended_mag > 0.0;
        i.jump = a_edge;
        i.jump_held = pad.a;
        i.b_held = pad.b;
        i.b_pressed = b_edge;
        i.l_pressed = l_edge;
        if b_edge {
            i.dive = true;
        }
        if l_edge && self.status.is_jumping() {
            i.dive = true;
        }
        if self.fludd.is_some() && !self.fludd_emitting {
            if pad.r || pad.l {
                i.lr_held = true;
            }
            if r_edge {
                i.r_pressed = true;
            }
        }
    }

    /// `makeHistory`: remembers the stick direction for spin detection.
    fn make_history(&mut self) {
        let cap = self.params.controller.stick_rotate_time.max(1) as usize;
        if self.intended_mag > 0.0 {
            self.stick_hist.push(self.intended_yaw);
            if self.stick_hist.len() >= cap {
                self.stick_hist.remove(0);
            }
        } else {
            self.stick_hist.clear();
        }
    }

    /// `checkStickRotate`: a full circle of the stick. Returns 2 (clockwise-ish, right spin)
    /// or 3 (left spin). The decomp leaves its quadrant flags uninitialised; we start them
    /// false, which is what the code intends.
    pub(crate) fn check_stick_rotate(&self) -> Option<u8> {
        let h = &self.stick_hist;
        if h.len() < 2 {
            return None;
        }
        let (mut inc, mut dec) = (0, 0);
        let mut q = [false; 4];
        for w in h.windows(2) {
            let a = w[0] as f32;
            if !(-24576.0..=24576.0).contains(&a) {
                q[0] = true;
            }
            if (-24576.0..=-8192.0).contains(&a) {
                q[1] = true;
            }
            if a > -8192.0 && a < 8192.0 {
                q[2] = true;
            }
            if (8192.0..=24576.0).contains(&a) {
                q[3] = true;
            }
            if a < w[1] as f32 {
                inc += 1;
            } else {
                dec += 1;
            }
        }
        if q.iter().all(|&b| b) {
            Some(if inc > dec { 2 } else { 3 })
        } else {
            None
        }
    }

    /// `checkCurrentPlane`: probes the world and sets the ground/slip/air input bits.
    fn check_current_plane(&mut self, host: &mut dyn Host) {
        self.radius = WALL_RADIUS;
        let p = self.pos;
        let (fy, g) = self.check_ground_plane(host, Vec3::new(p.x, p.y + 25.0, p.z));
        self.floor_y = fy;
        self.ground = g;
        let (ry, r) = self.check_roof_plane(host, Vec3::new(p.x, p.y + 80.0, p.z));
        self.roof_y = ry;
        self.roof = r;

        if !self.ground.flags.has(SurfaceFlags::ILLEGAL) {
            self.slope_angle = math::matan(self.ground.normal.z, self.ground.normal.x);
            if self.is_slip_start() {
                self.input.slip = true;
            }
            if self.pos.y > self.floor_y + STEP_UP {
                self.input.airborne = true;
            }
        }
    }

    /// `TMario::checkGroundPlane`: floor under `p`, skipping pass-through surfaces.
    pub(crate) fn check_ground_plane(&self, host: &mut dyn Host, p: Vec3) -> (f32, GroundHit) {
        let mut hit = host.ground(p);
        if let Some(h) = hit
            && h.flags.has(SurfaceFlags::MARIO_THROUGH)
        {
            hit = host.ground(Vec3::new(p.x, h.height - 1.0, p.z));
        }
        match hit {
            Some(h) => (h.height, h),
            None => (
                ILLEGAL_FLOOR,
                GroundHit {
                    height: ILLEGAL_FLOOR,
                    normal: Vec3::Y,
                    flags: SurfaceFlags(SurfaceFlags::ILLEGAL),
                },
            ),
        }
    }

    /// Ceiling above `p` (`checkRoof`); `f32::MAX` if open.
    pub(crate) fn check_roof_plane(&self, host: &mut dyn Host, p: Vec3) -> (f32, Option<RoofHit>) {
        match host.roof(p) {
            Some(r) => (r.height, Some(r)),
            None => (f32::MAX, None),
        }
    }

    // ---- state machine ------------------------------------------------------------------

    /// `stateMachine`: runs the current status's handler, re-running while it changed status.
    fn state_machine(&mut self, host: &mut dyn Host) {
        let mut guard = 0;
        loop {
            let changed = match self.status.kind() {
                Kind::Waiting => self.wait_main(host),
                Kind::Running => self.move_main(host),
                Kind::Jumping => self.jump_main(host),
                Kind::Action => self.action_main(host),
            };
            guard += 1;
            if !changed || guard > 16 {
                break;
            }
        }
    }

    /// `thinkHeight`/`thinkSituation` subset: void check.
    fn think_height(&mut self, host: &mut dyn Host) {
        if self.status != Status::Wait && self.ground.flags.has(SurfaceFlags::ILLEGAL) {
            // Falling past the world: let the host know once.
            if self.pos.y < self.floor_y + 1.0e5 && self.floor_y <= ILLEGAL_FLOOR + 1.0 {
                host.event(Event::Fell);
            }
        }
    }

    // ---- status changes -----------------------------------------------------------------

    /// `changePlayerStatus`. Returns true if the status changed.
    pub fn change_status(&mut self, to: Status, arg: u32, force: bool) -> bool {
        if !force && to == self.status {
            return false;
        }
        let to = match to.kind() {
            Kind::Running => self.set_status_to_running(to),
            Kind::Jumping => self.set_status_to_jumping(to, arg),
            _ => to,
        };
        self.prev_status = self.status;
        self.status = to;
        self.status_arg = arg;
        self.status_state = 0;
        self.status_timer = 0;
        true
    }

    /// `setStatusToRunning`.
    fn set_status_to_running(&mut self, st: Status) -> Status {
        let mag = self.intended_mag.min(8.0);
        match st {
            Status::Run => {
                if 0.0 <= self.forward_vel && self.forward_vel < mag {
                    self.forward_vel = mag;
                }
                st
            }
            Status::Slip => {
                if self.is_front_slip(false) {
                    Status::SlipFore
                } else {
                    Status::SlipBack
                }
            }
            _ => st,
        }
    }

    /// `setPlayerJumpSpeed`.
    fn set_player_jump_speed(&mut self, speed_mult: f32, force: f32) {
        self.vel.y = self.forward_vel * speed_mult + force;
    }

    /// `setStatusToJumping`: take-off velocities per jump type.
    fn set_status_to_jumping(&mut self, st: Status, _arg: u32) -> Status {
        self.fall_start_y = self.pos.y;
        let j = self.params.jump.clone();
        let mut event = None;
        match st {
            Status::Jump => {
                self.set_player_jump_speed(0.25, 42.0);
                self.forward_vel *= 0.8;
                event = Some(JumpKind::Normal);
            }
            Status::SecondJump => {
                self.set_player_jump_speed(j.sec_jump_speed_mult, j.sec_jump_force);
                self.forward_vel *= j.sec_jump_xz_mult;
                event = Some(JumpKind::Second);
            }
            Status::UltraJump => {
                self.set_player_jump_speed(j.ultra_jump_speed_mult, j.ultra_jump_force);
                self.forward_vel *= j.ultra_jump_xz_mult;
                event = Some(JumpKind::Triple);
            }
            Status::LeftRotateJump | Status::RightRotateJump => {
                self.set_player_jump_speed(0.25, j.rotate_jump_force_y);
                self.forward_vel *= 0.8;
                event = Some(JumpKind::Spin);
            }
            Status::BackJump => {
                self.forward_vel = j.back_jump_force;
                self.set_player_jump_speed(0.0, j.back_jump_force_y);
                event = Some(JumpKind::Back);
            }
            Status::WallJump => {
                self.set_player_jump_speed(0.0, 62.0);
                self.forward_vel = 24.0;
                event = Some(JumpKind::Wall);
            }
            Status::UTurnJump => {
                self.set_player_jump_speed(0.0, j.turn_jump_force);
                self.forward_vel = 8.0;
                self.face_yaw = self.intended_yaw;
                event = Some(JumpKind::UTurn);
            }
            Status::MissJump => {
                self.set_player_jump_speed(0.25, 42.0);
                event = Some(JumpKind::Normal);
            }
            Status::JumpCatch => {
                let speed = (15.0 + self.forward_vel).min(48.0);
                self.forward_vel = speed;
                self.slide_x = speed * sin(self.face_yaw);
                self.slide_z = speed * cos(self.face_yaw);
                self.vel.x = self.slide_x;
                self.vel.z = self.slide_z;
                event = Some(JumpKind::Dive);
            }
            Status::BroadJump => {
                self.forward_vel = j.broad_jump_force;
                self.vel.y = j.broad_jump_force_y;
            }
            Status::RotateBroadJump => {
                self.forward_vel = j.rot_broad_jump_force;
                self.vel.y = j.rot_broad_jump_force_y;
            }
            Status::Rocket => {
                // setStatusToJumping case ROCKET: a small initial pop, FLUDD does the rest.
                if self.fludd.is_some() {
                    self.set_player_jump_speed(0.0, 10.0);
                }
            }
            Status::ForceJump => {
                event = Some(JumpKind::Forced);
            }
            _ => {}
        }
        self.jump_peak_y = self.pos.y;
        self.short_hop = st.allows_short_hop();
        if let Some(k) = event {
            self.events.push(Event::Jump(k));
        }
        st
    }
}
