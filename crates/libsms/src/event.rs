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
    /// Actual floor contact in `jumpProcess`, before its caller changes the air status.
    /// `drop` is from the continuous airborne peak, including across a dive transition.
    /// Hosts choose their own fall-damage policy; this does not change Sunshine health.
    FallImpact {
        drop: f32,
        pos: Vec3,
        diving: bool,
        ground_pound: bool,
        cushioned: bool,
    },
    /// Ground pound hit the floor (`hipAttacking`). `super_`: it fell for longer than
    /// `mSuperHipAttackCt` ticks and came down faster (the game's flaming super pound). `drop`:
    /// units fallen, from the top of the wind-up (`unk104`) to the floor.
    GroundPound { super_: bool, drop: f32 },
    /// Hit a wall hard enough to bounce off (`playerRefrection`).
    WallBump,
    /// FLUDD fired water. The host spawns/simulates the water particles and applies damage;
    /// the sim already applied the nozzle's recoil to Mario.
    WaterEmit(WaterEmit),
    /// `checkWet`: real model-water droplets after leaving water, from WetWaterEmit.prm.
    BodyDrip { pos: Vec3, velocity: Vec3 },
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
    /// He touched a Shine Sprite at `pos`, turned `yaw` ([`crate::Mario::get_shine`]): he is
    /// falling to the floor if he was in the air, and the pickup's sound has been asked for.
    ShineTouch { pos: Vec3, yaw: i16 },
    /// He has landed with the Shine (`winDemo` calling `fireGetStar`): the host starts the Shine
    /// Get camera, letters and fanfare, and the stage ends. `pos` is where he stands.
    ShineDemo { pos: Vec3, yaw: i16 },
    /// A ground pound began (its wind-up, `changePlayerStatus(HIP_ATTACK)`).
    PoundStart,
    /// Host limits ([`crate::Limits`]): FLUDD started in mid-air and spent a use; `extra`:
    /// one of the host's extras, not the free one.
    AirNozzle { extra: bool },
    /// Host limits: a mid-air jump was spent.
    AirJump,
    /// Host limits: a belly slide turned wet. Charged hosts spend availability; timed hosts
    /// use this to report skill activation without consuming stock or starting a cooldown.
    WetSlide,
    /// A particle effect the game starts at this point (`TMarioParticleManager`), by its
    /// `.jpa` name: dust, stars, blurs. The sim doesn't draw; a view runs the effect.
    Particle(ParticleCue),
    /// A sound the game starts at this point (`startSoundActor` and friends), by its
    /// `MSD_SE_*` name. Ids without the `0x800` bit are held sounds the game re-requests every
    /// frame while they last; hosts keep them alive rather than restarting them.
    Sound(SoundCue),
}

/// One `TMarioParticleManager` request.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParticleCue {
    /// The effect's file name, `ms_*` (`/scene/mario/particle/<name>.jpa` and the common
    /// ones).
    pub name: &'static str,
    pub at: FxAt,
    /// `setGlobalScale` (1 unless the game shrinks it).
    pub scale: f32,
}

/// Where a [`ParticleCue`] goes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FxAt {
    /// `emit` / `emitWithRotate(id, pos, 0, yaw, 0)`: started once at `pos` (sim world),
    /// turned `yaw` about up; it runs out by itself.
    Once { pos: Vec3, yaw: i16 },
    /// `emitAndBindToMtxPtr(id, getCenterAnmMtx(), 1, this)`, asked for again every tick it
    /// lasts: on Mario's middle as drawn, let go the first frame it isn't asked for.
    Center,
    /// The same on the body's joint `n` (`getAnmMtx(n)`).
    Joint(u16),
    /// `emitAndBindToPosPtr(id, &mPosition, 1, this)`: held, at Mario's feet.
    Feet,
    /// One-shot `emitAndBindToPosPtr(.., &mPosition, 0, ..)`, following Mario.
    FeetFollow,
    /// `emitAndBindToMtxPtr(id, getEmitMtx(n), 1, this)` / `emitAndBindToPosPtr(id, &getEmitPos0(), ..)`:
    /// held on FLUDD's `n`th muzzle as drawn, asked for again every tick it lasts.
    Muzzle(u8),
    /// `emitAndBindToPosPtr(id, &getEmitPos0(), 0, this)`: started once on the `n`th muzzle and bound to
    /// it, following it for the effect's own lifetime (the Rocket's exhaust).
    MuzzleFollow(u8),
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
    /// Hit by the Turbo corkscrew dive (host extension): a dive at full tilt that hits harder.
    Corkscrew,
    /// Hit by a corkscrew that has spun for 1 second, in flames (host extension): one huge blow that
    /// bonks Mario over.
    SuperCorkscrew,
    /// Hit from below by Mario rising in a spin out of the Rocket's climb (host extension).
    RocketSpin,
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
    /// A light press of the Spray, under pressure fades: a short burst. Not the game's; its
    /// hits say so ([`crate::WaterEvent::Hit`]) so a host can make them count for more.
    pub burst: bool,
    /// Sprayed in a spin (a spin jump, or on the ground), which flies in every direction: a host's aim never bends it, and its
    /// hits say so ([`crate::WaterEvent::Hit`]) so a host can count them for more.
    pub spin: bool,
}
