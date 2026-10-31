//! The sambo flowers (libsms::flower) through the C ABI: placed by the host, stepped inside
//! `sms_sim_step`, read back as items and events. Water and stomps reach them through the
//! `Host::enemies` list the sim already queries, with ids from `libsms::flower::ID_BASE` up.

use std::ffi::{CStr, c_char};
use std::path::Path;

use glam::Vec3;
use libsms::flower::{Clips, FlowerEvent, Spawn};

use crate::{SmsSim, SmsVec3};

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

/// One flower, tower or head to draw.
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

/// Something that happened ([`sms_flowers_events`]).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SmsFlowerEvent {
    /// 1 bloomed (`a`: flowers of its unit still shut), 2 wilted, 3 a coin leaves a ring (pos, vel),
    /// 4 a tower rose, 5 a tower sank, 6 a bite (`a` 0 a tower's, 1 a head's; pos where from),
    /// 7 something died (pos; `a` 1 tower / 2 head), 8 a head hopped.
    pub kind: u32,
    pub a: i32,
    pub pos: SmsVec3,
    pub vel: SmsVec3,
}

impl SmsSim {
    pub(crate) fn push_flower_event(&mut self, e: &FlowerEvent) {
        let ev = |kind: u32, a: i32, pos: Vec3, vel: Vec3| SmsFlowerEvent {
            kind,
            a,
            pos: pos.into(),
            vel: vel.into(),
        };
        match e {
            FlowerEvent::Bloomed { pos, still_shut, .. } => {
                self.flower_events.push(ev(1, *still_shut as i32, *pos, Vec3::ZERO))
            }
            FlowerEvent::Wilted { unit, flower } => {
                let pos = self.flowers.units[*unit].flowers[*flower].pos;
                self.flower_events.push(ev(2, 0, pos, Vec3::ZERO));
            }
            FlowerEvent::CoinRing { coins, .. } => {
                self.flower_events.extend(coins.iter().map(|c| ev(3, 0, c.pos, c.vel)))
            }
            FlowerEvent::TowerAppeared { pos, .. } => {
                self.flower_events.push(ev(4, 0, *pos, Vec3::ZERO))
            }
            FlowerEvent::TowerHid { pos, .. } => self.flower_events.push(ev(5, 0, *pos, Vec3::ZERO)),
            FlowerEvent::TowerBite { from, .. } => {
                self.flower_events.push(ev(6, 0, *from, Vec3::ZERO))
            }
            FlowerEvent::HeadBit { from, .. } => {
                self.flower_events.push(ev(6, 1, *from, Vec3::ZERO))
            }
            FlowerEvent::EnemyDied { id, pos } => {
                let kind = if *id >= libsms::flower::ID_BASE + 0x20000 { 2 } else { 1 };
                self.flower_events.push(ev(7, kind, *pos, Vec3::ZERO));
            }
            FlowerEvent::HeadHopped { pos, .. } => {
                self.flower_events.push(ev(8, 0, *pos, Vec3::ZERO))
            }
        }
        if self.flower_events.len() > 512 {
            self.flower_events.drain(..256);
        }
    }
}

/// Empties the world and reads the three `.prm` files from `assets_dir`'s `params_enemy/`, with
/// the clip lengths in ticks. Call once per stage.
///
/// # Safety
/// `sim` from `sms_sim_new`; `assets_dir` nul-terminated or null; `clips` readable or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_flowers_setup(
    sim: *mut SmsSim,
    assets_dir: *const c_char,
    clips: *const SmsFlowerClips,
) {
    // SAFETY: as documented.
    let Some(s) = (unsafe { sim.as_mut() }) else { return };
    let load = |name: &str| -> Option<libsms::prm::Prm> {
        if assets_dir.is_null() {
            return None;
        }
        // SAFETY: as documented.
        let dir = unsafe { CStr::from_ptr(assets_dir) }.to_string_lossy().into_owned();
        let data = std::fs::read(Path::new(&dir).join("params_enemy").join(name)).ok()?;
        libsms::prm::Prm::parse(&data).ok()
    };
    let mut w = libsms::flower::FlowerWorld::new().with_params(
        load("samboflower.prm").as_ref(),
        load("hanasambo.prm").as_ref(),
        load("sambohead.prm").as_ref(),
    );
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
    s.flowers = w;
    s.flower_events.clear();
}

/// The rare roll's odds: `chance_per_mille` of any spawn per roll, and weights for a flower
/// ring, a single flower, an enemy ring and a tower.
///
/// # Safety
/// `sim` from `sms_sim_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_flowers_set_table(
    sim: *mut SmsSim,
    chance_per_mille: u32,
    w_ring: u32,
    w_single: u32,
    w_enemy_ring: u32,
    w_tower: u32,
) {
    // SAFETY: as documented.
    if let Some(s) = unsafe { sim.as_mut() } {
        s.flower_table = libsms::flower::SpawnTable {
            chance_per_mille,
            weights: [w_ring, w_single, w_enemy_ring, w_tower],
        };
    }
}

/// One roll with `r` and `s` uniform in [0, 1): 0 for nothing, else a spawn to hand to
/// [`sms_flowers_place`] (kind in the low byte, size above).
///
/// # Safety
/// `sim` from `sms_sim_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_flowers_roll(sim: *const SmsSim, r: f32, s: f32) -> u32 {
    // SAFETY: as documented.
    match unsafe { sim.as_ref() } {
        Some(sim) if r.is_finite() && s.is_finite() => sim
            .flower_table
            .pick(r.clamp(0.0, 0.9999), s.clamp(0.0, 0.9999))
            .map_or(0, pack),
        _ => 0,
    }
}

/// Puts a spawn at `pos` (sim units), a ring turned `yaw` degrees. Returns how many things it
/// added (0 for an unknown spawn or a full world).
///
/// # Safety
/// `sim` from `sms_sim_new`; `pos` readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_flowers_place(
    sim: *mut SmsSim,
    spawn: u32,
    pos: *const SmsVec3,
    yaw: f32,
) -> u32 {
    // SAFETY: as documented.
    let (Some(s), Some(p), Some(sp)) =
        (unsafe { sim.as_mut() }, unsafe { pos.as_ref() }, unpack(spawn))
    else {
        return 0;
    };
    let at = Vec3::from(*p);
    if !at.is_finite() || !yaw.is_finite() {
        return 0;
    }
    if s.flowers.units.len() + s.flowers.towers.len() + s.flowers.heads.len() > 48 {
        return 0;
    }
    s.flowers.place(sp, at, yaw) as u32
}

/// How many of `kind` (0 flowers, 1 towers, 2 heads) there are.
///
/// # Safety
/// `sim` from `sms_sim_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_flowers_count(sim: *const SmsSim, kind: u8) -> u32 {
    // SAFETY: as documented.
    let Some(s) = (unsafe { sim.as_ref() }) else { return 0 };
    (match kind {
        0 => s.flowers.units.iter().map(|u| u.flowers.len()).sum(),
        1 => s.flowers.towers.len(),
        2 => s.flowers.heads.len(),
        _ => 0,
    }) as u32
}

/// The `i`th of `kind`. Returns 0 past the end.
///
/// # Safety
/// `sim` from `sms_sim_new`; `out` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_flowers_get(
    sim: *const SmsSim,
    kind: u8,
    i: u32,
    out: *mut SmsFlowerItem,
) -> u8 {
    // SAFETY: as documented.
    let (Some(s), Some(out)) = (unsafe { sim.as_ref() }, unsafe { out.as_mut() }) else {
        return 0;
    };
    let i = i as usize;
    let w = &s.flowers;
    *out = match kind {
        0 => {
            let mut left = i;
            let mut found = None;
            for (ui, u) in w.units.iter().enumerate() {
                if left < u.flowers.len() {
                    let f = &u.flowers[left];
                    found = Some(SmsFlowerItem {
                        kind: 0,
                        state: f.phase as u8,
                        handle: (ui * 64 + left) as u32,
                        age: f.age,
                        pos: f.pos.into(),
                        ..Default::default()
                    });
                    break;
                }
                left -= u.flowers.len();
            }
            match found {
                Some(f) => f,
                None => return 0,
            }
        }
        1 => match w.towers.get(i) {
            Some(t) => SmsFlowerItem {
                kind: 1,
                state: t.state as u8,
                handle: i as u32,
                age: t.age,
                pos: t.pos.into(),
                yaw: t.yaw,
                ..Default::default()
            },
            None => return 0,
        },
        2 => match w.heads.get(i) {
            Some(h) => SmsFlowerItem {
                kind: 2,
                state: h.state as u8,
                handle: i as u32,
                age: h.age,
                pos: h.pos.into(),
                yaw: h.yaw,
                ..Default::default()
            },
            None => return 0,
        },
        _ => return 0,
    };
    1
}

/// Takes up to `max` pending events (oldest first) into `out`. Returns how many.
///
/// # Safety
/// `sim` from `sms_sim_new`; `out` has room for `max`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_flowers_events(
    sim: *mut SmsSim,
    out: *mut SmsFlowerEvent,
    max: u32,
) -> u32 {
    // SAFETY: as documented.
    let Some(s) = (unsafe { sim.as_mut() }) else { return 0 };
    if out.is_null() {
        return 0;
    }
    let n = s.flower_events.len().min(max as usize);
    for (i, e) in s.flower_events.drain(..n).enumerate() {
        // SAFETY: `out` has room for `max`.
        unsafe { out.add(i).write(e) };
    }
    n as u32
}
