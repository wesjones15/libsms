//! C ABI over `libsms`, for hosts that aren't Rust (the Risk of Rain 2 plugin loads us as a DLL
//! and calls in through P/Invoke).
//!
//! Units and axes are the sim's (about 1 unit = 1 cm, Y up, a facing angle `a` points along
//! `(sin a, 0, cos a)`); hosts convert. All structs are `#[repr(C)]` and plain data. Nothing
//! here unwinds into the caller: a panic inside the sim poisons that sim, and later calls on it
//! do nothing.
//!
//! The shape: fill an [`SmsHost`] with collision callbacks, create a sim with
//! [`sms_sim_new`], call [`sms_sim_step`] once per host frame with the pad and camera, then read
//! [`sms_sim_state`]. Hosts that draw Mario themselves add a view ([`view`]: the disc's models
//! posed and skinned, and the game's sounds). [`sms_struct_sizes`] lets a host check its copies
//! of these structs.

pub mod assets;
pub mod view;

use std::ffi::{CStr, CString, c_char, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use libsms::event::{EnemyHitKind, Event, JumpKind};
use libsms::water::{WaterEvent, WaterManager};
use libsms::{
    Assets, Enemy, GroundHit, Host, Pad, RoofHit, Sim, SurfaceFlags, TICK_HZ, Vec2, Vec3, WallHit,
    WaterEmit, math,
};
use view::SmsView;

/// ABI version, bumped when the C interface changes.
pub const SMS_ABI_VERSION: u32 = 27;

#[unsafe(no_mangle)]
pub extern "C" fn sms_abi_version() -> u32 {
    SMS_ABI_VERSION
}

/// Fixed simulation rate in Hz.
#[unsafe(no_mangle)]
pub extern "C" fn sms_tick_hz() -> u32 {
    libsms::TICK_HZ
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SmsVec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl From<Vec3> for SmsVec3 {
    fn from(v: Vec3) -> SmsVec3 {
        SmsVec3 {
            x: v.x,
            y: v.y,
            z: v.z,
        }
    }
}

impl From<SmsVec3> for Vec3 {
    fn from(v: SmsVec3) -> Vec3 {
        Vec3::new(v.x, v.y, v.z)
    }
}

/// [`libsms::GroundHit`].
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SmsGroundHit {
    pub height: f32,
    /// Unit normal, pointing up.
    pub normal: SmsVec3,
    /// [`libsms::SurfaceFlags`] bits.
    pub flags: u32,
}

/// [`libsms::RoofHit`].
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SmsRoofHit {
    pub height: f32,
    pub flags: u32,
}

/// [`libsms::WallHit`].
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SmsWallHit {
    /// Horizontal unit normal, pointing away from the wall.
    pub normal: SmsVec3,
    pub flags: u32,
}

/// Something that happened this step, for sound, particles and the like.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SmsEvent {
    /// [`SmsEventKind`] as a number.
    pub kind: u32,
    /// Kind-specific: the [`JumpKind`] index for jumps, 1 for strong landings and super
    /// pounds, damage for hits, the enemy's id for enemy and water hits.
    pub arg: i32,
    /// Kind-specific: the sound's info value, health left after a hit, how Mario hit an enemy
    /// (0 stomp, 1 ground pound, 2 dive or slide), a water particle's attack, the units a
    /// ground pound fell.
    pub value: f32,
    /// Where it happened (sounds, water); otherwise zero.
    pub pos: SmsVec3,
    /// The `MSD_SE_*` name for sounds, else null. Valid only during the callback.
    pub name: *const c_char,
}

/// Values of [`SmsEvent::kind`].
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmsEventKind {
    Other = 0,
    Jump = 1,
    Land = 2,
    GroundPound = 3,
    WallBump = 4,
    WaterEmit = 5,
    NozzleChanged = 6,
    TriggerFired = 7,
    Fell = 8,
    Damaged = 9,
    Down = 10,
    Sound = 11,
    /// Mario hit the enemy `arg` (`value`: 0 stomp, 1 ground pound, 2 dive or slide) from
    /// `pos`. Once per enemy per move.
    EnemyHit = 12,
    /// A water particle hit the enemy `arg` at `pos` with attack `value`, and was spent.
    WaterHit = 13,
    /// A ground pound began (its wind-up).
    PoundStart = 14,
    /// [`SmsLimits`]: FLUDD started in mid-air and spent a use; `arg` 1 if it was one of the
    /// host's extras, 0 the free one.
    AirNozzle = 15,
    /// [`SmsLimits`]: a mid-air jump was spent.
    AirJump = 16,
    /// [`SmsLimits`]: a belly slide turned wet and spent the charge.
    WetSlide = 17,
    /// As `WaterHit`, by a particle from a light press's burst of the Spray
    /// ([`libsms::WaterEmit::burst`]), which a host may make count for more.
    BurstHit = 18,
    /// Floor contact: value = drop in sim units; arg bits 1 dive, 2 ground pound,
    /// 4 cushioned (water/Yoshi/slow landing); pos = contact position.
    FallImpact = 19,
    /// Mario touched a Shine ([`sms_sim_shine_get`]): `pos` is the Shine, `arg` the yaw it
    /// turned him to. The Shine's pickup sound is a `Sound` event of its own.
    ShineTouch = 20,
    /// The Shine Get demo began: he has landed with the Shine over his head. `pos` is where he
    /// stands, `arg` his yaw: the host starts the camera, letters and fanfare.
    ShineDemo = 21,
    /// As `WaterHit`, by a particle from a spin's spray ([`libsms::WaterEmit::spin`]), which
    /// flies in every direction: a host may make it count for more.
    SpinHit = 22,
}

/// An enemy the host reports ([`libsms::Enemy`]): a standing cylinder.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SmsEnemy {
    /// The host's handle, passed back in [`SmsEventKind::EnemyHit`] / `WaterHit`.
    pub id: u32,
    /// Its feet.
    pub pos: SmsVec3,
    pub radius: f32,
    pub height: f32,
}

/// The host's world, as callbacks. Any of them may be null: no ground means a void, no roof
/// open sky, no walls nothing to bump into, no event sink drops events. `user` is passed back
/// unchanged.
#[repr(C)]
pub struct SmsHost {
    pub user: *mut c_void,
    /// Highest floor at or below `pos.y` (`TMap::checkGround`). Fill `out` and return 1, or
    /// return 0 for a void.
    pub ground: Option<unsafe extern "C" fn(*mut c_void, *const SmsVec3, *mut SmsGroundHit) -> u8>,
    /// Lowest ceiling at or above `pos.y` (`TMap::checkRoof`). Return 0 for open sky.
    pub roof: Option<unsafe extern "C" fn(*mut c_void, *const SmsVec3, *mut SmsRoofHit) -> u8>,
    /// Push a vertical cylinder at `center` with `radius` out of walls in XZ
    /// (`TMap::isTouchedWallsAndMoveXZ`): write the corrected center to `out_center`, up to
    /// `max` touched walls to `out_walls`, and return how many.
    pub walls: Option<
        unsafe extern "C" fn(
            *mut c_void,
            *const SmsVec3,
            f32,
            u32,
            *mut SmsVec3,
            *mut SmsWallHit,
        ) -> u32,
    >,
    pub event: Option<unsafe extern "C" fn(*mut c_void, *const SmsEvent)>,
    /// Enemies whose cylinders come within `radius` of `pos`: write up to `max` to `out` and
    /// return how many (more than strictly in reach is fine; the sim tests the cylinders).
    /// Mario stomps, pounds and is pushed by these, and FLUDD's water hits them.
    pub enemies:
        Option<unsafe extern "C" fn(*mut c_void, *const SmsVec3, f32, *mut SmsEnemy, u32) -> u32>,
    /// Shallow water at `pos`: write its surface's height to `out` and return 1, or return 0
    /// for none (`TPoolManager::getWaterLevel`). While it's above his feet Mario is in the
    /// water, and R fills FLUDD's tank from it.
    pub water: Option<unsafe extern "C" fn(*mut c_void, *const SmsVec3, *mut f32) -> u8>,
}

/// One frame of controller input.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SmsPad {
    /// Main stick on the unit disc, +x right, +y away from the camera. Ignored when
    /// `use_move` is set.
    pub stick_x: f32,
    pub stick_y: f32,
    /// World-space move direction (XZ, length 0..1), for hosts whose input is already camera
    /// relative. Used when `use_move` is nonzero: the sim turns it back into a stick.
    pub move_x: f32,
    pub move_z: f32,
    pub use_move: u8,
    /// Jump (GameCube A).
    pub a: u8,
    /// Dive / punch (B).
    pub b: u8,
    /// Ground pound in the air, crouch with FLUDD (L, fully clicked).
    pub l: u8,
    /// R fully clicked.
    pub r: u8,
    /// Switch nozzle (X).
    pub x: u8,
    /// Trigger analog values, 0..=255. R drives the spray.
    pub l_analog: u8,
    pub r_analog: u8,
}

impl SmsPad {
    fn to_pad(self, camera_yaw: math::Angle) -> Pad {
        let stick = if self.use_move != 0 {
            stick_for_move(self.move_x, self.move_z, camera_yaw)
        } else {
            Vec2::new(self.stick_x, self.stick_y)
        };
        Pad {
            stick,
            a: self.a != 0,
            b: self.b != 0,
            l: self.l != 0,
            r: self.r != 0,
            l_analog: self.l_analog,
            r_analog: self.r_analog,
            x: self.x != 0,
        }
    }
}

/// The stick that makes Mario want to go along world `(x, z)`: the inverse of
/// [`libsms::input::stick_intent`]'s `yaw = matan(-v, h) + camera_yaw`.
fn stick_for_move(x: f32, z: f32, camera_yaw: math::Angle) -> Vec2 {
    let len = (x * x + z * z).sqrt();
    if len < 1.0e-4 {
        return Vec2::ZERO;
    }
    let rel = x.atan2(z) - camera_yaw as f32 * std::f32::consts::PI / 32768.0;
    Vec2::new(rel.sin(), -rel.cos()) * len.min(1.0)
}

/// What the host reads back after a step.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SmsState {
    /// Feet.
    pub pos: SmsVec3,
    /// Units per 30 Hz frame (`mVel`).
    pub vel: SmsVec3,
    pub forward_vel: f32,
    pub anim_frame: f32,
    /// FLUDD's tank, 0..1 (0 without FLUDD).
    pub water: f32,
    /// Fraction of the way to the next tick, for render interpolation.
    pub alpha: f32,
    pub face_yaw: i16,
    /// Yaw to draw the model at.
    pub model_yaw: i16,
    pub health: i16,
    pub on_ground: u8,
    pub has_fludd: u8,
    /// 1 falling in a ground pound, 2 in a super pound (the game draws flames), else 0.
    pub pounding: u8,
    /// 1 standing in shallow water (the host's `water`).
    pub in_water: u8,
    /// 1 in the Shine Get pose and still falling to the floor, 2 holding the Shine up, else 0.
    pub win_demo: u8,
    /// The fitted nozzle: 0 without FLUDD, else `NozzleKind` + 1 (1 Spray, 2 Rocket, 5 Hover, 6 Turbo).
    pub nozzle: u8,
}

/// Host tuning that departs from the game on purpose ([`libsms::Options`]); zeroes and ones
/// are the game.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SmsTuning {
    /// Jump heights times this (1: the game's).
    pub jump_height: f32,
    /// Seconds an A press in the air is remembered for the landing (0: the game's, none).
    pub jump_buffer: f32,
    /// Seconds past the landing window in which a jump still chains (0: the game's).
    pub chain_grace: f32,
    /// Ground covered per tick times this (speed buffs; 1: the game's).
    pub move_speed: f32,
    /// The Spray nozzle's water reaches about this many times as far (1: the game's).
    pub spray_reach: f32,
    /// Stomp bounces times this high, on top of `jump_height` (1: the game's).
    pub stomp_bounce: f32,
    /// The Rocket nozzle flies about this many times as high (1: the game's).
    pub rocket_height: f32,
    /// A on a ground pound's landing leaps back up this share of the fall, at least a triple
    /// jump (0: the game's, no pound jump).
    pub pound_jump: f32,
    /// A timed within a few ticks of a stomp makes the bounce this many times as high (0 or 1:
    /// the game's).
    pub stomp_jump: f32,
    /// A tapped jump still rises this share of a full one before letting go of A cuts it
    /// (0: the game's).
    pub short_hop: f32,
    /// The belly slide's spray is this many times as big (0 or 1: the game's).
    pub slide_fx_scale: f32,
    /// Running speeds up and brakes this many times as quickly (a host's acceleration stat;
    /// below 1 the ground is slippery; 0 or 1: the game's).
    pub traction: f32,
    /// 1: enemies are solid to Mario (`keepDistance`); 0: the game's (most aren't).
    pub enemy_push: u8,
    /// 1: X steps through Spray, Hover, Rocket and Turbo; 0: the game's (Spray and one).
    pub cycle_nozzles: u8,
    /// 1: FLUDD's tank never runs dry; 0: the game's.
    pub unlimited_water: u8,
    /// 1: the Spray holds a steady output set by the trigger's depth; 0: the game's (each
    /// press a burst that fades, `Options::spray_pressure_fades`).
    pub spray_steady: u8,
    /// 1: standing in the host's shallow water wets the ground as FLUDD's water does, so a belly
    /// slide there glides wet; 0: the game's (only FLUDD's water does).
    pub shallow_wet: u8,
    /// 1: a wet belly slide also throws the old stand-in splashes, on top of the game's own
    /// spray effect; 0: only the game's.
    pub slide_splashes: u8,
    /// 1: after a knock-down's slide, A, the stick, B, L or R gets Mario up at once; 0: the
    /// game's (the whole get-up clip).
    pub knockdown_cancel: u8,
}

/// A host's own limits on FLUDD and the slide ([`libsms::Limits`]), for games that bill them
/// as abilities with charges and cooldowns. Not the game's: all zeroes is the game (no limits;
/// factors of 0 read as 1). Send it before each step: the sim counts `air_extra` and
/// `wet_slide_ready` down as it spends them (with [`SmsEventKind::AirNozzle`] and
/// [`SmsEventKind::WetSlide`]), and the host hands back what it has left.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SmsLimits {
    /// The Spray fires this many times as many drops, each taking that much less water.
    pub spray_rate: f32,
    /// Seconds the Spray fires before R must be let go (0: no limit).
    pub spray_seconds: f32,
    /// The Hover's burst lasts this many times as long.
    pub hover_time: f32,
    /// The Turbo's dash covers this many times as much ground.
    pub turbo_speed: f32,
    /// Seconds a wet slide glides before turning plain (0: as long as it lasts).
    pub wet_slide_seconds: f32,
    /// A Spray press lasts this many times as long before it fades (0 or 1: the game's).
    pub spray_fade: f32,
    /// Mid-air FLUDD uses on hand beyond the free one ([`Self::air_uses`]).
    pub air_extra: u32,
    /// Jumps in mid-air, each the next of the double/triple chain; back on landing.
    pub air_jumps: u32,
    /// 1: each start of a nozzle in mid-air spends a use: one free per time off the ground,
    /// then `air_extra`. 0: the game's (the tank is the only limit).
    pub air_uses: u8,
    /// 0: the game's (a belly slide over FLUDD's water glides); 1: that, only with a charge;
    /// 2: any belly slide glides, with a charge.
    pub wet_slide: u8,
    /// A wet slide charge is ready.
    pub wet_slide_ready: u8,
    /// Bit 0: a wet slide keeps its speed as if the stick were held forward. Bit 1: a slide
    /// over FLUDD's water glides for free (the game's), the charge only for dry ones.
    pub wet_slide_flags: u8,
}

/// Whether this sim's own FLUDD water has wetted the ground at `pos` (sim units), for another
/// Mario's sim to glide over: 1 wet, 0 dry or no sim.
///
/// # Safety
/// `sim` from [`sms_sim_new`] or null; `pos` readable or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_sim_wet_at(sim: *const SmsSim, pos: *const SmsVec3) -> u32 {
    // SAFETY: the caller promised valid pointers.
    match (unsafe { sim.as_ref() }, unsafe { pos.as_ref() }) {
        (Some(s), Some(p)) if !s.poisoned => u32::from(s.water.on_ground_near(Vec3::new(p.x, p.y, p.z))),
        _ => 0,
    }
}

/// Tells this sim whether another Mario's wet ground is under it for the next step.
///
/// # Safety
/// `sim` from [`sms_sim_new`] or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_sim_set_foreign_wet(sim: *mut SmsSim, wet: u32) {
    // SAFETY: the caller promised a valid pointer or null.
    if let Some(s) = unsafe { sim.as_mut() } {
        s.foreign_wet = wet != 0;
    }
}

/// An opaque sim handle.
pub struct SmsSim {
    pub(crate) sim: Sim,
    /// FLUDD's water (`TModelWaterManager`), fed by the sim's emits and moved every 4 ticks.
    pub(crate) water: WaterManager,
    /// Ticks since the water last moved.
    pub(crate) water_ticks: u32,
    /// Also throw the host stand-in splashes during a wet belly slide
    /// ([`WaterManager::slide_splashes`]), on top of the game's own `ms_m_watslide_*` spray.
    pub(crate) slide_splashes: bool,
    /// Another Mario's wet ground is under this one (the host says so before each step), so a
    /// belly slide glides over what a friend's FLUDD wetted as it does over his own.
    pub(crate) foreign_wet: bool,
    status: CString,
    clip: CString,
    pub(crate) poisoned: bool,
}

/// The host's callbacks, plus this tick's events for the view's sounds.
struct FfiHost<'a>(&'a SmsHost, Vec<Event>);

impl Host for FfiHost<'_> {
    fn ground(&self, pos: Vec3) -> Option<GroundHit> {
        let f = self.0.ground?;
        let mut out = SmsGroundHit::default();
        // SAFETY: the host promised a valid callback; both pointers live for the call.
        let hit = unsafe { f(self.0.user, &SmsVec3::from(pos), &mut out) };
        (hit != 0).then(|| GroundHit {
            height: out.height,
            normal: Vec3::from(out.normal).try_normalize().unwrap_or(Vec3::Y),
            flags: SurfaceFlags(out.flags),
        })
    }

    fn roof(&self, pos: Vec3) -> Option<RoofHit> {
        let f = self.0.roof?;
        let mut out = SmsRoofHit::default();
        // SAFETY: as above.
        let hit = unsafe { f(self.0.user, &SmsVec3::from(pos), &mut out) };
        (hit != 0).then_some(RoofHit {
            height: out.height,
            flags: SurfaceFlags(out.flags),
        })
    }

    fn walls(&self, center: Vec3, radius: f32, max: usize, out: &mut Vec<WallHit>) -> Vec3 {
        let Some(f) = self.0.walls else {
            return center;
        };
        let mut buf = [SmsWallHit::default(); 8];
        let max = max.min(buf.len());
        let mut moved = SmsVec3::from(center);
        // SAFETY: as above; `buf` has room for `max` walls.
        let n = unsafe {
            f(
                self.0.user,
                &SmsVec3::from(center),
                radius,
                max as u32,
                &mut moved,
                buf.as_mut_ptr(),
            )
        } as usize;
        for w in &buf[..n.min(max)] {
            let normal = Vec3::new(w.normal.x, 0.0, w.normal.z);
            if let Some(normal) = normal.try_normalize() {
                out.push(WallHit {
                    normal,
                    flags: SurfaceFlags(w.flags),
                });
            }
        }
        Vec3::new(moved.x, center.y, moved.z)
    }

    fn enemies(&self, pos: Vec3, radius: f32, out: &mut Vec<Enemy>) {
        let Some(f) = self.0.enemies else {
            return;
        };
        let mut buf = [SmsEnemy::default(); 32];
        // SAFETY: as above; `buf` has room for its length.
        let n = unsafe {
            f(
                self.0.user,
                &SmsVec3::from(pos),
                radius,
                buf.as_mut_ptr(),
                buf.len() as u32,
            )
        } as usize;
        out.extend(buf[..n.min(buf.len())].iter().map(|e| Enemy {
            id: e.id as u64,
            pos: e.pos.into(),
            radius: e.radius,
            height: e.height,
        }));
    }

    fn water_level(&self, pos: Vec3) -> Option<f32> {
        let f = self.0.water?;
        let mut level = 0.0;
        // SAFETY: as above.
        let hit = unsafe { f(self.0.user, &SmsVec3::from(pos), &mut level) };
        (hit != 0 && level.is_finite()).then_some(level)
    }

    fn event(&mut self, event: Event) {
        self.1.push(event);
        let Some(f) = self.0.event else {
            return;
        };
        let mut e = SmsEvent {
            kind: SmsEventKind::Other as u32,
            arg: 0,
            value: 0.0,
            pos: SmsVec3::default(),
            name: std::ptr::null(),
        };
        let mut name = None;
        match event {
            Event::Jump(k) => {
                e.kind = SmsEventKind::Jump as u32;
                e.arg = jump_index(k);
            }
            Event::Land { strong } => {
                e.kind = SmsEventKind::Land as u32;
                e.arg = strong as i32;
            }
            Event::FallImpact {
                drop,
                pos,
                diving,
                ground_pound,
                cushioned,
            } => {
                e.kind = SmsEventKind::FallImpact as u32;
                e.value = drop;
                e.pos = pos.into();
                e.arg = diving as i32 | ((ground_pound as i32) << 1) | ((cushioned as i32) << 2);
            }
            Event::GroundPound { super_, drop } => {
                e.kind = SmsEventKind::GroundPound as u32;
                e.arg = super_ as i32;
                e.value = drop;
            }
            Event::WallBump => e.kind = SmsEventKind::WallBump as u32,
            Event::WaterEmit(w) => {
                e.kind = SmsEventKind::WaterEmit as u32;
                e.arg = w.count as i32;
                e.value = w.power;
                e.pos = w.pos.into();
            }
            Event::NozzleChanged(_) => e.kind = SmsEventKind::NozzleChanged as u32,
            Event::TriggerFired(_) => e.kind = SmsEventKind::TriggerFired as u32,
            Event::Fell => e.kind = SmsEventKind::Fell as u32,
            Event::Damaged { damage, health, .. } => {
                e.kind = SmsEventKind::Damaged as u32;
                e.arg = damage as i32;
                e.value = health as f32;
            }
            Event::Down => e.kind = SmsEventKind::Down as u32,
            Event::EnemyHit { id, kind } => {
                e.kind = SmsEventKind::EnemyHit as u32;
                e.arg = id as i32;
                e.value = match kind {
                    EnemyHitKind::Trample => 0.0,
                    EnemyHitKind::HipDrop => 1.0,
                    EnemyHitKind::Punch => 2.0,
                };
            }
            Event::PoundStart => e.kind = SmsEventKind::PoundStart as u32,
            Event::AirNozzle { extra } => {
                e.kind = SmsEventKind::AirNozzle as u32;
                e.arg = extra as i32;
            }
            Event::AirJump => e.kind = SmsEventKind::AirJump as u32,
            Event::WetSlide => e.kind = SmsEventKind::WetSlide as u32,
            Event::ShineTouch { pos, yaw } => {
                e.kind = SmsEventKind::ShineTouch as u32;
                e.arg = yaw as i32;
                e.pos = pos.into();
            }
            Event::ShineDemo { pos, yaw } => {
                e.kind = SmsEventKind::ShineDemo as u32;
                e.arg = yaw as i32;
                e.pos = pos.into();
            }
            Event::Sound(s) => {
                e.kind = SmsEventKind::Sound as u32;
                e.value = s.info.unwrap_or(0.0);
                e.pos = s.pos.into();
                name = CString::new(s.name).ok();
            }
            _ => {}
        }
        if let Some(n) = &name {
            e.name = n.as_ptr();
        }
        // SAFETY: as above; `name` outlives the call.
        unsafe { f(self.0.user, &e) };
    }
}

/// The water's enemy hits, to the host's event callback.
fn water_hits(host: &SmsHost, events: &[WaterEvent]) {
    let Some(f) = host.event else {
        return;
    };
    for ev in events {
        if let WaterEvent::Hit {
            id,
            pos,
            attack,
            burst,
            spin,
        } = *ev
        {
            let kind = if burst {
                SmsEventKind::BurstHit
            } else if spin {
                SmsEventKind::SpinHit
            } else {
                SmsEventKind::WaterHit
            };
            let e = SmsEvent {
                kind: kind as u32,
                arg: id as i32,
                value: attack as f32,
                pos: pos.into(),
                name: std::ptr::null(),
            };
            // SAFETY: the caller of `sms_sim_step` promised a valid callback.
            unsafe { f(host.user, &e) };
        }
    }
}

fn jump_index(k: JumpKind) -> i32 {
    match k {
        JumpKind::Normal => 0,
        JumpKind::Second => 1,
        JumpKind::Triple => 2,
        JumpKind::Back => 3,
        JumpKind::Long => 4,
        JumpKind::UTurn => 5,
        JumpKind::Wall => 6,
        JumpKind::Spin => 7,
        JumpKind::Dive => 8,
        JumpKind::Forced => 9,
    }
}

/// Creates a Mario with FLUDD at `*pos` facing `yaw`, tuned and animated from the extracted
/// disc files in `assets_dir` (UTF-8, the folder holding `params_mario/`). Returns null if that
/// folder has no `params_mario/`. Free with [`sms_sim_free`].
///
/// # Safety
/// `assets_dir` must be a valid nul-terminated string and `pos` readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_sim_new(
    assets_dir: *const c_char,
    pos: *const SmsVec3,
    yaw: i16,
) -> *mut SmsSim {
    // SAFETY: the caller promised a readable pointer.
    let Some(pos) = (unsafe { pos.as_ref() }).copied() else {
        return std::ptr::null_mut();
    };
    if assets_dir.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: the caller promised a nul-terminated string.
    let dir = unsafe { CStr::from_ptr(assets_dir) }
        .to_string_lossy()
        .into_owned();
    let made = catch_unwind(|| {
        let dir = Path::new(&dir);
        if !dir.join("params_mario").is_dir() {
            return None;
        }
        let a = Assets::load(dir);
        let mut sim = Sim::new(a.params, a.anim_table, pos.into(), yaw);
        sim.mario.give_fludd();
        Some(sim)
    });
    match made {
        Ok(Some(sim)) => Box::into_raw(Box::new(SmsSim {
            water: WaterManager::new(&sim.mario.params),
            water_ticks: 0,
            slide_splashes: false,
            foreign_wet: false,
            sim,
            status: CString::default(),
            clip: CString::default(),
            poisoned: false,
        })),
        _ => std::ptr::null_mut(),
    }
}

/// # Safety
/// `sim` must come from [`sms_sim_new`] and not be used afterwards. Null is ignored.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_sim_free(sim: *mut SmsSim) {
    if !sim.is_null() {
        // SAFETY: the caller hands back ownership.
        drop(unsafe { Box::from_raw(sim) });
    }
}

/// Advances by `dt` seconds of host time: as many 120 Hz ticks as are due, each with `pad`.
/// `camera_yaw` is the facing angle from Mario toward the camera. FLUDD's water moves too,
/// through the same callbacks. With a `view` (may be null), its sounds follow each tick's
/// animation and events, and its splashes the water's. Returns the ticks run.
///
/// # Safety
/// `sim` from [`sms_sim_new`]; `host` and `pad` valid for the call; `view` null or from
/// `sms_view_new`; the host's callbacks must be safe to call with their documented arguments.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_sim_step(
    sim: *mut SmsSim,
    host: *const SmsHost,
    pad: *const SmsPad,
    camera_yaw: i16,
    dt: f32,
    view: *mut SmsView,
) -> u32 {
    // SAFETY: the caller promised valid pointers.
    let (Some(s), Some(host), Some(pad)) =
        (unsafe { sim.as_mut() }, unsafe { host.as_ref() }, unsafe {
            pad.as_ref()
        })
    else {
        return 0;
    };
    if s.poisoned {
        return 0;
    }
    let pad = pad.to_pad(camera_yaw);
    let ran = catch_unwind(AssertUnwindSafe(|| {
        let mut h = FfiHost(host, Vec::new());
        // Water on the ground under Mario makes a belly slide glide.
        s.sim.mario.water_underfoot = s.foreign_wet || s.water.on_ground_near(s.sim.mario.pos);
        let mut emits: Vec<WaterEmit> = Vec::new();
        let n = s
            .sim
            .advance_observed(&mut h, &pad, camera_yaw, dt, |mario, h| {
                let events = std::mem::take(&mut h.1);
                emits.extend(events.iter().filter_map(|e| match e {
                    Event::WaterEmit(w) => Some(*w),
                    _ => None,
                }));
                // SAFETY: the caller promised `view` is null or live.
                unsafe {
                    view::with_view(view, (), |v| {
                        v.view.events(&events, mario);
                        if let Some(a) = v.audio.as_mut() {
                            a.events(&events, mario);
                            a.tick(mario);
                        }
                    });
                }
            });
        // `TModelWaterManager`: emitted every tick, moved once a 30 Hz frame.
        for e in &emits {
            s.water.emit(e);
        }
        s.water_ticks += n;
        while s.water_ticks >= 4 {
            s.water_ticks -= 4;
            s.water.step(&h, s.sim.mario.pos);
            if s.slide_splashes && s.sim.mario.wet_sliding() {
                s.water.slide_splashes(s.sim.mario.pos);
            }
            water_hits(host, &s.water.events);
            // SAFETY: as above.
            unsafe {
                view::with_view(view, (), |v| {
                    v.water.step(&s.water.events);
                    if let Some(a) = v.audio.as_mut() {
                        a.water(&s.water.events);
                    }
                });
            }
            s.water.events.clear();
        }
        n
    }));
    match ran {
        Ok(n) => n,
        Err(_) => {
            s.poisoned = true;
            0
        }
    }
}

/// Reads Mario's state into `out`.
///
/// # Safety
/// `sim` from [`sms_sim_new`]; `out` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_sim_state(sim: *const SmsSim, out: *mut SmsState) -> u8 {
    // SAFETY: the caller promised valid pointers.
    let (Some(s), Some(out)) = (unsafe { sim.as_ref() }, unsafe { out.as_mut() }) else {
        return 0;
    };
    let m = &s.sim.mario;
    *out = SmsState {
        pos: m.pos.into(),
        vel: m.vel.into(),
        forward_vel: m.forward_vel,
        anim_frame: m.anim.frame,
        water: m.fludd.as_ref().map_or(0.0, |f| f.level(&m.params)),
        alpha: s.sim.alpha(),
        face_yaw: m.face_yaw,
        model_yaw: m.model_yaw,
        health: m.health,
        on_ground: m.on_ground() as u8,
        has_fludd: m.fludd.is_some() as u8,
        pounding: m.pounding(),
        in_water: m.in_water() as u8,
        win_demo: if m.win_demo_posing() {
            2
        } else {
            m.is_win_demo() as u8
        },
        nozzle: m.fludd.as_ref().map_or(0, |f| f.nozzle as u8 + 1),
    };
    1
}

/// Mario's status name (`Wait`, `Jump`, `HipAttack`...), valid until the next call on `sim`.
///
/// # Safety
/// `sim` from [`sms_sim_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_sim_status_name(sim: *mut SmsSim) -> *const c_char {
    // SAFETY: the caller promised a valid pointer.
    let Some(s) = (unsafe { sim.as_mut() }) else {
        return std::ptr::null();
    };
    s.status = CString::new(format!("{:?}", s.sim.mario.status)).unwrap_or_default();
    s.status.as_ptr()
}

/// The body animation playing (`wait`, `run`, `jump`...), valid until the next call on `sim`.
///
/// # Safety
/// `sim` from [`sms_sim_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_sim_clip_name(sim: *mut SmsSim) -> *const c_char {
    // SAFETY: the caller promised a valid pointer.
    let Some(s) = (unsafe { sim.as_mut() }) else {
        return std::ptr::null();
    };
    s.clip = CString::new(s.sim.mario.anim.clip).unwrap_or_default();
    s.clip.as_ptr()
}

/// Moves Mario to `*pos` at rest, for when the host relocates him (teleports, respawns).
///
/// # Safety
/// `sim` from [`sms_sim_new`]; `pos` readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_sim_teleport(sim: *mut SmsSim, pos: *const SmsVec3) {
    // SAFETY: the caller promised valid pointers.
    if let (Some(s), Some(pos)) = (unsafe { sim.as_mut() }, unsafe { pos.as_ref() }) {
        s.sim.mario.teleport((*pos).into());
        s.water.particles.clear();
    }
}

/// Fills FLUDD's tank (a new level, a refill button); the HUD's full-tank chime plays on the
/// next step if it wasn't full.
///
/// # Safety
/// `sim` from [`sms_sim_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_sim_refill(sim: *mut SmsSim) {
    // SAFETY: the caller promised a valid pointer.
    if let Some(s) = unsafe { sim.as_mut() } {
        s.sim.mario.refill_water();
    }
}

/// Sets the host's limits (see [`SmsLimits`]); a new sim has none, as the game.
///
/// # Safety
/// `sim` from [`sms_sim_new`]; `limits` readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_sim_set_limits(sim: *mut SmsSim, limits: *const SmsLimits) {
    // SAFETY: the caller promised valid pointers.
    let (Some(s), Some(l)) = (unsafe { sim.as_mut() }, unsafe { limits.as_ref() }) else {
        return;
    };
    let factor = |v: f32| {
        if v.is_finite() && v > 0.0 {
            v.clamp(0.1, 10.0)
        } else {
            1.0
        }
    };
    let ticks = |secs: f32| {
        if secs.is_finite() && secs > 0.0 {
            ((secs * TICK_HZ as f32).round() as u32).clamp(1, 1_000_000)
        } else {
            0
        }
    };
    s.sim.mario.limits = libsms::Limits {
        air_uses: l.air_uses != 0,
        air_extra: l.air_extra.min(1000),
        air_jumps: l.air_jumps.min(1000),
        spray_rate: factor(l.spray_rate),
        spray_ticks: ticks(l.spray_seconds),
        hover_time: factor(l.hover_time),
        spray_fade: factor(l.spray_fade),
        turbo_speed: factor(l.turbo_speed),
        wet_slide: (l.wet_slide != 0).then_some(libsms::WetSlide {
            ready: l.wet_slide_ready != 0,
            anywhere: l.wet_slide == 2,
            max_ticks: ticks(l.wet_slide_seconds),
            glide: l.wet_slide_flags & 1 != 0,
            water_free: l.wet_slide_flags & 2 != 0,
        }),
    };
}

/// The host's camera as the game's L-button camera (`isLButtonCamera`): while `on`, FLUDD's
/// nozzle points at `pitch` (radians, up positive) and a crouching Mario turns to `yaw`
/// (facing angle units). Send before each step; 0 turns it off.
///
/// # Safety
/// `sim` from [`sms_sim_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_sim_set_aim(sim: *mut SmsSim, on: u8, yaw: i16, pitch: f32) {
    // SAFETY: the caller promised a valid pointer.
    if let Some(s) = unsafe { sim.as_mut() } {
        s.sim.mario.aim = (on != 0 && pitch.is_finite()).then(|| libsms::Aim {
            yaw,
            pitch: (pitch.clamp(-1.5, 1.5) * (32768.0 / std::f32::consts::PI)) as i16,
        });
    }
}

/// The host's health ran out: Mario falls down for good (`loserExec`: the "die" clip and his
/// voices) and lies there until the host starts him again.
///
/// # Safety
/// `sim` from [`sms_sim_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_sim_down(sim: *mut SmsSim) {
    // SAFETY: the caller promised a valid pointer.
    if let Some(s) = unsafe { sim.as_mut() } {
        s.sim.mario.knock_out();
    }
}

/// Adds `share` of a full tank to FLUDD's (a host's own water: healing, pickups).
///
/// # Safety
/// `sim` from [`sms_sim_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_sim_add_water(sim: *mut SmsSim, share: f32) {
    // SAFETY: the caller promised a valid pointer.
    if let Some(s) = unsafe { sim.as_mut() } {
        s.sim.mario.add_water(share);
    }
}

/// The host's world throws Mario: his velocity becomes `*vel` (units per 30 Hz frame, as
/// [`SmsState::vel`]) and he flies as from a spring until he lands (jump pads, knockback).
///
/// # Safety
/// `sim` from [`sms_sim_new`]; `vel` readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_sim_launch(sim: *mut SmsSim, vel: *const SmsVec3) {
    // SAFETY: the caller promised valid pointers.
    if let (Some(s), Some(v)) = (unsafe { sim.as_mut() }, unsafe { vel.as_ref() }) {
        s.sim.mario.launch((*v).into());
    }
}

/// A world jump-pad flight: mVel velocity, gravity in mVel per 120 Hz tick.
/// # Safety
/// `sim` from [`sms_sim_new`]; `vel` readable (null is ignored).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_sim_launch_pad(sim: *mut SmsSim, vel: *const SmsVec3, gravity: f32) {
    if let (Some(s), Some(v)) = (unsafe { sim.as_mut() }, unsafe { vel.as_ref() }) {
        s.sim.mario.launch_pad((*v).into(), gravity);
    }
}

/// A hit from `from` that knocks Mario down (`damageExec`'s fall, voice and flash for the
/// disc's damage kind `kind`, an index into `DamageKind::ALL`; 0 is the common enemy) without
/// touching his sim health: the host keeps the health. Not while he flashes from the last, nor
/// in a dive. Returns 1 if it landed.
///
/// # Safety
/// `sim` from [`sms_sim_new`]; `from` readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_sim_knock_down(
    sim: *mut SmsSim,
    from: *const SmsVec3,
    kind: u8,
) -> u8 {
    // SAFETY: the caller promised valid pointers.
    if let (Some(s), Some(f)) = (unsafe { sim.as_mut() }, unsafe { from.as_ref() }) {
        let kind = libsms::DamageKind::ALL
            .get(kind as usize)
            .copied()
            .unwrap_or(libsms::DamageKind::EnemyCommon);
        return s.sim.mario.knock_down((*f).into(), kind) as u8;
    }
    0
}

/// Boosts the jump Mario is in: horizontal travel times `factor` until he lands (a host's
/// sprint-jump item). Ignored on the ground.
///
/// # Safety
/// `sim` from [`sms_sim_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_sim_boost_jump(sim: *mut SmsSim, factor: f32) {
    // SAFETY: the caller promised a valid pointer.
    if let Some(s) = unsafe { sim.as_mut() } {
        s.sim.mario.boost_jump(factor);
    }
}

/// Mario touches a Shine Sprite at `pos`, turned `yaw` (a 16-bit angle, 0x8000 a half turn): he
/// takes the win pose (see `libsms::shine`). Returns 1 if he took it, 0 if he already has one or
/// is down.
///
/// # Safety
/// `sim` from [`sms_sim_new`]; `pos` readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_sim_shine_get(sim: *mut SmsSim, pos: *const SmsVec3, yaw: i16) -> u8 {
    // SAFETY: the caller promised valid pointers.
    match (unsafe { sim.as_mut() }, unsafe { pos.as_ref() }) {
        (Some(s), Some(p)) => s.sim.mario.get_shine(Vec3::new(p.x, p.y, p.z), yaw) as u8,
        _ => 0,
    }
}

/// Lets Mario out of the Shine Get pose, for a host whose stage didn't end.
///
/// # Safety
/// `sim` from [`sms_sim_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_sim_shine_end(sim: *mut SmsSim) {
    // SAFETY: the caller promised a valid pointer.
    if let Some(s) = unsafe { sim.as_mut() } {
        s.sim.mario.end_win_demo();
    }
}

/// Sets host tuning (see [`SmsTuning`]); a new sim has the game's.
///
/// # Safety
/// `sim` from [`sms_sim_new`]; `tuning` readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_sim_set_tuning(sim: *mut SmsSim, tuning: *const SmsTuning) {
    // SAFETY: the caller promised valid pointers.
    if let (Some(s), Some(t)) = (unsafe { sim.as_mut() }, unsafe { tuning.as_ref() }) {
        let ticks = |secs: f32| (secs.max(0.0) * TICK_HZ as f32).round().min(1.0e4) as u32;
        let o = &mut s.sim.mario.options;
        o.jump_height = if t.jump_height.is_finite() {
            t.jump_height.clamp(0.1, 10.0)
        } else {
            1.0
        };
        o.jump_buffer_ticks = ticks(t.jump_buffer);
        o.chain_grace_ticks = ticks(t.chain_grace);
        let factor = |v: f32| {
            if v.is_finite() {
                v.clamp(0.1, 10.0)
            } else {
                1.0
            }
        };
        o.move_speed = factor(t.move_speed);
        o.traction = if t.traction > 0.0 { factor(t.traction) } else { 1.0 };
        o.enemy_push = t.enemy_push != 0;
        o.cycle_nozzles = t.cycle_nozzles != 0;
        o.unlimited_water = t.unlimited_water != 0;
        o.spray_pressure_fades = t.spray_steady == 0;
        o.shallow_water_wets = t.shallow_wet != 0;
        o.knockdown_cancel = t.knockdown_cancel != 0;
        s.slide_splashes = t.slide_splashes != 0;
        o.slide_fx_scale = if t.slide_fx_scale.is_finite() && t.slide_fx_scale > 0.0 {
            t.slide_fx_scale.clamp(0.1, 5.0)
        } else {
            1.0
        };
        s.water.reach = factor(t.spray_reach);
        o.stomp_bounce = factor(t.stomp_bounce);
        // The rocket's apex grows about as its lift to the power 1.9 (examples/report.rs:
        // x1.25 lift 1.52 times as high, x1.5 2.16, x2 3.76).
        o.rocket_thrust = factor(t.rocket_height).powf(1.0 / 1.9);
        o.pound_jump = if t.pound_jump.is_finite() {
            t.pound_jump.clamp(0.0, 2.0)
        } else {
            0.0
        };
        o.stomp_jump = if t.stomp_jump.is_finite() {
            t.stomp_jump.clamp(1.0, 10.0)
        } else {
            1.0
        };
        o.short_hop = if t.short_hop.is_finite() {
            t.short_hop.clamp(0.0, 1.0)
        } else {
            0.0
        };
    }
}

/// Sizes of the ABI's structs, for hosts to check their copies against: `SmsVec3`,
/// `SmsGroundHit`, `SmsRoofHit`, `SmsWallHit`, `SmsEvent`, `SmsHost`, `SmsPad`, `SmsState`,
/// `SmsModelInfo`, `SmsBatch`, `SmsTexture`, `SmsDraw`, `SmsViewFrame`, `SmsTuning`,
/// `SmsWaterVertex`, `SmsWaterInfo`, `SmsEnemy`, `SmsFxVertex`, `SmsFxBatch`, `SmsFxInfo`,
/// `SmsHudInfo`, `SmsLimits`, `SmsScreenFx`, `SmsScreenSprite`. Writes up to `max`;
/// returns how many there are.
///
/// # Safety
/// `out` null or writable for `max`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_struct_sizes(out: *mut u32, max: u32) -> u32 {
    use std::mem::size_of;
    let sizes = [
        size_of::<SmsVec3>(),
        size_of::<SmsGroundHit>(),
        size_of::<SmsRoofHit>(),
        size_of::<SmsWallHit>(),
        size_of::<SmsEvent>(),
        size_of::<SmsHost>(),
        size_of::<SmsPad>(),
        size_of::<SmsState>(),
        size_of::<view::SmsModelInfo>(),
        size_of::<view::SmsBatch>(),
        size_of::<view::SmsTexture>(),
        size_of::<view::SmsDraw>(),
        size_of::<view::SmsViewFrame>(),
        size_of::<SmsTuning>(),
        size_of::<view::SmsWaterVertex>(),
        size_of::<view::SmsWaterInfo>(),
        size_of::<SmsEnemy>(),
        size_of::<view::SmsFxVertex>(),
        size_of::<view::SmsFxBatch>(),
        size_of::<view::SmsFxInfo>(),
        size_of::<view::SmsHudInfo>(),
        size_of::<SmsLimits>(),
        size_of::<view::SmsScreenFx>(),
        size_of::<view::SmsScreenSprite>(),
        size_of::<view::SmsShineFrame>(),
        size_of::<view::SmsShineInfo>(),
        size_of::<view::SmsCameraShot>(),
    ];
    if !out.is_null() {
        for (i, s) in sizes.iter().take(max as usize).enumerate() {
            // SAFETY: as documented.
            unsafe { out.add(i).write(*s as u32) };
        }
    }
    sizes.len() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    unsafe extern "C" fn flat(_: *mut c_void, _: *const SmsVec3, out: *mut SmsGroundHit) -> u8 {
        // SAFETY: the sim passes a valid out pointer.
        unsafe {
            *out = SmsGroundHit {
                height: 0.0,
                normal: SmsVec3 {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                },
                flags: 0,
            }
        };
        1
    }

    unsafe extern "C" fn count(user: *mut c_void, e: *const SmsEvent) {
        // SAFETY: `user` is the test's counter; `e` is valid for the call.
        unsafe {
            if (*e).kind == SmsEventKind::Jump as u32 {
                *(user as *mut u32) += 1;
            }
        }
    }

    #[test]
    fn fall_impacts_preserve_drop_position_and_flags_through_the_c_abi() {
        unsafe extern "C" fn capture(user: *mut c_void, e: *const SmsEvent) {
            // SAFETY: the test passes a live SmsEvent output and FfiHost calls synchronously.
            unsafe {
                *(user as *mut SmsEvent) = *e;
            }
        }
        let mut received = SmsEvent {
            kind: 0,
            arg: 0,
            value: 0.0,
            pos: SmsVec3::default(),
            name: std::ptr::null(),
        };
        let host = SmsHost {
            user: &mut received as *mut SmsEvent as *mut c_void,
            ground: None,
            roof: None,
            walls: None,
            event: Some(capture),
            enemies: None,
            water: None,
        };
        let mut adapter = FfiHost(&host, Vec::new());
        for flags in 0..8 {
            adapter.event(Event::FallImpact {
                drop: 16000.0,
                pos: Vec3::new(12.0, 34.0, 56.0),
                diving: flags & 1 != 0,
                ground_pound: flags & 2 != 0,
                cushioned: flags & 4 != 0,
            });
            assert_eq!(received.kind, 19);
            assert_eq!(received.arg, flags);
            assert_eq!(received.value, 16000.0);
            assert_eq!(
                (received.pos.x, received.pos.y, received.pos.z),
                (12.0, 34.0, 56.0)
            );
        }
    }

    #[test]
    fn move_input_turns_into_the_matching_stick() {
        // Away from a camera behind Mario (camera at -Z, so its yaw points along -Z) is stick
        // up; to the camera's right is stick right.
        let cam = i16::MIN;
        let up = stick_for_move(0.0, 1.0, cam);
        assert!((up - Vec2::new(0.0, 1.0)).length() < 1.0e-3, "{up}");
        let cfg = libsms::params::Controller::default();
        for (x, z) in [(1.0, 0.0), (0.0, -1.0), (0.6, 0.8), (-0.8, 0.6)] {
            let s = stick_for_move(x, z, 0x1234);
            let (_, yaw) = libsms::input::stick_intent(s, 0x1234, 0, &cfg);
            let want = math::matan(z, x);
            // The per-axis dead zone bends the direction by up to about 3 degrees.
            assert!(
                math::diff(yaw, want).unsigned_abs() < 600,
                "{x},{z}: {yaw} vs {want}"
            );
        }
    }

    #[test]
    fn a_sim_runs_through_the_c_abi() {
        let Some(dir) = libsms::assets::find(None) else {
            eprintln!("skipping: no assets/");
            return;
        };
        let dir = CString::new(dir.to_string_lossy().as_bytes()).unwrap();
        let mut jumps = 0u32;
        let host = SmsHost {
            user: &mut jumps as *mut u32 as *mut c_void,
            ground: Some(flat),
            roof: None,
            walls: None,
            event: Some(count),
            enemies: None,
            water: None,
        };
        unsafe {
            let sim = sms_sim_new(dir.as_ptr(), &SmsVec3::default(), 0);
            assert!(!sim.is_null());
            let run = SmsPad {
                move_x: 1.0,
                use_move: 1,
                ..SmsPad::default()
            };
            for _ in 0..60 {
                sms_sim_step(sim, &host, &run, 0, 1.0 / 60.0, std::ptr::null_mut());
            }
            let mut st = SmsState::default();
            assert_eq!(sms_sim_state(sim, &mut st), 1);
            assert!(st.pos.x > 100.0, "ran along +X: {:?}", st.pos);
            assert!(st.pos.z.abs() < 0.2 * st.pos.x, "{:?}", st.pos);
            assert_eq!(st.has_fludd, 1);
            let jump = SmsPad { a: 1, ..run };
            for _ in 0..3 {
                sms_sim_step(sim, &host, &jump, 0, 1.0 / 60.0, std::ptr::null_mut());
            }
            sms_sim_state(sim, &mut st);
            assert!(st.pos.y > 0.0 && st.on_ground == 0, "{:?}", st);
            assert_eq!(jumps, 1);
            let name = CStr::from_ptr(sms_sim_status_name(sim)).to_str().unwrap();
            assert!(name.contains("Jump"), "{name}");
            sms_sim_teleport(sim, &SmsVec3::default());
            let launch = SmsVec3 {
                x: 160.0,
                y: 100.0,
                z: 0.0,
            };
            sms_sim_launch_pad(sim, &launch, 0.5);
            sms_sim_launch_pad(sim, std::ptr::null(), 0.5);
            sms_sim_launch_pad(sim, &launch, f32::NAN);
            sms_sim_launch_pad(std::ptr::null_mut(), &launch, 0.5);
            sms_sim_step(
                sim,
                &host,
                &SmsPad::default(),
                0,
                1.0 / 60.0,
                std::ptr::null_mut(),
            );
            sms_sim_state(sim, &mut st);
            assert!((st.pos.x - 80.0).abs() < 0.01, "{:?}", st.pos);
            assert!((st.pos.y - 49.75).abs() < 0.01, "{:?}", st.pos);
            sms_sim_free(sim);
        }
    }

    /// One enemy, id 10, at Mario's feet (`user`'s third counter 0) or 4 m in front of him (1).
    unsafe extern "C" fn one_enemy(
        user: *mut c_void,
        _: *const SmsVec3,
        _: f32,
        out: *mut SmsEnemy,
        max: u32,
    ) -> u32 {
        if max == 0 {
            return 0;
        }
        // SAFETY: `user` is the test's counters; the sim gave room for `max`.
        unsafe {
            let ahead = (*(user as *const [u32; 3]))[2] as f32;
            out.write(SmsEnemy {
                id: 10,
                pos: SmsVec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 400.0 * ahead,
                },
                radius: 60.0,
                height: 200.0,
            });
        }
        1
    }

    unsafe extern "C" fn count_hits(user: *mut c_void, e: *const SmsEvent) {
        // SAFETY: `user` is the test's counters; `e` is valid for the call.
        unsafe {
            let c = &mut *(user as *mut [u32; 3]);
            if (*e).arg == 10 && (*e).kind == SmsEventKind::EnemyHit as u32 {
                c[0] += 1;
            }
            if (*e).arg == 10 && (*e).kind == SmsEventKind::WaterHit as u32 {
                c[1] += 1;
            }
        }
    }

    #[test]
    fn enemies_are_stomped_and_sprayed_through_the_c_abi() {
        let Some(dir) = libsms::assets::find(None) else {
            eprintln!("skipping: no assets/");
            return;
        };
        let dir = CString::new(dir.to_string_lossy().as_bytes()).unwrap();
        for ahead in [0, 1] {
            let mut c = [0u32, 0, ahead];
            let host = SmsHost {
                user: &mut c as *mut [u32; 3] as *mut c_void,
                ground: Some(flat),
                roof: None,
                walls: None,
                event: Some(count_hits),
                enemies: Some(one_enemy),
                water: None,
            };
            let pad = if ahead == 0 {
                SmsPad {
                    a: 1,
                    ..SmsPad::default()
                }
            } else {
                SmsPad {
                    r: 1,
                    r_analog: 255,
                    ..SmsPad::default()
                }
            };
            unsafe {
                let sim = sms_sim_new(dir.as_ptr(), &SmsVec3::default(), 0);
                for _ in 0..120 {
                    sms_sim_step(sim, &host, &pad, 0, 1.0 / 60.0, std::ptr::null_mut());
                }
                sms_sim_free(sim);
            }
            if ahead == 0 {
                assert!(c[0] >= 1 && c[1] == 0, "stomped: {c:?}");
            } else {
                assert!(c[0] == 0 && c[1] > 10, "sprayed: {c:?}");
            }
        }
    }

    #[test]
    fn struct_sizes_are_what_hosts_mirror() {
        let mut sizes = [0u32; 27];
        let n = unsafe { sms_struct_sizes(sizes.as_mut_ptr(), 27) } as usize;
        // ror2-mario's Native.cs and ror2-common's view structs check these at load.
        assert_eq!(
            &sizes[..n],
            &[
                12, 20, 8, 16, 32, 56, 24, 52, 40, 24, 12, 64, 48, 56, 24, 28, 24, 36, 16, 12, 32,
                36, 72, 40, 52, 16, 40
            ]
        );
    }

    #[test]
    fn the_shine_has_its_own_view_and_mario_takes_it_through_the_host_abi() {
        let Some(dir) = libsms::assets::find(None) else {
            return;
        };
        if !dir.join("scene/mapobj/shine.bmd").is_file() {
            return;
        }
        let dir = CString::new(dir.to_string_lossy().as_bytes()).unwrap();
        unsafe {
            let shine = view::sms_view_new_look(dir.as_ptr(), 0, 3);
            assert!(!shine.is_null());
            let mut info = view::SmsShineInfo::default();
            assert_eq!(view::sms_view_shine_info(shine, &mut info), 1);
            assert_eq!((info.float_frames, info.demo_frames), (79.0, 419.0));
            assert!(info.camera_outside_frames > 400.0);
            let mut shot = view::SmsCameraShot::default();
            assert_eq!(view::sms_view_camera_shot(shine, 0, 0.0, &mut shot), 1);
            assert!(shot.fov > 20.0 && shot.fov < 90.0);
            let frame = view::SmsShineFrame {
                dt: 1.0 / 30.0,
                clip: 1,
                frame: 40.0,
                scale: 1.0,
                right: SmsVec3 { x: 1.0, y: 0.0, z: 0.0 },
                up: SmsVec3 { x: 0.0, y: 1.0, z: 0.0 },
                ..Default::default()
            };
            view::sms_view_shine_update(shine, &frame);
            let mut count = 0;
            assert!(!view::sms_view_positions(shine, &mut count).is_null());
            assert!(count > 0);
            view::sms_view_free(shine);

            // Mario takes the pose through the sim's own calls.
            let at = SmsVec3 { x: 0.0, y: 0.0, z: 0.0 };
            let sim = sms_sim_new(dir.as_ptr(), &at, 0);
            assert!(!sim.is_null());
            assert_eq!(sms_sim_shine_get(sim, &at, 0x1000), 1);
            assert_eq!(sms_sim_shine_get(sim, &at, 0x1000), 0, "already in the pose");
            sms_sim_shine_end(sim);
            sms_sim_free(sim);
        }
    }

    #[test]
    fn a_yellow_coin_mesh_and_pickup_effects_are_available_through_the_host_abi() {
        let Some(dir) = libsms::assets::find(None) else {
            return;
        };
        let dir = CString::new(dir.to_string_lossy().as_bytes()).unwrap();
        unsafe {
            let coin = view::sms_view_new_look(dir.as_ptr(), 0, 2);
            assert!(!coin.is_null());
            assert_eq!(view::sms_view_model_count(coin), 1);
            let mut model = view::SmsModelInfo::default();
            assert_eq!(view::sms_view_model(coin, 0, &mut model), 1);
            // The original yellow coin uses its material colour without a texture.
            assert!(model.vertex_count > 0 && model.index_count > 0 && model.batch_count > 0);
            let mut count = 0;
            assert!(!view::sms_view_positions(coin, &mut count).is_null());
            assert_eq!(count, model.vertex_count);
            view::sms_view_free(coin);

            let mario = view::sms_view_new(dir.as_ptr(), 0);
            assert!(!mario.is_null());
            view::sms_view_coin_get(
                mario,
                &SmsVec3 {
                    x: 10.0,
                    y: 20.0,
                    z: 30.0,
                },
            );
            view::with_view(mario, (), |v| {
                v.view.fx.advance(0);
                v.view.fx.build(Vec3::X, Vec3::Y);
            });
            let mut fx = view::SmsFxInfo::default();
            view::sms_view_fx_info(mario, &mut fx);
            assert!(
                fx.vertices > 0,
                "coin pickup effects did not reach the host"
            );
            view::sms_view_free(mario);
        }
    }

    #[test]
    fn a_disc_light_burst_wets_six_circles_from_his_feet_forward() {
        let Some(dir) = libsms::assets::find(None) else {
            return;
        };
        let dir = CString::new(dir.to_string_lossy().as_bytes()).unwrap();
        let host = SmsHost {
            user: std::ptr::null_mut(),
            ground: Some(flat),
            roof: None,
            walls: None,
            event: None,
            enemies: None,
            water: None,
        };
        unsafe {
            let sim = sms_sim_new(dir.as_ptr(), &SmsVec3::default(), 0);
            assert!(!sim.is_null());
            let pad = SmsPad {
                r_analog: 15,
                ..SmsPad::default()
            };
            for _ in 0..180 {
                sms_sim_step(sim, &host, &pad, 0, 1.0 / 120.0, std::ptr::null_mut());
            }
            let s = &*sim;
            let mario = s.sim.mario.pos;
            let mut trail: Vec<f32> = s
                .water
                .wet_patches
                .iter()
                .filter(|p| (p.pos.x - mario.x).abs() < 5.0 && p.pos.z >= mario.z - 5.0)
                .map(|p| p.pos.z - mario.z)
                .collect();
            trail.sort_by(f32::total_cmp);
            assert!(trail.len() >= 6, "a light burst left {} circles ahead: {trail:?}", trail.len());
            assert!(trail[0] < 5.0, "no circle at his feet: {trail:?}");
            assert!(trail[5] > trail[0] + 100.0, "the trail doesn't reach forward: {trail:?}");
            let first = s.water.wet_patches.iter().find(|p| (p.pos.x - mario.x).abs() < 5.0).unwrap();
            assert!(s.water.on_ground_near(first.pos));
            sms_sim_free(sim);
        }
    }

    #[test]
    fn wet_slide_scene_waves_reach_the_host_with_sound_disabled() {
        let Some(dir) = libsms::assets::find(None) else {
            return;
        };
        let dir = CString::new(dir.to_string_lossy().as_bytes()).unwrap();
        unsafe extern "C" fn wet(
            _: *mut c_void,
            pos: *const SmsVec3,
            out: *mut SmsGroundHit,
        ) -> u8 {
            unsafe {
                flat(std::ptr::null_mut(), pos, out);
                (*out).flags = SurfaceFlags::WET;
            }
            1
        }
        let host = SmsHost {
            user: std::ptr::null_mut(),
            ground: Some(wet),
            roof: None,
            walls: None,
            event: None,
            enemies: None,
            water: None,
        };
        unsafe {
            let sim = sms_sim_new(dir.as_ptr(), &SmsVec3::default(), 0);
            let v = view::sms_view_new(dir.as_ptr(), 0);
            assert!(!sim.is_null() && !v.is_null());
            let frame = view::SmsViewFrame {
                dt: 1.0 / 30.0,
                forward: SmsVec3 {
                    x: 0.0,
                    y: -0.8,
                    z: -0.6,
                },
                up: SmsVec3 {
                    x: 0.0,
                    y: 0.6,
                    z: -0.8,
                },
                ..Default::default()
            };
            let mut found = false;
            for tick in 0..360 {
                let pad = SmsPad {
                    stick_y: 1.0,
                    a: (tick == 120) as u8,
                    b: (tick == 131) as u8,
                    ..Default::default()
                };
                sms_sim_step(sim, &host, &pad, 0, 1.0 / 120.0, v);
                if tick % 4 != 0 {
                    continue;
                }
                view::sms_view_update(v, sim, &frame);
                let mut count = 0;
                let ptr = view::sms_view_fx_batches(v, &mut count);
                if ptr.is_null() {
                    continue;
                }
                let batches = std::slice::from_raw_parts(ptr, count as usize);
                for b in batches.iter().filter(|b| b.blend == 2) {
                    assert!((*sim).sim.mario.wet_sliding());
                    assert!(b.vertex_count >= 8);
                    let mut info = view::SmsTexture::default();
                    view::sms_view_fx_texture(v, b.texture, &mut info, std::ptr::null_mut(), 0);
                    let mut pixels = vec![0; (info.width * info.height * 4) as usize];
                    assert_eq!(
                        view::sms_view_fx_texture(
                            v,
                            b.texture,
                            &mut info,
                            pixels.as_mut_ptr(),
                            pixels.len() as u32
                        ),
                        1
                    );
                    assert!(
                        pixels
                            .chunks_exact(4)
                            .any(|p| p[1].abs_diff(128) > 2 || p[3].abs_diff(128) > 2)
                    );
                    found = true;
                }
                if found {
                    break;
                }
            }
            assert!(
                found,
                "the slide's scene-refraction ribbon never reached the host"
            );
            view::sms_view_free(v);
            sms_sim_free(sim);
        }
    }

    #[test]
    fn a_view_poses_the_sim_through_the_c_abi() {
        let Some(dir) = libsms::assets::find(None) else {
            eprintln!("skipping: no assets/");
            return;
        };
        let dir = CString::new(dir.to_string_lossy().as_bytes()).unwrap();
        let host = SmsHost {
            user: std::ptr::null_mut(),
            ground: Some(flat),
            roof: None,
            walls: None,
            event: None,
            enemies: None,
            water: None,
        };
        unsafe {
            let sim = sms_sim_new(dir.as_ptr(), &SmsVec3::default(), 0);
            let v = view::sms_view_new(dir.as_ptr(), 0);
            assert!(!sim.is_null() && !v.is_null());
            let frame = view::SmsViewFrame {
                dt: 1.0 / 60.0,
                forward: SmsVec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0,
                },
                up: SmsVec3 {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                },
                volume: 1.0,
                ..Default::default()
            };
            for _ in 0..10 {
                sms_sim_step(sim, &host, &SmsPad::default(), 0, 1.0 / 60.0, v);
                view::sms_view_update(v, sim, &frame);
            }
            let n = view::sms_view_draws(v, std::ptr::null_mut(), 0);
            assert!(n >= 3, "{n} draws");
            // Running starts dust. It must reach the view with sound disabled, even when
            // several sim steps happen between renders (events aren't just the last tick).
            let run = SmsPad {
                stick_y: 1.0,
                ..SmsPad::default()
            };
            let mut most_fx = 0;
            for _ in 0..10 {
                sms_sim_step(sim, &host, &run, 0, 1.0 / 60.0, v);
                sms_sim_step(sim, &host, &run, 0, 1.0 / 60.0, v);
                view::sms_view_update(v, sim, &frame);
                let mut fx = view::SmsFxInfo::default();
                assert_eq!(view::sms_view_fx_info(v, &mut fx), 1);
                most_fx = most_fx.max(fx.vertices);
            }
            assert!(most_fx > 0, "running dust never reached the view");
            let mut fx = view::SmsFxInfo::default();
            view::sms_view_fx_info(v, &mut fx);
            let before = fx.vertices;
            view::sms_view_set_fx(v, 1, 0);
            view::sms_view_update(v, sim, &frame);
            view::sms_view_fx_info(v, &mut fx);
            assert_eq!(fx.vertices, before, "PoundFlames hid movement dust");
            view::sms_view_set_fx(v, 0, 0);
            view::sms_view_update(v, sim, &frame);
            view::sms_view_fx_info(v, &mut fx);
            assert_eq!(fx.vertices, 0, "disabled particles still drew");
            view::sms_view_set_fx(v, 1, 1);
            let mut draws = vec![view::SmsDraw::default(); n as usize];
            view::sms_view_draws(v, draws.as_mut_ptr(), n);
            let mut count = 0;
            let pos = view::sms_view_positions(v, &mut count);
            assert!(!pos.is_null() && count > 1000, "{count} vertices");
            let mut info = view::SmsModelInfo::default();
            assert_eq!(view::sms_view_model(v, 0, &mut info), 1);
            let mut batches = vec![view::SmsBatch::default(); info.batch_count as usize];
            assert_eq!(
                view::sms_view_batches(v, 0, batches.as_mut_ptr(), info.batch_count),
                info.batch_count
            );
            let t = batches[0].texture;
            assert!(t >= 0);
            let mut tex = view::SmsTexture::default();
            assert_eq!(
                view::sms_view_texture(v, t as u32, &mut tex, std::ptr::null_mut(), 0),
                1
            );
            let mut px = vec![0u8; (tex.width * tex.height * 4) as usize];
            assert_eq!(
                view::sms_view_texture(v, t as u32, &mut tex, px.as_mut_ptr(), px.len() as u32),
                1
            );

            // Spraying fills the water with drops, drawn as quads.
            let spray = SmsPad {
                r_analog: 150,
                ..SmsPad::default()
            };
            let mut most = 0;
            let mut most_wet = 0;
            for _ in 0..120 {
                sms_sim_step(sim, &host, &spray, 0, 1.0 / 60.0, v);
                view::sms_view_update(v, sim, &frame);
                let mut w = view::SmsWaterInfo::default();
                assert_eq!(view::sms_view_water_info(v, &mut w), 1);
                let mut corners = 0;
                view::sms_view_water_vertices(v, &mut corners);
                assert_eq!(corners, 4 * (w.drop_quads + w.splash_quads + w.wet_quads));
                most = most.max(w.drop_quads);
                most_wet = most_wet.max(w.wet_quads);
            }
            assert!(most > 10, "{most} drops");
            assert!(
                most_wet > 0,
                "spray impacts never reached the terrain renderer"
            );
            assert_eq!(
                view::sms_view_water_texture(v, 0, &mut tex, std::ptr::null_mut(), 0),
                1
            );
            view::sms_view_free(v);
            sms_sim_free(sim);
        }
    }
}
