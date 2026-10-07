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

pub mod view;

use std::ffi::{CStr, CString, c_char, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use libsms::event::{Event, JumpKind};
use view::SmsView;
use libsms::{
    Assets, GroundHit, Host, Pad, RoofHit, Sim, SurfaceFlags, Vec2, Vec3, WallHit, math,
};

/// ABI version, bumped when the C interface changes.
pub const SMS_ABI_VERSION: u32 = 2;

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
    /// pounds, damage for hits.
    pub arg: i32,
    /// Kind-specific: the sound's info value, health left after a hit.
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
}

/// The host's world, as callbacks. Any of them may be null: no ground means a void, no roof
/// open sky, no walls nothing to bump into, no event sink drops events. `user` is passed back
/// unchanged.
#[repr(C)]
pub struct SmsHost {
    pub user: *mut c_void,
    /// Highest floor at or below `pos.y` (`TMap::checkGround`). Fill `out` and return 1, or
    /// return 0 for a void.
    pub ground:
        Option<unsafe extern "C" fn(*mut c_void, *const SmsVec3, *mut SmsGroundHit) -> u8>,
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
}

/// An opaque sim handle.
pub struct SmsSim {
    pub(crate) sim: Sim,
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
            Event::GroundPound { super_ } => {
                e.kind = SmsEventKind::GroundPound as u32;
                e.arg = super_ as i32;
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
    let dir = unsafe { CStr::from_ptr(assets_dir) }.to_string_lossy().into_owned();
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
/// `camera_yaw` is the facing angle from Mario toward the camera. With a `view` (may be null),
/// its sounds follow each tick's animation and events. Returns the ticks run.
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
        (unsafe { sim.as_mut() }, unsafe { host.as_ref() }, unsafe { pad.as_ref() })
    else {
        return 0;
    };
    if s.poisoned {
        return 0;
    }
    let pad = pad.to_pad(camera_yaw);
    let ran = catch_unwind(AssertUnwindSafe(|| {
        let mut h = FfiHost(host, Vec::new());
        s.sim.advance_observed(&mut h, &pad, camera_yaw, dt, |mario, h| {
            let events = std::mem::take(&mut h.1);
            // SAFETY: the caller promised `view` is null or live.
            unsafe {
                view::with_view(view, (), |v| {
                    if let Some(a) = v.audio.as_mut() {
                        a.events(&events, mario);
                        a.tick(mario);
                    }
                });
            }
        })
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
        let m = &mut s.sim.mario;
        m.pos = (*pos).into();
        m.prev_pos = m.pos;
        m.vel = Vec3::ZERO;
        m.forward_vel = 0.0;
    }
}

/// Sizes of the ABI's structs, for hosts to check their copies against: `SmsVec3`,
/// `SmsGroundHit`, `SmsRoofHit`, `SmsWallHit`, `SmsEvent`, `SmsHost`, `SmsPad`, `SmsState`,
/// `SmsModelInfo`, `SmsBatch`, `SmsTexture`, `SmsDraw`, `SmsViewFrame`. Writes up to `max`;
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
            assert!(math::diff(yaw, want).unsigned_abs() < 600, "{x},{z}: {yaw} vs {want}");
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
            sms_sim_free(sim);
        }
    }

    #[test]
    fn struct_sizes_are_what_hosts_mirror() {
        let mut sizes = [0u32; 16];
        let n = unsafe { sms_struct_sizes(sizes.as_mut_ptr(), 16) } as usize;
        // ror2-mario's Native.cs and ror2-common's view structs check these at load.
        assert_eq!(&sizes[..n], &[12, 20, 8, 16, 32, 40, 24, 48, 40, 24, 12, 64, 44]);
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
        };
        unsafe {
            let sim = sms_sim_new(dir.as_ptr(), &SmsVec3::default(), 0);
            let v = view::sms_view_new(dir.as_ptr(), 0);
            assert!(!sim.is_null() && !v.is_null());
            let frame = view::SmsViewFrame {
                dt: 1.0 / 60.0,
                forward: SmsVec3 { x: 0.0, y: 0.0, z: 1.0 },
                up: SmsVec3 { x: 0.0, y: 1.0, z: 0.0 },
                volume: 1.0,
                ..Default::default()
            };
            for _ in 0..10 {
                sms_sim_step(sim, &host, &SmsPad::default(), 0, 1.0 / 60.0, v);
                view::sms_view_update(v, sim, &frame);
            }
            let n = view::sms_view_draws(v, std::ptr::null_mut(), 0);
            assert!(n >= 3, "{n} draws");
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
            assert_eq!(view::sms_view_texture(v, t as u32, &mut tex, std::ptr::null_mut(), 0), 1);
            let mut px = vec![0u8; (tex.width * tex.height * 4) as usize];
            assert_eq!(
                view::sms_view_texture(v, t as u32, &mut tex, px.as_mut_ptr(), px.len() as u32),
                1
            );
            view::sms_view_free(v);
            sms_sim_free(sim);
        }
    }
}
