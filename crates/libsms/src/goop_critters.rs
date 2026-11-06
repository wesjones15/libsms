//! The goop's creatures, on Bowser Jr.'s side: Swoopin' Stus that grow out of goop and leap at
//! what's near, and the little Piranha Plants that spit goop, swell up when sprayed and burst.
//!
//! * **Swoopin' Stu** (`TNameKuri`, Enemy/namekuri.cpp; tuning `namekuri.prm`). The game
//!   launches them from goop (`TNameKuriLauncher::stateLaunch`: up at 4 a tick, the `name_set`
//!   clip). Here [`Critters`] grows one out of a random spot of goop near its owner every
//!   [`Critters::spawn_interval`] ticks. A Stu crawls after its target at `mSLMarchSpeed*`,
//!   and inside `mSLJumpAttackRadius` it winds up (`mSLJumpMaxFrame`, squashing by
//!   `mSLJumpMaxScale`) and leaps (`TNerveNameKuriJumpAttack`, under `mSLJumpAttackGravity`, at
//!   `mSLJumpAttackSp`, turning `mSLJumpAttackTurnSp` a tick). Touching the target in a leap is
//!   its attack (`attackToMario`: `HIT_MESSAGE_ATTACK`, then it dies). It lives
//!   `mSLAliveTime` ticks, swelling 1% a tick over the last 100 (`moveObject`). Water kills it
//!   (`behaveToWater`: melting if it's on the ground, `setMeltAnm`); a stomp or old age kills it
//!   and it leaves a puddle of goop (`TNerveSmallEnemyDie`: its `unk150` has 0x20, so
//!   `stamp(.., mSLStampRange * 32 * scale)`). Each Stu takes the next of the seven tints
//!   (`nameKuriTevColorData`, `getNextColorIdx`).
//! * **Goop Piranha** (`TPakkun`, Enemy/pakkun.cpp; `pakkun.prm`). It grows where its seed
//!   lands (`TNervePakkunGenerate`: the seed falls, `HIDE`, then `TNervePakkunAppear` plays
//!   `pakun_set`). It stays (`TNervePakkunStay`) and, `mSLReadyTime` after its last shot with
//!   a target inside `mSLShootRange`, shoots (`TNervePakkunShoot`: the seed shows in its mouth at
//!   clip frame 60, `shootIn`, and leaves at 70, `onShootLiner`: at `mSLSeedSpeedS` toward the
//!   target, 5 down, under `mSLSeedGravityS`). Water a little above its root
//!   (`mIgnoreHitWaterY`, 50) makes it freeze and swell (`behaveToWater` ->
//!   `TNervePakkunFreeze`: `pakun_hit_start`, `pakun_hit` while it keeps being sprayed,
//!   `pakun_hit_end`); the root joint is drawn scaled by the swell (`PakkunRootCallback`, up to
//!   `mRootExplosionScaleRate`, 2) — enough water and it bursts (`setAfterDeadEffect`:
//!   `ms_enm_disap_a`/`_b` and the smoke sound) and drops coins (`generateItem`). A stomp kills
//!   it (`pakun_down`).
//! * **Goop shots** (`TPakkunSeed`): fly, skip along the ground (`behaveToHitGround`), hit what
//!   they touch (20-unit cylinder), rest `mSLGenerateSeedTime` as a hazard, then splat
//!   goop where they land (`rebirth` -> `seedPollute`: `mSLStampRange * 32 * unk158`).
//!
//! Targets come from the host each tick ([`Target`]): its enemies, and the Marios — but a Mario
//! only while he's within [`Critters::mario_range`] (the user's rule: the critters go for Mario
//! when he's close). Hits come back as [`CritterEvent::Hit`]; the host deals them (a Mario
//! takes `DamageKind::Namekuri`, which leaves 100 goop on him).
//!
//! Reconstructed (the decomp doesn't have it): the Stu's leap nerve (the vertical speed is
//! solved to reach the target), the Piranha's swell (`unk1B8` is never written in the ported
//! code; here it grows with the water taken, [`Critters::burst_water`]), the seed's
//! `unk158` scale ([`Critters::shot_goop_scale`]), and where the mouth is (a fixed offset, not
//! the `null_seed` joint). Spawn counts and the spawn timer are host choices.

use std::path::Path;

use glam::Vec3;

#[path = "goop_gatekeeper.rs"]
mod gatekeeper;
pub use gatekeeper::{GateKeeper, KEEPER_TICKS_PER_FRAME, KeeperParams, KeeperState, keeper_clip, keeper_clip_ticks};

use crate::damage::DamageKind;
use crate::goop::{GoopField, GoopKind};
use crate::host::{Enemy, Host, SurfaceFlags};
use crate::prm::Prm;
use crate::skeleton::Clip;

/// Ids handed out here start at this, above the host's own and [`crate::flower::ID_BASE`]'s.
pub const ID_BASE: u64 = 0x5000_0000;

/// Something a critter can go for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Target {
    pub id: u64,
    /// Its feet.
    pub pos: Vec3,
    pub radius: f32,
    pub height: f32,
    /// A Mario: only targeted inside [`Critters::mario_range`].
    pub mario: bool,
}

impl Target {
    fn reached_by(&self, pos: Vec3, radius: f32, height: f32) -> bool {
        Enemy { id: self.id, pos: self.pos, radius: self.radius, height: self.height }
            .reached_by(pos, radius, height)
    }
}

/// `namekuri.prm` (`TNameKuriSaveLoadParams` + the walker's).
#[derive(Debug, Clone, PartialEq)]
pub struct StuParams {
    pub alive_time: u32,
    pub jump_attack_gravity: f32,
    pub jump_attack_sp: f32,
    /// Degrees a tick.
    pub jump_attack_turn_sp: f32,
    /// Ticks between leaps (`mSLJumpPrepareFrame`).
    pub jump_prepare_frame: u32,
    /// Clip frames of the wind-up (`mSLJumpMaxFrame`).
    pub jump_max_frame: f32,
    pub jump_max_angle: f32,
    pub jump_max_scale: f32,
    pub jump_attack_radius: f32,
    /// Degrees either side of its facing the target may be to leap.
    pub jump_attack_angle: f32,
    pub march_speed_low: f32,
    pub march_speed_high: f32,
    pub turn_speed: f32,
    pub search_length: f32,
    pub search_height: f32,
    pub body_scale_low: f32,
    pub body_scale_high: f32,
    pub attack_radius: f32,
    pub attack_height: f32,
    pub damage_radius: f32,
    pub damage_height: f32,
    pub stamp_range: f32,
    pub gravity: f32,
}

impl Default for StuParams {
    /// The PAL disc's `namekuri.prm` (the code defaults where the file has none).
    fn default() -> Self {
        StuParams {
            alive_time: 2000,
            jump_attack_gravity: 0.135,
            jump_attack_sp: 6.0,
            jump_attack_turn_sp: 1.0,
            jump_prepare_frame: 600,
            jump_max_frame: 10.0,
            jump_max_angle: 45.0,
            jump_max_scale: 0.5,
            jump_attack_radius: 350.0,
            jump_attack_angle: 180.0,
            march_speed_low: 1.0,
            march_speed_high: 1.5,
            turn_speed: 0.2,
            search_length: 800.0,
            search_height: 300.0,
            body_scale_low: 0.6,
            body_scale_high: 1.0,
            attack_radius: 50.0,
            attack_height: 30.0,
            damage_radius: 40.0,
            damage_height: 90.0,
            stamp_range: 15.0,
            gravity: 0.15,
        }
    }
}

impl StuParams {
    pub fn from_prm(p: &Prm) -> StuParams {
        let d = StuParams::default();
        let f = |k: &str, v: f32| p.f32(k).unwrap_or(v);
        let i = |k: &str, v: u32| p.i32(k).map_or(v, |x| x.max(0) as u32);
        let n = |k: &str, v: f32| p.i32(k).map_or(v, |x| x as f32);
        StuParams {
            alive_time: i("mSLAliveTime", d.alive_time),
            jump_attack_gravity: f("mSLJumpAttackGravity", d.jump_attack_gravity),
            jump_attack_sp: f("mSLJumpAttackSp", d.jump_attack_sp),
            jump_attack_turn_sp: f("mSLJumpAttackTurnSp", d.jump_attack_turn_sp),
            jump_prepare_frame: i("mSLJumpPrepareFrame", d.jump_prepare_frame),
            jump_max_frame: f("mSLJumpMaxFrame", d.jump_max_frame),
            jump_max_angle: f("mSLJumpMaxAngle", d.jump_max_angle),
            jump_max_scale: f("mSLJumpMaxScale", d.jump_max_scale),
            jump_attack_radius: f("mSLJumpAttackRadius", d.jump_attack_radius),
            jump_attack_angle: f("mSLJumpAttackAngle", d.jump_attack_angle),
            march_speed_low: f("mSLMarchSpeedLow", d.march_speed_low),
            march_speed_high: f("mSLMarchSpeedHigh", d.march_speed_high),
            turn_speed: f("mSLTurnSpeedLow", d.turn_speed),
            search_length: f("mSLSearchLength", d.search_length),
            search_height: f("mSLSearchHeight", d.search_height),
            body_scale_low: f("mSLBodyScaleLow", d.body_scale_low),
            body_scale_high: f("mSLBodyScaleHigh", d.body_scale_high),
            attack_radius: n("mSLAttackRadius", d.attack_radius),
            attack_height: n("mSLAttackHeight", d.attack_height),
            damage_radius: n("mSLDamageRadius", d.damage_radius),
            damage_height: n("mSLDamageHeight", d.damage_height),
            stamp_range: p.u8("mSLStampRange").map_or(d.stamp_range, |v| v as f32),
            gravity: d.gravity,
        }
    }
}

/// `pakkun.prm` (`TPakkunSaveLoadParams` + the small enemy's).
#[derive(Debug, Clone, PartialEq)]
pub struct PiranhaParams {
    pub ready_time: u32,
    pub shoot_range: f32,
    pub seed_gravity_s: f32,
    pub seed_speed_s: f32,
    pub seed_gravity_c: f32,
    pub seed_speed_c: f32,
    pub search_length: f32,
    pub search_height: f32,
    /// Degrees a tick.
    pub turn_speed: f32,
    pub stamp_range: f32,
    pub damage_radius: f32,
    pub damage_height: f32,
    pub body_scale: f32,
    /// Ticks a shot rests on the ground before it sinks into goop.
    pub generate_seed_time: u32,
    /// Percent chances of an egg and an item when it dies (`generateItem`).
    pub gen_egg_rate: f32,
    pub gen_item_rate: f32,
}

impl Default for PiranhaParams {
    fn default() -> Self {
        PiranhaParams {
            ready_time: 500,
            shoot_range: 1200.0,
            seed_gravity_s: 0.01,
            seed_speed_s: 20.0,
            seed_gravity_c: 0.24466065,
            seed_speed_c: 15.0,
            search_length: 2000.0,
            search_height: 800.0,
            turn_speed: 3.0,
            stamp_range: 25.0,
            damage_radius: 78.0,
            damage_height: 200.0,
            body_scale: 1.108,
            generate_seed_time: 300,
            gen_egg_rate: 10.0,
            gen_item_rate: 50.0,
        }
    }
}

impl PiranhaParams {
    pub fn from_prm(p: &Prm) -> PiranhaParams {
        let d = PiranhaParams::default();
        let f = |k: &str, v: f32| p.f32(k).unwrap_or(v);
        let n = |k: &str, v: f32| p.i32(k).map_or(v, |x| x as f32);
        PiranhaParams {
            ready_time: p.i32("mSLReadyTime").map_or(d.ready_time, |x| x.max(0) as u32),
            shoot_range: f("mSLShootRange", d.shoot_range),
            seed_gravity_s: f("mSLSeedGravityS", d.seed_gravity_s),
            seed_speed_s: f("mSLSeedSpeedS", d.seed_speed_s),
            seed_gravity_c: f("mSLSeedGravityC", d.seed_gravity_c),
            seed_speed_c: f("mSLSeedSpeedC", d.seed_speed_c),
            search_length: f("mSLSearchLength", d.search_length),
            search_height: f("mSLSearchHeight", d.search_height),
            turn_speed: f("mSLTurnSpeedLow", d.turn_speed),
            stamp_range: p.u8("mSLStampRange").map_or(d.stamp_range, |v| v as f32),
            damage_radius: n("mSLDamageRadius", d.damage_radius),
            damage_height: n("mSLDamageHeight", d.damage_height),
            body_scale: f("mSLBodyScaleLow", d.body_scale),
            generate_seed_time: p.i32("mSLGenerateSeedTime").map_or(d.generate_seed_time, |x| x.max(0) as u32),
            gen_egg_rate: f("mSLGenEggRate", d.gen_egg_rate),
            gen_item_rate: f("mSLGenItemRate", d.gen_item_rate),
        }
    }
}

/// The Stu's clips by `setBckAnm` index (`namekuri2/name_*.bck` in file order).
pub mod stu_clip {
    pub const DOWN: u8 = 0;
    pub const HIT: u8 = 1;
    pub const JUMP_END: u8 = 2;
    pub const JUMP_START: u8 = 3;
    pub const LAND: u8 = 4;
    pub const SET: u8 = 5;
    pub const WAIT: u8 = 6;
    pub const WALK: u8 = 7;
    pub const NAMES: [&str; 8] =
        ["down", "hit", "jump_end", "jump_start", "land", "set", "wait", "walk"];
}

/// The Piranha's clips by `PAKKUN_ANM_*` (`pakkun/pakun_*.bck` in file order).
pub mod piranha_clip {
    pub const CRUSH_TO_HIDE: u8 = 0;
    pub const DAMAGE: u8 = 1;
    pub const DOWN: u8 = 2;
    pub const HIDE: u8 = 3;
    pub const HIT: u8 = 4;
    pub const HIT_END: u8 = 5;
    pub const HIT_START: u8 = 6;
    pub const SET: u8 = 7;
    pub const SHOOT: u8 = 8;
    pub const WAIT: u8 = 9;
    pub const NAMES: [&str; 10] = [
        "crush_to_hide", "damage", "down", "hide", "hit", "hit_end", "hit_start", "set", "shoot",
        "wait",
    ];
}

/// Clip lengths in ticks (a `.bck` frame is two 120 Hz ticks; see [`crate::flower::Clips`]).
#[derive(Debug, Clone, PartialEq)]
pub struct CritterClips {
    pub stu: [u32; 8],
    pub piranha: [u32; 10],
}

impl Default for CritterClips {
    fn default() -> Self {
        CritterClips { stu: [60; 8], piranha: [120; 10] }
    }
}

impl CritterClips {
    /// Reads the lengths from the extracted clips under `assets/enemy/` (missing ones keep
    /// the defaults).
    pub fn from_dir(assets: &Path) -> CritterClips {
        let mut c = CritterClips::default();
        let len = |p: std::path::PathBuf| {
            std::fs::read(p).ok().and_then(|d| Clip::parse(&d)).map(|c| (c.frames * 2.0).max(1.0) as u32)
        };
        for (i, n) in stu_clip::NAMES.iter().enumerate() {
            if let Some(t) = len(assets.join(format!("enemy/namekuri2/name_{n}.bck"))) {
                c.stu[i] = t;
            }
        }
        for (i, n) in piranha_clip::NAMES.iter().enumerate() {
            if let Some(t) = len(assets.join(format!("enemy/pakkun/pakun_{n}.bck"))) {
                c.piranha[i] = t;
            }
        }
        c
    }
}

/// `nameKuriTevColorData`: a Stu's body tint (TEV register 0), RGBA, signed 10-bit.
pub const STU_TINTS: [[i16; 4]; 7] = [
    [0x64, 0x0, 0xA, 0xA9],
    [0x0, 0x0, 0x64, 0xA9],
    [0x5D, 0x54, 0x0, 0xA9],
    [0x0, 0x4C, 0x10, 0xA9],
    [0x63, 0x4, 0x60, 0xA9],
    [0x0, 0x44, 0x75, 0xA9],
    [0x64, 0x28, 0x0, 0xA9],
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StuState {
    /// Rising out of the goop (`name_set`).
    Generate,
    Walk,
    /// Winding up a leap (`name_jump_start`).
    Prepare,
    Leap,
    Land,
    /// Dying (`name_down`, or `name_hit` melting); gone when its clip ends.
    Die { melted: bool },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stu {
    pub id: u64,
    pub pos: Vec3,
    pub vel: Vec3,
    /// Facing, radians: along `(sin, 0, cos)`.
    pub yaw: f32,
    /// `mScaling`, drawn on top of the clip.
    pub scale: f32,
    /// The wind-up's squash (1 when none) and lean (radians, `unk1AC`).
    pub squash: f32,
    pub lean: f32,
    pub tint: [i16; 4],
    pub state: StuState,
    /// Clip index ([`stu_clip`]) and ticks into it.
    pub clip: u8,
    pub clip_time: u32,
    pub age: u32,
    pub target: Option<u64>,
    timer: u32,
    cooldown: u32,
    airborne: bool,
    base_scale: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PiranhaState {
    Appear,
    Stay,
    Shoot,
    /// Sprayed: swelling (`TNervePakkunFreeze`).
    Swell,
    /// Stomped (`pakun_down`); gone when it ends.
    Down,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Piranha {
    pub id: u64,
    pub pos: Vec3,
    pub yaw: f32,
    /// The root joint's scale (`unk1B8`): 1, up to 2 when about to burst.
    pub swell: f32,
    pub state: PiranhaState,
    pub clip: u8,
    pub clip_time: u32,
    pub target: Option<u64>,
    /// Water taken.
    pub water: f32,
    timer: u32,
    since_shot: u32,
    sprayed: bool,
    aim: Vec3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShotKind {
    /// A goop ball: hits what it touches and splats goop where it lands.
    Goop,
    /// A Piranha seed: a Piranha grows where it lands.
    Seed,
    /// Shadow Mario's brush blob (a host attack; the disc's Shadow Mario only paints at his
    /// feet): flies to what the player aims at and splats its own goop kind on whatever it
    /// touches first, a target, the ground or a wall.
    Paint,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Shot {
    pub id: u64,
    pub pos: Vec3,
    pub vel: Vec3,
    pub gravity: f32,
    pub kind: ShotKind,
    /// Who shot it (a host id, or a Piranha's), so it doesn't hit its shooter.
    pub owner: u64,
    /// Goop it splats, radius.
    pub splat: f32,
    /// The goop kind it splats (the critters' own when None).
    pub goop: Option<GoopKind>,
    /// Its hit's share of a critter hit's damage (1 = a goop ball's).
    pub damage: f32,
    /// `TPakkunSeed`'s spin about Z (degrees, `unk170`).
    pub spin: f32,
    age: u32,
    /// Ticks at rest on the ground (0: flying).
    rest: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CritterKind {
    Stu,
    Piranha,
    /// The big goop Piranha ([`GateKeeper`]).
    GateKeeper,
}

/// What the critters did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CritterEvent {
    Spawned { id: u64, kind: CritterKind, pos: Vec3 },
    /// `HIT_MESSAGE_ATTACK` on `target`: the host deals it (`kind` for a Mario).
    /// `scale`: the share of a critter hit's damage (a spray blob's is small).
    Hit { id: u64, target: u64, mario: bool, pos: Vec3, kind: DamageKind, scale: f32 },
    Died { id: u64, pos: Vec3, melted: bool },
    /// A Piranha burst from water: `coins` to drop (`generateItem`).
    Burst { id: u64, pos: Vec3, coins: u8 },
    /// A shot left (`id` the shot's).
    Shot { id: u64, from: Vec3, kind: ShotKind },
    /// A shot landed and splatted goop of `radius`.
    Splat { id: u64, pos: Vec3, radius: f32 },
    Sound { name: &'static str, pos: Vec3 },
    /// A particle effect by its `.jpa` name.
    Effect { name: &'static str, pos: Vec3, scale: f32 },
}

/// Every critter.
#[derive(Debug, Clone)]
pub struct Critters {
    pub stu_params: StuParams,
    pub piranha_params: PiranhaParams,
    pub clips: CritterClips,
    pub stus: Vec<Stu>,
    pub piranhas: Vec<Piranha>,
    pub shots: Vec<Shot>,
    pub events: Vec<CritterEvent>,
    /// The big goop Piranhas ([`GateKeeper`]), their tuning and clip lengths (ticks).
    pub keepers: Vec<GateKeeper>,
    pub keeper_params: KeeperParams,
    pub keeper_clips: [u32; 19],
    /// Whether Gatekeepers grow out of big piles by themselves (off: only [`Critters::spawn_keeper`]).
    pub keeper_spawning: bool,
    keeper_cooldown: u32,
    keeper_look: u32,
    /// Stus alive at once, at most (the disc's stages keep 2 a launcher).
    pub max_stus: usize,
    pub max_piranhas: usize,
    /// Ticks between Stus growing out of the goop (0: none grow by themselves).
    pub spawn_interval: u32,
    /// How far from the owner a Stu may grow.
    pub spawn_radius: f32,
    /// A Mario is a target only this close.
    pub mario_range: f32,
    /// Water (particle attack) a Piranha takes to burst.
    pub burst_water: f32,
    /// A goop shot's splat is `mSLStampRange * 32` times this.
    pub shot_goop_scale: f32,
    /// What the goop is made of.
    pub goop_kind: GoopKind,
    next_id: u64,
    tint: usize,
    spawn_timer: u32,
    seed: u32,
}

impl Default for Critters {
    fn default() -> Self {
        Critters::new(StuParams::default(), PiranhaParams::default(), CritterClips::default())
    }
}

fn dir_of(yaw: f32) -> Vec3 {
    Vec3::new(yaw.sin(), 0.0, yaw.cos())
}

fn yaw_to(from: Vec3, to: Vec3) -> f32 {
    (to.x - from.x).atan2(to.z - from.z)
}

/// Turns `from` toward `to` by at most `step` (radians).
fn turn(from: f32, to: f32, step: f32) -> f32 {
    let d = (to - from + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    from + d.clamp(-step, step)
}

impl Critters {
    pub fn new(stu: StuParams, piranha: PiranhaParams, clips: CritterClips) -> Critters {
        Critters {
            stu_params: stu,
            piranha_params: piranha,
            clips,
            stus: Vec::new(),
            piranhas: Vec::new(),
            shots: Vec::new(),
            events: Vec::new(),
            keepers: Vec::new(),
            keeper_params: KeeperParams::default(),
            keeper_clips: [400; 19],
            keeper_spawning: true,
            keeper_cooldown: 0,
            keeper_look: 0,
            max_stus: 6,
            max_piranhas: 4,
            spawn_interval: 600,
            spawn_radius: 1500.0,
            mario_range: 400.0,
            burst_water: 8.0,
            shot_goop_scale: 0.25,
            goop_kind: GoopKind::Sink,
            next_id: ID_BASE,
            tint: 0,
            spawn_timer: 0,
            seed: 0x9E37_79B9,
        }
    }

    /// The disc's tuning and clip lengths from an extracted `assets/` folder.
    pub fn from_dir(assets: &Path) -> Critters {
        let prm = |n: &str| std::fs::read(assets.join("params_enemy").join(n)).ok().and_then(|d| Prm::parse(&d).ok());
        let mut c = Critters::new(
            prm("namekuri.prm").map_or_else(StuParams::default, |p| StuParams::from_prm(&p)),
            prm("pakkun.prm").map_or_else(PiranhaParams::default, |p| PiranhaParams::from_prm(&p)),
            CritterClips::from_dir(assets),
        );
        c.keeper_clips = keeper_clip_ticks(assets);
        c
    }

    fn rand(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed >> 8) as f32 / (1u32 << 24) as f32
    }

    fn id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn sound(&mut self, name: &'static str, pos: Vec3) {
        self.events.push(CritterEvent::Sound { name, pos });
    }

    fn effect(&mut self, name: &'static str, pos: Vec3, scale: f32) {
        self.events.push(CritterEvent::Effect { name, pos, scale });
    }

    /// Whether an id is in the critters' range (any world's).
    pub fn owns_id(id: u64) -> bool {
        (ID_BASE..ID_BASE + 0x1000_0000).contains(&id)
    }

    /// Whether an id is one of these critters' (or their shots').
    pub fn owns(&self, id: u64) -> bool {
        self.stus.iter().any(|s| s.id == id)
            || self.piranhas.iter().any(|p| p.id == id)
            || self.shots.iter().any(|s| s.id == id)
            || self.keepers.iter().any(|k| k.id == id)
    }

    /// Hit cylinders for the host's world (what FLUDD's water and Mario's stomps hit).
    pub fn enemies(&self) -> Vec<Enemy> {
        let s = &self.stu_params;
        let p = &self.piranha_params;
        let stus = self.stus.iter().filter(|x| !matches!(x.state, StuState::Die { .. })).map(|x| Enemy {
            id: x.id,
            pos: x.pos,
            radius: s.damage_radius * x.scale,
            height: s.damage_height * x.scale,
        });
        let piranhas = self.piranhas.iter().filter(|x| x.state != PiranhaState::Down).map(|x| Enemy {
            id: x.id,
            pos: x.pos,
            radius: p.damage_radius * x.swell,
            height: p.damage_height,
        });
        stus.chain(piranhas).collect()
    }

    /// Grows a Stu out of the goop at `pos` (`TNameKuriLauncher::stateLaunch`).
    pub fn spawn_stu(&mut self, pos: Vec3) -> Option<u64> {
        let alive = self.stus.iter().filter(|s| !matches!(s.state, StuState::Die { .. })).count();
        if alive >= self.max_stus {
            return None;
        }
        let id = self.id();
        let p = &self.stu_params;
        let base = p.body_scale_low + (p.body_scale_high - p.body_scale_low) * 0.5;
        let tint = STU_TINTS[self.tint];
        self.tint = (self.tint + 1) % STU_TINTS.len();
        let yaw = self.rand() * std::f32::consts::TAU;
        self.stus.push(Stu {
            id,
            pos,
            vel: Vec3::new(0.0, 4.0, 0.0),
            yaw,
            scale: base,
            squash: 1.0,
            lean: 0.0,
            tint,
            state: StuState::Generate,
            clip: stu_clip::SET,
            clip_time: 0,
            age: 0,
            target: None,
            timer: 0,
            cooldown: 0,
            airborne: true,
            base_scale: base,
        });
        self.events.push(CritterEvent::Spawned { id, kind: CritterKind::Stu, pos });
        Some(id)
    }

    /// A Piranha appears at `pos` (`TNervePakkunAppear`).
    pub fn spawn_piranha(&mut self, pos: Vec3, yaw: f32) -> Option<u64> {
        if self.piranhas.len() >= self.max_piranhas {
            return None;
        }
        let id = self.id();
        self.piranhas.push(Piranha {
            id,
            pos,
            yaw,
            swell: 1.0,
            state: PiranhaState::Appear,
            clip: piranha_clip::SET,
            clip_time: 0,
            target: None,
            water: 0.0,
            timer: 0,
            since_shot: 0,
            sprayed: false,
            aim: pos,
        });
        self.events.push(CritterEvent::Spawned { id, kind: CritterKind::Piranha, pos });
        self.sound("MSD_SE_EN_PAKKUN_SEED_SINK", pos);
        Some(id)
    }

    /// Throws a seed a Piranha grows from where it lands (`onShootCurve`'s arc, under
    /// `mSLSeedGravityC`).
    pub fn throw_seed(&mut self, from: Vec3, vel: Vec3, owner: u64) -> u64 {
        let id = self.id();
        let g = self.piranha_params.seed_gravity_c;
        self.shots.push(Shot {
            id, pos: from, vel, gravity: g, kind: ShotKind::Seed, owner, splat: 0.0, goop: None, damage: 1.0, spin: 0.0, age: 0, rest: 0,
        });
        self.events.push(CritterEvent::Shot { id, from, kind: ShotKind::Seed });
        id
    }

    /// Fires a goop ball (`TPakkunSeed` shot straight: `onShootLiner`).
    pub fn shoot_goop(&mut self, from: Vec3, vel: Vec3, gravity: f32, splat: f32, owner: u64) -> u64 {
        let id = self.id();
        self.shots.push(Shot {
            id, pos: from, vel, gravity, kind: ShotKind::Goop, owner, splat, goop: None, damage: 1.0, spin: 0.0, age: 0, rest: 0,
        });
        self.events.push(CritterEvent::Shot { id, from, kind: ShotKind::Goop });
        self.sound("MSD_SE_EN_PAKKUN_SEED_FLY", from);
        id
    }

    /// Fires one of Shadow Mario's brush blobs ([`ShotKind::Paint`]): `kind` goop of `splat`
    /// radius where it lands, `damage` of a critter hit to what it hits. No sound of its own
    /// (the swing has its whoosh).
    #[allow(clippy::too_many_arguments)]
    pub fn shoot_paint(&mut self, from: Vec3, vel: Vec3, gravity: f32, splat: f32, kind: GoopKind, damage: f32, owner: u64) -> u64 {
        let id = self.id();
        self.shots.push(Shot {
            id, pos: from, vel, gravity, kind: ShotKind::Paint, owner, splat, goop: Some(kind), damage, spin: 0.0, age: 0, rest: 0,
        });
        self.events.push(CritterEvent::Shot { id, from, kind: ShotKind::Paint });
        id
    }

    /// FLUDD's water hit critter `id` with `attack` (`HIT_MESSAGE_SPRAYED_BY_WATER`); `at` is the
    /// drop. Returns whether `id` was one of these.
    pub fn water_hit(&mut self, id: u64, at: Vec3, attack: i16) -> bool {
        if let Some(i) = self.stus.iter().position(|s| s.id == id) {
            let s = &mut self.stus[i];
            if !matches!(s.state, StuState::Die { .. }) {
                // `behaveToWater`: dies, melting if it's on the ground.
                let melted = !s.airborne;
                s.state = StuState::Die { melted };
                s.clip = if melted { stu_clip::HIT } else { stu_clip::DOWN };
                s.clip_time = 0;
                s.vel = Vec3::ZERO;
                let pos = s.pos;
                self.effect("ms_enm_wathit", at, 1.0);
                self.effect("ms_deadnamekli_o", pos, 1.0);
                self.effect("ms_deadnamekli_n", pos, 1.0);
                self.sound(if melted { "MSD_SE_EN_NAMEKURI_DOWN_WT" } else { "MSD_SE_EN_NAMEKURI_DOWN" }, pos);
            }
            return true;
        }
        if let Some(i) = self.piranhas.iter().position(|p| p.id == id) {
            let burst = self.burst_water.max(1.0);
            let p = &mut self.piranhas[i];
            // `behaveToWater`: water at the root doesn't count (`mIgnoreHitWaterY`).
            if at.y <= p.pos.y + 50.0 || p.state == PiranhaState::Down {
                return true;
            }
            p.water += attack.max(1) as f32;
            p.sprayed = true;
            p.swell = 1.0 + (p.water / burst).min(1.0);
            if p.state != PiranhaState::Swell {
                p.state = PiranhaState::Swell;
                p.clip = piranha_clip::HIT_START;
                p.clip_time = 0;
            }
            self.effect("ms_enm_wathit", at, 1.0);
            self.sound("MSD_SE_EN_COMMON_W_HIT_OK", at);
            return true;
        }
        if let Some(i) = self.shots.iter().position(|s| s.id == id) {
            // Water knocks a shot down where it is.
            self.shots[i].vel = Vec3::new(0.0, -5.0, 0.0);
            return true;
        }
        false
    }

    /// Mario stomped or pounded critter `id` (`HIT_MESSAGE_TRAMPLE`/`HIP_DROP`). Returns whether
    /// it was one of these.
    pub fn stomp(&mut self, id: u64) -> bool {
        if let Some(s) = self.stus.iter_mut().find(|s| s.id == id) {
            if !matches!(s.state, StuState::Die { .. }) {
                s.state = StuState::Die { melted: false };
                s.clip = stu_clip::DOWN;
                s.clip_time = 0;
                let pos = s.pos;
                self.sound("MSD_SE_EN_NAMEKURI_DOWN", pos);
            }
            return true;
        }
        if let Some(p) = self.piranhas.iter_mut().find(|p| p.id == id) {
            if p.state != PiranhaState::Down {
                p.state = PiranhaState::Down;
                p.clip = piranha_clip::DOWN;
                p.clip_time = 0;
            }
            return true;
        }
        false
    }

    /// Picks the nearest target for something at `pos`: an enemy inside `length` (and
    /// `height` up or down), or a Mario inside `mario_range`.
    fn pick(&self, pos: Vec3, length: f32, height: f32, targets: &[Target]) -> Option<Target> {
        targets
            .iter()
            .filter(|t| (t.pos.y - pos.y).abs() < height)
            .filter(|t| {
                let d = (t.pos - pos).with_y(0.0).length();
                d < if t.mario { self.mario_range } else { length }
            })
            .min_by(|a, b| {
                let da = a.pos.distance_squared(pos);
                let db = b.pos.distance_squared(pos);
                da.total_cmp(&db)
            })
            .copied()
    }

    /// One tick. `owner` is where Bowser Jr. is (Stus grow near him); `targets` what the
    /// critters may go for this tick.
    pub fn step(&mut self, host: &dyn Host, goop: &mut GoopField, owner: Option<Vec3>, targets: &[Target]) {
        if let Some(o) = owner
            && self.spawn_interval > 0
        {
            self.spawn_timer += 1;
            if self.spawn_timer >= self.spawn_interval {
                self.spawn_timer = 0;
                let r = self.spawn_radius;
                let mut seed = self.seed;
                let spot = goop.random_spot(o, r, || {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    (seed >> 8) as f32 / (1u32 << 24) as f32
                });
                self.seed = seed;
                if let Some(p) = spot {
                    self.spawn_stu(p);
                }
            }
        }
        self.step_stus(host, goop, targets);
        self.step_piranhas(targets);
        self.step_keepers(goop, owner, targets);
        self.step_shots(host, goop, targets);
    }

    fn step_stus(&mut self, host: &dyn Host, goop: &mut GoopField, targets: &[Target]) {
        let p = self.stu_params.clone();
        let clips = self.clips.stu;
        let mut stus = std::mem::take(&mut self.stus);
        let mut gone = Vec::new();
        for s in stus.iter_mut() {
            s.clip_time += 1;
            s.age += 1;
            s.cooldown = s.cooldown.saturating_sub(1);
            let clip_done = s.clip_time >= clips[s.clip as usize];
            match s.state {
                StuState::Die { melted } => {
                    if clip_done {
                        if !melted {
                            let r = p.stamp_range * 32.0 * s.scale;
                            goop.stamp(host, s.pos, r, self.goop_kind);
                        }
                        gone.push(s.id);
                        self.events.push(CritterEvent::Died { id: s.id, pos: s.pos, melted });
                    }
                    continue;
                }
                StuState::Generate => {
                    if !s.airborne && clip_done {
                        s.state = StuState::Walk;
                        s.clip = stu_clip::WALK;
                        s.clip_time = 0;
                    }
                }
                StuState::Walk | StuState::Land => {
                    if s.state == StuState::Land && clip_done {
                        s.state = StuState::Walk;
                        s.clip = stu_clip::WALK;
                        s.clip_time = 0;
                    }
                    let t = self.pick(s.pos, p.search_length, p.search_height, targets);
                    s.target = t.map(|t| t.id);
                    if let Some(t) = t {
                        let want = yaw_to(s.pos, t.pos);
                        s.yaw = turn(s.yaw, want, p.turn_speed.to_radians() * 8.0);
                        let speed = p.march_speed_high;
                        s.vel.x = s.yaw.sin() * speed;
                        s.vel.z = s.yaw.cos() * speed;
                        let d = (t.pos - s.pos).with_y(0.0).length();
                        let off = turn(0.0, want - s.yaw, 10.0).abs().to_degrees();
                        if d < p.jump_attack_radius && off <= p.jump_attack_angle * 0.5 && s.cooldown == 0 && !s.airborne {
                            s.state = StuState::Prepare;
                            s.clip = stu_clip::JUMP_START;
                            s.clip_time = 0;
                            s.timer = 0;
                            s.vel.x = 0.0;
                            s.vel.z = 0.0;
                        }
                        if s.clip != stu_clip::WALK && s.state == StuState::Walk {
                            s.clip = stu_clip::WALK;
                            s.clip_time = 0;
                        }
                    } else {
                        s.vel.x = s.yaw.sin() * p.march_speed_low;
                        s.vel.z = s.yaw.cos() * p.march_speed_low;
                        if self.seed & 0xFF == 0 {
                            s.yaw += 0.5;
                        }
                    }
                }
                StuState::Prepare => {
                    s.timer += 1;
                    let wind = (p.jump_max_frame * 2.0).max(1.0);
                    let k = (s.timer as f32 / wind).min(1.0);
                    s.squash = 1.0 + p.jump_max_scale * k;
                    s.lean = -p.jump_max_angle.to_radians() * k;
                    if let Some(t) = s.target.and_then(|id| targets.iter().find(|t| t.id == id)) {
                        s.yaw = turn(s.yaw, yaw_to(s.pos, t.pos), p.jump_attack_turn_sp.to_radians() * 4.0);
                    }
                    if s.timer as f32 >= wind {
                        // The leap: forward at `mSLJumpAttackSp`, up enough to come down on the
                        // target under `mSLJumpAttackGravity`.
                        let (d, dy) = s
                            .target
                            .and_then(|id| targets.iter().find(|t| t.id == id))
                            .map_or((p.jump_attack_radius, 0.0), |t| {
                                ((t.pos - s.pos).with_y(0.0).length(), t.pos.y + t.height * 0.5 - s.pos.y)
                            });
                        let sp = p.jump_attack_sp.max(0.1);
                        let ticks = (d / sp).max(8.0);
                        let g = p.jump_attack_gravity;
                        let vy = ((dy + 0.5 * g * ticks * ticks) / ticks).clamp(2.0, 20.0);
                        s.vel = dir_of(s.yaw) * sp + Vec3::Y * vy;
                        s.state = StuState::Leap;
                        s.clip = stu_clip::JUMP_END;
                        s.clip_time = 0;
                        s.squash = 1.0;
                        s.airborne = true;
                        s.cooldown = p.jump_prepare_frame;
                        self.events.push(CritterEvent::Sound { name: "MSD_SE_EN_NAMEKURI_JUMP", pos: s.pos });
                    }
                }
                StuState::Leap => {
                    if let Some(t) = s.target.and_then(|id| targets.iter().find(|t| t.id == id)) {
                        let want = yaw_to(s.pos, t.pos);
                        s.yaw = turn(s.yaw, want, p.jump_attack_turn_sp.to_radians());
                        let h = s.vel.with_y(0.0).length();
                        s.vel.x = s.yaw.sin() * h;
                        s.vel.z = s.yaw.cos() * h;
                        s.lean = s.vel.y.atan2(h.max(0.01));
                        if t.reached_by(s.pos, p.attack_radius * s.scale, p.attack_height * s.scale) {
                            // `attackToMario`, then the leap's end kills it.
                            self.events.push(CritterEvent::Hit {
                                id: s.id,
                                target: t.id,
                                mario: t.mario,
                                pos: s.pos,
                                kind: DamageKind::Namekuri,
                                scale: 1.0,
                            });
                            s.state = StuState::Die { melted: false };
                            s.clip = stu_clip::DOWN;
                            s.clip_time = 0;
                            s.vel = Vec3::ZERO;
                            continue;
                        }
                    }
                }
            }
            // Old age: swell over the last 100 ticks, then die (and splat).
            if s.age + 100 > p.alive_time && s.scale < s.base_scale * 2.0 {
                s.scale *= 1.01;
            }
            if s.age > p.alive_time {
                s.state = StuState::Die { melted: false };
                s.clip = stu_clip::DOWN;
                s.clip_time = 0;
                s.vel = Vec3::ZERO;
                continue;
            }
            // `TLiveActor::bind`: gravity, the ground, the walls.
            let g = if s.state == StuState::Leap { p.jump_attack_gravity } else { p.gravity };
            s.vel.y = (s.vel.y - g).max(-40.0);
            let mut next = s.pos + s.vel;
            match host.ground(next + Vec3::Y * 30.0) {
                Some(gh) if !gh.flags.has(SurfaceFlags::ILLEGAL) => {
                    if next.y <= gh.height + 0.05 {
                        next.y = gh.height;
                        if s.airborne && s.state == StuState::Leap {
                            s.state = StuState::Land;
                            s.clip = stu_clip::LAND;
                            s.clip_time = 0;
                            s.lean = 0.0;
                        }
                        s.airborne = false;
                        s.vel.y = 0.0;
                        if s.state == StuState::Land {
                            s.vel.x = 0.0;
                            s.vel.z = 0.0;
                        }
                    } else {
                        s.airborne = true;
                    }
                }
                _ => {
                    // Off the world: gone.
                    gone.push(s.id);
                    self.events.push(CritterEvent::Died { id: s.id, pos: s.pos, melted: true });
                    continue;
                }
            }
            let mut walls = Vec::new();
            let pushed = host.walls(next + Vec3::Y * 20.0, 20.0, 2, &mut walls);
            next.x = pushed.x;
            next.z = pushed.z;
            if !walls.is_empty() && s.state == StuState::Walk {
                s.yaw += std::f32::consts::FRAC_PI_2;
            }
            s.pos = next;
            if s.state == StuState::Walk && s.clip_time % 60 == 1 {
                self.events.push(CritterEvent::Sound { name: "MSD_SE_EN_NAMEKURI_WALK", pos: s.pos });
            }
        }
        stus.retain(|s| !gone.contains(&s.id));
        self.stus = stus;
    }

    fn step_piranhas(&mut self, targets: &[Target]) {
        let p = self.piranha_params.clone();
        let clips = self.clips.piranha;
        let mut list = std::mem::take(&mut self.piranhas);
        let mut gone = Vec::new();
        let mut shots: Vec<(Vec3, Vec3, u64)> = Vec::new();
        for f in list.iter_mut() {
            f.clip_time += 1;
            f.since_shot += 1;
            let clip_done = f.clip_time >= clips[f.clip as usize];
            let t = self.pick(f.pos, p.search_length, p.search_height, targets);
            f.target = t.map(|t| t.id);
            if let Some(t) = t
                && f.state != PiranhaState::Down
            {
                f.yaw = turn(f.yaw, yaw_to(f.pos, t.pos), p.turn_speed.to_radians());
            }
            match f.state {
                PiranhaState::Appear => {
                    if clip_done {
                        f.state = PiranhaState::Stay;
                        f.clip = piranha_clip::WAIT;
                        f.clip_time = 0;
                    }
                }
                PiranhaState::Stay => {
                    if clip_done {
                        f.clip_time = 0;
                    }
                    if let Some(t) = t
                        && f.since_shot >= p.ready_time
                        && (t.pos - f.pos).length() < p.shoot_range
                    {
                        f.state = PiranhaState::Shoot;
                        f.clip = piranha_clip::SHOOT;
                        f.clip_time = 0;
                        f.aim = t.pos + Vec3::Y * t.height * 0.5;
                    }
                }
                PiranhaState::Shoot => {
                    // `checkPass(70)`: the seed leaves (clip frame 70 = tick 140).
                    // (Short stand-in clips fire on their last tick.)
                    let fire = 140.min(clips[piranha_clip::SHOOT as usize].saturating_sub(1).max(1));
                    if f.clip_time == fire {
                        let mouth = f.pos + Vec3::Y * 120.0 * p.body_scale + dir_of(f.yaw) * 40.0;
                        let mut d = (f.aim - mouth).with_y(0.0);
                        if d.length_squared() < 1e-6 {
                            d = dir_of(f.yaw);
                        }
                        let v = d.normalize() * p.seed_speed_s + Vec3::new(0.0, -5.0, 0.0);
                        shots.push((mouth, v, f.id));
                        f.since_shot = 0;
                    }
                    if clip_done {
                        f.state = PiranhaState::Stay;
                        f.clip = piranha_clip::WAIT;
                        f.clip_time = 0;
                    }
                }
                PiranhaState::Swell => {
                    if f.water >= self.burst_water {
                        // `setAfterDeadEffect` and `generateItem`.
                        let roll = self.seed_rand();
                        let coins = if roll * 100.0 < p.gen_egg_rate + p.gen_item_rate { 2 } else { 1 };
                        let pos = f.pos + Vec3::Y * 80.0;
                        self.events.push(CritterEvent::Effect { name: "ms_enm_disap_a", pos, scale: f.swell });
                        self.events.push(CritterEvent::Effect { name: "ms_enm_disap_b", pos, scale: f.swell });
                        self.events.push(CritterEvent::Sound { name: "MSD_SE_EN_COMMON_SMOKE", pos });
                        self.events.push(CritterEvent::Burst { id: f.id, pos, coins });
                        gone.push(f.id);
                        continue;
                    }
                    if clip_done {
                        f.clip_time = 0;
                        // `TNervePakkunFreeze`: start → loop while sprayed → end → back.
                        if f.clip == piranha_clip::HIT_START || f.sprayed {
                            f.clip = piranha_clip::HIT;
                        } else if f.clip == piranha_clip::HIT {
                            f.clip = piranha_clip::HIT_END;
                        } else {
                            f.state = PiranhaState::Stay;
                            f.clip = piranha_clip::WAIT;
                            f.water = 0.0;
                            f.swell = 1.0;
                        }
                        f.sprayed = false;
                    }
                }
                PiranhaState::Down => {
                    if clip_done {
                        gone.push(f.id);
                        self.events.push(CritterEvent::Died { id: f.id, pos: f.pos, melted: false });
                    }
                }
            }
        }
        list.retain(|f| !gone.contains(&f.id));
        self.piranhas = list;
        let (g, splat) = (p.seed_gravity_s, p.stamp_range * 32.0 * self.shot_goop_scale);
        for (from, v, owner) in shots {
            self.shoot_goop(from, v, g, splat, owner);
        }
    }

    fn seed_rand(&mut self) -> f32 {
        self.rand()
    }

    fn step_shots(&mut self, host: &dyn Host, goop: &mut GoopField, targets: &[Target]) {
        let rest_time = self.piranha_params.generate_seed_time;
        let mut shots = std::mem::take(&mut self.shots);
        let mut gone = Vec::new();
        let mut plants = Vec::new();
        for s in shots.iter_mut() {
            s.age += 1;
            // `TPakkunSeed::moveObject`: spins while it flies, settles once it rests.
            s.spin = if s.rest == 0 { (s.spin + 5.0) % 360.0 } else { (s.spin + 5.0).min(360.0) };
            if s.rest == 0 {
                s.vel.y = (s.vel.y - s.gravity).max(-40.0);
            }
            let mut next = s.pos + s.vel;
            let kind = s.goop.unwrap_or(self.goop_kind);
            if s.kind != ShotKind::Seed
                && let Some(t) = targets.iter().find(|t| t.id != s.owner && t.reached_by(next, 20.0, 20.0))
            {
                self.events.push(CritterEvent::Hit {
                    id: s.id, target: t.id, mario: t.mario, pos: next, kind: DamageKind::EnemyCommon, scale: s.damage,
                });
                if s.splat > 0.0 {
                    let ground = host.ground(next + Vec3::Y * 50.0).map_or(next.y, |g| g.height);
                    goop.stamp(host, Vec3::new(next.x, ground, next.z), s.splat * 0.5, kind);
                }
                gone.push(s.id);
                continue;
            }
            if s.kind == ShotKind::Paint {
                // Splats on the first thing it touches.
                let ground = host.ground(next + Vec3::Y * 20.0).filter(|g| next.y <= g.height + 0.05);
                let mut walls = Vec::new();
                let pushed = host.walls(next + Vec3::Y * 10.0, 20.0, 1, &mut walls);
                let landed = match (ground, walls.first()) {
                    (Some(g), _) if !g.flags.has(SurfaceFlags::WATER_SURFACE) => Some(Vec3::new(next.x, g.height, next.z)),
                    (Some(_), _) => None,
                    (None, Some(_)) => {
                        // A wall: the goop runs down it to the floor below.
                        let at = Vec3::new(pushed.x, next.y, pushed.z);
                        Some(host.ground(at + Vec3::Y * 20.0).map_or(at, |g| Vec3::new(at.x, g.height, at.z)))
                    }
                    (None, None) if s.age > 240 => None,
                    (None, None) => {
                        s.pos = next;
                        continue;
                    }
                };
                if let Some(at) = landed
                    && s.splat > 0.0
                {
                    goop.stamp(host, at, s.splat, kind);
                    self.events.push(CritterEvent::Splat { id: s.id, pos: at, radius: s.splat });
                }
                gone.push(s.id);
                continue;
            }
            let ground = host.ground(next + Vec3::Y * 20.0);
            let Some(g) = ground else {
                if s.age > 600 {
                    gone.push(s.id);
                }
                s.pos = next;
                continue;
            };
            let water = g.flags.has(SurfaceFlags::WATER_SURFACE);
            if next.y <= g.height + 0.05 {
                next.y = g.height;
                match s.kind {
                    ShotKind::Seed => {
                        plants.push((next, s.vel.x.atan2(s.vel.z)));
                        gone.push(s.id);
                        continue;
                    }
                    ShotKind::Paint => unreachable!("paint shots are handled above"),
                    ShotKind::Goop if s.rest == 0 && s.vel.y.abs() >= 1.0 && !water => {
                        // `behaveToHitGround` for a straight shot: skips along the ground.
                        s.vel.x *= 0.8;
                        s.vel.z *= 0.8;
                        s.vel.y = 0.1 * s.vel.y.abs();
                    }
                    ShotKind::Goop => {
                        // Comes to rest (`unk168`), still a hazard (its collision stays on)
                        // until `mSLGenerateSeedTime` passes; then it sinks into a splat of
                        // goop (`rebirth` -> `seedPollute`, `createPakkunSmoke`).
                        if s.rest == 0 {
                            s.vel = Vec3::new(0.0, -0.3, 0.0);
                            self.events.push(CritterEvent::Effect { name: "ms_packn_hd_smoke", pos: next, scale: 1.0 });
                        }
                        s.rest += 1;
                        if s.rest > rest_time || water {
                            if s.splat > 0.0 && !water {
                                goop.stamp(host, next, s.splat, kind);
                                self.events.push(CritterEvent::Splat { id: s.id, pos: next, radius: s.splat });
                            }
                            self.events.push(CritterEvent::Sound { name: "MSD_SE_EN_PAKKUN_SEED_SINK", pos: next });
                            gone.push(s.id);
                            continue;
                        }
                    }
                }
            }
            if s.age > 2400 {
                gone.push(s.id);
                continue;
            }
            let mut walls = Vec::new();
            let pushed = host.walls(next + Vec3::Y * 10.0, 20.0, 1, &mut walls);
            if let Some(w) = walls.first() {
                // `behaveToHitWall`: bounce off at 1.5 times the normal speed; a straight
                // shot drops.
                let r = -1.5 * s.vel.dot(w.normal);
                s.vel += w.normal * r;
                if s.kind == ShotKind::Goop {
                    s.vel.y = -5.0;
                }
            }
            s.pos = Vec3::new(pushed.x, next.y, pushed.z);
        }
        shots.retain(|s| !gone.contains(&s.id));
        self.shots = shots;
        for (at, yaw) in plants {
            // The plant sits in a little goop of its own.
            goop.stamp(host, at, 160.0, self.goop_kind);
            self.spawn_piranha(at, yaw);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::FlatHost;

    fn target(id: u64, x: f32, mario: bool) -> Target {
        Target { id, pos: Vec3::new(x, 0.0, 0.0), radius: 50.0, height: 150.0, mario }
    }

    #[test]
    fn stus_grow_out_of_goop_near_their_owner() {
        let mut c = Critters::default();
        c.spawn_interval = 10;
        let mut goop = GoopField::new();
        let host = FlatHost::default();
        // No goop, no Stus.
        for _ in 0..30 {
            c.step(&host, &mut goop, Some(Vec3::ZERO), &[]);
        }
        assert!(c.stus.is_empty());
        goop.stamp(&host, Vec3::new(500.0, 0.0, 0.0), 300.0, GoopKind::Sink);
        for _ in 0..30 {
            c.step(&host, &mut goop, Some(Vec3::ZERO), &[]);
        }
        assert!(!c.stus.is_empty());
        let s = &c.stus[0];
        assert!(goop.density(Vec3::new(s.pos.x, 0.0, s.pos.z)) > 0 || s.pos.distance(Vec3::new(500.0, 0.0, 0.0)) < 400.0);
        assert_ne!(c.stus.get(1).map(|t| t.tint), Some(s.tint), "each takes the next tint");
    }

    #[test]
    fn a_stu_crawls_up_leaps_and_hits_an_enemy() {
        let mut c = Critters::default();
        c.spawn_interval = 0;
        let mut goop = GoopField::new();
        let host = FlatHost::default();
        c.spawn_stu(Vec3::ZERO);
        let enemy = [target(42, 600.0, false)];
        let mut hit = false;
        for _ in 0..2000 {
            c.step(&host, &mut goop, None, &enemy);
            if c.events.drain(..).any(|e| matches!(e, CritterEvent::Hit { target: 42, mario: false, .. })) {
                hit = true;
                break;
            }
        }
        assert!(hit, "the Stu never reached its target: {:?}", c.stus.first().map(|s| (s.pos, s.state)));
    }

    #[test]
    fn mario_is_only_a_target_up_close() {
        let mut c = Critters::default();
        let far = [target(1, 700.0, true)];
        assert!(c.pick(Vec3::ZERO, 800.0, 300.0, &far).is_none());
        let near = [target(1, 300.0, true), target(2, 700.0, false)];
        assert_eq!(c.pick(Vec3::ZERO, 800.0, 300.0, &near).map(|t| t.id), Some(1));
        c.mario_range = 0.0;
        assert_eq!(c.pick(Vec3::ZERO, 800.0, 300.0, &near).map(|t| t.id), Some(2));
    }

    #[test]
    fn water_melts_a_stu_on_the_ground_and_a_stomp_leaves_goop() {
        let mut c = Critters::default();
        c.spawn_interval = 0;
        let mut goop = GoopField::new();
        let host = FlatHost::default();
        let a = c.spawn_stu(Vec3::ZERO).unwrap();
        let b = c.spawn_stu(Vec3::new(3000.0, 0.0, 0.0)).unwrap();
        for _ in 0..200 {
            c.step(&host, &mut goop, None, &[]);
        }
        assert!(c.water_hit(a, Vec3::ZERO, 1));
        assert!(c.stomp(b));
        for _ in 0..300 {
            c.step(&host, &mut goop, None, &[]);
        }
        assert!(c.stus.is_empty());
        let died: Vec<_> = c.events.iter().filter_map(|e| match e {
            CritterEvent::Died { id, melted, .. } => Some((*id, *melted)),
            _ => None,
        }).collect();
        assert!(died.contains(&(a, true)) && died.contains(&(b, false)));
        assert!(goop.is_polluted(Vec3::new(3000.0, 0.0, 0.0)) || goop.degree() > 0, "the stomped one splats");
    }

    #[test]
    fn a_piranha_shoots_goop_at_an_enemy() {
        let mut c = Critters::default();
        c.piranha_params.ready_time = 10;
        let mut goop = GoopField::new();
        let host = FlatHost::default();
        c.spawn_piranha(Vec3::ZERO, 0.0).unwrap();
        let enemy = [target(9, 560.0, false)];
        let mut shot = false;
        let mut hit = false;
        for _ in 0..1500 {
            c.step(&host, &mut goop, None, &enemy);
            for e in c.events.drain(..) {
                shot |= matches!(e, CritterEvent::Shot { kind: ShotKind::Goop, .. });
                hit |= matches!(e, CritterEvent::Hit { target: 9, .. });
            }
        }
        assert!(shot && hit, "shot {shot} hit {hit}");
    }

    #[test]
    fn spraying_a_piranha_swells_it_until_it_bursts() {
        let mut c = Critters::default();
        let mut goop = GoopField::new();
        let host = FlatHost::default();
        let id = c.spawn_piranha(Vec3::ZERO, 0.0).unwrap();
        // Water at its root doesn't count.
        c.water_hit(id, Vec3::new(0.0, 10.0, 0.0), 1);
        assert_eq!(c.piranhas[0].water, 0.0);
        let mut burst = None;
        for i in 0..400 {
            if i % 10 == 0 {
                c.water_hit(id, Vec3::new(0.0, 120.0, 0.0), 1);
            }
            c.step(&host, &mut goop, None, &[]);
            if let Some(p) = c.piranhas.first() {
                assert!(p.swell >= 1.0 && p.swell <= 2.0);
            }
            if let Some(CritterEvent::Burst { coins, .. }) = c.events.iter().find(|e| matches!(e, CritterEvent::Burst { .. })) {
                burst = Some(*coins);
                break;
            }
        }
        assert!(burst.is_some_and(|n| n >= 1));
        assert!(c.piranhas.is_empty());
    }

    #[test]
    fn a_seed_grows_a_piranha_where_it_lands() {
        let mut c = Critters::default();
        let mut goop = GoopField::new();
        let host = FlatHost::default();
        c.throw_seed(Vec3::new(0.0, 100.0, 0.0), Vec3::new(0.0, 8.0, 10.0), 1);
        for _ in 0..300 {
            c.step(&host, &mut goop, None, &[]);
        }
        assert_eq!(c.piranhas.len(), 1);
        assert!(c.piranhas[0].pos.z > 100.0);
        assert!(goop.is_polluted(c.piranhas[0].pos));
    }

    #[test]
    fn the_disc_tuning_loads_when_present() {
        let Some(dir) = crate::assets::find(None) else {
            println!("SKIP: no assets/");
            return;
        };
        let c = Critters::from_dir(&dir);
        if !dir.join("params_enemy/namekuri.prm").is_file() {
            println!("SKIP: no namekuri.prm");
            return;
        }
        assert_eq!(c.stu_params.alive_time, 2000);
        assert_eq!(c.stu_params.jump_attack_radius, 350.0);
        assert_eq!(c.piranha_params.ready_time, 500);
        assert_eq!(c.piranha_params.seed_speed_s, 20.0);
        if dir.join("enemy/pakkun/pakun_shoot.bck").is_file() {
            assert!(c.clips.piranha[piranha_clip::SHOOT as usize] > 140, "the shot leaves before the clip ends");
        }
    }
}
