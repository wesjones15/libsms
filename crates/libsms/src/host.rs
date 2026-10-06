//! The only way the sim sees the world. Units are the decomp's (about 1 unit = 1 cm; Mario is
//! 160 tall); hosts convert. Y is up. A facing angle `a` points along `(sin a, 0, cos a)`.
//!
//! The three queries mirror Sunshine's collision API (Map/Map.hpp `TMap`):
//! `checkGround`, `checkRoof` and `isTouchedWallsAndMoveXZ`.

use glam::Vec3;

use crate::event::Event;

/// Per-surface properties the locomotion code branches on. Hosts classify their own geometry
/// into these (Sunshine reads them from the collision triangle's type, `TBGCheckData`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SurfaceFlags(pub u32);

impl SurfaceFlags {
    pub const NONE: SurfaceFlags = SurfaceFlags(0);
    /// Void / out of bounds (`isIllegalData`): blocks walking, reverts jumps.
    pub const ILLEGAL: u32 = 1 << 0;
    /// Mario passes through it (`isMarioThrough`).
    pub const MARIO_THROUGH: u32 = 1 << 1;
    /// Always slippery (`isUnk1`/force slip: slider-style polluted or steep surfaces).
    pub const FORCE_SLIP: u32 = 1 << 2;
    /// Slide-park slider (`isSlider`).
    pub const SLIDER: u32 = 1 << 3;
    /// Slippery above 45 degrees (`isUnk2`).
    pub const SLIP_45: u32 = 1 << 4;
    /// Never slips regardless of slope (`isUnk3`).
    pub const NO_SLIP: u32 = 1 << 5;
    /// Wet ground (`isWetGround`): water slip parameters.
    pub const WET: u32 = 1 << 6;
    /// Surface of a body of water (`isWaterSurface`).
    pub const WATER_SURFACE: u32 = 1 << 7;
    /// Chain-link fence wall (`isFence`).
    pub const FENCE: u32 = 1 << 8;
    /// Walls Mario cannot wall-jump off (`isNoWallJump`).
    pub const NO_WALL_JUMP: u32 = 1 << 9;
    /// Ledges Mario cannot grab (`isNoLedgeGrab`).
    pub const NO_LEDGE_GRAB: u32 = 1 << 10;
    /// Ground-pound breaks through (`isGroundPoundToPassThrough`).
    pub const POUND_THROUGH: u32 = 1 << 11;

    pub fn has(self, flag: u32) -> bool {
        self.0 & flag != 0
    }

    pub fn with(self, flag: u32) -> SurfaceFlags {
        SurfaceFlags(self.0 | flag)
    }
}

/// Result of a downward probe.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GroundHit {
    pub height: f32,
    /// Unit normal, pointing up.
    pub normal: Vec3,
    pub flags: SurfaceFlags,
}

/// Result of an upward probe.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoofHit {
    pub height: f32,
    pub flags: SurfaceFlags,
}

/// A wall touched by a cylinder test.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WallHit {
    /// Horizontal unit normal, pointing away from the wall surface (toward open space).
    pub normal: Vec3,
    pub flags: SurfaceFlags,
}

/// Something Mario can pick up (a `TTakeActor`: fruit, barrels, ...).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Takeable {
    /// The host's handle for it.
    pub id: u64,
    pub pos: Vec3,
    /// Its hit radius (`getDamageRadius`).
    pub radius: f32,
}

pub trait Host {
    /// Highest floor at or below `pos.y` (`TMap::checkGround`). `None` means a void.
    fn ground(&self, pos: Vec3) -> Option<GroundHit>;

    /// Lowest ceiling at or above `pos.y` (`TMap::checkRoof`). `None` means open sky.
    fn roof(&self, _pos: Vec3) -> Option<RoofHit> {
        None
    }

    /// Pushes a vertical cylinder at `center` (radius `radius`) out of any walls it overlaps in
    /// XZ (`TMap::isTouchedWallsAndMoveXZ`). Appends up to `max` touched walls to `out` and
    /// returns the corrected center (Y unchanged).
    fn walls(&self, center: Vec3, _radius: f32, _max: usize, _out: &mut Vec<WallHit>) -> Vec3 {
        center
    }

    /// Gameplay events for the host to turn into sound, particles, damage and so on.
    fn event(&mut self, _event: Event) {}

    /// Takeable objects whose hit cylinders Mario's (`radius`) overlaps at `pos`. Sunshine's
    /// hit check calls `TMario::hitWantToTake` for these each tick.
    fn takeables(&self, _pos: Vec3, _radius: f32, _out: &mut Vec<Takeable>) {}

    /// `receiveMessage(HIT_MESSAGE_TAKE)`: Mario's hand closes on `id`. Return whether it is
    /// now held; the host then keeps it at Mario's right hand (`getTakingMtx`) until an
    /// [`Event::Put`] or [`Event::Throw`].
    fn take(&mut self, _id: u64) -> bool {
        false
    }
}

/// Infinite flat floor at y = 0, no ceiling, optionally one infinite wall plane. For tests.
#[derive(Debug, Clone, Copy, Default)]
pub struct FlatHost {
    /// `Some((x, normal_sign))`: a wall at `x`; Mario's side is `x < wall_x` when the normal
    /// sign is -1 (wall faces -X) and `x > wall_x` when it is +1.
    pub wall_x: Option<(f32, f32)>,
    /// Ceiling height, if any.
    pub ceiling: Option<f32>,
}

impl Host for FlatHost {
    fn ground(&self, _pos: Vec3) -> Option<GroundHit> {
        Some(GroundHit {
            height: 0.0,
            normal: Vec3::Y,
            flags: SurfaceFlags::NONE,
        })
    }

    fn roof(&self, pos: Vec3) -> Option<RoofHit> {
        self.ceiling.filter(|c| *c >= pos.y).map(|c| RoofHit {
            height: c,
            flags: SurfaceFlags::NONE,
        })
    }

    fn walls(&self, center: Vec3, radius: f32, _max: usize, out: &mut Vec<WallHit>) -> Vec3 {
        let Some((wx, sign)) = self.wall_x else {
            return center;
        };
        let dist = (center.x - wx) * sign;
        if dist < radius {
            out.push(WallHit {
                normal: Vec3::new(sign, 0.0, 0.0),
                flags: SurfaceFlags::NONE,
            });
            Vec3::new(wx + sign * radius, center.y, center.z)
        } else {
            center
        }
    }
}
