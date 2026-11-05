//! Petey Piranha (`TBossPakkun`, Enemy/bosspakkun.cpp; tuning `bosspakkun.prm`) as Bowser
//! Jr.'s sidekick: he keeps near an owner and turns his boss attacks on the owner's targets.
//!
//! What is the game's (cited per function below):
//! * **The nerve chain** (`TSpine`): a current nerve and a stack. `pushAfterCurrent` stacks
//!   the next nerves (last pushed runs first), `pushNerve` interrupts (the current one goes on
//!   the stack), `reset` + `setNext` replaces everything. [`Petey::step`] runs it once a tick.
//! * **Waiting** (`TNerveBPWait`): a target within `mSLSwingLength` and within 60 degrees of his
//!   facing gets a head swing (`TNerveBPSwing`, `bosspaku_head`, [`TBPHeadHit::throwActor`]
//!   flings it sideways and up at 100); a close one he isn't facing gets a pivot and a vomit.
//!   After the wait clip ends he picks an attack.
//! * **Vomit** (`TNerveBPVomit`): `bosspaku_pollut_start` at `mSLVomitAnmRate`; his mouth is the
//!   weak point between clip frames 25 and 165; then `bosspaku_pollut_end` and the goop pours
//!   out in front of him (`TBPVomit`: the `bosspakuPollut_white` model is stamped every frame
//!   while its clip plays), with the wind on the water (`gpModelWaterManager->wind`) and one
//!   chance in five of a coin at tick 500.
//! * **Goop ball** (`TNerveBPCannon` on the ground, `TNerveBPFlyCannon` at tick 168 in the
//!   air; `TBPPolDrop`): launched from the mouth with `SMSCalcJumpVelocityXZ` at
//!   `mSLPollBallSpeed` under 0.1 gravity; where it lands it splats `pollut_ball_stamp` at
//!   `mSLPollBallStampScale` (`drop`: `ms_osendan`, `MSD_SE_BS_BSPAKU_POLLUT_GND`). A wall
//!   (80 wide) ends it. Touching a target is an attack.
//! * **Tornado** (`TNerveBPTornado`, `TBPTornado`): at tick 150 of `bosspaku_tornado` a
//!   whirlwind starts at him and heads for the target at `mSLTornadoSpeed`, circling its path
//!   at `t * mSLTornadoRollSpeed` (t runs from `mSLTornadoMoveInit` up by `mSLTornadoMoveInc` a
//!   tick until `mSLTornadoMoveLimit`), hugging the ground; a target it touches (150 wide, 600
//!   tall) is attacked and thrown up at 100, and it fades (`vanish`).
//! * **Water and the belly** (`TBPHeadHit::receiveMessage`, `gotWaterDamage`,
//!   `TNerveBPSwallow`): water into the open mouth from within `mSLDamageAngle` of his facing
//!   fills the water mark by one (to `mSLWaterMarkLimit`) and he swallows (`bosspaku_water_hit`)
//!   until `mSLWaterHitTimer` ticks pass without water. A full belly topples him
//!   (`TNerveBPTumbleIn`, `bosspaku_down`, the dust at 336 and the shake at 348), and he lies
//!   with his navel up for `mSLTumbleTime` (`TNerveBPTumble`: the weak point is the navel).
//!   He gets up with `bosspaku_getup` and `bosspaku_return`, spouting the water
//!   (`resetWaterMark`: the `bosspakuwater.prm` emitter 250 over his head) as the belly
//!   empties (`limit / 100` a tick).
//! * **Belly stomp** (`TBPNavel::receiveMessage`, `gotHipDropDamage`): a ground pound on the
//!   navel costs a hit point; he reacts (`TNerveBPStompReact`, `bosspaku_down_hit`) and gets
//!   up (`TNerveBPGetUp`: `getup`, `panpan`), or at 0 dies (`TNerveBPPreDie`,
//!   `TNerveBPDie` with the `bosspaku_end` model). Standing on the navel without pounding
//!   makes him flinch (`checkMarioRiding` -> `TNerveBPJumpReact`).
//! * **Flying** (`TNerveBPTakeOff`: `fly_start`, then up at 5 a tick; `TNerveBPFly`:
//!   `flyToCurPathNode` at `mSLFlySpeed`; `TNerveBPHover`: `hovering`, the weak point is his
//!   whole body, cannon shots at targets in range, back to flying after `mSLHoverTimer`;
//!   `TNerveBPTouchDown`: down at 5 a tick, `land`). Water or a hit on him while he hovers
//!   brings him down (`gotFlyingDamage` -> `TNerveBPFall`: `fall_start`, `fall_loop` until
//!   he lands, `fall_end` with a big shake, `getup`).
//! * **Sleep** (`TNerveBPSleep`, `bosspaku_sleep`, `ms_poi_zzz`; `TNerveBPBreakSleep`).
//! * The belly's joint scales (`TBossPakkunMtxCalc::calcBellyScale`) and the head turning to
//!   the target (`calcHeadDir`), for the view: [`Petey::belly_scale`], [`Petey::head_yaw`].
//!
//! Host choices (the game has a stage and Mario, not an owner): he flies to the owner when the
//! owner is more than [`Petey::leash`] away (the graph nodes of the second fight become a point
//! [`Petey::follow_distance`] from the owner, [`Petey::fly_height`] over its ground); the
//! attack he picks (`TNerveBPWait`'s stage rules become: a tornado one time in
//! `mSLTornadoProp` when none is out, the goop ball past [`Petey::vomit_reach`], else the
//! vomit), always after pivoting to the target; he sleeps after [`Petey::sleep_after`] idle
//! ticks; [`Petey::flying_form`] keeps him in the air (hovering by the owner instead of
//! landing). Targets are [`Target`]s as for the critters (a Mario only within
//! [`Petey::mario_range`]); a moving target is re-aimed each tick of a pivot.
//!
//! Reconstructed: where the joints are (mouth, head, navel: offsets from his feet unless the
//! host sets [`Petey::joints`]), the vomit's footprint (discs marching out to
//! [`Petey::vomit_reach`]: the stamped model's shape is the host's to mask), the ball stamp's
//! size ([`Petey::ball_stamp_radius`] before `mSLPollBallStampScale`), the fall's gravity, the
//! fly speed being a speed per tick (the game adds `mSLFlySpeed` to a velocity each tick),
//! the tornado's fade length (`trunade.brk`), the vomit and the head swing hitting each target
//! once, and the goop ball checking the ground from its first tick (the game's second-fight
//! rule; the first fight waits 60 ticks).

use std::path::Path;

use glam::Vec3;

use crate::damage::DamageKind;
use crate::goop::{GoopField, GoopKind};
use crate::goop_critters::Target;
use crate::host::{Enemy, Host};
use crate::prm::Prm;
use crate::skeleton::Clip;

/// Petey's id and his hazards' start here (above [`crate::goop_critters::ID_BASE`]'s range).
pub const ID_BASE: u64 = 0x6000_0000;

/// The clips by `TBossPakkun`'s bck index (the archive's `.bck` files in name order).
pub mod clip {
    pub const VOMIT_POOL: u8 = 0;
    pub const VOMIT_POOL_WHITE: u8 = 1;
    pub const BALL_END: u8 = 2;
    pub const DOWN: u8 = 3;
    pub const DOWN_END: u8 = 4;
    pub const DOWN_HIT: u8 = 5;
    pub const DOWN_LOOP: u8 = 6;
    pub const END: u8 = 7;
    pub const FALL_END: u8 = 8;
    pub const FALL_LOOP: u8 = 9;
    pub const FALL_START: u8 = 10;
    pub const FLY: u8 = 11;
    pub const FLY_POLLUT: u8 = 12;
    pub const FLY_START: u8 = 13;
    pub const GETUP: u8 = 14;
    pub const HEAD: u8 = 15;
    pub const HOVERING: u8 = 16;
    pub const JUMP_REACTION: u8 = 17;
    pub const LAND: u8 = 18;
    pub const PANPAN: u8 = 19;
    pub const POLLUT_END: u8 = 20;
    pub const POLLUT_START: u8 = 21;
    pub const RETURN: u8 = 22;
    pub const SLEEP: u8 = 23;
    pub const TORNADO: u8 = 24;
    pub const WAIT: u8 = 25;
    pub const WATER_HIT: u8 = 26;
    pub const BALL: u8 = 27;
    pub const BALL_STAMP: u8 = 28;
    pub const TRUNADE: u8 = 29;
    pub const COUNT: usize = 30;
    /// File stems under `assets/enemy/bosspakkun/`.
    pub const NAMES: [&str; COUNT] = [
        "bosspakupollut",
        "bosspakupollut_white",
        "bosspaku_ball_end",
        "bosspaku_down",
        "bosspaku_down_end",
        "bosspaku_down_hit",
        "bosspaku_down_loop",
        "bosspaku_end",
        "bosspaku_fall_end",
        "bosspaku_fall_loop",
        "bosspaku_fall_start",
        "bosspaku_fly",
        "bosspaku_fly_pollut",
        "bosspaku_fly_start",
        "bosspaku_getup",
        "bosspaku_head",
        "bosspaku_hovering",
        "bosspaku_jump_reaction",
        "bosspaku_land",
        "bosspaku_panpan",
        "bosspaku_pollut_end",
        "bosspaku_pollut_start",
        "bosspaku_return",
        "bosspaku_sleep",
        "bosspaku_tornado",
        "bosspaku_wait",
        "bosspaku_water_hit",
        "pollut_ball",
        "pollut_ball_stamp",
        "trunade",
    ];

    /// The clip's `.bas` sound table (`bosspakkun_bastable`), if it has one.
    pub fn bas(index: u8) -> Option<&'static str> {
        match index {
            0 | 1 | 5 | 7 | 9 | 25 | 27..=29 => None,
            i if (i as usize) < COUNT => Some(NAMES[i as usize]),
            _ => None,
        }
    }
}

/// `bosspakkun.prm` (`TBossPakkunParams` and the spine enemy's).
#[derive(Debug, Clone, PartialEq)]
pub struct PeteyParams {
    pub wait_frame: u32,
    pub water_mark_limit: i32,
    pub swing_length: f32,
    pub poll_ball_stamp_scale: f32,
    pub tumble_time: u32,
    /// `mSLAnmBlendTime0`: blend ticks between the paired clips of `changeBck`.
    pub anm_blend_time: f32,
    pub fly_speed: f32,
    /// Degrees a tick.
    pub pivot_speed: f32,
    pub pivot_speed_aware: f32,
    pub vomit_anm_rate: f32,
    pub head_homing_limit: f32,
    pub damage_angle: f32,
    pub tornado_prop: f32,
    pub tornado_speed: f32,
    pub tornado_roll_speed: f32,
    pub tornado_move_init: f32,
    pub tornado_move_inc: f32,
    pub tornado_move_limit: f32,
    pub water_hit_timer: u32,
    pub hover_timer: u32,
    pub poll_ball_range: f32,
    pub poll_ball_speed: f32,
    pub poll_ball_front: f32,
    pub hit_points: u8,
}

impl Default for PeteyParams {
    /// The PAL disc's `bosspakkun.prm` (the code default for `mSLPollBallFront`, which the
    /// file lacks).
    fn default() -> Self {
        PeteyParams {
            wait_frame: 0,
            water_mark_limit: 250,
            swing_length: 600.0,
            poll_ball_stamp_scale: 2.0,
            tumble_time: 1000,
            anm_blend_time: 120.0,
            fly_speed: 5.0,
            pivot_speed: 0.5,
            pivot_speed_aware: 1.6,
            vomit_anm_rate: 1.6,
            head_homing_limit: 30.0,
            damage_angle: 180.0,
            tornado_prop: 0.4,
            tornado_speed: 2.0,
            tornado_roll_speed: 0.06,
            tornado_move_init: 10000.0,
            tornado_move_inc: 1.0,
            tornado_move_limit: 10720.0,
            water_hit_timer: 20,
            hover_timer: 1200,
            poll_ball_range: 10000.0,
            poll_ball_speed: 30.0,
            poll_ball_front: 1000.0,
            hit_points: 3,
        }
    }
}

impl PeteyParams {
    pub fn from_prm(p: &Prm) -> PeteyParams {
        let d = PeteyParams::default();
        let f = |k: &str, v: f32| p.f32(k).unwrap_or(v);
        let i = |k: &str, v: u32| p.i32(k).map_or(v, |x| x.max(0) as u32);
        PeteyParams {
            wait_frame: i("mSLWaitFrameStg0", d.wait_frame),
            water_mark_limit: p.i32("mSLWaterMarkLimit").unwrap_or(d.water_mark_limit).max(1),
            swing_length: f("mSLSwingLength", d.swing_length),
            poll_ball_stamp_scale: f("mSLPollBallStampScale", d.poll_ball_stamp_scale),
            tumble_time: i("mSLTumbleTime", d.tumble_time),
            anm_blend_time: p.i32("mSLAnmBlendTime0").map_or(d.anm_blend_time, |x| x as f32),
            fly_speed: f("mSLFlySpeed", d.fly_speed),
            pivot_speed: f("mSLPivotSpeed", d.pivot_speed),
            pivot_speed_aware: f("mSLPivotSpeedAware", d.pivot_speed_aware),
            vomit_anm_rate: f("mSLVomitAnmRate", d.vomit_anm_rate),
            head_homing_limit: f("mSLHeadHomingLimit", d.head_homing_limit),
            damage_angle: f("mSLDamageAngle", d.damage_angle),
            tornado_prop: f("mSLTornadoProp", d.tornado_prop),
            tornado_speed: f("mSLTornadoSpeed", d.tornado_speed),
            tornado_roll_speed: f("mSLTornadoRollSpeed", d.tornado_roll_speed),
            tornado_move_init: f("mSLTornadoMoveInit", d.tornado_move_init),
            tornado_move_inc: f("mSLTornadoMoveInc", d.tornado_move_inc),
            tornado_move_limit: f("mSLTornadoMoveLimit", d.tornado_move_limit),
            water_hit_timer: i("mSLWaterHitTimer", d.water_hit_timer),
            hover_timer: i("mSLHoverTimer", d.hover_timer),
            poll_ball_range: f("mSLPollBallRange", d.poll_ball_range),
            poll_ball_speed: f("mSLPollBallSpeed", d.poll_ball_speed),
            poll_ball_front: f("mSLPollBallFront", d.poll_ball_front),
            hit_points: p.i32("mSLHitPointMax").map_or(d.hit_points, |x| x.clamp(1, 255) as u8),
        }
    }
}

/// Clip lengths in ticks (a `.bck` frame is two 120 Hz ticks; see [`crate::flower::Clips`]).
#[derive(Debug, Clone, PartialEq)]
pub struct PeteyClips {
    pub ticks: [u32; clip::COUNT],
}

impl Default for PeteyClips {
    fn default() -> Self {
        PeteyClips { ticks: [120; clip::COUNT] }
    }
}

impl PeteyClips {
    /// Reads the lengths from `assets/enemy/bosspakkun/` (missing ones keep the defaults).
    pub fn from_dir(assets: &Path) -> PeteyClips {
        let mut c = PeteyClips::default();
        for (i, n) in clip::NAMES.iter().enumerate() {
            let p = assets.join(format!("enemy/bosspakkun/{n}.bck"));
            if let Some(t) = std::fs::read(p).ok().and_then(|d| Clip::parse(&d)).map(|c| (c.frames * 2.0).max(1.0) as u32) {
                c.ticks[i] = t;
            }
        }
        c
    }
}

/// `mWeakPoint`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum WeakPoint {
    #[default]
    None = 0,
    /// Lying belly-up: a ground pound on the navel hurts him.
    Navel = 1,
    /// Mouth open mid-vomit: water fills his belly.
    Mouth = 2,
    /// Hovering: water (or a hit) brings him down.
    Fly = 3,
}

/// His nerves (`TNerveBP*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum State {
    Wait = 0,
    Pivot,
    Swing,
    Vomit,
    Cannon,
    Tornado,
    Swallow,
    TumbleIn,
    Tumble,
    TumbleOut,
    StompReact,
    GetUp,
    JumpReact,
    PreDie,
    Die,
    TakeOff,
    Fly,
    TouchDown,
    Hover,
    FlyPivot,
    FlyCannon,
    Fall,
    Sleep,
    BreakSleep,
}

/// Where his joints are this tick, if the host poses the model (else offsets from his feet).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Joints {
    /// Joint 18 (the head; the ball and the water spout start here).
    pub mouth: Vec3,
    /// Joint 6 (the navel).
    pub navel: Vec3,
}

/// `TBPPolDrop`.
#[derive(Debug, Clone, PartialEq)]
pub struct Ball {
    pub id: u64,
    pub pos: Vec3,
    pub vel: Vec3,
    age: u32,
    /// `unk88`: the last good ground under it.
    ground: f32,
    /// Ticks left of the stamp clip once landed (0: flying).
    pub stamping: u32,
}

/// `TBPTornado`.
#[derive(Debug, Clone, PartialEq)]
pub struct Tornado {
    pub id: u64,
    pub pos: Vec3,
    /// `unk7C`: the point it circles, heading for `goal` (`unk70`).
    pub center: Vec3,
    pub goal: Vec3,
    /// `unk94`.
    t: f32,
    /// `unk88`: last tick's move (it keeps drifting while it fades).
    delta: Vec3,
    /// Ticks left fading (`unk98 == 2`); 0 while it runs.
    pub vanishing: u32,
}

/// The goop pouring out (`TBPVomit`).
#[derive(Debug, Clone, PartialEq)]
pub struct Vomit {
    pub origin: Vec3,
    pub yaw: f32,
    pub age: u32,
    pub len: u32,
    hit: Vec<u64>,
}

/// What Petey did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PeteyEvent {
    /// `changeBck`: play `clip` (its `.bas` too, [`clip::bas`]), blending over `blend` ticks.
    Clip { clip: u8, blend: f32 },
    /// A sound; `held` ones keep playing while they keep coming each tick.
    Sound { name: &'static str, pos: Vec3, held: bool },
    /// A particle effect by its `.jpa` name, on model joint `joint` when there is one;
    /// `held` ones are bound emitters that live while they keep coming each tick.
    Effect { name: &'static str, pos: Vec3, joint: Option<u16>, scale: f32, held: bool },
    /// `HIT_MESSAGE_ATTACK` on `target` (plus `HIT_MESSAGE_THROWN` along `throw`).
    Hit { target: u64, mario: bool, pos: Vec3, throw: Option<Vec3>, kind: DamageKind },
    /// Goop of `radius` painted at `pos`.
    Splat { pos: Vec3, radius: f32 },
    BallLaunched { id: u64, pos: Vec3, vel: Vec3 },
    TornadoLaunched { id: u64, pos: Vec3 },
    /// Camera shake and rumble (`rumblePad`: 0.4, 0.7 or 1 before the distance falloff).
    Shake { pos: Vec3, power: f32 },
    /// `resetWaterMark`: the `bosspakuwater.prm` water spout.
    WaterSpout { pos: Vec3 },
    /// `gpModelWaterManager->wind` while he vomits.
    Wind { dir: Vec3 },
    /// A coin (`makeObjAppear(.., 0x20000002)`).
    Coin { pos: Vec3 },
    /// The water mark rose (`gotWaterDamage`).
    Gulp { mark: i32 },
    /// A stomp took a hit point.
    Damaged { hp: u8 },
    /// `TNerveBPDie` ended: in the game the boss Shine appears here.
    Defeated { pos: Vec3 },
}

fn dir_of(yaw: f32) -> Vec3 {
    Vec3::new(yaw.sin(), 0.0, yaw.cos())
}

/// `MsGetRotFromZaxisY`, in radians.
fn yaw_to(from: Vec3, to: Vec3) -> f32 {
    (to.x - from.x).atan2(to.z - from.z)
}

/// `MsAngleDiff` in degrees, wrapped to -180..180.
fn diff_deg(a: f32, b: f32) -> f32 {
    (a - b + 180.0).rem_euclid(360.0) - 180.0
}

/// `SMSCalcJumpVelocityXZ` (MarioUtil/MathUtil.cpp): `speed` across the ground, and the
/// vertical speed that reaches `to` after the whole number of steps that takes under
/// `gravity` a tick.
pub fn jump_velocity(to: Vec3, from: Vec3, speed: f32, gravity: f32) -> Vec3 {
    let hor = Vec3::new(to.x - from.x, 0.0, to.z - from.z);
    let mag = hor.length().max(1.0);
    let steps = (mag / speed) as i32;
    let tri = ((steps + 1) * steps / 2) as f32;
    let k = speed / mag;
    Vec3::new(hor.x * k, k * (to.y - from.y) + tri * gravity * k, hor.z * k)
}

/// Petey.
#[derive(Debug, Clone)]
pub struct Petey {
    pub id: u64,
    pub params: PeteyParams,
    pub clips: PeteyClips,
    /// His feet.
    pub pos: Vec3,
    /// Facing (radians, 0 = +Z).
    pub yaw: f32,
    pub vel: Vec3,
    pub hp: u8,
    pub alive: bool,
    pub weak_point: WeakPoint,
    pub state: State,
    /// `TSpine::getTime`.
    pub time: u32,
    stack: Vec<State>,
    /// Current clip and ticks into it.
    pub clip: u8,
    pub clip_time: u32,
    clip_fresh: bool,
    /// `unk184`: the head's turn to the target (degrees).
    pub head_yaw: f32,
    // The water (`unk170` hits, `unk174` timer, `unk178` mark, `unk17C` emptying,
    // `unk1B8`/`unk1BC` the belly's settle).
    water_hits: u32,
    water_timer: u32,
    pub water_mark: i32,
    emptying: bool,
    settle: u32,
    settling: bool,
    pub ball: Option<Ball>,
    pub tornado: Option<Tornado>,
    pub vomit: Option<Vomit>,
    pub events: Vec<PeteyEvent>,
    /// What his goop is.
    pub goop_kind: GoopKind,
    /// Stay airborne: hover by the owner instead of landing.
    pub flying_form: bool,
    /// He flies to the owner past this (XZ).
    pub leash: f32,
    /// How far from the owner he settles.
    pub follow_distance: f32,
    /// Flying height over the ground.
    pub fly_height: f32,
    /// He attacks targets this close.
    pub attack_range: f32,
    /// A Mario is a target only this close.
    pub mario_range: f32,
    /// The vomit pours from this far in front of him out to [`Self::vomit_reach`].
    pub vomit_near: f32,
    pub vomit_reach: f32,
    pub vomit_radius: f32,
    /// The ball's stamp radius at scale 1.
    pub ball_stamp_radius: f32,
    /// Mouth offsets from the feet when [`Self::joints`] is `None`.
    pub mouth_height: f32,
    pub mouth_front: f32,
    pub joints: Option<Joints>,
    /// `mTurnSpeed` while flying (degrees a tick).
    pub fly_turn_speed: f32,
    pub fall_gravity: f32,
    /// Idle ticks before he dozes off (0: never).
    pub sleep_after: u32,
    /// `trunade.brk`'s fade, ticks.
    pub tornado_fade: u32,
    goal: Vec3,
    aim: Option<u64>,
    swing_hit: Vec<u64>,
    idle: u32,
    next_id: u64,
    seed: u32,
}

impl Default for Petey {
    fn default() -> Self {
        Petey::new(PeteyParams::default(), PeteyClips::default(), Vec3::ZERO)
    }
}

impl Petey {
    pub fn new(params: PeteyParams, clips: PeteyClips, pos: Vec3) -> Petey {
        let hp = params.hit_points;
        Petey {
            id: ID_BASE,
            params,
            clips,
            pos,
            yaw: 0.0,
            vel: Vec3::ZERO,
            hp,
            alive: true,
            weak_point: WeakPoint::None,
            state: State::Wait,
            time: 0,
            stack: Vec::new(),
            clip: clip::WAIT,
            clip_time: 0,
            clip_fresh: true,
            head_yaw: 0.0,
            water_hits: 0,
            water_timer: 0,
            water_mark: 0,
            emptying: false,
            settle: 0,
            settling: false,
            ball: None,
            tornado: None,
            vomit: None,
            events: Vec::new(),
            goop_kind: GoopKind::Sink,
            flying_form: false,
            leash: 1500.0,
            follow_distance: 500.0,
            fly_height: 800.0,
            attack_range: 2500.0,
            mario_range: 400.0,
            vomit_near: 300.0,
            vomit_reach: 1800.0,
            vomit_radius: 300.0,
            ball_stamp_radius: 300.0,
            mouth_height: 450.0,
            mouth_front: 150.0,
            joints: None,
            fly_turn_speed: 2.0,
            fall_gravity: 0.5,
            sleep_after: 7200,
            tornado_fade: 60,
            goal: pos,
            aim: None,
            swing_hit: Vec::new(),
            idle: 0,
            next_id: ID_BASE + 1,
            seed: 0x2545_F491,
        }
    }

    /// The disc's tuning and clip lengths from an extracted `assets/` folder.
    pub fn from_dir(assets: &Path, pos: Vec3) -> Petey {
        let params = std::fs::read(assets.join("params_enemy/bosspakkun.prm"))
            .ok()
            .and_then(|d| Prm::parse(&d).ok())
            .map_or_else(PeteyParams::default, |p| PeteyParams::from_prm(&p));
        Petey::new(params, PeteyClips::from_dir(assets), pos)
    }

    /// Whether an id is Petey's or one of his hazards'.
    pub fn owns(&self, id: u64) -> bool {
        id == self.id || self.ball.as_ref().is_some_and(|b| b.id == id) || self.tornado.as_ref().is_some_and(|t| t.id == id)
    }

    /// His head's hit cylinder (`TBPHeadHit`: 300 wide, 500 tall), for the host's attacks.
    pub fn as_enemy(&self) -> Enemy {
        Enemy { id: self.id, pos: self.pos, radius: 300.0, height: 500.0 }
    }

    /// The mouth (joint 18).
    pub fn mouth(&self) -> Vec3 {
        self.joints.map_or(self.pos + Vec3::Y * self.mouth_height + dir_of(self.yaw) * self.mouth_front, |j| j.mouth)
    }

    /// The navel (joint 6).
    pub fn navel(&self) -> Vec3 {
        self.joints.map_or(self.pos + Vec3::Y * 250.0, |j| j.navel)
    }

    /// The clip's frame now (`.bck` frames).
    pub fn clip_frame(&self) -> f32 {
        let len = self.clip_len(self.clip);
        (self.clip_time % len) as f32 * self.clip_rate(self.clip) / 2.0
    }

    /// How full the belly looks, 0..1 with an overshoot (`calcBellyScale`'s
    /// `JMAHermiteInterpolation(p, 0, 0, 10, 1, 1, 0)`).
    pub fn belly(&self) -> f32 {
        let p = if self.emptying {
            self.settle as f32 / 50.0
        } else {
            self.water_mark.min(self.params.water_mark_limit) as f32 / self.params.water_mark_limit as f32
        };
        let t = p.clamp(0.0, 1.0);
        let (t2, t3) = (t * t, t * t * t);
        (t3 - 2.0 * t2 + t) * 10.0 + (-2.0 * t3 + 3.0 * t2)
    }

    /// `calcBellyScale`'s scale for joint 4 or 36 (others: `None`).
    pub fn belly_scale(&self, joint: u16) -> Option<Vec3> {
        let (start, goal) = match joint {
            4 => (Vec3::new(1.0, 0.9, 0.9), Vec3::new(1.3, 1.7, 1.7)),
            36 => (Vec3::new(1.0, 0.8, 0.8), Vec3::new(1.4, 1.4, 1.6)),
            _ => return None,
        };
        Some(start + (goal - start) * self.belly())
    }

    fn rand(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed >> 8) as f32 / (1u32 << 24) as f32
    }

    fn new_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        if self.next_id >= ID_BASE + 0x1000_0000 {
            self.next_id = ID_BASE + 1;
        }
        id
    }

    fn sound(&mut self, name: &'static str, pos: Vec3, held: bool) {
        self.events.push(PeteyEvent::Sound { name, pos, held });
    }

    fn effect(&mut self, name: &'static str, pos: Vec3, joint: Option<u16>, scale: f32, held: bool) {
        self.events.push(PeteyEvent::Effect { name, pos, joint, scale, held });
    }

    /// `rumblePad`'s strengths by type.
    fn shake(&mut self, kind: u8) {
        let power = [0.4, 0.7, 1.0][kind.min(2) as usize];
        self.events.push(PeteyEvent::Shake { pos: self.pos, power });
    }

    /// Playback speed against the usual (`SMSGetAnmFrameRate`, 2): `changeBck` sets
    /// `pollut_start` to `mSLVomitAnmRate` (1.6 on the disc: 0.8 of the usual speed).
    fn clip_rate(&self, c: u8) -> f32 {
        if c == clip::POLLUT_START { (self.params.vomit_anm_rate / 2.0).max(0.01) } else { 1.0 }
    }

    fn clip_len(&self, c: u8) -> u32 {
        ((self.clips.ticks[c as usize] as f32 / self.clip_rate(c)).round() as u32).max(1)
    }

    /// `curAnmEndsNext`.
    fn ends_next(&self) -> bool {
        let len = self.clip_len(self.clip);
        self.clip_time % len == len - 1
    }

    /// `isCurAnmAlreadyEnd`.
    fn already_end(&self) -> bool {
        self.clip_time + 1 >= self.clip_len(self.clip)
    }

    /// `changeBck`: a clip already playing keeps going unless it's ending.
    fn set_clip(&mut self, c: u8) {
        if self.clip == c && !self.ends_next() {
            return;
        }
        let prev = self.clip;
        let paired = matches!(
            (prev, c),
            (clip::WAIT, clip::POLLUT_START)
                | (clip::WAIT, clip::WATER_HIT)
                | (clip::LAND, clip::WAIT)
                | (clip::BALL_END, clip::WAIT)
                | (clip::POLLUT_END, clip::WAIT)
                | (clip::WATER_HIT, clip::RETURN)
        );
        let blend = if paired { self.params.anm_blend_time } else { 0.1 * self.clips.ticks[c as usize] as f32 / 2.0 };
        self.clip = c;
        self.clip_time = 0;
        self.clip_fresh = true;
        self.events.push(PeteyEvent::Clip { clip: c, blend });
    }

    /// `setNext` after `reset`.
    fn goto(&mut self, s: State) {
        self.stack.clear();
        self.state = s;
        self.time = 0;
    }

    /// `pushNerve`: interrupts, resuming the current nerve afterwards.
    fn interrupt(&mut self, s: State) {
        self.stack.push(self.state);
        self.state = s;
        self.time = 0;
    }

    fn push(&mut self, s: State) {
        self.stack.push(s);
    }

    /// Turns toward [`Self::goal`] at `speed` degrees a tick (`turnToCurPathNode`); true once
    /// facing it.
    fn turn_to_goal(&mut self, speed: f32) -> bool {
        let want = yaw_to(self.pos, self.goal).to_degrees();
        let d = diff_deg(want, self.yaw.to_degrees());
        let done = d.abs() < speed;
        let step = d.clamp(-speed, speed);
        self.yaw = (self.yaw.to_degrees() + step).rem_euclid(360.0).to_radians();
        done
    }

    fn eligible(&self, t: &Target) -> bool {
        let d = t.pos.distance(self.pos);
        d < self.attack_range && (!t.mario || d < self.mario_range)
    }

    fn nearest<'a>(&self, targets: &'a [Target]) -> Option<&'a Target> {
        targets
            .iter()
            .filter(|t| t.id != self.id && self.eligible(t))
            .min_by(|a, b| a.pos.distance_squared(self.pos).total_cmp(&b.pos.distance_squared(self.pos)))
    }

    fn aim_at(&mut self, t: &Target) {
        self.aim = Some(t.id);
        self.goal = t.pos;
    }

    /// Where he settles by the owner: `follow_distance` from it on his side.
    fn follow_point(&self, owner: Vec3) -> Vec3 {
        let away = (self.pos - owner).with_y(0.0);
        let dir = if away.length_squared() > 1.0 { away.normalize() } else { Vec3::Z };
        owner + dir * self.follow_distance
    }

    fn ground_at(&self, host: &dyn Host, p: Vec3, fallback: f32) -> f32 {
        host.ground(p + Vec3::Y * 50.0).map_or(fallback, |g| g.height)
    }

    // --- What the host does to him -------------------------------------------------------

    /// Water hits him at `at`, sprayed from `from` (`TBPHeadHit::receiveMessage` and the
    /// body's `receiveMessage` for the flying weak point). Returns whether it counted.
    pub fn water_hit(&mut self, at: Vec3, from: Vec3) -> bool {
        if !self.alive {
            return false;
        }
        match self.weak_point {
            WeakPoint::Fly => {
                if at.y < self.pos.y - 300.0 || at.y > self.pos.y + 1500.0 {
                    return false;
                }
                self.flying_damage();
                true
            }
            WeakPoint::Mouth => {
                let off = diff_deg(yaw_to(self.mouth(), from).to_degrees(), self.yaw.to_degrees());
                if off.abs() < 0.5 * self.params.damage_angle {
                    self.water_damage();
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// A hit on him while he hovers (the body's `receiveMessage` for actor type
    /// 0x1000000d) brings him down; any other time it does nothing. Returns whether it did.
    pub fn knock(&mut self, at: Vec3) -> bool {
        if self.alive && self.weak_point == WeakPoint::Fly && at.y >= self.pos.y - 300.0 && at.y <= self.pos.y + 1500.0 {
            self.flying_damage();
            return true;
        }
        if self.alive && self.state == State::Sleep {
            self.goto(State::BreakSleep);
            return true;
        }
        false
    }

    /// A ground pound on the navel (`TBPNavel::receiveMessage` -> `gotHipDropDamage`).
    /// Returns whether it hurt him.
    pub fn stomp(&mut self) -> bool {
        if !self.alive || self.weak_point != WeakPoint::Navel {
            return false;
        }
        self.hp = self.hp.saturating_sub(1);
        self.weak_point = WeakPoint::None;
        self.events.push(PeteyEvent::Damaged { hp: self.hp });
        if self.hp == 0 {
            if !matches!(self.state, State::PreDie | State::Die) {
                self.state = State::PreDie;
                self.time = 0;
                self.sound("MSD_SE_BS_BSPAKU_DOWN", self.pos, false);
            }
        } else if self.state != State::TumbleOut {
            self.sound("MSD_SE_BS_BSPAKU_DAMAGE", self.pos, false);
            self.goto(State::Wait);
            self.interrupt(State::GetUp);
            self.interrupt(State::StompReact);
        }
        true
    }

    /// Someone stands on the navel without pounding (`checkMarioRiding` ->
    /// `TNerveBPJumpReact`).
    pub fn trample(&mut self) {
        if self.alive && self.weak_point == WeakPoint::Navel && self.state != State::JumpReact {
            self.interrupt(State::JumpReact);
        }
    }

    /// Back to full health, standing at `pos` (a host choice; the game's boss stays dead).
    pub fn respawn(&mut self, pos: Vec3) {
        let (params, clips, id, kind, ff) = (self.params.clone(), self.clips.clone(), self.id, self.goop_kind, self.flying_form);
        let keep = self.clone();
        *self = Petey::new(params, clips, pos);
        self.id = id;
        self.goop_kind = kind;
        self.flying_form = ff;
        self.leash = keep.leash;
        self.follow_distance = keep.follow_distance;
        self.fly_height = keep.fly_height;
        self.attack_range = keep.attack_range;
        self.mario_range = keep.mario_range;
        self.vomit_near = keep.vomit_near;
        self.vomit_reach = keep.vomit_reach;
        self.vomit_radius = keep.vomit_radius;
        self.ball_stamp_radius = keep.ball_stamp_radius;
        self.mouth_height = keep.mouth_height;
        self.mouth_front = keep.mouth_front;
        self.fly_turn_speed = keep.fly_turn_speed;
        self.fall_gravity = keep.fall_gravity;
        self.sleep_after = keep.sleep_after;
        self.tornado_fade = keep.tornado_fade;
        self.next_id = keep.next_id;
        self.seed = keep.seed;
    }

    /// `gotFlyingDamage`.
    fn flying_damage(&mut self) {
        self.weak_point = WeakPoint::None;
        self.goto(State::Fall);
        self.sound("MSD_SE_BS_BSPAKU_FALL", self.pos, false);
    }

    /// `gotWaterDamage`.
    fn water_damage(&mut self) {
        if self.emptying {
            return;
        }
        self.water_hits += 1;
        if self.water_mark < self.params.water_mark_limit {
            self.water_mark += 1;
        }
        self.water_timer = self.params.water_hit_timer;
        self.events.push(PeteyEvent::Gulp { mark: self.water_mark });
        if self.state != State::Swallow {
            self.goto(State::Swallow);
        }
    }

    /// `resetWaterMark`.
    fn reset_water_mark(&mut self) {
        if !self.emptying {
            self.emptying = true;
            self.water_timer = 0;
            self.water_hits = 0;
            self.settle = 50;
            self.events.push(PeteyEvent::WaterSpout { pos: self.mouth() + Vec3::Y * 250.0 });
        }
    }

    // --- The tick --------------------------------------------------------------------------

    /// One tick. `owner` is where Bowser Jr. stands; `targets` what Petey may go for.
    pub fn step(&mut self, host: &dyn Host, goop: &mut GoopField, owner: Vec3, targets: &[Target]) {
        // `TBossPakkun::perform`: the hazards first, even once he's gone.
        self.step_ball(host, goop, targets);
        self.step_vomit(host, goop, targets);
        self.step_tornado(host, targets);
        if !self.alive {
            return;
        }
        // The water mark empties, the hit timer counts on, the belly settles.
        if self.emptying {
            if self.water_mark <= 0 {
                self.emptying = false;
                self.water_mark = 0;
            } else {
                let step = (self.params.water_mark_limit / 100).max(1);
                self.water_mark = (self.water_mark - step).max(0);
            }
        }
        if !self.emptying && self.water_timer > 0 {
            self.water_timer -= 1;
            self.water_hits += 1;
        }
        if self.settling {
            if self.settle == 0 {
                self.settling = false;
            } else {
                self.settle -= 1;
            }
        }
        self.held_effects();
        self.head_homing(targets);

        // `TSpineBase::update`.
        let done = self.nerve(host, owner, targets);
        self.time += 1;
        if done {
            self.state = self.stack.pop().unwrap_or(State::Wait);
            self.time = 0;
        }
        if self.clip_fresh {
            self.clip_fresh = false;
        } else {
            self.clip_time += 1;
        }
    }

    /// The emitters `perform` binds each frame by clip.
    fn held_effects(&mut self) {
        let mouth = self.mouth();
        let top = self.pos + Vec3::Y * 300.0;
        match self.clip {
            clip::FLY | clip::FLY_START | clip::FLY_POLLUT => {
                self.effect("ms_bopa_blur2", top, Some(38), 1.0, true);
                self.effect("ms_bopa_blur2", top, Some(46), 1.0, true);
            }
            _ => {}
        }
        if matches!(self.clip, clip::FLY | clip::DOWN_LOOP | clip::WATER_HIT) {
            self.effect("ms_bopa_ase", mouth, Some(20), 1.0, true);
        }
        if self.clip == clip::SLEEP {
            self.effect("ms_poi_zzz", mouth, Some(20), 2.5, true);
        }
    }

    /// `calcHeadDir`: during the wait clip the head turns toward the nearest target, at most
    /// `mSLHeadHomingLimit` and a degree a tick; otherwise it turns back.
    fn head_homing(&mut self, targets: &[Target]) {
        let want = if self.clip == clip::WAIT {
            self.nearest(targets)
                .map_or(0.0, |t| diff_deg(yaw_to(self.pos, t.pos).to_degrees(), self.yaw.to_degrees()))
        } else {
            0.0
        };
        let lim = self.params.head_homing_limit;
        let want = want.clamp(-lim, lim);
        self.head_yaw += (want - self.head_yaw).clamp(-1.0, 1.0);
    }

    fn nerve(&mut self, host: &dyn Host, owner: Vec3, targets: &[Target]) -> bool {
        let t = self.time;
        let p = self.params.clone();
        // A pivot's or attack's target moves: aim at where it is now.
        if let Some(id) = self.aim
            && let Some(tg) = targets.iter().find(|x| x.id == id)
        {
            self.goal = tg.pos;
        }
        match self.state {
            State::Wait => {
                if t == 0 {
                    self.set_clip(clip::WAIT);
                }
                self.stand(host);
                if self.flying_form {
                    // A host choice: in his flying form he goes straight back up.
                    self.push(State::TakeOff);
                    return true;
                }
                let near = self.nearest(targets).copied();
                if let Some(tg) = near
                    && tg.pos.distance_squared(self.pos) < p.swing_length * p.swing_length
                {
                    self.idle = 0;
                    let off = diff_deg(yaw_to(self.pos, tg.pos).to_degrees(), self.yaw.to_degrees());
                    self.push(State::Wait);
                    if off.abs() < 60.0 {
                        self.push(State::Swing);
                    } else {
                        self.push(State::Vomit);
                        self.aim_at(&tg);
                        self.push(State::Pivot);
                    }
                    return true;
                }
                if t < p.wait_frame || !self.already_end() {
                    return false;
                }
                let far = (owner - self.pos).with_y(0.0).length() > self.leash;
                if far {
                    self.idle = 0;
                    self.push(State::TakeOff);
                    return true;
                }
                if let Some(tg) = near {
                    self.idle = 0;
                    self.push(State::Wait);
                    let d = tg.pos.with_y(0.0).distance(self.pos.with_y(0.0));
                    if self.tornado.is_none() && self.rand() < p.tornado_prop {
                        self.push(State::Tornado);
                    } else if d > self.vomit_reach && self.ball.is_none() {
                        self.push(State::Cannon);
                    } else {
                        self.push(State::Vomit);
                    }
                    self.aim_at(&tg);
                    self.push(State::Pivot);
                    return true;
                }
                self.idle += t.max(1);
                if self.sleep_after > 0 && self.idle >= self.sleep_after {
                    self.idle = 0;
                    self.push(State::Sleep);
                } else {
                    self.push(State::Wait);
                }
                true
            }
            State::Pivot => {
                if t == 0 {
                    self.set_clip(clip::WAIT);
                }
                let close = self.goal.distance_squared(self.pos) < p.swing_length * p.swing_length;
                self.turn_to_goal(if close { p.pivot_speed_aware } else { p.pivot_speed })
            }
            State::Swing => {
                if t == 0 {
                    self.set_clip(clip::HEAD);
                    self.swing_hit.clear();
                    let m = self.mouth();
                    self.effect("ms_bopa_swing2", m, Some(18), 1.0, false);
                }
                self.swing(targets);
                self.ends_next()
            }
            State::Vomit => {
                if t == 0 {
                    self.set_clip(clip::POLLUT_START);
                }
                if self.clip == clip::POLLUT_START {
                    let f = self.clip_frame();
                    self.weak_point = if 25.0 < f && f < 165.0 { WeakPoint::Mouth } else { WeakPoint::None };
                }
                if self.clip == clip::POLLUT_END && t == 500 && self.rand() < 0.2 {
                    let at = self.pos + dir_of(self.yaw) * 700.0 + Vec3::Y;
                    self.events.push(PeteyEvent::Coin { pos: at });
                }
                if self.ends_next() {
                    if self.clip == clip::POLLUT_START {
                        self.weak_point = WeakPoint::None;
                        self.set_clip(clip::POLLUT_END);
                        self.vomit = Some(Vomit {
                            origin: self.pos,
                            yaw: self.yaw,
                            age: 0,
                            len: self.clip_len(clip::VOMIT_POOL),
                            hit: Vec::new(),
                        });
                        self.shake(1);
                    } else {
                        return true;
                    }
                }
                if self.clip == clip::POLLUT_END {
                    self.events.push(PeteyEvent::Wind { dir: dir_of(self.yaw) });
                }
                false
            }
            State::Cannon => {
                if t == 0 {
                    self.set_clip(clip::POLLUT_START);
                }
                if self.ends_next() {
                    if self.clip == clip::POLLUT_START {
                        self.set_clip(clip::BALL_END);
                        self.launch_ball(host);
                    } else {
                        return true;
                    }
                }
                false
            }
            State::Tornado => {
                if t == 0 {
                    self.set_clip(clip::TORNADO);
                    let top = self.pos + Vec3::Y * 300.0;
                    self.effect("ms_bopa_swing1", top, Some(3), 1.0, false);
                    self.effect("ms_bopa_blur1", top, Some(43), 1.0, false);
                    self.effect("ms_bopa_blur1", top, Some(44), 1.0, false);
                }
                if t == 150 {
                    self.launch_tornado(targets);
                }
                self.already_end()
            }
            State::Swallow => {
                if t == 0 {
                    self.set_clip(clip::WATER_HIT);
                }
                if self.water_mark >= p.water_mark_limit {
                    self.push(State::TumbleIn);
                    self.weak_point = WeakPoint::None;
                    self.water_hits = 0;
                    return true;
                }
                let m = self.mouth();
                self.effect("ms_bopa_wathit", m, Some(18), 1.0, true);
                self.effect("ms_bopa_wathit_w", m, Some(18), 1.0, true);
                if self.water_hits != 0 {
                    self.set_clip(clip::WATER_HIT);
                    self.water_hits = 0;
                    return false;
                }
                self.weak_point = WeakPoint::None;
                self.push(State::Wait);
                true
            }
            State::TumbleIn => {
                if t == 0 {
                    self.set_clip(clip::DOWN);
                }
                if t == 336 {
                    self.effect("ms_bopa_down", self.pos, Some(14), 1.0, false);
                }
                if t == 348 {
                    self.shake(2);
                }
                if self.ends_next() {
                    self.push(State::Tumble);
                    return true;
                }
                false
            }
            State::Tumble => {
                if t == 0 {
                    self.set_clip(clip::DOWN_LOOP);
                    self.weak_point = WeakPoint::Navel;
                }
                self.effect("ms_bopa_jita", self.pos, Some(0), 1.0, true);
                if t % 120 == 60 {
                    self.shake(0);
                }
                if t >= p.tumble_time {
                    self.weak_point = WeakPoint::None;
                    self.push(State::TumbleOut);
                    return true;
                }
                false
            }
            State::TumbleOut => {
                if t == 0 {
                    self.set_clip(clip::GETUP);
                    self.shake(0);
                }
                if self.ends_next() {
                    if self.clip == clip::GETUP {
                        self.set_clip(clip::RETURN);
                    } else {
                        self.push(State::Wait);
                        return true;
                    }
                }
                if self.clip == clip::RETURN {
                    let f = self.clip_frame();
                    if 140.0 < f && f < 160.0 {
                        self.reset_water_mark();
                    }
                    if 35.0 < f {
                        self.settling = true;
                    }
                }
                false
            }
            State::GetUp | State::BreakSleep => {
                if t == 0 {
                    self.set_clip(clip::GETUP);
                    self.shake(0);
                }
                if self.ends_next() {
                    if self.clip == clip::GETUP && self.state == State::GetUp {
                        self.set_clip(clip::PANPAN);
                    } else {
                        return true;
                    }
                }
                false
            }
            State::StompReact => {
                if t == 0 {
                    self.set_clip(clip::DOWN_HIT);
                }
                if t == 30 {
                    self.reset_water_mark();
                }
                if t == 50 {
                    self.settling = true;
                }
                self.ends_next()
            }
            State::JumpReact => {
                if t == 0 {
                    self.set_clip(clip::JUMP_REACTION);
                }
                self.ends_next()
            }
            State::PreDie => {
                if t == 0 {
                    self.set_clip(clip::DOWN_HIT);
                    self.reset_water_mark();
                }
                if self.ends_next() {
                    self.push(State::Die);
                    return true;
                }
                false
            }
            State::Die => {
                if t == 0 {
                    // The `bosspaku_end` model plays its own clip 7.
                    self.set_clip(clip::END);
                }
                if self.ends_next() {
                    self.alive = false;
                    self.weak_point = WeakPoint::None;
                    self.events.push(PeteyEvent::Defeated { pos: self.pos });
                    return true;
                }
                false
            }
            State::TakeOff => {
                if t == 0 {
                    self.set_clip(clip::FLY_START);
                    self.vel = Vec3::ZERO;
                }
                if self.clip == clip::FLY_START && self.ends_next() {
                    self.set_clip(clip::FLY);
                }
                if self.clip == clip::FLY {
                    self.pos.y += 5.0;
                    let top = self.ground_at(host, self.pos, self.pos.y) + self.fly_height;
                    if self.pos.y >= top {
                        self.pos.y = top;
                        self.push(State::Fly);
                        return true;
                    }
                }
                false
            }
            State::Fly => {
                if t == 0 {
                    self.set_clip(clip::FLY);
                }
                let mut goal = self.follow_point(owner);
                goal.y = self.ground_at(host, goal, owner.y) + self.fly_height;
                self.goal = goal;
                let to = (goal - self.pos).with_y(0.0);
                if to.length() < 100.0 {
                    self.vel = Vec3::ZERO;
                    self.push(if self.flying_form { State::Hover } else { State::TouchDown });
                    return true;
                }
                self.aim = None;
                let want = yaw_to(self.pos, goal).to_degrees();
                let d = diff_deg(want, self.yaw.to_degrees()).clamp(-self.fly_turn_speed, self.fly_turn_speed);
                self.yaw = (self.yaw.to_degrees() + d).rem_euclid(360.0).to_radians();
                // Faster the further behind he is (a host choice), never past the point.
                let far = (owner - self.pos).with_y(0.0).length();
                let speed = p.fly_speed * (far / self.leash.max(1.0)).clamp(1.0, 3.0);
                let step = to.normalize() * speed.min(to.length());
                self.vel = step;
                self.pos += step;
                self.pos.y += (goal.y - self.pos.y).clamp(-5.0, 5.0);
                false
            }
            State::TouchDown => {
                if t == 0 {
                    self.set_clip(clip::FLY);
                }
                if self.clip == clip::FLY {
                    self.pos.y -= 5.0;
                    let g = self.ground_at(host, self.pos.with_y(self.pos.y + 5.0), owner.y);
                    if g > self.pos.y {
                        self.pos.y = g;
                        self.set_clip(clip::LAND);
                    }
                }
                self.clip == clip::LAND && self.ends_next()
            }
            State::Hover => {
                if t == 0 {
                    self.set_clip(clip::HOVERING);
                    self.weak_point = WeakPoint::Fly;
                }
                let range = p.poll_ball_range.min(self.attack_range);
                if let Some(tg) = self.nearest(targets).copied()
                    && tg.pos.distance(self.pos) < range
                    && self.ball.is_none()
                {
                    self.push(State::Hover);
                    self.push(State::FlyCannon);
                    self.aim_at(&tg);
                    self.push(State::FlyPivot);
                    return true;
                }
                let far = (owner - self.pos).with_y(0.0).length() > self.leash;
                if far || t >= p.hover_timer {
                    self.weak_point = WeakPoint::None;
                    self.push(State::Fly);
                    return true;
                }
                if !self.flying_form {
                    self.weak_point = WeakPoint::None;
                    self.push(State::TouchDown);
                    return true;
                }
                false
            }
            State::FlyPivot => {
                if t == 0 {
                    self.set_clip(clip::FLY);
                }
                self.turn_to_goal(p.pivot_speed)
            }
            State::FlyCannon => {
                if t == 0 {
                    self.set_clip(clip::FLY_POLLUT);
                }
                if t == 168 {
                    self.launch_ball(host);
                }
                self.ends_next()
            }
            State::Fall => {
                if t == 0 {
                    self.set_clip(clip::FALL_START);
                    self.vel = Vec3::ZERO;
                }
                if matches!(self.clip, clip::FALL_START | clip::FALL_LOOP) {
                    self.vel.y = (self.vel.y - self.fall_gravity).max(-40.0);
                    self.pos.y += self.vel.y;
                    let g = self.ground_at(host, self.pos.with_y(self.pos.y - self.vel.y), owner.y);
                    let landed = self.pos.y <= g;
                    if landed {
                        self.pos.y = g;
                        self.vel = Vec3::ZERO;
                    }
                    if self.clip == clip::FALL_START {
                        if self.ends_next() {
                            self.set_clip(clip::FALL_LOOP);
                        }
                    } else if landed {
                        self.set_clip(clip::FALL_END);
                        self.shake(2);
                    }
                } else if self.clip == clip::FALL_END {
                    if self.ends_next() {
                        self.set_clip(clip::GETUP);
                        self.shake(0);
                    }
                } else if self.clip == clip::GETUP && self.ends_next() {
                    self.push(State::Wait);
                    return true;
                }
                false
            }
            State::Sleep => {
                if t == 0 {
                    self.set_clip(clip::SLEEP);
                }
                let far = (owner - self.pos).with_y(0.0).length() > self.leash;
                if far || self.nearest(targets).is_some() {
                    self.goto(State::BreakSleep);
                    // `goto` restarted the chain; this nerve isn't done, the new one starts.
                    self.time = 0;
                    return false;
                }
                false
            }
        }
    }

    /// Keeps him on the ground while he stands.
    fn stand(&mut self, host: &dyn Host) {
        if let Some(g) = host.ground(self.pos + Vec3::Y * 50.0)
            && (g.height - self.pos.y).abs() < 200.0
        {
            self.pos.y = g.height;
        }
    }

    /// `TBPHeadHit::throwActor` during `bosspaku_head`: targets the swinging head reaches
    /// (within `mSLSwingLength` in front, the head's 500-tall cylinder) are attacked and
    /// thrown sideways and up at 100.
    fn swing(&mut self, targets: &[Target]) {
        let reach = self.params.swing_length;
        for t in targets.iter().filter(|t| t.id != self.id && (!t.mario || t.pos.distance(self.pos) < self.mario_range.max(reach))) {
            if self.swing_hit.contains(&t.id) {
                continue;
            }
            let to = (t.pos - self.pos).with_y(0.0);
            if to.length() > reach + t.radius || t.pos.y > self.pos.y + 500.0 || t.pos.y + t.height < self.pos.y {
                continue;
            }
            let off = diff_deg(yaw_to(self.pos, t.pos).to_degrees(), self.yaw.to_degrees());
            if off.abs() > 90.0 {
                continue;
            }
            // `direction` points from it to Petey; the throw is up x direction, doubled, plus up.
            let dir = (self.pos - t.pos).normalize_or(Vec3::Z);
            let side = Vec3::Y.cross(dir).normalize_or(Vec3::X);
            let throw = (side * 2.0 + Vec3::Y).normalize() * 100.0;
            self.swing_hit.push(t.id);
            self.events.push(PeteyEvent::Hit { target: t.id, mario: t.mario, pos: t.pos, throw: Some(throw), kind: DamageKind::EnemyCommon });
        }
    }

    /// `launchPolDrop`: from the mouth toward the target (the game leads Mario by
    /// `mSLPollBallFront` along his facing; a [`Target`] has no facing, so it aims at the feet).
    fn launch_ball(&mut self, host: &dyn Host) {
        if self.ball.is_some() {
            return;
        }
        let from = self.mouth();
        let to = self.goal.with_y(self.ground_at(host, self.goal, self.goal.y));
        let vel = jump_velocity(to, from, self.params.poll_ball_speed, 0.1);
        let id = self.new_id();
        self.ball = Some(Ball { id, pos: from, vel, age: 0, ground: from.y, stamping: 0 });
        self.events.push(PeteyEvent::BallLaunched { id, pos: from, vel });
    }

    /// `TBPTornado::launch` at the target.
    fn launch_tornado(&mut self, targets: &[Target]) {
        if self.tornado.is_some() {
            return;
        }
        let goal = self
            .aim
            .and_then(|id| targets.iter().find(|t| t.id == id))
            .map_or(self.pos + dir_of(self.yaw) * 1500.0, |t| t.pos);
        let id = self.new_id();
        self.tornado = Some(Tornado {
            id,
            pos: self.pos,
            center: self.pos,
            goal,
            t: self.params.tornado_move_init,
            delta: Vec3::ZERO,
            vanishing: 0,
        });
        self.events.push(PeteyEvent::TornadoLaunched { id, pos: self.pos });
    }

    /// `TBPPolDrop::move`, `drop` and its hit check.
    fn step_ball(&mut self, host: &dyn Host, goop: &mut GoopField, targets: &[Target]) {
        let Some(mut b) = self.ball.take() else { return };
        if b.stamping > 0 {
            b.stamping -= 1;
            if b.stamping > 0 {
                self.ball = Some(b);
            }
            return;
        }
        let mut next = b.pos + b.vel;
        b.vel.y -= 0.1;
        b.age += 1;
        let ground = host.ground(Vec3::new(next.x, b.pos.y, next.z)).map(|g| g.height + 1.0);
        let gy = ground.unwrap_or(b.ground);
        b.ground = gy;
        let radius = self.ball_stamp_radius * self.params.poll_ball_stamp_scale;
        if next.y < gy {
            next.y = gy;
            b.pos = next;
            self.ball_drop(host, goop, &mut b, radius, true);
            self.ball = Some(b);
            return;
        }
        let mut walls = Vec::new();
        let pushed = host.walls(next, 80.0, 4, &mut walls);
        if !walls.is_empty() || pushed.with_y(0.0).distance(next.with_y(0.0)) > 0.01 || b.age > 1200 {
            return;
        }
        self.sound("MSD_SE_BS_BSPAKU_POLLUT_IMI", next, true);
        self.sound("MSD_SE_BS_BSPAKU_POLLUT_FLY", next, true);
        // The ball's 100-wide, 200-tall cylinder.
        if let Some(t) = targets.iter().find(|t| t.id != self.id && t.reached_by_ball(next)) {
            self.events.push(PeteyEvent::Hit { target: t.id, mario: t.mario, pos: next, throw: None, kind: DamageKind::EnemyCommon });
            self.shake(2);
            // On the ground it stamps where it hit (`unk80 = 2`), in the air it's gone.
            let tg = self.ground_at(host, t.pos, t.pos.y);
            if (t.pos.y - tg).abs() < 4.0 {
                b.pos = Vec3::new(next.x, tg, next.z);
                self.ball_drop(host, goop, &mut b, radius, false);
                self.ball = Some(b);
            }
            return;
        }
        b.pos = next;
        self.ball = Some(b);
    }

    fn ball_drop(&mut self, host: &dyn Host, goop: &mut GoopField, b: &mut Ball, radius: f32, loud: bool) {
        b.vel = Vec3::ZERO;
        b.stamping = self.clip_len(clip::BALL_STAMP);
        goop.stamp(host, b.pos, radius, self.goop_kind);
        self.events.push(PeteyEvent::Splat { pos: b.pos, radius });
        if loud {
            self.effect("ms_osendan", b.pos, None, 1.0, false);
            self.sound("MSD_SE_BS_BSPAKU_POLLUT_GND", b.pos, false);
            self.events.push(PeteyEvent::Shake { pos: b.pos, power: 1.0 });
        }
    }

    /// `TBPVomit::perform`: the pool spreads out in front of him while its clip plays.
    fn step_vomit(&mut self, host: &dyn Host, goop: &mut GoopField, targets: &[Target]) {
        let Some(mut v) = self.vomit.take() else { return };
        v.age += 1;
        let p = (v.age as f32 / v.len.max(1) as f32).min(1.0);
        if v.age % 4 == 1 || v.age >= v.len {
            let c = v.origin + dir_of(v.yaw) * (self.vomit_near + (self.vomit_reach - self.vomit_near) * p);
            let c = c.with_y(self.ground_at(host, c, v.origin.y));
            goop.stamp(host, c, self.vomit_radius, self.goop_kind);
            self.events.push(PeteyEvent::Splat { pos: c, radius: self.vomit_radius });
            // Enemies caught in the wave are hit once (a host choice: the goop itself is what
            // hurts a Mario).
            for t in targets.iter().filter(|t| !t.mario && t.id != self.id) {
                if !v.hit.contains(&t.id) && t.pos.with_y(0.0).distance(c.with_y(0.0)) < self.vomit_radius + t.radius && (t.pos.y - c.y).abs() < 300.0 {
                    v.hit.push(t.id);
                    self.events.push(PeteyEvent::Hit { target: t.id, mario: false, pos: t.pos, throw: None, kind: DamageKind::Namekuri });
                }
            }
        }
        if v.age < v.len {
            self.vomit = Some(v);
        }
    }

    /// `TBPTornado::perform`.
    fn step_tornado(&mut self, host: &dyn Host, targets: &[Target]) {
        let Some(mut tw) = self.tornado.take() else { return };
        let p = &self.params;
        if tw.vanishing > 0 {
            tw.pos += tw.delta;
            tw.vanishing -= 1;
            if tw.vanishing > 0 {
                self.tornado = Some(tw);
            }
            return;
        }
        let fade = self.tornado_fade.max(1);
        tw.t += p.tornado_move_inc;
        let to = tw.goal - tw.center;
        if tw.t > p.tornado_move_limit || to.length() < 100.0 {
            tw.vanishing = fade;
            self.tornado = Some(tw);
            return;
        }
        let angle = (360.0 - (tw.t as i32 % 360) as f32).to_radians();
        let radius = tw.t * p.tornado_roll_speed;
        tw.center += to.normalize() * p.tornado_speed;
        let mut next = Vec3::new(tw.center.x + radius * angle.cos(), tw.center.y, tw.center.z + radius * angle.sin());
        if let Some(g) = host.ground(next + Vec3::Y * 200.0) {
            next.y = g.height;
        }
        let mut walls = Vec::new();
        let pushed = host.walls(next, 80.0, 4, &mut walls);
        let mut gone = !walls.is_empty() || pushed.with_y(0.0).distance(next.with_y(0.0)) > 0.01;
        tw.delta = next - tw.pos;
        tw.pos = next;
        let me = self.id;
        if let Some(t) = targets.iter().find(|t| t.id != me && t.reached_by_tornado(next)) {
            self.events.push(PeteyEvent::Hit { target: t.id, mario: t.mario, pos: next, throw: Some(Vec3::Y * 100.0), kind: DamageKind::EnemyCommon });
            gone = true;
        }
        for name in ["ms_bopa_tr_rock", "ms_bopa_tr_smoke", "ms_bopa_tr_weed"] {
            self.effect(name, next, None, 2.0, true);
        }
        self.sound("MSD_SE_BS_BSPAKU_TORNADO", next, true);
        if gone {
            tw.vanishing = fade;
        }
        self.tornado = Some(tw);
    }
}

trait Reach {
    fn reached_by_ball(&self, at: Vec3) -> bool;
    fn reached_by_tornado(&self, at: Vec3) -> bool;
}

impl Reach for Target {
    fn reached_by_ball(&self, at: Vec3) -> bool {
        Enemy { id: self.id, pos: self.pos, radius: self.radius, height: self.height }.reached_by(at, 100.0, 200.0)
    }

    fn reached_by_tornado(&self, at: Vec3) -> bool {
        Enemy { id: self.id, pos: self.pos, radius: self.radius, height: self.height }.reached_by(at, 150.0, 600.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::FlatHost;

    fn enemy(id: u64, pos: Vec3) -> Target {
        Target { id, pos, radius: 50.0, height: 150.0, mario: false }
    }

    fn quiet() -> Petey {
        let mut p = Petey::default();
        // Long enough for the tornado's launch at 150 and the flying shot at 168.
        p.clips.ticks[clip::TORNADO as usize] = 400;
        p.clips.ticks[clip::FLY_POLLUT as usize] = 400;
        p.clips.ticks[clip::POLLUT_START as usize] = 400;
        p.params.tornado_prop = 0.0;
        p.sleep_after = 0;
        p
    }

    fn run(p: &mut Petey, goop: &mut GoopField, owner: Vec3, targets: &[Target], ticks: u32, mut stop: impl FnMut(&Petey, &[PeteyEvent]) -> bool) -> bool {
        let host = FlatHost::default();
        for _ in 0..ticks {
            p.step(&host, goop, owner, targets);
            let ev = std::mem::take(&mut p.events);
            if stop(p, &ev) {
                return true;
            }
        }
        false
    }

    #[test]
    fn jump_velocity_reaches_the_target() {
        let from = Vec3::new(0.0, 400.0, 0.0);
        let to = Vec3::new(0.0, 0.0, 1500.0);
        let v0 = jump_velocity(to, from, 30.0, 0.1);
        assert!((v0.with_y(0.0).length() - 30.0).abs() < 0.01);
        // Fly it like `TBPPolDrop::move` (move, then gravity).
        let (mut p, mut v) = (from, v0);
        while p.y > 0.0 {
            p += v;
            v.y -= 0.1;
        }
        assert!((p.z - 1500.0).abs() < 60.0, "landed at {}", p.z);
    }

    #[test]
    fn he_turns_to_an_enemy_and_vomits_goop_at_it() {
        let mut p = quiet();
        let mut goop = GoopField::new();
        // Behind him, inside the vomit's reach but past the swing.
        let t = [enemy(7, Vec3::new(0.0, 0.0, -1200.0))];
        let mut states = Vec::new();
        let mut hit = false;
        let ok = run(&mut p, &mut goop, Vec3::ZERO, &t, 6000, |p, ev| {
            if states.last() != Some(&p.state) {
                states.push(p.state);
            }
            hit |= ev.iter().any(|e| matches!(e, PeteyEvent::Hit { target: 7, .. }));
            hit && p.vomit.is_none()
        });
        assert!(ok, "states {states:?}");
        assert!(states.contains(&State::Pivot) && states.contains(&State::Vomit));
        assert!(goop.is_polluted(Vec3::new(0.0, 0.0, -1200.0)), "goop where the enemy stood");
        assert!(goop.is_polluted(Vec3::new(0.0, 0.0, -500.0)), "and on the way");
        assert!(!goop.is_polluted(Vec3::new(0.0, 0.0, 800.0)), "nothing behind him");
    }

    #[test]
    fn the_mouth_is_open_only_mid_vomit() {
        let mut p = quiet();
        let mut goop = GoopField::new();
        let t = [enemy(7, Vec3::new(0.0, 0.0, 1200.0))];
        let mut opened = None;
        let mut closed_after = false;
        run(&mut p, &mut goop, Vec3::ZERO, &t, 3000, |p, _| {
            if p.weak_point == WeakPoint::Mouth && opened.is_none() {
                opened = Some(p.clip_frame());
            }
            if opened.is_some() && p.weak_point == WeakPoint::None && p.state == State::Vomit {
                closed_after = true;
            }
            p.clip == clip::POLLUT_END
        });
        let f = opened.expect("mouth opened");
        assert!(f > 25.0 && f < 27.0, "opens just past frame 25 (was {f})");
        assert!(closed_after);
    }

    #[test]
    fn a_far_enemy_gets_the_goop_ball() {
        let mut p = quiet();
        let mut goop = GoopField::new();
        let at = Vec3::new(0.0, 0.0, 2300.0);
        let t = [Target { id: 9, pos: at, radius: 50.0, height: 150.0, mario: false }];
        let mut launched = false;
        let mut splat = None;
        run(&mut p, &mut goop, Vec3::ZERO, &t, 4000, |p, ev| {
            launched |= ev.iter().any(|e| matches!(e, PeteyEvent::BallLaunched { .. }));
            for e in ev {
                if let PeteyEvent::Splat { pos, radius } = e {
                    splat = Some((*pos, *radius));
                }
            }
            splat.is_some() && p.state != State::Cannon
        });
        assert!(launched);
        let (pos, r) = splat.expect("the ball splatted");
        assert!(pos.distance(at) < 300.0, "landed at {pos}");
        assert_eq!(r, 600.0, "300 x mSLPollBallStampScale 2");
        assert!(goop.is_polluted(at));
    }

    #[test]
    fn a_close_enemy_in_front_gets_the_head_swing() {
        let mut p = quiet();
        let mut goop = GoopField::new();
        let t = [enemy(3, Vec3::new(100.0, 0.0, 400.0))];
        let mut throw = None;
        run(&mut p, &mut goop, Vec3::ZERO, &t, 600, |p, ev| {
            for e in ev {
                if let PeteyEvent::Hit { target: 3, throw: Some(v), .. } = e {
                    throw = Some(*v);
                }
            }
            throw.is_some() && p.state == State::Swing
        });
        let v = throw.expect("thrown");
        assert!((v.length() - 100.0).abs() < 0.01);
        assert!(v.y > 0.0 && v.with_y(0.0).length() > v.y, "sideways and up");
    }

    #[test]
    fn water_in_the_mouth_topples_him_and_a_stomp_hurts() {
        let mut p = quiet();
        p.params.water_mark_limit = 40;
        let mut goop = GoopField::new();
        let host = FlatHost::default();
        let t = [enemy(7, Vec3::new(0.0, 0.0, 1200.0))];
        let sprayer = Vec3::new(0.0, 0.0, 900.0);
        // Water from behind him doesn't count.
        let mut tick = 0;
        while p.weak_point != WeakPoint::Mouth {
            p.step(&host, &mut goop, Vec3::ZERO, &t);
            tick += 1;
            assert!(tick < 3000, "never opened");
        }
        assert!(!p.water_hit(p.mouth(), Vec3::new(0.0, 0.0, -900.0)));
        // Spray the open mouth.
        for _ in 0..60 {
            p.water_hit(p.mouth(), sprayer);
            p.step(&host, &mut goop, Vec3::ZERO, &t);
        }
        assert!(p.belly() > 0.9, "belly {}", p.belly());
        let mut saw_tumble = false;
        for _ in 0..2000 {
            p.step(&host, &mut goop, Vec3::ZERO, &[]);
            if p.weak_point == WeakPoint::Navel {
                saw_tumble = true;
                break;
            }
        }
        assert!(saw_tumble && p.state == State::Tumble);
        assert!(p.stomp());
        assert_eq!(p.hp, 2);
        assert_eq!(p.state, State::StompReact);
        assert!(!p.stomp(), "no second stomp until he's down again");
        let mut spout = false;
        for _ in 0..1000 {
            p.step(&host, &mut goop, Vec3::ZERO, &[]);
            spout |= p.events.drain(..).any(|e| matches!(e, PeteyEvent::WaterSpout { .. }));
            if p.state == State::Wait {
                break;
            }
        }
        assert!(spout);
        assert_eq!(p.state, State::Wait);
        for _ in 0..200 {
            p.step(&host, &mut goop, Vec3::ZERO, &[]);
        }
        assert_eq!(p.water_mark, 0, "the belly emptied");
    }

    #[test]
    fn three_stomps_defeat_him() {
        let mut p = quiet();
        let mut goop = GoopField::new();
        let host = FlatHost::default();
        for _ in 0..3 {
            p.weak_point = WeakPoint::Navel;
            assert!(p.stomp());
            for _ in 0..1500 {
                p.step(&host, &mut goop, Vec3::ZERO, &[]);
                if !p.alive || p.state == State::Wait {
                    break;
                }
            }
        }
        assert!(!p.alive);
        assert_eq!(p.hp, 0);
        p.respawn(Vec3::ZERO);
        assert!(p.alive && p.hp == 3);
    }

    #[test]
    fn standing_on_the_navel_makes_him_flinch() {
        let mut p = quiet();
        p.state = State::Tumble;
        p.weak_point = WeakPoint::Navel;
        p.trample();
        assert_eq!(p.state, State::JumpReact);
        let mut goop = GoopField::new();
        run(&mut p, &mut goop, Vec3::ZERO, &[], 400, |p, _| p.state == State::Tumble);
        assert_eq!(p.state, State::Tumble, "back to lying there");
    }

    #[test]
    fn he_flies_after_a_far_owner_and_lands_by_it() {
        let mut p = quiet();
        let mut goop = GoopField::new();
        let owner = Vec3::new(4000.0, 0.0, 0.0);
        let mut states = Vec::new();
        let mut max_y = 0.0f32;
        let ok = run(&mut p, &mut goop, owner, &[], 6000, |p, _| {
            if states.last() != Some(&p.state) {
                states.push(p.state);
            }
            max_y = max_y.max(p.pos.y);
            p.state == State::Wait && p.pos.x > 3000.0
        });
        assert!(ok, "states {states:?} at {}", p.pos);
        assert!(states.starts_with(&[State::Wait, State::TakeOff, State::Fly, State::TouchDown]));
        assert!((max_y - 800.0).abs() < 6.0);
        assert!(p.pos.y.abs() < 0.01);
        assert!((p.pos.distance(owner) - p.follow_distance).abs() < 120.0);
    }

    #[test]
    fn hovering_he_shoots_and_water_brings_him_down() {
        let mut p = quiet();
        p.flying_form = true;
        let mut goop = GoopField::new();
        let host = FlatHost::default();
        let t = [enemy(5, Vec3::new(0.0, 0.0, 1500.0))];
        let mut launched = false;
        for _ in 0..4000 {
            p.step(&host, &mut goop, Vec3::ZERO, &t);
            launched |= p.events.drain(..).any(|e| matches!(e, PeteyEvent::BallLaunched { .. }));
            if launched && p.weak_point == WeakPoint::Fly {
                break;
            }
        }
        assert!(launched, "state {:?}", p.state);
        assert_eq!(p.weak_point, WeakPoint::Fly);
        assert!(p.pos.y > 700.0);
        assert!(p.water_hit(p.pos + Vec3::Y * 200.0, Vec3::new(0.0, 0.0, 1000.0)));
        assert_eq!(p.state, State::Fall);
        p.flying_form = false;
        for _ in 0..2000 {
            p.step(&host, &mut goop, Vec3::ZERO, &[]);
            if p.state == State::Wait {
                break;
            }
        }
        assert_eq!(p.state, State::Wait);
        assert!(p.pos.y.abs() < 0.01);
    }

    #[test]
    fn the_tornado_circles_toward_its_target_and_throws_it() {
        let mut p = quiet();
        p.params.tornado_prop = 1.0;
        let mut goop = GoopField::new();
        let host = FlatHost::default();
        let t = [enemy(4, Vec3::new(0.0, 0.0, 1000.0))];
        let mut launched = false;
        for _ in 0..2000 {
            p.step(&host, &mut goop, Vec3::ZERO, &t);
            launched |= p.events.drain(..).any(|e| matches!(e, PeteyEvent::TornadoLaunched { .. }));
            if launched {
                break;
            }
        }
        assert!(launched);
        // It circles `t * mSLTornadoRollSpeed` (600 and up) out from a centre that heads for
        // the target, so a target standing still at the goal is never touched: put one where
        // the ring passes.
        for _ in 0..100 {
            p.step_tornado(&host, &[]);
        }
        let tw = p.tornado.clone().expect("running");
        let r = tw.pos.with_y(0.0).distance(tw.center.with_y(0.0));
        assert!((r - 606.0).abs() < 3.0, "ring radius {r}");
        assert!(tw.center.z > 150.0 && tw.center.x.abs() < 1.0, "centre heads for the target: {}", tw.center);
        let mut ahead = p.clone();
        ahead.step_tornado(&host, &[]);
        let spot = ahead.tornado.as_ref().unwrap().pos;
        let victim = [enemy(8, spot)];
        p.events.clear();
        p.step_tornado(&host, &victim);
        let thrown = p.events.iter().find_map(|e| match e {
            PeteyEvent::Hit { target: 8, throw, .. } => *throw,
            _ => None,
        });
        assert_eq!(thrown, Some(Vec3::Y * 100.0));
        assert!(p.tornado.as_ref().expect("fading").vanishing > 0);
    }

    #[test]
    fn a_lone_tornado_runs_out() {
        let mut p = quiet();
        let host = FlatHost::default();
        let far = [enemy(1, Vec3::new(0.0, 0.0, 100000.0))];
        p.aim = Some(1);
        p.launch_tornado(&far);
        let mut ticks = 0;
        while p.tornado.is_some() {
            p.step_tornado(&host, &far);
            ticks += 1;
            assert!(ticks < 2000);
        }
        // It runs until t passes `mSLTornadoMoveLimit` (721 ticks from `mSLTornadoMoveInit`),
        // then fades.
        assert_eq!(ticks, 721 + p.tornado_fade);
    }

    #[test]
    fn he_dozes_off_and_wakes_for_an_enemy() {
        let mut p = quiet();
        p.sleep_after = 300;
        let mut goop = GoopField::new();
        assert!(run(&mut p, &mut goop, Vec3::ZERO, &[], 3000, |p, _| p.state == State::Sleep));
        let t = [enemy(1, Vec3::new(0.0, 0.0, 1500.0))];
        assert!(run(&mut p, &mut goop, Vec3::ZERO, &t, 10, |p, _| p.state == State::BreakSleep));
    }

    #[test]
    fn marios_count_only_up_close() {
        let p = quiet();
        let far = Target { id: 2, pos: Vec3::new(0.0, 0.0, 1000.0), radius: 50.0, height: 160.0, mario: true };
        let near = Target { pos: Vec3::new(0.0, 0.0, 300.0), ..far };
        assert!(p.nearest(&[far]).is_none());
        assert!(p.nearest(&[near]).is_some());
    }

    #[test]
    fn the_disc_tuning_and_clips_load_when_present() {
        let Some(dir) = crate::assets::find(None) else {
            println!("SKIP: no assets/");
            return;
        };
        if !dir.join("params_enemy/bosspakkun.prm").is_file() || !dir.join("enemy/bosspakkun").is_dir() {
            println!("SKIP: no bosspakkun assets");
            return;
        }
        let p = Petey::from_dir(&dir, Vec3::ZERO);
        assert_eq!(p.params, PeteyParams::default(), "the defaults are the PAL disc's");
        for (i, n) in clip::NAMES.iter().enumerate() {
            assert!(dir.join(format!("enemy/bosspakkun/{n}.bck")).is_file(), "{n}.bck");
            assert_ne!(p.clips.ticks[i], 0);
        }
        for i in 0..clip::COUNT as u8 {
            if let Some(b) = clip::bas(i) {
                assert!(dir.join(format!("enemy/bosspakkun/bas/{b}.bas")).is_file(), "{b}.bas");
            }
        }
        // The mouth opens at frame 25 of `pollut_start` (164 frames: open to its end).
        let f = p.clips.ticks[clip::POLLUT_START as usize] as f32 / 2.0;
        assert!(f > 25.0, "pollut_start is {f} frames");
        println!("clip ticks: {:?}", p.clips.ticks);
    }
}
