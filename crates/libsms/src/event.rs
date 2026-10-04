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
}
