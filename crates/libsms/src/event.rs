//! Things the sim tells the host about. The sim itself plays no sounds and draws nothing.

use glam::Vec3;

use crate::params::NozzleKind;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    /// Mario left the ground with a jump of this kind.
    Jump(JumpKind),
    /// Mario touched down. `strong` = fell far/fast enough for the heavy landing effect
    /// (`askStrongGroundTouch`).
    Land { strong: bool },
    /// Ground pound hit the floor (`hipAttacking`). `super_` after the long wind-up.
    GroundPound { super_: bool },
    /// Hit a wall hard enough to bounce off (`playerRefrection`).
    WallBump,
    /// FLUDD fired water. The host spawns/simulates the water particles and applies damage;
    /// the sim already applied the nozzle's recoil to Mario.
    WaterEmit(WaterEmit),
    /// FLUDD nozzle changed.
    NozzleChanged(NozzleKind),
    /// The hover/rocket trigger finished charging and fired (`TNozzleTrigger::movement`).
    TriggerFired(NozzleKind),
    /// Mario fell into a void (no legal ground for too long).
    Fell,
    /// Mario set the held object down in front of him (`HIT_MESSAGE_PUT`): the object places
    /// itself its radius plus Mario's plus 10 along `yaw` from `pos` (`TMapObjGeneral::put`).
    Put { id: u64, pos: Vec3, yaw: i16 },
    /// Mario let go of the held object with a throw (`HIT_MESSAGE_THROWN`): it starts at
    /// `pos` and flies along `yaw` at `power` (`mThrowPower`) times its own throw speed, plus
    /// its share of Mario's velocity `vel` (`TMapObjGeneral::thrown`).
    Throw {
        id: u64,
        pos: Vec3,
        yaw: i16,
        power: f32,
        vel: Vec3,
    },
    /// Yoshi swallowed `id` (`HIT_MESSAGE_UNK8` then `UNKB`), off his tongue or out of
    /// Mario's hands as he hatched: the object is gone (fruit grows back on its own).
    Eaten { id: u64 },
    /// Yoshi swallowed the enemy `id` off his tongue (`HIT_MESSAGE_UNKB`: `kill`).
    EnemyEaten { id: u64 },
    /// Mario hit the enemy `id` (`TMario::hitNormal`'s `receiveMessage`). The host decides
    /// what it does to the enemy; Mario has already reacted (a stomp bounces him).
    EnemyHit { id: u64, kind: EnemyHitKind },
    /// A hit landed (`damageExec`): `damage` health lost, `health` left. `yoshi`: Mario was
    /// riding, so Yoshi took it and threw him off instead. Hosts shake the camera and rumble.
    Damaged {
        damage: u8,
        health: i16,
        yoshi: bool,
    },
    /// Out of health (`loserExec`): Mario lies down for good until the host restarts him.
    Down,
    /// A sound the game starts at this point (`startSoundActor` and friends), by its
    /// `MSD_SE_*` name. Ids without the `0x800` bit are held sounds the game re-requests every
    /// frame while they last; hosts keep them alive rather than restarting them.
    Sound(SoundCue),
}

/// How Mario hit an enemy (the `HIT_MESSAGE_*` he sent).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnemyHitKind {
    /// Landed on it from above (`HIT_MESSAGE_TRAMPLE`, `trampleExec`).
    Trample,
    /// Ground pounded onto it (`HIT_MESSAGE_HIP_DROP`).
    HipDrop,
    /// Dived or slid into it (`HIT_MESSAGE_PUNCH`, then `HIT_MESSAGE_TRAMPLE`).
    Punch,
}

/// One `startSoundActor` / `startSoundActorWithInfo` call.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoundCue {
    /// `MSD_SE_*` name from MSound/SoundEffects.hpp.
    pub name: &'static str,
    pub pos: Vec3,
    /// `startSoundActorWithInfo`'s value, which the sound's registered modulation curve turns
    /// into pitch or volume (`JALSystem::processModFunc`).
    pub info: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JumpKind {
    Normal,
    Second,
    Triple,
    Back,
    Long,
    UTurn,
    Wall,
    Spin,
    Dive,
    Forced,
}

/// One burst of water from a FLUDD emitter (`TWaterEmitInfo`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaterEmit {
    pub nozzle: NozzleKind,
    pub pos: Vec3,
    /// Unit direction.
    pub dir: Vec3,
    /// Emit power (`mPow`), decomp units per tick scale.
    pub power: f32,
    /// Particles this tick (`mNum`).
    pub count: u32,
    /// Damage per particle (`mAttack`).
    pub attack: i16,
    pub hit_radius: f32,
    pub hit_height: f32,
    /// Velocity the particles inherit from Mario: `mVel.xz * 0.125` (`getEmitPosDirSpeed`).
    pub inherit: Vec3,
    /// Particle size and the random spreads (`mSize`, `mSizeTremble`, `mDirTremble`,
    /// `mPowTremble`), and which `/Mario/WaterParticle` type they are (`mType`).
    pub size: f32,
    pub size_tremble: f32,
    pub dir_tremble: f32,
    pub pow_tremble: f32,
    pub particle_type: i16,
}
