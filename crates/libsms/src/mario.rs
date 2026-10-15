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
use crate::params::NozzleKind;
use crate::host::{Fruit, GroundHit, Host, RoofHit, SurfaceFlags, WallHit};
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
    /// A stumble from a light hit (`SAFE_BACK_DOWN`, `mDownType` 0).
    SafeBackDown,
    SafeForeDown,
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
    /// Bouncing off an enemy's head (`MARIO_STATUS_TRAMPLE`); the arg is which stomp in a
    /// row (0..2, `ANIM_STEP1..3`).
    Trample,
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
    Take,
    TakePose,
    Putting,
    Pitching,
    // Special (type 0x300): hanging from a ledge (MarioSpecial.cpp)
    Hanging,
    Ascend,
    Descend,
    HangJumping,
    /// Thrown off Yoshi (status 0x89C, `getOffYoshi(true)`): a jump that lands lying down.
    YoshiThrown,
    /// Out of health (`DOWN_LOSER`, `loserDown`): lying down for good.
    DownLoser,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Waiting,
    Running,
    Jumping,
    Action,
    Special,
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
            | ShortForeDown | SafeBackDown | SafeForeDown | CatchDown => Kind::Running,
            CatchLost | Take | TakePose | Putting | Pitching => Kind::Action,
            Hanging | Ascend | Descend | HangJumping | DownLoser => Kind::Special,
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
            Jump | SecondJump | WallJump | MissJump | RotateBroadJump | SlipFall | Trample
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
    /// Carrying something (`UPPER_STATE_HOLDING_OBJECT`).
    HoldingObject,
}

/// Build-time options for behaviours that need states this crate does not have yet, and host
/// tuning that departs from the game on purpose. The defaults are the game.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Options {
    /// `TNozzleBase::movement` (WaterGun.cpp): the continuous nozzles' output is
    /// `(pressed - gate)` where the gate climbs toward the pressed level at `mTriggerRate`
    /// per tick, so each press is a burst. A light press (the trigger's first tenth, under the
    /// Spray's `mEmitPowScale` of 10) is a short burst that lands a few metres ahead; a full
    /// press fires at full reach for about 5 s, then fades to nothing until the trigger is let
    /// go or pressed deeper. true (the default): the game's. false: a steady output set by
    /// the trigger's depth, for hosts that want to hold the Spray on.
    pub spray_pressure_fades: bool,
    /// Jump heights times this (1: the game's), for hosts whose worlds are built bigger. The
    /// take-off speed scales by its square root, so arcs keep their shape and last longer.
    pub jump_height: f32,
    /// Ticks an A press that did nothing in the air is remembered, so pressing just before
    /// landing still jumps (and chains). 0: the game's (no buffer).
    pub jump_buffer_ticks: u32,
    /// Ticks after a landing window (`*JumpSlip`, 0x10 ticks) in which a jump still chains
    /// into the double or triple jump. 0: the game's.
    pub chain_grace_ticks: u32,
    /// Keep Mario out of every enemy he touches (`keepDistance`), as the game does only for
    /// some (Kumokun, the Manta's children...). Off: the game's; most enemies he walks into
    /// simply hurt him.
    pub enemy_push: bool,
    /// X steps through every nozzle (Spray, Hover, Rocket, Turbo) instead of swapping Spray
    /// with the one secondary nozzle the stage gave Mario. Off: the game's.
    pub cycle_nozzles: bool,
    /// Ground covered per tick, walking and in the air, times this (a host's speed buffs).
    /// Speeds, and so animation, stay the game's. 1: the game's.
    pub move_speed: f32,
    /// Stomp bounces (`mTramplePowStep1..3`) times this high, on top of `jump_height`, for a
    /// bouncier feel. 1: the game's.
    pub stomp_bounce: f32,
    /// The Rocket nozzle's lift (its recoil, `mReactionY`) times this. 1: the game's.
    pub rocket_thrust: f32,
    /// A on landing from a ground pound leaps (as the triple jump) back up this share of the
    /// height fallen, never lower than a triple jump. 0: the game's (the landing ignores A).
    pub pound_jump: f32,
    /// A pressed within [`STOMP_JUMP_WINDOW`] ticks of a stomp, before or after, makes the
    /// bounce this many times as high. 1: the game's (A does nothing).
    pub stomp_jump: f32,
    /// Letting go of A early (`isFallCancel`) cuts a jump short only once it has risen this
    /// share of its full height, so a tap still hops this high or a little more. 0: the game's.
    pub short_hop: f32,
    /// Standing in shallow water (the host's `water_level`) wets the ground as FLUDD's water
    /// does, so a belly slide there glides wet. false: the game's (only FLUDD's water).
    pub shallow_water_wets: bool,
    /// The belly slide's spray (`ms_m_watslide_*`) times this big. 1: the game's.
    pub slide_fx_scale: f32,
    /// FLUDD's tank never runs dry. false: the game's.
    pub unlimited_water: bool,
    /// Once a knock-down's slide has nearly spent itself, A, the stick, B, L or R gets him up at
    /// once instead of after the whole get-up clip. false: the game's.
    pub knockdown_cancel: bool,
}

/// Ticks either side of a stomp in which A counts for [`Options::stomp_jump`].
pub const STOMP_JUMP_WINDOW: u32 = 15;

impl Default for Options {
    fn default() -> Options {
        Options {
            spray_pressure_fades: true,
            jump_height: 1.0,
            jump_buffer_ticks: 0,
            chain_grace_ticks: 0,
            enemy_push: false,
            cycle_nozzles: false,
            move_speed: 1.0,
            stomp_bounce: 1.0,
            rocket_thrust: 1.0,
            pound_jump: 0.0,
            stomp_jump: 1.0,
            short_hop: 0.0,
            shallow_water_wets: false,
            slide_fx_scale: 1.0,
            unlimited_water: false,
            knockdown_cancel: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Mario {
    pub params: Params,
    pub anims: AnimTable,
    pub options: Options,
    /// A host's own limits on FLUDD and the slide (charges, cooldowns); the default is the
    /// game's: none.
    pub limits: crate::limits::Limits,
    pub(crate) lim: crate::limits::LimitState,
    /// The game's L-button camera (`CPolarSubCamera::isLButtonCamera`), as the host's camera
    /// sees it: set while it's on, None otherwise. Sent by the host before each step.
    pub aim: Option<crate::fludd::Aim>,

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
    /// Airborne peak for host fall damage; changing jump/dive status must not reset it.
    pub(crate) fall_peak_y: f32,
    /// Y at take-off, for the hard-landing check (`unk2A8.y`).
    pub(crate) fall_start_y: f32,
    /// `soundMovement`'s spin jump state: the voice is due on the next spin (`unk04 & 0x10`),
    /// and ticks spent spinning (`unk22`).
    spin_voice_due: bool,
    spin_ticks: u16,
    pub(crate) short_hop: bool,
    /// Host extension: gravity per 120 Hz tick during an unsteered jump-pad flight.
    pub(crate) pad_gravity: Option<f32>,
    /// Height a jump rises before letting go of A cuts it ([`Options::short_hop`]).
    pub(crate) hop_floor_y: f32,

    pub anim: AnimState,

    pub fludd: Option<Fludd>,
    pub upper: UpperState,
    /// The pumping arm layer (`M3UModelMario` slot 1): whether it is applied, its clip and
    /// its frame controller (`getPumpFrame`).
    pub(crate) upper_on: bool,
    pub(crate) upper_clip: Option<crate::anim::ClipName>,
    pub upper_ctrl: crate::anim::FrameCtrl,
    pub(crate) pump_cooldown: i32,
    /// What Mario carries (`mHeldObject`), by the host's id.
    pub held: Option<u64>,
    /// What his hand is closing on (`unk384`).
    pub(crate) take_target: Option<u64>,
    /// Which fruit `held` / `take_target` are, if they are fruit.
    pub held_fruit: Option<Fruit>,
    pub(crate) take_fruit: Option<Fruit>,
    /// Yoshi (`mYoshi`), in his egg until one hatches.
    pub yoshi: crate::yoshi::Yoshi,
    /// The stage's Yoshi eggs (`TEggYoshi`).
    pub eggs: Vec<crate::egg::Egg>,
    /// `MsRandF`'s state.
    pub(crate) seed: u32,
    pub(crate) dash_speed: f32,
    pub(crate) dash_timer: i32,
    pub(crate) fludd_emitting: bool,
    pub(crate) hover_target_y: f32,
    /// In shallow water (`MARIO_FLAG_IN_SHALLOW_WATER`, `thinkWaterSurface`).
    pub(crate) in_water: bool,
    /// The surface over his feet when `in_water` (`mFloorPosition.z`).
    pub(crate) water_surface: f32,
    /// The tank was full last tick, for the HUD's full-tank chime.
    pub(crate) was_full: bool,
    /// Set by the host each step: FLUDD water stuck to the ground is near Mario's feet
    /// (`TModelWaterManager::askHitWaterParticleOnGround`, [`crate::water::WaterManager::on_ground_near`]).
    pub water_underfoot: bool,
    /// `MARIO_FLAG_RECENTLY_LEFT_WATER` (misnamed): touching that water this tick.
    pub(crate) wet_ground: bool,
    /// Units the last ground pound fell, for [`Options::pound_jump`].
    pub(crate) last_pound_drop: f32,
    /// A pound jump's drop, consumed by its take-off.
    pub(crate) pound_rebound: Option<f32>,
    /// Ticks since A was pressed, and a stomp's late-press window ([`Options::stomp_jump`]):
    /// ticks since the bounce and the boosted take-off speed.
    pub(crate) a_age: u32,
    pub(crate) stomp_late: Option<(u32, f32)>,
    /// `mHealth`, out of `mHPMax`.
    pub health: i16,
    /// `mInvincibilityFrames`: ticks left of the flashing after a hit.
    pub invincible: i16,
    /// Gold coins picked up.
    pub coins: u32,

    pub ticks: u64,

    /// Ticks left on a buffered A press ([`Options::jump_buffer_ticks`]).
    pub(crate) jump_buffer: u32,
    /// The chained jump still on offer after a landing window closed, and ticks left
    /// ([`Options::chain_grace_ticks`]).
    pub(crate) chain_grace: Option<(Status, u32)>,
    /// Enemies Mario has hit since his status last changed. Sunshine's enemies react to the
    /// first message (and die or flinch); hosts' enemies may outlast it, so a pound or a
    /// slide through one reports it once, not every tick it overlaps.
    pub(crate) enemies_hit: Vec<u64>,
}

impl Mario {
    pub fn new(params: Params, anims: AnimTable, pos: Vec3, face_yaw: Angle) -> Mario {
        let anim = AnimState::new(clip::WAIT, &anims);
        let health = params.mario.hp_max;
        Mario {
            params,
            anims,
            options: Options::default(),
            limits: crate::limits::Limits::default(),
            lim: crate::limits::LimitState::default(),
            aim: None,
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
            fall_peak_y: pos.y,
            fall_start_y: pos.y,
            spin_voice_due: true,
            spin_ticks: 0,
            short_hop: false,
            pad_gravity: None,
            hop_floor_y: 0.0,
            anim,
            fludd: None,
            upper: UpperState::Idle,
            upper_on: false,
            upper_clip: None,
            upper_ctrl: crate::anim::FrameCtrl {
                rate: 0.0,
                ..crate::anim::FrameCtrl::new(AnimTable::FALLBACK)
            },
            pump_cooldown: 0,
            held: None,
            take_target: None,
            held_fruit: None,
            take_fruit: None,
            yoshi: crate::yoshi::Yoshi::default(),
            eggs: Vec::new(),
            seed: 0x1234_5679,
            dash_speed: 0.0,
            dash_timer: 0,
            fludd_emitting: false,
            hover_target_y: pos.y,
            in_water: false,
            water_surface: 0.0,
            was_full: true,
            water_underfoot: false,
            wet_ground: false,
            last_pound_drop: 0.0,
            pound_rebound: None,
            a_age: u32::MAX,
            stomp_late: None,
            health,
            invincible: 0,
            coins: 0,
            ticks: 0,
            jump_buffer: 0,
            chain_grace: None,
            enemies_hit: Vec::new(),
        }
    }

    /// `MsRandF`: 0..1.
    pub(crate) fn rand(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Gives Mario FLUDD with a full Spray tank.
    pub fn give_fludd(&mut self) {
        self.fludd = Some(Fludd::new(&self.params));
    }

    pub fn has_fludd(&self) -> bool {
        self.fludd.is_some()
    }

    /// Fills FLUDD's tank (`TWaterGun::resetWaterToFull`), as a host's own refill (a new level,
    /// a button); the HUD's full-tank chime plays if it wasn't full.
    pub fn refill_water(&mut self) {
        let p = self.params.clone();
        if let Some(f) = self.fludd.as_mut() {
            f.refill(&p);
        }
    }

    /// Adds `share` of a full tank to FLUDD's (a host's own source of water: healing, a
    /// pickup); the full-tank chime plays as it fills. Not the game's.
    pub fn add_water(&mut self, share: f32) {
        if !(share > 0.0) {
            return;
        }
        let p = &self.params;
        if let Some(f) = self.fludd.as_mut() {
            let max = f.max_water(p);
            let add = (share.min(1.0) * max as f32).round() as i32;
            f.water = (f.water + add.max(1)).min(max);
        }
    }

    /// Host relocation starts a fresh fall; it must not retain a previous airborne peak.
    pub fn teleport(&mut self, pos: Vec3) {
        if !pos.is_finite() {
            return;
        }
        self.pos = pos;
        self.pad_gravity = None;
        self.prev_pos = pos;
        self.vel = Vec3::ZERO;
        self.forward_vel = 0.0;
        self.fall_peak_y = pos.y;
    }

    /// The host's world throws Mario: his velocity becomes `vel` (units per 30 Hz frame,
    /// `mVel`) and he flies as from a spring (`FORCE_JUMP`), facing along the throw, or away
    /// from it when it's mostly backwards (he flies back, as from a hit). For jump pads,
    /// knockback and the like; not the game's own (its springs set their own speeds).
    pub fn launch(&mut self, vel: Vec3) {
        if !vel.is_finite() {
            return;
        }
        self.pad_gravity = None;
        let flat = Vec3::new(vel.x, 0.0, vel.z);
        let speed = flat.length();
        if speed > 0.5 {
            let yaw = math::matan(vel.z, vel.x);
            let ahead = (-0x4000..=0x4000).contains(&math::diff(yaw, self.face_yaw));
            if ahead {
                self.face_yaw = yaw;
                self.forward_vel = speed;
            } else {
                self.face_yaw = yaw.wrapping_add(i16::MIN);
                self.forward_vel = -speed;
            }
            self.model_yaw = self.face_yaw;
        } else {
            self.forward_vel = 0.0;
        }
        self.slide_x = vel.x;
        self.slide_z = vel.z;
        self.change_status(Status::ForceJump, 0, true);
        self.vel = vel;
    }

    /// Host extension: preserve a world jump pad's full velocity and gravity until collision.
    /// Velocity is mVel (units per 30 Hz frame), gravity is mVel lost per 120 Hz tick.
    pub fn launch_pad(&mut self, vel: Vec3, gravity: f32) {
        if !vel.is_finite() || !gravity.is_finite() || gravity <= 0.0 {
            return;
        }
        self.launch(vel);
        self.pad_gravity = Some(gravity);
        // jumpProcess moves before gravity: centre the first velocity on the tick.
        self.vel.y -= 0.5 * gravity;
    }

    /// Standing in shallow water ([`Host::water_level`] above his feet).
    pub fn in_water(&self) -> bool {
        self.in_water
    }

    /// Current floor height under Mario (`mFloorPosition.y`).
    pub fn floor_y(&self) -> f32 {
        self.floor_y
    }

    /// 1 while falling in a ground pound, 2 once it's a super pound (`hipAttacking` state 3,
    /// where the game draws Mario in flames: `emitBlurHipDropSuper`), else 0.
    pub fn pounding(&self) -> u8 {
        match (self.status, self.status_state) {
            (Status::HipDrop, 2) => 1,
            (Status::HipDrop, 3) => 2,
            _ => 0,
        }
    }

    /// The Turbo's dash is on (`MARIO_FLAG_FLUDD_EMITTING`): `TMarioEffect` draws the water-boost
    /// cone and binds `ms_watboost_a/b` to FLUDD's muzzle while it lasts.
    pub fn dashing(&self) -> bool {
        self.fludd_emitting
    }

    /// Starts particle effect `name` (a view draws it; MarioParticle.cpp).
    pub(crate) fn fx(&mut self, name: &'static str, at: crate::event::FxAt) {
        self.fx_scaled(name, at, 1.0);
    }

    pub(crate) fn fx_scaled(&mut self, name: &'static str, at: crate::event::FxAt, scale: f32) {
        self.events.push(Event::Particle(crate::event::ParticleCue {
            name,
            at,
            scale,
        }));
    }

    /// `emitParticle(id)` / `emitParticle(id, rot)`: once, at Mario's feet.
    pub(crate) fn fx_once(&mut self, name: &'static str, yaw: i16) {
        let pos = self.pos;
        self.fx(name, crate::event::FxAt::Once { pos, yaw });
    }

    /// `rocketEffectStart`: the Rocket nozzle's launch. `ms_m_rocket_d` and `_c` and a ground pound's
    /// `ms_hipdrop_a` burst at his feet; `_a`, `_b` and `_b2` (the exhaust, bound to the muzzle) are
    /// started on the muzzle, which they follow for their own second or so.
    pub(crate) fn rocket_effect_start(&mut self) {
        self.fx_once("ms_m_rocket_d", 0);
        self.fx_once("ms_m_rocket_c", 0);
        self.fx_once("ms_hipdrop_a", 0);
        for name in ["ms_m_rocket_a", "ms_m_rocket_b", "ms_m_rocket_b2"] {
            self.fx(name, crate::event::FxAt::MuzzleFollow(0));
        }
    }

    /// `emitSmoke`: a puff of dust as Mario sets off, unless he stands in a pool.
    pub(crate) fn emit_smoke(&mut self, yaw: i16) {
        if !self.in_water {
            self.fx_once("ms_mariwalk1_a", yaw);
        }
    }

    /// `strongTouchDownEffect`: a heavy landing's dust ring and puff.
    pub(crate) fn strong_touch_down_effect(&mut self) {
        let yaw = self.face_yaw;
        self.fx_once("ms_jump_ed_a", yaw);
        self.fx_once("ms_jump_ed_b", yaw);
    }

    /// `smallTouchDownEffect`: a light landing's puff, at 0.7.
    pub(crate) fn small_touch_down_effect(&mut self) {
        let pos = self.pos;
        self.fx_scaled(
            "ms_jump_ed_b",
            crate::event::FxAt::Once { pos, yaw: 0 },
            0.7,
        );
    }

    /// `emitParticle(PARTICLE_MS_DMG_C)`: the stars of bumping into something.
    pub(crate) fn bump_stars(&mut self) {
        self.fx_once("ms_dmg_c", 0);
    }

    /// `frontSlipEffect`, every tick of a belly slide: a wet one throws up spray, a dry one
    /// dust (not under water; sand's own dust isn't modelled).
    pub(crate) fn front_slip_effect(&mut self) {
        use crate::event::FxAt;
        if self.wet_sliding() || self.water_underfoot {
            let scale = self.options.slide_fx_scale;
            self.fx_scaled("ms_m_watslide_a", FxAt::Center, scale);
            self.fx_scaled("ms_m_watslide_b", FxAt::Center, scale);
            self.fx_scaled("ms_m_watslide_c", FxAt::Center, scale);
        } else if !self.in_water {
            self.fx("ms_m_slipsmoke", FxAt::Joint(0));
        }
    }

    /// `emitBlurSpinJump`: the spin jump's swirl.
    pub(crate) fn emit_blur_spin_jump(&mut self) {
        use crate::event::FxAt;
        self.fx("ms_m_blur2", FxAt::Center);
        self.fx("ms_m_blur2sp", FxAt::Center);
    }

    /// Belly sliding over FLUDD's water (`catching` with `MARIO_FLAG_RECENTLY_LEFT_WATER`).
    pub fn wet_sliding(&self) -> bool {
        self.status == Status::Catch && self.status_state == 1
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
        let pad = if self.pad_gravity.is_some() {
            &Pad::default()
        } else {
            pad
        };
        // The host's limits may hold R back (`limits.rs`); the game's never do.
        let pad = &self.limit_pad(pad);
        self.camera_yaw = camera_yaw;
        self.pad = *pad;
        self.prev_pos = self.pos;

        self.check_player_action(host, pad);
        if self.limit_air_jump() {
            self.input.jump = false;
        }
        self.check_yoshi_collision(host);
        self.check_enemy_collision(host);
        self.check_yoshi_get_off();
        self.think_yoshi_head_collision(host);
        self.state_machine(host);
        self.check_take(host);
        self.state_machine_upper(pad);
        self.think_water_surface(host);
        self.think_height(host);
        self.think_params();
        self.gun_exec(host, pad);
        self.yoshi_movement(host);
        self.eggs_tick(host);

        self.anim.advance();
        // `calcAnim`: the pumping arm, FLUDD's handles and nozzle clip, then Yoshi's.
        self.add_upper();
        self.fludd_calc_animation();
        if let Some(f) = self.fludd.as_mut() {
            f.nozzle_frame_update();
        }
        self.yoshi_calc_anim();
        self.sound_movement();
        self.pad_prev = *pad;
        for e in self.events.drain(..) {
            host.event(e);
        }
        self.ticks += 1;
    }

    /// `soundMovement` (MarioSound.cpp), off Yoshi: a spin jump, however it started (a take-off
    /// or `considerJumpRotate` in the air), calls out once and whooshes on its 10th tick.
    fn sound_movement(&mut self) {
        if self.on_yoshi() {
            return;
        }
        let spinning = matches!(
            self.status,
            Status::LeftRotateJump | Status::RightRotateJump
        );
        let mut cue = |name| {
            self.events.push(Event::Sound(crate::event::SoundCue {
                name,
                pos: self.pos,
                info: None,
            }))
        };
        if spinning {
            if self.spin_voice_due {
                cue("MSD_SE_MV24_JUMP_SPECIAL_01");
                self.spin_voice_due = false;
            }
            self.spin_ticks = self.spin_ticks.saturating_add(1);
            if self.spin_ticks == 10 {
                cue("MSD_SE_MA_ROLL_JUMP");
            }
        } else {
            self.spin_voice_due = true;
            self.spin_ticks = 0;
        }
    }

    /// `thinkWaterSurface` (MarioMove.cpp), the pool half: Mario is in shallow water while a
    /// pool's surface is above his feet. No swimming: deeper water is still wading.
    fn think_water_surface(&mut self, host: &mut dyn Host) {
        let level = host
            .water_level(self.pos)
            .filter(|&level| level > self.pos.y);
        self.in_water = level.is_some();
        if let Some(level) = level {
            self.water_surface = level;
        }
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
        self.a_age = if a_edge {
            0
        } else {
            self.a_age.saturating_add(1)
        };
        // A late press for a stomp jump: the boosted bounce, less the gravity since (and
        // whatever letting go of A cut off before the press).
        if let Some((ticks, boosted)) = self.stomp_late.take()
            && self.status == Status::Trample
        {
            if a_edge {
                let v = boosted - self.params.jump.gravity * ticks as f32;
                self.vel.y = self.vel.y.max(v);
                self.events.push(Event::Jump(JumpKind::Triple));
            } else if ticks < STOMP_JUMP_WINDOW {
                self.stomp_late = Some((ticks + 1, boosted));
            }
        }
        let b_edge = pad.b && !prev.b;
        let l_edge = pad.l && !prev.l;
        let r_edge = pad.r && !prev.r;

        // Host tuning: a press in the air waits for the landing; a closed chain window lingers.
        self.jump_buffer = if a_edge {
            self.options.jump_buffer_ticks
        } else {
            self.jump_buffer.saturating_sub(1)
        };
        if let Some((_, t)) = &mut self.chain_grace {
            *t = t.saturating_sub(1);
            if *t == 0 || self.status.is_jumping() {
                self.chain_grace = None;
            }
        }
        let buffered =
            self.jump_buffer > 0 && matches!(self.status.kind(), Kind::Waiting | Kind::Running);

        let i = &mut self.input;
        i.stick = self.intended_mag > 0.0;
        i.jump = a_edge || buffered;
        i.jump_held = pad.a;
        // Riding Yoshi, B is his tongue (`checkController`: `!onYoshi()`).
        let on_yoshi = self.yoshi.state == crate::yoshi::YoshiState::Mounted;
        i.b_held = pad.b && !on_yoshi;
        i.b_pressed = b_edge && !on_yoshi;
        i.l_pressed = l_edge;
        if b_edge && !on_yoshi {
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
        // `unk15C`: Yoshi is wider.
        self.radius = if self.on_yoshi() { 80.0 } else { WALL_RADIUS };
        let p = self.pos;
        let (fy, g) = self.check_ground_plane(host, Vec3::new(p.x, p.y + 25.0, p.z));
        self.floor_y = fy;
        self.ground = g;
        let (ry, r) = self.check_roof_plane(host, Vec3::new(p.x, p.y + 80.0, p.z));
        self.roof_y = ry;
        self.roof = r;

        // `checkCurrentPlane`'s `isTouchGround4cm` water check.
        let wet = self.water_underfoot
            || self.ground.flags.has(SurfaceFlags::WET)
            || (self.options.shallow_water_wets && self.in_water);
        self.wet_ground = wet && self.pos.y <= self.floor_y + 4.0;

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
                Kind::Special => self.special_main(host),
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
        if to == Status::HipDrop && self.status != Status::HipDrop {
            self.events.push(Event::PoundStart);
        }
        self.prev_status = self.status;
        self.status = to;
        if to != Status::ForceJump {
            self.pad_gravity = None;
        }
        self.status_arg = arg;
        self.enemies_hit.clear();
        self.lim.wet_decided = false;
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
    fn set_status_to_jumping(&mut self, st: Status, arg: u32) -> Status {
        self.fall_start_y = self.pos.y;
        let j = self.params.jump.clone();
        self.jump_buffer = 0;
        // A plain jump just after a landing window still chains, at the window's speeds
        // (`jumpSlipEvents`), when the host asked for grace.
        let st = match (st, self.chain_grace.take()) {
            (Status::Jump, Some((Status::SecondJump, _)))
                if self.forward_vel >= j.sec_jump_enable_sp =>
            {
                Status::SecondJump
            }
            (Status::Jump, Some((Status::UltraJump, _)))
                if self.forward_vel >= j.tri_jump_enable_sp =>
            {
                Status::UltraJump
            }
            _ => st,
        };
        let mut event = None;
        match st {
            Status::Jump | Status::YoshiThrown => {
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
                event = Some(JumpKind::Long);
            }
            Status::RotateBroadJump => {
                self.forward_vel = j.rot_broad_jump_force;
                self.vel.y = j.rot_broad_jump_force_y;
                event = Some(JumpKind::Long);
            }
            Status::Rocket => {
                // setStatusToJumping case ROCKET: each nozzle's start voice, the Rocket's burst of
                // effects, and a small initial pop; FLUDD does the rest.
                if let Some(nozzle) = self.fludd.as_ref().map(|f| f.nozzle) {
                    match nozzle {
                        NozzleKind::Rocket => {
                            self.voice("MSD_SE_MV24_JUMP_SPECIAL_01");
                            self.rocket_effect_start();
                        }
                        NozzleKind::Turbo => self.voice("MSD_SE_MV15_EXERT_INST_01"),
                        NozzleKind::Hover => self.voice("MSD_SE_MV21_JUMP_SMALL_01"),
                        _ => {}
                    }
                    self.set_player_jump_speed(0.0, 10.0);
                }
            }
            Status::ForceJump => {
                event = Some(JumpKind::Forced);
            }
            Status::Trample => {
                // By stomp in a row: the small, mid and large jump voices.
                let (pow, kind) = match arg {
                    0 => (self.params.mario.trample_pow_step1, JumpKind::Normal),
                    1 => (self.params.mario.trample_pow_step2, JumpKind::Second),
                    _ => (self.params.mario.trample_pow_step3, JumpKind::Triple),
                };
                self.set_player_jump_speed(0.25, pow);
                // `trampleExec`'s kick (every enemy here takes it).
                self.events.push(Event::Sound(crate::event::SoundCue {
                    name: "MSD_SE_MA_KICK_ENEMY",
                    pos: self.pos,
                    info: None,
                }));
                if self.options.stomp_bounce != 1.0 && self.vel.y > 0.0 {
                    // A height multiplier: the take-off speed by its square root.
                    self.vel.y *= self.options.stomp_bounce.max(0.0).sqrt();
                }
                self.forward_vel *= 0.8;
                event = Some(kind);
            }
            _ => {}
        }
        if self.options.jump_height != 1.0
            && self.vel.y > 0.0
            && event.is_some_and(|k| !matches!(k, JumpKind::Forced | JumpKind::Dive))
        {
            self.vel.y *= self.options.jump_height.max(0.0).sqrt();
        }
        // Host tuning: a stomp with A timed well bounces higher, now or on a late press.
        self.stomp_late = None;
        if st == Status::Trample && self.options.stomp_jump > 1.0 && self.vel.y > 0.0 {
            let boosted = self.vel.y * self.options.stomp_jump.sqrt();
            if self.a_age <= STOMP_JUMP_WINDOW {
                self.vel.y = boosted;
                event = Some(JumpKind::Triple);
            } else {
                self.stomp_late = Some((0, boosted));
            }
        }
        // Host tuning: the pound jump climbs back a share of the fall. The apex of a take-off
        // at `v` under `g` a tick, moving `v / 4` a tick, is about `v^2 / 8g`.
        let pound = self.pound_rebound.take();
        if let Some(drop) = pound {
            let h = drop * self.options.pound_jump;
            let v = (8.0 * self.params.jump.gravity * h).sqrt();
            self.vel.y = self.vel.y.max(v);
        }
        if self.on_yoshi() {
            self.vel.y *= self.params.yoshi.jump_yoshi_mult;
            self.yoshi.flutter = 0;
            self.yoshi.flutter_timer_reset();
        }
        self.jump_peak_y = self.pos.y;
        // A pound jump always goes full height.
        self.short_hop = st.allows_short_hop() && pound.is_none();
        // Host tuning: the cut waits until the jump has risen a share of its full apex
        // (`v^2 / 8g`, as above).
        let full = self.vel.y.max(0.0).powi(2) / (8.0 * self.params.jump.gravity);
        self.hop_floor_y = self.pos.y + full * self.options.short_hop.clamp(0.0, 1.0);
        if let Some(k) = event {
            self.events.push(Event::Jump(k));
        }
        st
    }
}
