//! The sambo flowers (libsms::flower) through the C ABI, as a world of their own that a host owns
//! (on a server, one for the whole stage; on a client, a mirror of the server's snapshots) and
//! hands to each Mario's sim with [`sms_sim_set_flowers`]. A sim's water and stomps find the
//! world's things through the `Host::enemies` list the sim already queries (ids from
//! `libsms::flower::ID_BASE` up) and report them as hits; the host applies them where the world
//! is stepped (the server) with [`sms_fw_hit`].

use std::ffi::{CStr, c_char};
use std::path::Path;

use glam::Vec3;
use libsms::FixedClock;
use libsms::flower::{Clips, FlowerEvent, FlowerWorld, Item, Spawn};

use crate::{FfiHost, SmsHost, SmsSim, SmsVec3};

/// What a roll or a placement is: the low byte is the kind, the rest the size.
pub const SPAWN_NONE: u32 = 0;
pub const SPAWN_FLOWER_RING: u32 = 1;
pub const SPAWN_SINGLE: u32 = 2;
pub const SPAWN_ENEMY_RING: u32 = 3;
pub const SPAWN_TOWER: u32 = 4;

fn pack(s: Spawn) -> u32 {
    match s {
        Spawn::FlowerRing { flowers } => SPAWN_FLOWER_RING | (flowers as u32) << 8,
        Spawn::SingleFlower => SPAWN_SINGLE,
        Spawn::EnemyRing { heads } => SPAWN_ENEMY_RING | (heads as u32) << 8,
        Spawn::Tower => SPAWN_TOWER,
    }
}

fn unpack(v: u32) -> Option<Spawn> {
    let n = (v >> 8) as usize;
    match v & 0xFF {
        SPAWN_FLOWER_RING => Some(Spawn::FlowerRing { flowers: n.clamp(2, 12) }),
        SPAWN_SINGLE => Some(Spawn::SingleFlower),
        SPAWN_ENEMY_RING => Some(Spawn::EnemyRing { heads: n.clamp(2, 8) }),
        SPAWN_TOWER => Some(Spawn::Tower),
        _ => None,
    }
}

/// Clip lengths in ticks (a clip's frames * 2: the clips run at 60 a second, the sim at 120).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SmsFlowerClips {
    pub flower_open: u32,
    pub tower_appear: u32,
    pub tower_hide: u32,
    pub tower_attack_start: u32,
    pub tower_attack: u32,
    pub tower_attack_end: u32,
    pub tower_freeze: u32,
    pub tower_die: u32,
}

/// One flower, tower or head: what to draw, and what a client's mirror is made from.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SmsFlowerItem {
    /// 0 flower, 1 tower, 2 head.
    pub kind: u8,
    /// The phase / `TowerState` / `HeadState`, in the enums' order.
    pub state: u8,
    pub _pad: [u8; 2],
    /// A handle that stays the same while the thing lives (flower: unit * 64 + flower).
    pub handle: u32,
    /// Ticks in that state.
    pub age: u32,
    pub pos: SmsVec3,
    /// Facing in degrees (about +Y, 0 along +Z).
    pub yaw: f32,
}

/// Something that happened ([`sms_fw_events`]).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SmsFlowerEvent {
    /// 1 bloomed (`a`: flowers of its unit still shut), 2 wilted, 3 a coin leaves a ring (pos, vel),
    /// 4 a tower rose, 5 a tower sank, 6 a bite (`a` 0 a tower's, 1 a head's; pos where from;
    /// `target` which of the Marios passed to the step), 7 something died (pos; `a` 1 tower /
    /// 2 head), 8 a head hopped, 9 a tower was frozen by water.
    pub kind: u32,
    pub a: i32,
    pub target: i32,
    pub pos: SmsVec3,
    pub vel: SmsVec3,
}

/// A hit a sim reported: 0 water, 1 a stomp or other attack.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SmsFlowerHit {
    pub id: u64,
    pub kind: u32,
    pub _pad: u32,
}

/// An opaque flower world.
pub struct SmsFlowers {
    pub(crate) world: FlowerWorld,
    events: Vec<SmsFlowerEvent>,
    table: libsms::flower::SpawnTable,
    clock: FixedClock,
    /// Hits sims reported since the host last took them.
    pub(crate) hits: Vec<(u64, u8)>,
}

impl SmsFlowers {
    fn push(&mut self, e: &FlowerEvent) {
        let ev = |kind: u32, a: i32, target: i32, pos: Vec3, vel: Vec3| SmsFlowerEvent {
            kind,
            a,
            target,
            pos: pos.into(),
            vel: vel.into(),
        };
        match e {
            FlowerEvent::Bloomed { pos, still_shut, .. } => {
                self.events.push(ev(1, *still_shut as i32, 0, *pos, Vec3::ZERO))
            }
            FlowerEvent::Wilted { unit, flower } => {
                let pos = self.world.units[*unit].flowers[*flower].pos;
                self.events.push(ev(2, 0, 0, pos, Vec3::ZERO));
            }
            FlowerEvent::CoinRing { coins, .. } => {
                self.events.extend(coins.iter().map(|c| ev(3, 0, 0, c.pos, c.vel)))
            }
            FlowerEvent::TowerAppeared { pos, .. } => self.events.push(ev(4, 0, 0, *pos, Vec3::ZERO)),
            FlowerEvent::TowerHid { pos, .. } => self.events.push(ev(5, 0, 0, *pos, Vec3::ZERO)),
            FlowerEvent::TowerBite { from, target, .. } => {
                self.events.push(ev(6, 0, *target as i32, *from, Vec3::ZERO))
            }
            FlowerEvent::HeadBit { from, target, .. } => {
                self.events.push(ev(6, 1, *target as i32, *from, Vec3::ZERO))
            }
            FlowerEvent::EnemyDied { id, pos } => {
                let kind = if *id >= libsms::flower::ID_BASE + 0x20000 { 2 } else { 1 };
                self.events.push(ev(7, kind, 0, *pos, Vec3::ZERO));
            }
            FlowerEvent::HeadHopped { pos, .. } => self.events.push(ev(8, 0, 0, *pos, Vec3::ZERO)),
        }
        if self.events.len() > 512 {
            self.events.drain(..256);
        }
    }
}

/// A new, empty world with the core's defaults (see [`sms_fw_setup`]).
#[unsafe(no_mangle)]
pub extern "C" fn sms_fw_new() -> *mut SmsFlowers {
    Box::into_raw(Box::new(SmsFlowers {
        world: FlowerWorld::new(),
        events: Vec::new(),
        table: libsms::flower::SpawnTable::default(),
        clock: FixedClock::default(),
        hits: Vec::new(),
    }))
}

/// # Safety
/// `fw` from [`sms_fw_new`], not used afterwards, and no sim still holding it
/// ([`sms_sim_set_flowers`] with null first). Null is ignored.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_fw_free(fw: *mut SmsFlowers) {
    if !fw.is_null() {
        // SAFETY: the caller hands back ownership.
        drop(unsafe { Box::from_raw(fw) });
    }
}

/// Gives `sim` the world its water and stomps hit (null to let go). The host keeps the world
/// alive while a sim holds it.
///
/// # Safety
/// `sim` from `sms_sim_new`; `fw` null or from [`sms_fw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_sim_set_flowers(sim: *mut SmsSim, fw: *mut SmsFlowers) {
    // SAFETY: as documented.
    if let Some(s) = unsafe { sim.as_mut() } {
        s.flower_world = fw;
    }
}

/// Empties the world and reads the three `.prm` files from `assets_dir`'s `params_enemy/`, with
/// the clip lengths in ticks. Call once per stage.
///
/// # Safety
/// `fw` from [`sms_fw_new`]; `assets_dir` nul-terminated or null; `clips` readable or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_fw_setup(
    fw: *mut SmsFlowers,
    assets_dir: *const c_char,
    clips: *const SmsFlowerClips,
) {
    // SAFETY: as documented.
    let Some(s) = (unsafe { fw.as_mut() }) else { return };
    let load = |name: &str| -> Option<libsms::prm::Prm> {
        if assets_dir.is_null() {
            return None;
        }
        // SAFETY: as documented.
        let dir = unsafe { CStr::from_ptr(assets_dir) }.to_string_lossy().into_owned();
        let data = std::fs::read(Path::new(&dir).join("params_enemy").join(name)).ok()?;
        libsms::prm::Prm::parse(&data).ok()
    };
    let mut w = FlowerWorld::new().with_params(
        load("samboflower.prm").as_ref(),
        load("hanasambo.prm").as_ref(),
        load("sambohead.prm").as_ref(),
    );
    if !assets_dir.is_null() {
        // SAFETY: as documented.
        let dir = unsafe { CStr::from_ptr(assets_dir) }.to_string_lossy().into_owned();
        w = w.with_rig(Path::new(&dir));
    }
    // SAFETY: as documented.
    if let Some(c) = unsafe { clips.as_ref() } {
        let d = Clips::default();
        let t = |v: u32, d: u32| if v == 0 { d } else { v };
        w.clips = Clips {
            flower_open: t(c.flower_open, d.flower_open),
            tower_appear: t(c.tower_appear, d.tower_appear),
            tower_hide: t(c.tower_hide, d.tower_hide),
            tower_attack_start: t(c.tower_attack_start, d.tower_attack_start),
            tower_attack: t(c.tower_attack, d.tower_attack),
            tower_attack_end: t(c.tower_attack_end, d.tower_attack_end),
            tower_freeze: t(c.tower_freeze, d.tower_freeze),
            tower_die: t(c.tower_die, d.tower_die),
        };
    }
    s.world = w;
    s.events.clear();
    s.hits.clear();
}

/// The rare roll's odds: `chance_per_mille` of any spawn per roll, and weights for a flower
/// ring, a single flower, an enemy ring and a tower.
///
/// # Safety
/// `fw` from [`sms_fw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_fw_set_table(
    fw: *mut SmsFlowers,
    chance_per_mille: u32,
    w_ring: u32,
    w_single: u32,
    w_enemy_ring: u32,
    w_tower: u32,
) {
    // SAFETY: as documented.
    if let Some(s) = unsafe { fw.as_mut() } {
        s.table = libsms::flower::SpawnTable {
            chance_per_mille,
            weights: [w_ring, w_single, w_enemy_ring, w_tower],
        };
    }
}

/// One roll with `r` and `s` uniform in [0, 1): 0 for nothing, else a spawn to hand to
/// [`sms_fw_place`] (kind in the low byte, size above).
///
/// # Safety
/// `fw` from [`sms_fw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_fw_roll(fw: *const SmsFlowers, r: f32, s: f32) -> u32 {
    // SAFETY: as documented.
    match unsafe { fw.as_ref() } {
        Some(f) if r.is_finite() && s.is_finite() => f
            .table
            .pick(r.clamp(0.0, 0.9999), s.clamp(0.0, 0.9999))
            .map_or(0, pack),
        _ => 0,
    }
}

/// Puts a spawn at `pos` (sim units), a ring turned `yaw` degrees. Returns how many things it
/// added (0 for an unknown spawn or a full world).
///
/// # Safety
/// `fw` from [`sms_fw_new`]; `pos` readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_fw_place(
    fw: *mut SmsFlowers,
    spawn: u32,
    pos: *const SmsVec3,
    yaw: f32,
) -> u32 {
    // SAFETY: as documented.
    let (Some(s), Some(p), Some(sp)) =
        (unsafe { fw.as_mut() }, unsafe { pos.as_ref() }, unpack(spawn))
    else {
        return 0;
    };
    let at = Vec3::from(*p);
    if !at.is_finite() || !yaw.is_finite() {
        return 0;
    }
    let w = &mut s.world;
    if w.units.len() + w.towers.len() + w.heads.len() > 48 {
        return 0;
    }
    w.place(sp, at, yaw) as u32
}

/// Steps the world by `dt` seconds (as many 120 Hz ticks as are due) with every Mario's feet
/// (`marios`, `count` of them: the enemies wake for and bite the nearest). The server only: a
/// client's world is a mirror. Returns the ticks run.
///
/// # Safety
/// `fw` from [`sms_fw_new`]; `host` a valid host (its ground callback is used); `marios` has
/// `count` entries.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_fw_step(
    fw: *mut SmsFlowers,
    host: *const SmsHost,
    marios: *const SmsVec3,
    count: u32,
    dt: f32,
) -> u32 {
    // SAFETY: as documented.
    let (Some(s), Some(host)) = (unsafe { fw.as_mut() }, unsafe { host.as_ref() }) else {
        return 0;
    };
    let feet: Vec<Vec3> = if marios.is_null() {
        Vec::new()
    } else {
        // SAFETY: as documented.
        unsafe { std::slice::from_raw_parts(marios, count.min(64) as usize) }
            .iter()
            .map(|v| Vec3::from(*v))
            .filter(|v| v.is_finite())
            .collect()
    };
    let n = s.clock.accumulate(dt);
    let h = FfiHost(host, Vec::new(), Vec::new());
    let mut out = Vec::new();
    for _ in 0..n {
        s.world.step(&h, &feet, &mut out);
    }
    for e in &out {
        s.push(e);
    }
    n
}

/// Applies a hit (0 water, 1 a stomp or other attack) on `id` where the world is stepped.
///
/// # Safety
/// `fw` from [`sms_fw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_fw_hit(fw: *mut SmsFlowers, id: u64, kind: u32) {
    // SAFETY: as documented.
    let Some(s) = (unsafe { fw.as_mut() }) else { return };
    if !FlowerWorld::owns(id) {
        return;
    }
    let mut out = Vec::new();
    s.world.hit(id, kind.min(1) as u8, &mut out);
    for e in &out {
        s.push(e);
    }
}

/// Takes up to `max` hits the sims holding this world reported into `out`. Returns how many.
///
/// # Safety
/// `fw` from [`sms_fw_new`]; `out` has room for `max`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_fw_take_hits(
    fw: *mut SmsFlowers,
    out: *mut SmsFlowerHit,
    max: u32,
) -> u32 {
    // SAFETY: as documented.
    let Some(s) = (unsafe { fw.as_mut() }) else { return 0 };
    if out.is_null() {
        return 0;
    }
    let n = s.hits.len().min(max as usize);
    for (i, (id, kind)) in s.hits.drain(..n).enumerate() {
        // SAFETY: `out` has room for `max`.
        unsafe { out.add(i).write(SmsFlowerHit { id, kind: kind as u32, _pad: 0 }) };
    }
    if s.hits.len() > 256 {
        s.hits.clear();
    }
    n as u32
}

/// How many of `kind` (0 flowers, 1 towers, 2 heads) there are.
///
/// # Safety
/// `fw` from [`sms_fw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_fw_count(fw: *const SmsFlowers, kind: u8) -> u32 {
    // SAFETY: as documented.
    let Some(s) = (unsafe { fw.as_ref() }) else { return 0 };
    (match kind {
        0 => s.world.units.iter().map(|u| u.flowers.len()).sum(),
        1 => s.world.towers.len(),
        2 => s.world.heads.len(),
        _ => 0,
    }) as u32
}

fn item_of(i: &Item) -> SmsFlowerItem {
    SmsFlowerItem {
        kind: i.kind,
        state: i.state,
        handle: i.handle,
        age: i.age,
        pos: i.pos.into(),
        yaw: i.yaw,
        ..Default::default()
    }
}

/// Every thing (flowers, then towers, then heads) into `out`, up to `max`. Returns how many there
/// are (more than `max` means some did not fit). The server sends this as the snapshot.
///
/// # Safety
/// `fw` from [`sms_fw_new`]; `out` has room for `max` or is null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_fw_items(
    fw: *const SmsFlowers,
    out: *mut SmsFlowerItem,
    max: u32,
) -> u32 {
    // SAFETY: as documented.
    let Some(s) = (unsafe { fw.as_ref() }) else { return 0 };
    let items = s.world.items();
    if !out.is_null() {
        for (i, it) in items.iter().take(max as usize).enumerate() {
            // SAFETY: `out` has room for `max`.
            unsafe { out.add(i).write(item_of(it)) };
        }
    }
    items.len() as u32
}

/// The `i`th of `kind`. Returns 0 past the end.
///
/// # Safety
/// `fw` from [`sms_fw_new`]; `out` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_fw_get(
    fw: *const SmsFlowers,
    kind: u8,
    i: u32,
    out: *mut SmsFlowerItem,
) -> u8 {
    // SAFETY: as documented.
    let (Some(s), Some(out)) = (unsafe { fw.as_ref() }, unsafe { out.as_mut() }) else {
        return 0;
    };
    let items = s.world.items();
    match items.iter().filter(|it| it.kind == kind).nth(i as usize) {
        Some(it) => {
            *out = item_of(it);
            1
        }
        None => 0,
    }
}

/// Makes this world a copy of a snapshot (a client's: it is not stepped).
///
/// # Safety
/// `fw` from [`sms_fw_new`]; `items` has `count` entries.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_fw_mirror(fw: *mut SmsFlowers, items: *const SmsFlowerItem, count: u32) {
    // SAFETY: as documented.
    let Some(s) = (unsafe { fw.as_mut() }) else { return };
    if items.is_null() {
        return;
    }
    // SAFETY: as documented.
    let list: Vec<Item> = unsafe { std::slice::from_raw_parts(items, count.min(512) as usize) }
        .iter()
        .filter(|i| Vec3::from(i.pos).is_finite() && i.yaw.is_finite())
        .map(|i| Item {
            kind: i.kind,
            state: i.state,
            handle: i.handle,
            age: i.age,
            pos: i.pos.into(),
            yaw: i.yaw,
        })
        .collect();
    s.world.mirror(&list);
}

/// Takes up to `max` pending events (oldest first) into `out`. Returns how many.
///
/// # Safety
/// `fw` from [`sms_fw_new`]; `out` has room for `max`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_fw_events(
    fw: *mut SmsFlowers,
    out: *mut SmsFlowerEvent,
    max: u32,
) -> u32 {
    // SAFETY: as documented.
    let Some(s) = (unsafe { fw.as_mut() }) else { return 0 };
    if out.is_null() {
        return 0;
    }
    let n = s.events.len().min(max as usize);
    for (i, e) in s.events.drain(..n).enumerate() {
        // SAFETY: `out` has room for `max`.
        unsafe { out.add(i).write(e) };
    }
    n as u32
}
