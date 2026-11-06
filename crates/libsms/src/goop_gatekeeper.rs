//! The big goop Piranha Plant, on Bowser Jr.'s side: it grows out of a big enough pile of
//! goop, sways, turns to face what's near and hurls goop at it from its mouth, then dives back
//! in when its time is up or its goop is cleaned away.
//!
//! From `TBiancoGateKeeper` (Enemy/gatekeeper.cpp), the Gatekeeper of Bianco Hills and the
//! goop gates; its clips are `gatekeeper/gene_pakkun_*.bck` (index order in [`keeper_clip`]).
//! The game's own nerves, at their times (the spine counts 60 Hz frames, two 120 Hz ticks; its
//! clips play at half a frame a spine frame, so a `.bck` frame is four ticks):
//! * **Appear** (`TNerveBGKAppear`): `gene_pakkun_appear1`; `ms_gkpa_kemuri` at spine frame 8.
//! * **Wait** (`TNerveBGKWait`): the swaying loop, `gene_pakkun_wait3` (clip 0x12). Its drool
//!   and splashes are `emitParticles` (`ms_gkpa_bota` at the head, joint 6; `ms_gkpa_yodare_*`
//!   at the jaws, 5/8/9), every frame outside sleep and the launch.
//! * **Launch** (`TNerveBGKLaunchGoro`): `gene_pakkun_shoot1`, and `launchGorogoro` at spine
//!   frame 0x34 sends a rolling goop ball out of it.
//! * **Dive** (`TNerveBGKDive`): `gene_pakkun_dive1`; `ms_gkpa_kemuri` at spine frame 0xF0;
//!   gone when the clip ends.
//!
//! Host behaviour (not the game's; the disc's Gatekeeper sits in one place for a whole level):
//! one grows where Bowser Jr.'s goop covers at least [`KeeperParams::pile_texels`] texels within
//! [`KeeperParams::pile_radius`], near its owner; it lives [`KeeperParams::life`] ticks or until
//! its pile is cleaned below half; it picks targets like the other critters and its launch is a
//! goop blob ([`super::ShotKind::Paint`] of the critters' goop) arced at the target, not a Gorogoro.
//! It isn't hurt by water or stomps (the game's needs the goop under it cleaned).

use glam::Vec3;

use super::{CritterEvent, Critters, Target, turn, yaw_to};
use crate::goop::GoopField;

/// The Gatekeeper's clips, in its `.bck`s' name order (`changeBck` indices).
pub mod keeper_clip {
    pub const APPEAR: u8 = 0;
    pub const DIVE: u8 = 6;
    pub const SHOOT: u8 = 8;
    pub const WAIT: u8 = 0x12;
    pub const NAMES: [&str; 19] = [
        "appear1", "appear1_end", "appear1_start", "damage1", "damage2", "dead1", "dive1", "iyaiya1",
        "shoot1", "towait2", "wait1", "wait2", "wait2_close", "wait2_end", "wait2_loop", "wait2_open",
        "wait2_openloop", "wait2_start", "wait3",
    ];
    /// Which loop.
    pub const LOOPS: [bool; 19] = [
        false, false, false, false, false, false, false, false, false, false, true, true, false, false,
        true, false, true, false, true,
    ];
}

/// 120 Hz ticks a `.bck` frame takes (half a frame a 60 Hz spine frame).
pub const KEEPER_TICKS_PER_FRAME: u32 = 4;

/// Host tuning for the Gatekeeper (see the module).
#[derive(Debug, Clone, PartialEq)]
pub struct KeeperParams {
    /// One grows only where this many goop texels lie within `pile_radius`.
    pub pile_radius: f32,
    pub pile_texels: u32,
    /// How far from the owner a pile is looked for.
    pub search_radius: f32,
    /// Ticks between looks for a pile, and after one dives before another may grow.
    pub look_every: u32,
    pub cooldown: u32,
    pub max: usize,
    /// Ticks it stays up.
    pub life: u32,
    /// Its size (the disc's is a boss; this one fits among the host's enemies).
    pub scale: f32,
    /// Reach, ticks between launches, turn rate (degrees a tick).
    pub shoot_range: f32,
    pub shoot_every: u32,
    pub turn_speed: f32,
    /// The mouth over its root, and forward of it, at scale 1.
    pub mouth_height: f32,
    pub mouth_forward: f32,
    /// The goop ball: speed a tick, gravity, splat radius, damage share.
    pub ball_speed: f32,
    pub ball_gravity: f32,
    pub ball_splat: f32,
    pub ball_damage: f32,
}

impl Default for KeeperParams {
    fn default() -> Self {
        KeeperParams {
            pile_radius: 400.0,
            pile_texels: 110,
            search_radius: 2500.0,
            look_every: 60,
            cooldown: 1800,
            max: 1,
            life: 2400,
            scale: 0.7,
            shoot_range: 3500.0,
            shoot_every: 300,
            turn_speed: 1.0,
            mouth_height: 520.0,
            mouth_forward: 160.0,
            ball_speed: 32.0,
            ball_gravity: 0.3,
            ball_splat: 260.0,
            ball_damage: 2.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeeperState {
    Appear,
    Wait,
    Shoot,
    Dive,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GateKeeper {
    pub id: u64,
    pub pos: Vec3,
    pub yaw: f32,
    pub scale: f32,
    pub state: KeeperState,
    pub clip: u8,
    /// 120 Hz ticks into the clip ([`KEEPER_TICKS_PER_FRAME`] a frame).
    pub clip_time: u32,
    pub target: Option<u64>,
    /// Ticks up.
    pub age: u32,
    since_shot: u32,
    aim: Vec3,
    /// The pile's texels when it grew.
    pile: u32,
}

impl GateKeeper {
    /// Its clip's frame now.
    pub fn frame(&self) -> f32 {
        self.clip_time as f32 / KEEPER_TICKS_PER_FRAME as f32
    }

    /// Where its goop leaves from.
    pub fn mouth(&self, p: &KeeperParams) -> Vec3 {
        self.pos + Vec3::Y * p.mouth_height * self.scale + super::dir_of(self.yaw) * p.mouth_forward * self.scale
    }
}

/// Clip lengths in ticks, read from the extracted clips (missing ones keep 400).
pub fn keeper_clip_ticks(assets: &std::path::Path) -> [u32; 19] {
    let mut out = [400; 19];
    for (i, n) in keeper_clip::NAMES.iter().enumerate() {
        if let Some(c) = std::fs::read(assets.join(format!("enemy/gatekeeper/gene_pakkun_{n}.bck")))
            .ok()
            .and_then(|d| crate::skeleton::Clip::parse(&d))
        {
            out[i] = ((c.frames * KEEPER_TICKS_PER_FRAME as f32) as u32).max(1);
        }
    }
    out
}

impl Critters {
    /// Grows a Gatekeeper at `pos` (a pile's middle), facing `yaw`.
    pub fn spawn_keeper(&mut self, pos: Vec3, yaw: f32, pile: u32) -> Option<u64> {
        if self.keepers.len() >= self.keeper_params.max {
            return None;
        }
        let id = self.id();
        self.keepers.push(GateKeeper {
            id,
            pos,
            yaw,
            scale: self.keeper_params.scale,
            state: KeeperState::Appear,
            clip: keeper_clip::APPEAR,
            clip_time: 0,
            target: None,
            age: 0,
            since_shot: 0,
            aim: pos,
            pile,
        });
        self.events.push(CritterEvent::Spawned { id, kind: super::CritterKind::GateKeeper, pos });
        self.sound("MSD_SE_EN_GATEKEEPER_APPEAR", pos);
        Some(id)
    }

    /// Looks for a pile near `owner` now and then, and runs the Gatekeepers a tick.
    pub(super) fn step_keepers(&mut self, goop: &GoopField, owner: Option<Vec3>, targets: &[Target]) {
        let p = self.keeper_params.clone();
        self.keeper_cooldown = self.keeper_cooldown.saturating_sub(1);
        self.keeper_look = self.keeper_look.saturating_add(1);
        if let Some(o) = owner
            && self.keeper_spawning
            && self.keeper_cooldown == 0
            && self.keepers.len() < p.max
            && self.keeper_look >= p.look_every
        {
            self.keeper_look = 0;
            // A few goop spots near the owner; the most covered one wins.
            let mut best: Option<(Vec3, u32)> = None;
            for _ in 0..6 {
                let mut seed = self.seed;
                let spot = goop.random_spot(o, p.search_radius, || {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    (seed >> 8) as f32 / (1u32 << 24) as f32
                });
                self.seed = seed;
                let Some(s) = spot else { break };
                let (n, middle) = goop.coverage(s, p.pile_radius);
                if n >= p.pile_texels && best.is_none_or(|(_, b)| n > b) {
                    best = Some((middle, n));
                }
            }
            if let Some((at, n)) = best {
                let yaw = yaw_to(at, o) + std::f32::consts::PI;
                self.spawn_keeper(at, yaw, n);
            }
        }

        let ticks = self.keeper_clips;
        let mut list = std::mem::take(&mut self.keepers);
        let mut gone = Vec::new();
        let mut balls: Vec<(Vec3, Vec3, u64)> = Vec::new();
        for k in list.iter_mut() {
            k.clip_time += 1;
            k.age += 1;
            k.since_shot += 1;
            let len = ticks[k.clip as usize];
            let clip_done = k.clip_time >= len;
            let t = self.pick(k.pos, p.shoot_range, 2000.0, targets);
            k.target = t.map(|t| t.id);
            if let Some(t) = t
                && matches!(k.state, KeeperState::Wait | KeeperState::Shoot)
            {
                k.yaw = turn(k.yaw, yaw_to(k.pos, t.pos), p.turn_speed.to_radians());
            }
            // Its pile cleaned below half sends it under early.
            let cleaned = k.age % 30 == 0 && goop.coverage(k.pos, p.pile_radius).0 * 2 < k.pile;
            let spine = k.clip_time / 2;
            match k.state {
                KeeperState::Appear => {
                    if k.clip_time == 16 {
                        let at = k.pos + Vec3::Y * 100.0 * k.scale;
                        self.effect("ms_gkpa_kemuri", at, k.scale);
                    }
                    if clip_done {
                        k.state = KeeperState::Wait;
                        k.clip = keeper_clip::WAIT;
                        k.clip_time = 0;
                        k.since_shot = p.shoot_every / 2;
                    }
                }
                KeeperState::Wait | KeeperState::Shoot if k.age >= p.life || cleaned => {
                    k.state = KeeperState::Dive;
                    k.clip = keeper_clip::DIVE;
                    k.clip_time = 0;
                    self.sound("MSD_SE_EN_GATEKEEPER_HIDE", k.pos);
                }
                KeeperState::Wait => {
                    if clip_done {
                        k.clip_time = 0;
                    }
                    if k.clip_time == 1 {
                        self.sound("MSD_SE_EN_GATEKEEPER_WAIT", k.pos);
                    }
                    if let Some(t) = t
                        && k.since_shot >= p.shoot_every
                    {
                        k.state = KeeperState::Shoot;
                        k.clip = keeper_clip::SHOOT;
                        k.clip_time = 0;
                        k.aim = t.pos + Vec3::Y * t.height * 0.5;
                    }
                }
                KeeperState::Shoot => {
                    // `launchGorogoro` at spine frame 0x34.
                    if k.clip_time == 0x34 * 2 {
                        let from = k.mouth(&p);
                        balls.push((from, ball_velocity(from, k.aim, k.yaw, p.ball_speed, p.ball_gravity), k.id));
                        self.effect("ms_gkpa_bota", from, k.scale);
                        k.since_shot = 0;
                    }
                    if clip_done {
                        k.state = KeeperState::Wait;
                        k.clip = keeper_clip::WAIT;
                        k.clip_time = 0;
                    }
                }
                KeeperState::Dive => {
                    if spine == 0xF0 && k.clip_time % 2 == 0 {
                        self.effect("ms_gkpa_kemuri", k.pos, k.scale);
                    }
                    if clip_done {
                        gone.push(k.id);
                        self.events.push(CritterEvent::Died { id: k.id, pos: k.pos, melted: false });
                        continue;
                    }
                }
            }
            // `emitParticles`: drool from the jaws while it's up (not mid-launch).
            if matches!(k.state, KeeperState::Wait) && k.age % 45 == 0 {
                let mouth = k.mouth(&p);
                self.effect(if k.age % 90 == 0 { "ms_gkpa_yodare_l" } else { "ms_gkpa_yodare_s" }, mouth, k.scale);
            }
        }
        if !gone.is_empty() {
            self.keeper_cooldown = p.cooldown;
        }
        list.retain(|k| !gone.contains(&k.id));
        self.keepers = list;
        let kind = self.goop_kind;
        for (from, vel, owner) in balls {
            self.shoot_paint(from, vel, p.ball_gravity, p.ball_splat, kind, p.ball_damage, owner);
            self.sound("MSD_SE_EN_PAKKUN_SEED_FLY", from);
        }
    }
}

/// An arc from `from` that falls onto `to` under `gravity` at about `speed` a tick.
fn ball_velocity(from: Vec3, to: Vec3, yaw: f32, speed: f32, gravity: f32) -> Vec3 {
    let d = to - from;
    if d.length() < 50.0 {
        return super::dir_of(yaw) * speed;
    }
    let ticks = (d.length() / speed.max(1.0)).clamp(1.0, 240.0);
    d / ticks + Vec3::Y * (gravity * (ticks + 1.0) * 0.5)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goop::GoopKind;
    use crate::host::FlatHost;

    fn critters() -> Critters {
        let mut c = Critters::default();
        c.spawn_interval = 0;
        c.keeper_spawning = true;
        c
    }

    #[test]
    fn a_gatekeeper_grows_only_from_a_big_pile() {
        let host = FlatHost::default();
        let mut g = GoopField::new();
        let mut c = critters();
        g.stamp(&host, Vec3::new(500.0, 0.0, 0.0), 120.0, GoopKind::Sink);
        for _ in 0..600 {
            c.step(&host, &mut g, Some(Vec3::ZERO), &[]);
        }
        assert!(c.keepers.is_empty(), "a small splat grows none");
        g.stamp(&host, Vec3::new(500.0, 0.0, 0.0), 420.0, GoopKind::Sink);
        for _ in 0..120 {
            c.step(&host, &mut g, Some(Vec3::ZERO), &[]);
        }
        assert_eq!(c.keepers.len(), 1, "a big pile grows one");
        assert!((c.keepers[0].pos.with_y(0.0) - Vec3::new(500.0, 0.0, 0.0)).length() < 300.0);
    }

    #[test]
    fn it_hurls_goop_at_an_enemy_then_dives() {
        let host = FlatHost::default();
        let mut g = GoopField::new();
        let mut c = critters();
        g.stamp(&host, Vec3::ZERO, 450.0, GoopKind::Sink);
        c.spawn_keeper(Vec3::ZERO, 0.0, 400).unwrap();
        let enemy = Target { id: 7, pos: Vec3::new(1500.0, 0.0, 300.0), radius: 80.0, height: 160.0, mario: false };
        let (mut hits, mut splats) = (0, 0);
        let life = c.keeper_params.life;
        for _ in 0..life + 2000 {
            c.step(&host, &mut g, None, &[enemy]);
            for e in c.events.drain(..) {
                match e {
                    CritterEvent::Hit { target: 7, scale, .. } => {
                        hits += 1;
                        assert!(scale > 1.0, "a Gatekeeper's ball hits harder");
                    }
                    CritterEvent::Splat { .. } => splats += 1,
                    _ => {}
                }
            }
        }
        assert!(hits + splats >= 3, "it kept launching ({hits} hits, {splats} splats)");
        assert!(hits >= 1, "its balls reach the enemy");
        assert!(c.keepers.is_empty(), "it dived after its life");
    }

    #[test]
    fn cleaning_its_pile_sends_it_under() {
        let host = FlatHost::default();
        let mut g = GoopField::new();
        let mut c = critters();
        g.stamp(&host, Vec3::ZERO, 450.0, GoopKind::Sink);
        let n = g.coverage(Vec3::ZERO, c.keeper_params.pile_radius).0;
        c.spawn_keeper(Vec3::ZERO, 0.0, n).unwrap();
        for _ in 0..600 {
            c.step(&host, &mut g, None, &[]);
        }
        assert_eq!(c.keepers[0].state, KeeperState::Wait);
        g.clean(Vec3::ZERO, 2000.0);
        for _ in 0..40 {
            c.step(&host, &mut g, None, &[]);
        }
        assert_eq!(c.keepers.first().map(|k| k.state), Some(KeeperState::Dive));
    }

    #[test]
    fn the_disc_clip_lengths_load_when_present() {
        let Some(dir) = crate::assets::find(None) else { eprintln!("SKIP: Sunshine assets absent"); return };
        if !dir.join("enemy/gatekeeper/gene_pakkun_shoot1.bck").is_file() {
            eprintln!("SKIP: gatekeeper not extracted");
            return;
        }
        let t = keeper_clip_ticks(&dir);
        // shoot1 is 59 frames; the launch (spine frame 0x34) falls inside it.
        assert!(t[keeper_clip::SHOOT as usize] > 0x34 * 2, "{:?}", t);
        assert!(t[keeper_clip::DIVE as usize] > 0xF0 * 2, "{:?}", t);
    }
}
