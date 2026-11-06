//! The goop, its critters and Bowser Jr. through the C ABI.
//!
//! The host owns one goop world ([`sms_goop_new`]): the [`GoopField`] and the [`Critters`]
//! (Swoopin' Stus, goop Piranhas, their shots). It hands it to each Mario's sim with
//! [`sms_sim_set_goop`]; then that Mario stands in its goop (`Host::goop`), FLUDD's water
//! cleans it (`WaterManager::cleans`), running on oil spreads oil, his water and stomps hit the
//! critters (ids from `libsms::goop_critters::ID_BASE`, kept from the host's own enemy hits),
//! and a sim made Bowser Jr. ([`sms_sim_set_koopajr`]) paints with R. The host steps the world
//! once a frame ([`sms_goop_step`]) with the targets the critters may go for, deals the hits
//! they report ([`sms_goop_events`]) and draws the goop chunks and critters.

use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char};
use std::path::Path;

use libsms::FixedClock;
use libsms::goop::{GoopEvent, GoopField, GoopKind, GoopOp};
use libsms::goop_critters::{CritterEvent, CritterKind, Critters, PiranhaState, ShotKind, StuState, Target};
use libsms::koopajr::{Form, JrEvent, KoopaJr};
use libsms::{Enemy, Vec3};

use crate::{FfiHost, SmsHost, SmsSim, SmsVec3};

/// What a goop event is ([`SmsGoopEvent::kind`]).
pub mod ev {
    pub const PAINTED: u32 = 1;
    pub const ERASED: u32 = 2;
    pub const AMBIENT: u32 = 3;
    pub const SPAWNED: u32 = 10;
    pub const HIT: u32 = 11;
    pub const DIED: u32 = 12;
    pub const BURST: u32 = 13;
    pub const SHOT: u32 = 14;
    pub const SPLAT: u32 = 15;
    pub const SOUND: u32 = 16;
    pub const EFFECT: u32 = 17;
    pub const JR_FORM: u32 = 20;
    pub const JR_HIDDEN: u32 = 21;
    pub const JR_APPEARED: u32 = 22;
    pub const JR_DRENCHED: u32 = 23;
    pub const JR_SIGN: u32 = 24;
}

/// One thing that happened. `name` (a sound or effect, or an ambient kind's first effect) is
/// nul-terminated and lives as long as the world.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SmsGoopEvent {
    pub kind: u32,
    /// Per kind: the goop kind (painted, ambient), the critter kind (spawned), 1 if a Mario was
    /// hit (hit), melted (died), coins (burst), the shot kind (shot), the form (jr form).
    pub a: i32,
    /// The critter or shot, or the hit's target.
    pub id: u64,
    /// For a hit: the critter or shot that hit.
    pub from: u64,
    pub pos: SmsVec3,
    /// A radius, size, scale, a damage kind's `DamageKind::ALL` index, or a yaw.
    pub value: f32,
    pub name: *const c_char,
}

/// Something the critters may go for: an enemy, or a Mario (only up close).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SmsGoopTarget {
    pub id: u64,
    pub pos: SmsVec3,
    pub radius: f32,
    pub height: f32,
    pub mario: u8,
    pub _pad: [u8; 7],
}

/// A critter or shot to draw.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SmsCritter {
    pub id: u64,
    pub pos: SmsVec3,
    /// Facing (radians, along `(sin, 0, cos)`).
    pub yaw: f32,
    /// Overall scale, and a Stu's wind-up squash and lean (radians).
    pub scale: f32,
    pub squash: f32,
    pub lean: f32,
    /// Clip ticks in.
    pub clip_time: u32,
    /// 0 Stu, 1 Piranha, 2 goop shot, 3 seed.
    pub kind: u8,
    /// Clip index (`goop_critters::stu_clip` / `piranha_clip`).
    pub clip: u8,
    /// Stu: 0 rising, 1 walking, 2 winding up, 3 leaping, 4 landing, 5 dying. Piranha: 0
    /// appearing, 1 waiting, 2 shooting, 3 swelling, 4 down.
    pub state: u8,
    pub _pad: u8,
    /// A Stu's TEV tint (`nameKuriTevColorData`, signed 10-bit RGBA).
    pub tint: [i16; 4],
}

/// Bowser Jr.'s state for the host.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SmsJrState {
    /// 0 Shadow Mario, 1 Bowser Jr.
    pub form: u8,
    pub hidden: u8,
    pub painting: u8,
    /// `koopajr::jr_clip`.
    pub jr_clip: u8,
    pub paint_kind: u8,
    pub _pad: [u8; 3],
    /// `mWaterCounter`.
    pub water: u32,
    pub sign_timer: u32,
    pub shot_timer: u32,
}

/// What a goop op is ([`SmsGoopOp::kind`]).
pub mod op {
    /// A stamp: `pos`, radius `a`, `goop` kind.
    pub const STAMP: u8 = 1;
    /// Bowser Jr.'s sign: `pos`, half-side `a`, yaw `b` (radians), `goop` kind.
    pub const SIGN: u8 = 2;
    /// A clean: `pos`, radius `a`.
    pub const CLEAN: u8 = 3;
    /// A goop ball from `pos` at `vel`, gravity `a`, splat radius `b`, shooter `id`.
    pub const GOOP_SHOT: u8 = 4;
    /// A Piranha seed from `pos` at `vel`, thrower `id`.
    pub const SEED: u8 = 5;
    /// A stomp or pound on critter `id`.
    pub const STOMP: u8 = 6;
    /// FLUDD's water on critter `id` at `pos` with `attack`.
    pub const WATER: u8 = 7;
}

/// One change a mirror's sims made to its world, for the server to redo on its own
/// ([`sms_goop_take_ops`], [`sms_goop_apply_ops`]).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SmsGoopOp {
    pub id: u64,
    pub pos: SmsVec3,
    pub vel: SmsVec3,
    pub a: f32,
    pub b: f32,
    pub kind: u8,
    pub goop: u8,
    pub attack: i16,
    pub _pad: u32,
}

/// An opaque goop world.
pub struct SmsGoop {
    pub(crate) field: GoopField,
    pub(crate) critters: Critters,
    clock: FixedClock,
    events: Vec<SmsGoopEvent>,
    names: HashMap<&'static str, CString>,
    /// Bowser Jr.'s sign from the disc ([`libsms_present::goop_surface::sign_mask`]) and its
    /// half-side, handed to a Bowser Jr. sim standing in this world.
    pub(crate) sign: Option<(libsms::goop::Mask, f32)>,
    /// Each kind's look for drawing, read by [`sms_goop_setup`].
    surface: Option<libsms_present::goop_surface::GoopSurface>,
    /// A networked client's copy ([`sms_goop_set_mirror`]): the server's critters as last
    /// sent, drawn and aimed at in place of stepped ones.
    mirror: Option<Vec<SmsCritter>>,
    /// The critter changes a mirror's sims made (the goop's own are `field.log`).
    ops: Vec<SmsGoopOp>,
}

impl SmsGoop {
    /// What a sim's water and stomps may hit: the stepped critters, or a mirror's copies.
    pub(crate) fn enemies(&self) -> Vec<Enemy> {
        let Some(list) = self.mirror.as_ref() else { return self.critters.enemies() };
        let (s, p) = (&self.critters.stu_params, &self.critters.piranha_params);
        list.iter()
            .filter_map(|c| match (c.kind, c.state) {
                (0, st) if st != 5 => Some(Enemy {
                    id: c.id,
                    pos: c.pos.into(),
                    radius: s.damage_radius * c.scale,
                    height: s.damage_height * c.scale,
                }),
                (1, st) if st != 4 => Some(Enemy {
                    id: c.id,
                    pos: c.pos.into(),
                    radius: p.damage_radius * c.scale,
                    height: p.damage_height,
                }),
                _ => None,
            })
            .collect()
    }

    /// A stomp or pound on critter `id`: landed, or logged for the server on a mirror.
    pub(crate) fn stomp(&mut self, id: u64) {
        if self.mirror.is_some() {
            self.ops.push(SmsGoopOp { id, kind: op::STOMP, ..Default::default() });
        } else {
            self.critters.stomp(id);
        }
    }

    /// FLUDD's water on critter `id`: landed, or logged for the server on a mirror.
    pub(crate) fn water_hit(&mut self, id: u64, pos: Vec3, attack: i16) {
        if self.mirror.is_some() {
            self.ops.push(SmsGoopOp { id, pos: pos.into(), attack, kind: op::WATER, ..Default::default() });
        } else {
            self.critters.water_hit(id, pos, attack);
        }
    }

    /// On a mirror, a shot a sim just made goes to the server instead (its critters aren't
    /// stepped here).
    fn shot_to_server(&mut self, id: u64) {
        if self.mirror.is_none() {
            return;
        }
        let Some(i) = self.critters.shots.iter().position(|x| x.id == id) else { return };
        let x = self.critters.shots.remove(i);
        let seed = x.kind == ShotKind::Seed;
        self.ops.push(SmsGoopOp {
            id: x.owner,
            pos: x.pos.into(),
            vel: x.vel.into(),
            a: x.gravity,
            b: x.splat,
            kind: if seed { op::SEED } else { op::GOOP_SHOT },
            ..Default::default()
        });
    }

    fn name(&mut self, n: &'static str) -> *const c_char {
        self.names
            .entry(n)
            .or_insert_with(|| CString::new(n).unwrap_or_default())
            .as_ptr()
    }

    fn push(&mut self, kind: u32, a: i32, id: u64, pos: Vec3, value: f32, name: Option<&'static str>) {
        let name = name.map_or(std::ptr::null(), |n| self.name(n));
        self.events.push(SmsGoopEvent { kind, a, id, from: 0, pos: pos.into(), value, name });
        if self.events.len() > 1024 {
            self.events.drain(..512);
        }
    }

    pub(crate) fn drain_field_events(&mut self) {
        for e in std::mem::take(&mut self.field.events) {
            match e {
                GoopEvent::Painted { pos, radius, kind } => {
                    self.push(ev::PAINTED, kind as i32, 0, pos, radius, None)
                }
                GoopEvent::Erased { pos, size } => {
                    self.push(ev::ERASED, 0, 0, pos, size, Some("ms_raku_kie"))
                }
                GoopEvent::Ambient { pos, kind } => {
                    let look = kind.look();
                    for fx in look.effects {
                        self.push(ev::AMBIENT, kind as i32, 0, pos, look.effect_scale, Some(fx));
                    }
                    if let Some(s) = look.sound {
                        self.push(ev::SOUND, kind as i32, 0, pos, 1.0, Some(s));
                    }
                }
            }
        }
    }

    fn drain_critter_events(&mut self) {
        for e in std::mem::take(&mut self.critters.events) {
            match e {
                CritterEvent::Spawned { id, kind, pos } => {
                    self.push(ev::SPAWNED, (kind == CritterKind::Piranha) as i32, id, pos, 0.0, None)
                }
                CritterEvent::Hit { id, target, mario, pos, kind } => {
                    let k = libsms::DamageKind::ALL.iter().position(|d| *d == kind).unwrap_or(0);
                    self.push(ev::HIT, mario as i32, target, pos, k as f32, None);
                    if let Some(last) = self.events.last_mut() {
                        last.from = id;
                    }
                }
                CritterEvent::Died { id, pos, melted } => self.push(ev::DIED, melted as i32, id, pos, 0.0, None),
                CritterEvent::Burst { id, pos, coins } => self.push(ev::BURST, coins as i32, id, pos, 0.0, None),
                CritterEvent::Shot { id, from, kind } => {
                    self.push(ev::SHOT, (kind == ShotKind::Seed) as i32, id, from, 0.0, None)
                }
                CritterEvent::Splat { id, pos, radius } => self.push(ev::SPLAT, 0, id, pos, radius, None),
                CritterEvent::Sound { name, pos } => self.push(ev::SOUND, 0, 0, pos, 1.0, Some(name)),
                CritterEvent::Effect { name, pos, scale } => self.push(ev::EFFECT, 0, 0, pos, scale, Some(name)),
            }
        }
    }

    pub(crate) fn take_jr_events(&mut self, jr: &mut KoopaJr) {
        for e in std::mem::take(&mut jr.events) {
            match e {
                JrEvent::FormChanged { form, pos } => self.push(ev::JR_FORM, form as i32, jr.id, pos, 0.0, None),
                JrEvent::Hidden { pos } => self.push(ev::JR_HIDDEN, 0, jr.id, pos, 0.0, None),
                JrEvent::Appeared { pos } => self.push(ev::JR_APPEARED, 0, jr.id, pos, 0.0, None),
                JrEvent::Drenched { pos } => self.push(ev::JR_DRENCHED, 0, jr.id, pos, 0.0, None),
                JrEvent::Painted { pos, radius, kind } => self.push(ev::PAINTED, kind as i32, jr.id, pos, radius, None),
                JrEvent::Sign { pos, yaw } => self.push(ev::JR_SIGN, 0, jr.id, pos, yaw, None),
                JrEvent::Sound { name, pos } => self.push(ev::SOUND, 0, jr.id, pos, 1.0, Some(name)),
                JrEvent::Effect { name, pos, scale } => self.push(ev::EFFECT, 0, jr.id, pos, scale, Some(name)),
            }
        }
    }
}

/// A new, empty goop world with the core's defaults: 90 s lifetime, the oldest goop going past
/// 48k texels.
#[unsafe(no_mangle)]
pub extern "C" fn sms_goop_new() -> *mut SmsGoop {
    libcommon::ffi::guard("sms_goop_new", std::ptr::null_mut(), || {
        Box::into_raw(Box::new(SmsGoop {
            field: GoopField::new(),
            critters: Critters::default(),
            clock: FixedClock::default(),
            events: Vec::new(),
            names: HashMap::new(),
            sign: None,
            surface: None,
            mirror: None,
            ops: Vec::new(),
        }))
    })
}

/// # Safety
/// `g` from [`sms_goop_new`], not used afterwards, and no sim still holding it. Null is ignored.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_goop_free(g: *mut SmsGoop) {
    libcommon::ffi::guard("sms_goop_free", (), || {
        if !g.is_null() {
            // SAFETY: the caller hands back ownership.
            drop(unsafe { Box::from_raw(g) });
        }
    })
}

/// Gives `sim` the goop world it stands in, cleans and hits (null to let go). Not owned.
///
/// # Safety
/// `sim` from `sms_sim_new`; `g` null or from [`sms_goop_new`], alive while a sim holds it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_sim_set_goop(sim: *mut SmsSim, g: *mut SmsGoop) {
    libcommon::ffi::guard("sms_sim_set_goop", (), || {
        // SAFETY: as documented.
        if let Some(s) = unsafe { sim.as_mut() } {
            s.goop_world = g;
        }
    })
}

/// Empties the world and reads the critters' tuning and clips from `assets_dir`
/// (`params_enemy/namekuri.prm`, `pakkun.prm`, `enemy/namekuri2/*.bck`, `enemy/pakkun/*.bck`),
/// the stamps' shapes (`scene/map/pollute.bti`, `clean.bti`), Bowser Jr.'s sign and each kind's
/// look for [`sms_goop_chunk_image`] (the `scene/map/pollution/` layers). Null keeps the
/// defaults (soft discs, the kinds' tints).
///
/// # Safety
/// `g` from [`sms_goop_new`]; `assets_dir` nul-terminated or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_goop_setup(g: *mut SmsGoop, assets_dir: *const c_char) {
    libcommon::ffi::guard("sms_goop_setup", (), || {
        // SAFETY: as documented.
        let Some(s) = (unsafe { g.as_mut() }) else { return };
        s.field.clear();
        s.events.clear();
        let keep = (s.critters.max_stus, s.critters.max_piranhas, s.critters.spawn_interval, s.critters.mario_range);
        s.critters = if assets_dir.is_null() {
            Critters::default()
        } else {
            // SAFETY: as documented.
            let dir = unsafe { CStr::from_ptr(assets_dir) }.to_string_lossy().into_owned();
            let dir = Path::new(&dir);
            if let Some((pollute, clean)) = libsms_present::goop_surface::stamp_masks(dir) {
                s.field.pollute = pollute;
                s.field.clean = clean;
            }
            s.sign = libsms_present::goop_surface::sign_mask(dir, 64);
            s.surface = Some(libsms_present::goop_surface::GoopSurface::load(dir));
            Critters::from_dir(dir)
        };
        (s.critters.max_stus, s.critters.max_piranhas, s.critters.spawn_interval, s.critters.mario_range) = keep;
    })
}

/// Empties the world (the goop, the critters, their shots and the queued events) and keeps
/// what [`sms_goop_setup`] read, for a host starting a new stage.
///
/// # Safety
/// `g` from [`sms_goop_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_goop_clear(g: *mut SmsGoop) {
    libcommon::ffi::guard("sms_goop_clear", (), || {
        if let Some(s) = unsafe { g.as_mut() } {
            s.field.clear();
            s.field.events.clear();
            s.critters.stus.clear();
            s.critters.piranhas.clear();
            s.critters.shots.clear();
            s.critters.events.clear();
            s.events.clear();
            s.ops.clear();
            if let Some(log) = s.field.log.as_mut() {
                log.clear();
            }
            if let Some(m) = s.mirror.as_mut() {
                m.clear();
            }
        }
    })
}

/// Makes the world a networked client's copy of the server's (`on` nonzero) or a world of its
/// own. A copy's critters are the server's as last given ([`sms_goop_mirror_critters`]), never
/// stepped, and what its sims do to it (paint, cleaning, shots, hits on critters) is logged
/// for the server ([`sms_goop_take_ops`]); its goop is the server's chunks
/// ([`sms_goop_chunk_set`]) plus its own sims' paint and cleaning until those come back.
///
/// # Safety
/// `g` from [`sms_goop_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_goop_set_mirror(g: *mut SmsGoop, on: u8) {
    libcommon::ffi::guard("sms_goop_set_mirror", (), || {
        // SAFETY: as documented.
        let Some(s) = (unsafe { g.as_mut() }) else { return };
        if (on != 0) == s.mirror.is_some() {
            return;
        }
        s.ops.clear();
        if on != 0 {
            s.mirror = Some(Vec::new());
            s.field.log = Some(Vec::new());
            s.critters.stus.clear();
            s.critters.piranhas.clear();
            s.critters.shots.clear();
            s.critters.events.clear();
        } else {
            s.mirror = None;
            s.field.log = None;
        }
    })
}

/// A mirror's critters, as the server last sent them (ignored unless [`sms_goop_set_mirror`]).
///
/// # Safety
/// `g` from [`sms_goop_new`]; `list` holds `n` (or is null).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_goop_mirror_critters(g: *mut SmsGoop, list: *const SmsCritter, n: u32) {
    libcommon::ffi::guard("sms_goop_mirror_critters", (), || {
        // SAFETY: as documented.
        let Some(s) = (unsafe { g.as_mut() }) else { return };
        let Some(m) = s.mirror.as_mut() else { return };
        m.clear();
        if list.is_null() {
            return;
        }
        // SAFETY: as documented.
        let src = unsafe { std::slice::from_raw_parts(list, n.min(4096) as usize) };
        m.extend(src.iter().copied().filter(|c| Vec3::from(c.pos).is_finite() && c.scale.is_finite()));
    })
}

/// Takes up to `max` of a mirror's logged changes (oldest first: the goop's, then the
/// critters'). Returns how many were written.
///
/// # Safety
/// `g` from [`sms_goop_new`]; `out` has room for `max`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_goop_take_ops(g: *mut SmsGoop, out: *mut SmsGoopOp, max: u32) -> u32 {
    libcommon::ffi::guard("sms_goop_take_ops", 0, || {
        // SAFETY: as documented.
        let Some(s) = (unsafe { g.as_mut() }) else { return 0 };
        if out.is_null() {
            return 0;
        }
        let max = max as usize;
        let mut n = 0usize;
        if let Some(log) = s.field.log.as_mut() {
            let take = log.len().min(max);
            for o in log.drain(..take) {
                let x = match o {
                    GoopOp::Stamp { pos, radius, kind } => {
                        SmsGoopOp { pos: pos.into(), a: radius, goop: kind as u8, kind: op::STAMP, ..Default::default() }
                    }
                    GoopOp::Mask { pos, half, yaw, kind } => SmsGoopOp {
                        pos: pos.into(),
                        a: half,
                        b: yaw,
                        goop: kind as u8,
                        kind: op::SIGN,
                        ..Default::default()
                    },
                    GoopOp::Clean { pos, radius } => {
                        SmsGoopOp { pos: pos.into(), a: radius, kind: op::CLEAN, ..Default::default() }
                    }
                };
                // SAFETY: as documented.
                unsafe { *out.add(n) = x };
                n += 1;
            }
        }
        let take = s.ops.len().min(max - n);
        for x in s.ops.drain(..take) {
            // SAFETY: as documented.
            unsafe { *out.add(n) = x };
            n += 1;
        }
        n as u32
    })
}

/// The server redoes a client's logged changes on its own world. `owner` is the host's id for
/// the client's Bowser Jr. (his shots' shooter; 0 for none). Values out of range are skipped.
/// Returns how many were applied; their events queue as the world's own.
///
/// # Safety
/// `g` from [`sms_goop_new`]; `host` valid; `ops` holds `n`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_goop_apply_ops(
    g: *mut SmsGoop,
    host: *const SmsHost,
    ops: *const SmsGoopOp,
    n: u32,
    owner: u64,
) -> u32 {
    libcommon::ffi::guard("sms_goop_apply_ops", 0, || {
        // SAFETY: as documented.
        let (Some(s), Some(host)) = (unsafe { g.as_mut() }, unsafe { host.as_ref() }) else { return 0 };
        if ops.is_null() {
            return 0;
        }
        // SAFETY: as documented.
        let list = unsafe { std::slice::from_raw_parts(ops, n.min(4096) as usize) };
        let h = FfiHost(host, Vec::new(), Vec::new(), std::ptr::null());
        let mut done = 0u32;
        for o in list {
            let pos: Vec3 = o.pos.into();
            let vel: Vec3 = o.vel.into();
            if !pos.is_finite() || !vel.is_finite() || !o.a.is_finite() || !o.b.is_finite() {
                continue;
            }
            let size_ok = o.a > 0.0 && o.a <= 4096.0;
            let fast_ok = vel.length() < 20000.0;
            let kind = GoopKind::from_u8(o.goop);
            let applied = match o.kind {
                op::STAMP if size_ok => {
                    s.field.stamp(&h, pos, o.a, kind);
                    true
                }
                op::SIGN if size_ok => {
                    match s.sign.clone() {
                        Some((mask, _)) => s.field.stamp_mask(&h, pos, o.a, o.b, &mask, kind),
                        None => s.field.stamp(&h, pos, o.a, kind),
                    }
                    true
                }
                op::CLEAN if size_ok => {
                    s.field.clean(pos, o.a);
                    true
                }
                op::GOOP_SHOT if fast_ok && o.a.abs() <= 100.0 && o.b >= 0.0 && o.b <= 4096.0 => {
                    s.critters.shoot_goop(pos, vel, o.a, o.b, owner);
                    true
                }
                op::SEED if fast_ok => {
                    s.critters.throw_seed(pos, vel, owner);
                    true
                }
                op::STOMP if Critters::owns_id(o.id) => s.critters.stomp(o.id),
                op::WATER if Critters::owns_id(o.id) => s.critters.water_hit(o.id, pos, o.attack),
                _ => false,
            };
            done += applied as u32;
        }
        s.drain_critter_events();
        s.drain_field_events();
        done
    })
}

/// Puts a chunk as the server has it (on a mirror): 256 densities, kinds (`GoopKind` as u8)
/// and painted heights, row by row. All-zero densities remove it.
///
/// # Safety
/// `g` from [`sms_goop_new`]; each array holds 256.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_goop_chunk_set(
    g: *mut SmsGoop,
    kx: i32,
    kz: i32,
    density: *const u8,
    kinds: *const u8,
    heights: *const f32,
) {
    libcommon::ffi::guard("sms_goop_chunk_set", (), || {
        // SAFETY: as documented.
        let Some(s) = (unsafe { g.as_mut() }) else { return };
        if density.is_null() || kinds.is_null() || heights.is_null() {
            return;
        }
        // SAFETY: as documented.
        let (d, k, h) = unsafe {
            (
                std::slice::from_raw_parts(density, 256),
                std::slice::from_raw_parts(kinds, 256),
                std::slice::from_raw_parts(heights, 256),
            )
        };
        let mut density = [0u8; 256];
        let mut kind = [GoopKind::Sink; 256];
        let mut height = [0f32; 256];
        for i in 0..256 {
            if h[i].is_finite() {
                density[i] = d[i];
                height[i] = h[i];
            }
            kind[i] = GoopKind::from_u8(k[i]);
        }
        s.field.set_chunk((kx, kz), &density, &kind, &height);
    })
}

/// Removes a chunk (one the server no longer has).
///
/// # Safety
/// `g` from [`sms_goop_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_goop_chunk_drop(g: *mut SmsGoop, kx: i32, kz: i32) {
    libcommon::ffi::guard("sms_goop_chunk_drop", (), || {
        // SAFETY: as documented.
        if let Some(s) = unsafe { g.as_mut() } {
            s.field.drop_chunk((kx, kz));
        }
    })
}

/// Goop tuning: `lifetime` and `fade` in ticks (0 lifetime: until cleaned), the caps.
///
/// # Safety
/// `g` from [`sms_goop_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_goop_tune(g: *mut SmsGoop, lifetime: u32, fade: u32, max_texels: u32, max_chunks: u32) {
    libcommon::ffi::guard("sms_goop_tune", (), || {
        // SAFETY: as documented.
        if let Some(s) = unsafe { g.as_mut() } {
            s.field.lifetime = lifetime;
            s.field.fade = fade;
            s.field.max_texels = (max_texels as usize).max(256);
            s.field.max_chunks = (max_chunks as usize).max(1);
        }
    })
}

/// Critter tuning: how many at once, the Stu spawn interval (0: none by themselves), how close
/// a Mario must be to be a target, the water a Piranha takes to burst, and the goop kind they
/// splat.
///
/// # Safety
/// `g` from [`sms_goop_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_goop_critter_tune(
    g: *mut SmsGoop,
    max_stus: u32,
    max_piranhas: u32,
    spawn_interval: u32,
    mario_range: f32,
    burst_water: f32,
    goop_kind: u8,
) {
    libcommon::ffi::guard("sms_goop_critter_tune", (), || {
        // SAFETY: as documented.
        if let Some(s) = unsafe { g.as_mut() } {
            let c = &mut s.critters;
            c.max_stus = max_stus as usize;
            c.max_piranhas = max_piranhas as usize;
            c.spawn_interval = spawn_interval;
            c.mario_range = mario_range.max(0.0);
            c.burst_water = burst_water.max(1.0);
            c.goop_kind = GoopKind::from_u8(goop_kind);
        }
    })
}

/// Paints goop of `kind` (`GoopKind` as u8).
///
/// # Safety
/// `g` from [`sms_goop_new`]; `host` valid (its ground callback is used).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_goop_stamp(g: *mut SmsGoop, host: *const SmsHost, pos: SmsVec3, radius: f32, kind: u8) {
    libcommon::ffi::guard("sms_goop_stamp", (), || {
        // SAFETY: as documented.
        let (Some(s), Some(host)) = (unsafe { g.as_mut() }, unsafe { host.as_ref() }) else { return };
        let h = FfiHost(host, Vec::new(), Vec::new(), std::ptr::null());
        s.field.stamp(&h, pos.into(), radius, GoopKind::from_u8(kind));
    })
}

/// Cleans goop off a disc; returns the texels that lost goop.
///
/// # Safety
/// `g` from [`sms_goop_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_goop_clean(g: *mut SmsGoop, pos: SmsVec3, radius: f32) -> u32 {
    libcommon::ffi::guard("sms_goop_clean", 0, || {
        // SAFETY: as documented.
        unsafe { g.as_mut() }.map_or(0, |s| s.field.clean(pos.into(), radius))
    })
}

/// The goop kind at a point, or -1.
///
/// # Safety
/// `g` from [`sms_goop_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_goop_kind_at(g: *const SmsGoop, pos: SmsVec3) -> i32 {
    libcommon::ffi::guard("sms_goop_kind_at", -1, || {
        // SAFETY: as documented.
        unsafe { g.as_ref() }.and_then(|s| s.field.kind_at(pos.into())).map_or(-1, |k| k as i32)
    })
}

/// Steps the world by `dt`: the goop's lifetime, the critters, and the ambient effects round
/// `players`. `owner` (null for none) is where Stus grow near. Returns the ticks run.
///
/// # Safety
/// `g` from [`sms_goop_new`]; `host` valid; `targets` has `count` entries and `players`
/// `nplayers` (either may be null).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_goop_step(
    g: *mut SmsGoop,
    host: *const SmsHost,
    owner: *const SmsVec3,
    targets: *const SmsGoopTarget,
    count: u32,
    players: *const SmsVec3,
    nplayers: u32,
    dt: f32,
) -> u32 {
    libcommon::ffi::guard("sms_goop_step", 0, || {
        // SAFETY: as documented.
        let (Some(s), Some(host)) = (unsafe { g.as_mut() }, unsafe { host.as_ref() }) else { return 0 };
        // SAFETY: as documented.
        let owner = unsafe { owner.as_ref() }.map(|v| Vec3::from(*v)).filter(|v| v.is_finite());
        let targets: Vec<Target> = if targets.is_null() {
            Vec::new()
        } else {
            // SAFETY: as documented.
            unsafe { std::slice::from_raw_parts(targets, count.min(256) as usize) }
                .iter()
                .map(|t| Target { id: t.id, pos: t.pos.into(), radius: t.radius, height: t.height, mario: t.mario != 0 })
                .filter(|t| t.pos.is_finite())
                .collect()
        };
        let players: Vec<Vec3> = if players.is_null() {
            Vec::new()
        } else {
            // SAFETY: as documented.
            unsafe { std::slice::from_raw_parts(players, nplayers.min(16) as usize) }
                .iter()
                .map(|v| Vec3::from(*v))
                .filter(|v| v.is_finite())
                .collect()
        };
        let n = s.clock.accumulate(dt);
        let h = FfiHost(host, Vec::new(), Vec::new(), std::ptr::null());
        for _ in 0..n {
            s.critters.step(&h, &mut s.field, owner, &targets);
            s.field.ambient(&players);
            s.field.step();
        }
        s.drain_critter_events();
        s.drain_field_events();
        n
    })
}

/// Which of `enemies` stand in goop now (each at most once every `period` ticks): their ids
/// and the goop kinds into `ids`/`kinds`, at most `max`. Returns how many.
///
/// # Safety
/// `g` from [`sms_goop_new`]; `enemies` has `count`; `ids` and `kinds` have room for `max`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_goop_contacts(
    g: *mut SmsGoop,
    enemies: *const SmsGoopTarget,
    count: u32,
    period: u32,
    ids: *mut u64,
    kinds: *mut u8,
    max: u32,
) -> u32 {
    libcommon::ffi::guard("sms_goop_contacts", 0, || {
        // SAFETY: as documented.
        let Some(s) = (unsafe { g.as_mut() }) else { return 0 };
        if enemies.is_null() || ids.is_null() || kinds.is_null() {
            return 0;
        }
        // SAFETY: as documented.
        let list: Vec<Enemy> = unsafe { std::slice::from_raw_parts(enemies, count.min(1024) as usize) }
            .iter()
            .map(|t| Enemy { id: t.id, pos: t.pos.into(), radius: t.radius, height: t.height })
            .collect();
        let hits = s.field.contacts(&list, period);
        let n = hits.len().min(max as usize);
        for (i, (id, k)) in hits.into_iter().take(n).enumerate() {
            // SAFETY: as documented.
            unsafe {
                *ids.add(i) = id;
                *kinds.add(i) = k as u8;
            }
        }
        n as u32
    })
}

/// A critter or shot was hit by water (`attack` its particle's) or stomped (`stomp` nonzero).
/// Returns whether `id` was one of the world's.
///
/// # Safety
/// `g` from [`sms_goop_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_goop_hit(g: *mut SmsGoop, id: u64, pos: SmsVec3, attack: i16, stomp: u8) -> u8 {
    libcommon::ffi::guard("sms_goop_hit", 0, || {
        // SAFETY: as documented.
        let Some(s) = (unsafe { g.as_mut() }) else { return 0 };
        let r = if stomp != 0 { s.critters.stomp(id) } else { s.critters.water_hit(id, pos.into(), attack) };
        s.drain_critter_events();
        r as u8
    })
}

/// Grows a Stu at `pos`, or throws a Piranha seed from `pos` at `vel`. Returns its id, or 0.
///
/// # Safety
/// `g` from [`sms_goop_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_goop_spawn(g: *mut SmsGoop, piranha_seed: u8, pos: SmsVec3, vel: SmsVec3, owner: u64) -> u64 {
    libcommon::ffi::guard("sms_goop_spawn", 0, || {
        // SAFETY: as documented.
        let Some(s) = (unsafe { g.as_mut() }) else { return 0 };
        let id = if piranha_seed != 0 {
            s.critters.throw_seed(pos.into(), vel.into(), owner)
        } else {
            s.critters.spawn_stu(pos.into()).unwrap_or(0)
        };
        s.drain_critter_events();
        id
    })
}

/// The critters and shots to draw, at most `max`. Returns how many there are.
///
/// # Safety
/// `g` from [`sms_goop_new`]; `out` has room for `max` (or is null to count).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_goop_critters(g: *const SmsGoop, out: *mut SmsCritter, max: u32) -> u32 {
    libcommon::ffi::guard("sms_goop_critters", 0, || {
        // SAFETY: as documented.
        let Some(s) = (unsafe { g.as_ref() }) else { return 0 };
        if let Some(list) = s.mirror.as_ref() {
            if !out.is_null() {
                for (i, x) in list.iter().take(max as usize).enumerate() {
                    // SAFETY: as documented.
                    unsafe { *out.add(i) = *x };
                }
            }
            return list.len() as u32;
        }
        let c = &s.critters;
        let mut all = Vec::with_capacity(c.stus.len() + c.piranhas.len() + c.shots.len());
        for x in &c.stus {
            let state = match x.state {
                StuState::Generate => 0,
                StuState::Walk => 1,
                StuState::Prepare => 2,
                StuState::Leap => 3,
                StuState::Land => 4,
                StuState::Die { .. } => 5,
            };
            all.push(SmsCritter {
                id: x.id, pos: x.pos.into(), yaw: x.yaw, scale: x.scale, squash: x.squash, lean: x.lean,
                clip_time: x.clip_time, kind: 0, clip: x.clip, state, _pad: 0, tint: x.tint,
            });
        }
        for x in &c.piranhas {
            let state = match x.state {
                PiranhaState::Appear => 0,
                PiranhaState::Stay => 1,
                PiranhaState::Shoot => 2,
                PiranhaState::Swell => 3,
                PiranhaState::Down => 4,
            };
            all.push(SmsCritter {
                id: x.id, pos: x.pos.into(), yaw: x.yaw, scale: x.swell, squash: 1.0, lean: 0.0,
                clip_time: x.clip_time, kind: 1, clip: x.clip, state, _pad: 0, tint: [0; 4],
            });
        }
        for x in &c.shots {
            all.push(SmsCritter {
                id: x.id, pos: x.pos.into(), yaw: x.vel.x.atan2(x.vel.z), scale: 1.0, squash: 1.0,
                lean: x.spin.to_radians(), clip_time: 0, kind: if x.kind == ShotKind::Seed { 3 } else { 2 },
                clip: 0, state: 0, _pad: 0, tint: [0; 4],
            });
        }
        if !out.is_null() {
            for (i, x) in all.iter().take(max as usize).enumerate() {
                // SAFETY: as documented.
                unsafe { *out.add(i) = *x };
            }
        }
        all.len() as u32
    })
}

/// Takes up to `max` events (oldest first). Returns how many were written.
///
/// # Safety
/// `g` from [`sms_goop_new`]; `out` has room for `max`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_goop_events(g: *mut SmsGoop, out: *mut SmsGoopEvent, max: u32) -> u32 {
    libcommon::ffi::guard("sms_goop_events", 0, || {
        // SAFETY: as documented.
        let Some(s) = (unsafe { g.as_mut() }) else { return 0 };
        if out.is_null() {
            return 0;
        }
        let n = s.events.len().min(max as usize);
        for (i, e) in s.events.drain(..n).enumerate() {
            // SAFETY: as documented.
            unsafe { *out.add(i) = e };
        }
        n as u32
    })
}

/// The goop's chunks: up to `max` keys (`kx, kz` pairs into `keys`) and their versions.
/// Returns how many chunks there are. A chunk is 16x16 texels of 32 units from
/// `(kx * 512, kz * 512)`.
///
/// # Safety
/// `g` from [`sms_goop_new`]; `keys` has room for `2 * max`, `versions` for `max` (or null).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_goop_chunk_list(g: *const SmsGoop, keys: *mut i32, versions: *mut u32, max: u32) -> u32 {
    libcommon::ffi::guard("sms_goop_chunk_list", 0, || {
        // SAFETY: as documented.
        let Some(s) = (unsafe { g.as_ref() }) else { return 0 };
        if !keys.is_null() {
            for (i, (k, c)) in s.field.chunks().take(max as usize).enumerate() {
                // SAFETY: as documented.
                unsafe {
                    *keys.add(2 * i) = k.0;
                    *keys.add(2 * i + 1) = k.1;
                    if !versions.is_null() {
                        *versions.add(i) = c.version;
                    }
                }
            }
        }
        s.field.chunk_count() as u32
    })
}

/// One chunk's 256 texels (row-major from -X/-Z): densities, kinds and heights (any may be
/// null). Returns 0 if there's no such chunk.
///
/// # Safety
/// `g` from [`sms_goop_new`]; each non-null output has room for 256.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_goop_chunk(
    g: *const SmsGoop,
    kx: i32,
    kz: i32,
    density: *mut u8,
    kinds: *mut u8,
    heights: *mut f32,
) -> u8 {
    libcommon::ffi::guard("sms_goop_chunk", 0, || {
        // SAFETY: as documented.
        let Some(s) = (unsafe { g.as_ref() }) else { return 0 };
        let Some((_, c)) = s.field.chunks().find(|(k, _)| *k == (kx, kz)) else { return 0 };
        for i in 0..256 {
            // SAFETY: as documented.
            unsafe {
                if !density.is_null() {
                    *density.add(i) = c.density[i];
                }
                if !kinds.is_null() {
                    *kinds.add(i) = c.kind[i] as u8;
                }
                if !heights.is_null() {
                    *heights.add(i) = c.height[i];
                }
            }
        }
        1
    })
}

/// Chunk `(kx, kz)`'s goop as a `res` x `res` RGBA8 picture (rows from -Z, columns from -X, as
/// the chunk's texels; `res` at most 128) into `rgba` (`res * res * 4` bytes): each kind's look
/// from the disc laid in world space (one repeat every 400 units), alpha from the density, soft
/// round the kind's threshold. The host lays it over the chunk (512 units a side from
/// `(kx, kz) * 512`) at its texels' heights and redraws when the chunk's version changes.
/// Returns 0 if there's no such chunk.
///
/// # Safety
/// `g` from [`sms_goop_new`]; `rgba` has room for `res * res * 4` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_goop_chunk_image(g: *mut SmsGoop, kx: i32, kz: i32, res: u32, rgba: *mut u8) -> u8 {
    libcommon::ffi::guard("sms_goop_chunk_image", 0, || {
        // SAFETY: as documented.
        let Some(s) = (unsafe { g.as_mut() }) else { return 0 };
        if rgba.is_null() || !s.field.chunks().any(|(k, _)| k == (kx, kz)) {
            return 0;
        }
        let res = res.clamp(1, libsms_present::goop_surface::MAX_CHUNK_IMAGE);
        let surface = s.surface.get_or_insert_with(Default::default);
        let img = surface.chunk_image(&s.field, (kx, kz), res);
        // SAFETY: as documented.
        unsafe { std::ptr::copy_nonoverlapping(img.as_ptr(), rgba, img.len()) };
        1
    })
}

/// [`sms_goop_chunk_image`] `time` seconds into each kind's swim (the layers' btk slide their
/// warp one repeat every 2000 game frames; the picture blends the nearest of 12 bakes). A host
/// redraws its nearby chunks every second or so with its clock to show it.
///
/// # Safety
/// As [`sms_goop_chunk_image`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_goop_chunk_image_at(
    g: *mut SmsGoop,
    kx: i32,
    kz: i32,
    res: u32,
    time: f32,
    rgba: *mut u8,
) -> u8 {
    libcommon::ffi::guard("sms_goop_chunk_image_at", 0, || {
        // SAFETY: as documented.
        let Some(s) = (unsafe { g.as_mut() }) else { return 0 };
        if rgba.is_null() || !time.is_finite() || !s.field.chunks().any(|(k, _)| k == (kx, kz)) {
            return 0;
        }
        let res = res.clamp(1, libsms_present::goop_surface::MAX_CHUNK_IMAGE);
        let surface = s.surface.get_or_insert_with(Default::default);
        let img = surface.chunk_image_at(&s.field, (kx, kz), res, time);
        // SAFETY: as documented.
        unsafe { std::ptr::copy_nonoverlapping(img.as_ptr(), rgba, img.len()) };
        1
    })
}

/// A goop kind's look at full density (the tile [`sms_goop_chunk_image`] repeats), RGBA8 into
/// `rgba` when it has room for `cap` bytes; returns the side length (tiles are square), 0
/// without one (no [`sms_goop_setup`] with assets).
///
/// # Safety
/// `g` from [`sms_goop_new`]; `rgba` null or writable for `cap` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_goop_tile(g: *const SmsGoop, kind: u8, rgba: *mut u8, cap: u32) -> u32 {
    libcommon::ffi::guard("sms_goop_tile", 0, || {
        // SAFETY: as documented.
        let Some(s) = (unsafe { g.as_ref() }) else { return 0 };
        let Some(t) = s.surface.as_ref().and_then(|sf| sf.tile(GoopKind::from_u8(kind))) else { return 0 };
        let b = &t.image;
        if !rgba.is_null() && b.rgba.len() <= cap as usize {
            // SAFETY: as documented.
            unsafe { std::ptr::copy_nonoverlapping(b.rgba.as_ptr(), rgba, b.rgba.len()) };
        }
        b.width
    })
}

/// A goop kind's ambient look: tint (3 floats into `tint`) and its layer model's name.
///
/// # Safety
/// `tint` has room for 3 or is null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_goop_kind_surface(kind: u8, tint: *mut f32) -> *const c_char {
    libcommon::ffi::guard("sms_goop_kind_surface", std::ptr::null(), || {
        let k = GoopKind::from_u8(kind);
        if !tint.is_null() {
            for (i, v) in k.look().tint.iter().enumerate() {
                // SAFETY: as documented.
                unsafe { *tint.add(i) = *v };
            }
        }
        static NAMES: [&CStr; 7] = [
            c"pollution00", c"pollution01", c"pollution02", c"pollution03", c"pollution04",
            c"pollution05", c"pollution06",
        ];
        let i = k.surface().trim_start_matches("pollution").parse::<usize>().unwrap_or(0).min(6);
        NAMES[i].as_ptr()
    })
}

/// Makes `sim` Bowser Jr. (`on` nonzero: no FLUDD, goop never catches him, R paints) or not.
/// `id` is the host's id for him (his shots pass through him).
///
/// # Safety
/// `sim` from `sms_sim_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_sim_set_koopajr(sim: *mut SmsSim, on: u8, id: u64) {
    libcommon::ffi::guard("sms_sim_set_koopajr", (), || {
        // SAFETY: as documented.
        let Some(s) = (unsafe { sim.as_mut() }) else { return };
        if on == 0 {
            s.jr = None;
            s.sim.mario.goop.immune = false;
            return;
        }
        let mut jr = KoopaJr::default();
        jr.id = id;
        jr.attach(&mut s.sim.mario);
        s.jr = Some(jr);
    })
}

/// Bowser Jr.'s actions: 0 change form, 1 hide in goop, 2 come out, 3 paint the sign, 4 shoot
/// a goop ball, 5 spit a Piranha seed, 6 sprayed with water (returns 1 when drenched), 7 next
/// paint kind. Returns whether it happened. The sim needs its goop world for 1, 3, 4 and 5.
///
/// # Safety
/// `sim` from `sms_sim_new`; `host` valid or null (needed for 3).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_jr_action(sim: *mut SmsSim, host: *const SmsHost, action: u32) -> u8 {
    libcommon::ffi::guard("sms_jr_action", 0, || {
        // SAFETY: as documented.
        let Some(s) = (unsafe { sim.as_mut() }) else { return 0 };
        // SAFETY: the host keeps a world it set alive (`sms_sim_set_goop`).
        let mut world = unsafe { s.goop_world.as_mut() };
        let Some(jr) = s.jr.as_mut() else { return 0 };
        let m = &mut s.sim.mario;
        let done = match (action, world.as_deref_mut()) {
            (0, _) => jr.change_form(m),
            (1, Some(w)) => jr.hide(m, &w.field),
            (2, _) => {
                let was = jr.hidden;
                jr.appear(m);
                was
            }
            (3, Some(w)) => match unsafe { host.as_ref() } {
                Some(host) => {
                    let h = FfiHost(host, Vec::new(), Vec::new(), std::ptr::null());
                    jr.paint_sign(m, &h, &mut w.field)
                }
                None => false,
            },
            (4, Some(w)) => match jr.shoot(m, &mut w.critters) {
                Some(id) => {
                    w.shot_to_server(id);
                    true
                }
                None => false,
            },
            (5, Some(w)) => match jr.spit_seed(m, &mut w.critters) {
                Some(id) => {
                    w.shot_to_server(id);
                    true
                }
                None => false,
            },
            (6, _) => jr.water_hit(m),
            (7, _) => {
                let all = GoopKind::ALL;
                let i = all.iter().position(|k| *k == jr.paint_kind).unwrap_or(0);
                jr.paint_kind = all[(i + 1) % all.len()];
                true
            }
            _ => false,
        };
        if let Some(w) = world {
            w.take_jr_events(jr);
            w.drain_critter_events();
            w.drain_field_events();
        } else {
            jr.events.clear();
        }
        done as u8
    })
}

/// Sets the kind Bowser Jr. paints (`GoopKind` as u8).
///
/// # Safety
/// `sim` from `sms_sim_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_jr_set_paint_kind(sim: *mut SmsSim, kind: u8) {
    libcommon::ffi::guard("sms_jr_set_paint_kind", (), || {
        // SAFETY: as documented.
        if let Some(jr) = unsafe { sim.as_mut() }.and_then(|s| s.jr.as_mut()) {
            jr.paint_kind = GoopKind::from_u8(kind);
        }
    })
}

/// Bowser Jr.'s state; 0 if the sim isn't him.
///
/// # Safety
/// `sim` from `sms_sim_new`; `out` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_jr_state(sim: *const SmsSim, out: *mut SmsJrState) -> u8 {
    libcommon::ffi::guard("sms_jr_state", 0, || {
        // SAFETY: as documented.
        let (Some(s), Some(out)) = (unsafe { sim.as_ref() }, unsafe { out.as_mut() }) else { return 0 };
        let Some(jr) = s.jr.as_ref() else { return 0 };
        *out = SmsJrState {
            form: (jr.form == Form::Jr) as u8,
            hidden: jr.hidden as u8,
            painting: jr.painting as u8,
            jr_clip: jr.jr_clip(&s.sim.mario),
            paint_kind: jr.paint_kind as u8,
            _pad: [0; 3],
            water: jr.water,
            sign_timer: jr.sign_timer,
            shot_timer: jr.shot_timer,
        };
        1
    })
}
