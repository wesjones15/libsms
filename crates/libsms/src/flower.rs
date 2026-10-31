//! Sunshine's sambo flowers (Enemy/hanasambo.cpp: `TSamboFlower`, `TSamboFlowerCoinUnit`,
//! `THanaSambo`, `TSamboHead`).
//!
//! Three things grow out of it:
//!
//! * **Flower units**: plain flowers (`TSamboFlower`) that bloom when FLUDD's water hits them
//!   (`receiveMessage`, `HIT_MESSAGE_SPRAYED_BY_WATER` -> `bloom`). A flower plays `flower_hit`
//!   to open and holds `flower_fwait`. A flower in a coin unit (`TSamboFlowerCoinUnit`, the
//!   "フラワー（コイン用）" actors that share an index with a "コイン（フラワー用）" marker) wilts
//!   again `mSLBloomTimer` ticks after it opened, unless every flower of the unit is open at
//!   once: `checkGenCoin` then throws one coin per flower out of the marker on a circle of
//!   `mSLCoinCircleR`, and the flowers stay open for good. A lone flower is a unit of one.
//! * **Sambo towers** (`THanaSambo`): the flower-headed Pokey. It hides under the ground until
//!   Mario is inside `mSLAppearDist`, bursts up (`Appear`), waits (`Wait`) and bites at Mario
//!   inside `mSLAttackDist` every `mSLAttackInterval` ticks (`Attack`), sinks back past
//!   `mSLHideDist`. A stomp or ground pound kills it (`THanaSamboHead::receiveMessage`); FLUDD
//!   water makes it freeze for a while (`waterDamage`, `TNerveHanaSamboFreeze`).
//! * **Sambo heads** (`TSamboHead`): the flower that hops after Mario, in rings.
//!
//! The actors' nerve time (`TSpine::getTime`) counts one per tick of the 120 Hz sim
//! ([`crate::clock`]), so the disc's tick counts are used as they are. Distances are the game's
//! units. Clip lengths are not known here: a host passes them in [`Clips`].
//!
//! Where the disc has data this uses it: `params_enemy/{samboflower,hanasambo,sambohead}.prm`
//! ([`FlowerParams::from_prm`] and friends); the `Default`s are those files' values on the
//! PAL disc. What is a stand-in is marked: the tower's biting head is a cylinder at the
//! base (not the posed head joint), and a head's hop distance is `mSLMoveDist` over the air time.

use glam::Vec3;

use crate::host::{Enemy, Host};
use crate::prm::Prm;

/// Ids this module hands out through [`Host::enemies`]-style queries start here, well above any
/// host's own (so one callback can tell them apart).
pub const ID_BASE: u64 = 0x4000_0000;

/// `samboflower.prm`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlowerParams {
    /// `mSLBloomTimer`: ticks an open coin flower waits before it wilts.
    pub bloom_timer: u32,
    /// `mSLCoinCircleR`: the radius the coins leave the marker on.
    pub coin_circle_r: f32,
    /// `mSLCoinVelocityXZ`, `mSLCoinVelocityY`.
    pub coin_vel_xz: f32,
    pub coin_vel_y: f32,
    /// `mSLBodyRadius`, `mSLHeadHeight`: the flower's hit cylinder.
    pub body_radius: f32,
    pub head_height: f32,
}

impl Default for FlowerParams {
    fn default() -> Self {
        FlowerParams {
            bloom_timer: 300,
            coin_circle_r: 100.0,
            coin_vel_xz: 2.0,
            coin_vel_y: 10.0,
            body_radius: 30.0,
            head_height: 120.0,
        }
    }
}

impl FlowerParams {
    pub fn from_prm(p: &Prm) -> FlowerParams {
        let d = FlowerParams::default();
        FlowerParams {
            bloom_timer: p.i32("mSLBloomTimer").map_or(d.bloom_timer, |v| v.max(1) as u32),
            coin_circle_r: p.f32("mSLCoinCircleR").unwrap_or(d.coin_circle_r),
            coin_vel_xz: p.f32("mSLCoinVelocityXZ").unwrap_or(d.coin_vel_xz),
            coin_vel_y: p.f32("mSLCoinVelocityY").unwrap_or(d.coin_vel_y),
            body_radius: p.f32("mSLBodyRadius").unwrap_or(d.body_radius),
            head_height: p.f32("mSLHeadHeight").unwrap_or(d.head_height),
        }
    }
}

/// `hanasambo.prm`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TowerParams {
    pub attack_dist: f32,
    pub attack_interval: u32,
    pub appear_dist: f32,
    pub hide_dist: f32,
    pub attacking_time: u32,
    pub head_attack_radius: f32,
    pub head_attack_height: f32,
    pub head_damage_radius: f32,
    pub head_damage_height: f32,
    pub head_height: f32,
}

impl Default for TowerParams {
    fn default() -> Self {
        TowerParams {
            attack_dist: 500.0,
            attack_interval: 150,
            appear_dist: 1000.0,
            hide_dist: 2000.0,
            attacking_time: 250,
            head_attack_radius: 60.0,
            head_attack_height: 20.0,
            head_damage_radius: 120.0,
            head_damage_height: 40.0,
            head_height: 120.0,
        }
    }
}

impl TowerParams {
    pub fn from_prm(p: &Prm) -> TowerParams {
        let d = TowerParams::default();
        let t = |k: &str, v: u32| p.i32(k).map_or(v, |x| x.max(1) as u32);
        let f = |k: &str, v: f32| p.f32(k).unwrap_or(v);
        TowerParams {
            attack_dist: f("mSLAttackDist", d.attack_dist),
            attack_interval: t("mSLAttackInterval", d.attack_interval),
            appear_dist: f("mSLAppearDist", d.appear_dist),
            hide_dist: f("mSLHideDist", d.hide_dist),
            attacking_time: t("mSLAttackingTime", d.attacking_time),
            head_attack_radius: f("mSLHeadAttackRadius", d.head_attack_radius),
            head_attack_height: f("mSLHeadAttackHeight", d.head_attack_height),
            head_damage_radius: f("mSLHeadDamageRadius", d.head_damage_radius),
            head_damage_height: f("mSLHeadDamageHeight", d.head_damage_height),
            head_height: f("mSLHeadHeight", d.head_height),
        }
    }
}

/// `sambohead.prm`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeadParams {
    pub appear_dist: f32,
    pub hide_dist: f32,
    /// `mSLMoveDist`: how far one hop carries it.
    pub move_dist: f32,
    pub move_gravity: f32,
    pub jump_sp: f32,
    pub jump_prepare_time: u32,
    /// `mSLHitJumpSpXZ`, `mSLHitJumpSpY`, `mSLHitJumpGravity`: the leap water throws it into.
    pub hit_jump_sp_xz: f32,
    pub hit_jump_sp_y: f32,
    pub hit_jump_gravity: f32,
    pub body_scale: f32,
    pub head_height: f32,
}

impl Default for HeadParams {
    fn default() -> Self {
        HeadParams {
            appear_dist: 800.0,
            hide_dist: 1000.0,
            move_dist: 300.0,
            move_gravity: 0.3,
            jump_sp: 8.0,
            jump_prepare_time: 300,
            hit_jump_sp_xz: 12.0,
            hit_jump_sp_y: 10.0,
            hit_jump_gravity: 0.4,
            body_scale: 1.5,
            head_height: 120.0,
        }
    }
}

impl HeadParams {
    pub fn from_prm(p: &Prm) -> HeadParams {
        let d = HeadParams::default();
        let f = |k: &str, v: f32| p.f32(k).unwrap_or(v);
        HeadParams {
            appear_dist: f("mSLAppearDist", d.appear_dist),
            hide_dist: f("mSLHideDist", d.hide_dist),
            move_dist: f("mSLMoveDist", d.move_dist),
            move_gravity: f("mSLMoveGravity", d.move_gravity),
            jump_sp: f("mSLJumpSp", d.jump_sp),
            jump_prepare_time: p
                .i32("mSLJumpPrepareTime")
                .map_or(d.jump_prepare_time, |x| x.max(1) as u32),
            hit_jump_sp_xz: f("mSLHitJumpSpXZ", d.hit_jump_sp_xz),
            hit_jump_sp_y: f("mSLHitJumpSpY", d.hit_jump_sp_y),
            hit_jump_gravity: f("mSLHitJumpGravity", d.hit_jump_gravity),
            body_scale: f("mSLBodyScaleLow", d.body_scale),
            head_height: f("mSLHeadHeight", d.head_height),
        }
    }
}

/// How long the clips run, in ticks (a host knows its `.bck` lengths; the disc's clips run at
/// 30 frames a second, so a clip is `frames * 4` ticks).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Clips {
    /// `flower_hit`: a flower opening (or, backwards, closing).
    pub flower_open: u32,
    /// `sambo_fset`: the tower bursting out of the ground.
    pub tower_appear: u32,
    /// `sambo_fhide`: sinking back.
    pub tower_hide: u32,
    /// `sambo_fattack_start`, `sambo_fattack`, `sambo_fattack_end`.
    pub tower_attack_start: u32,
    pub tower_attack: u32,
    pub tower_attack_end: u32,
    /// `sambo_fhit_start`.. the freeze: how long water holds it.
    pub tower_freeze: u32,
    /// `sambo_down`: the death clip.
    pub tower_die: u32,
}

impl Default for Clips {
    fn default() -> Self {
        Clips {
            flower_open: 60,
            tower_appear: 100,
            tower_hide: 100,
            tower_attack_start: 60,
            tower_attack: 60,
            tower_attack_end: 60,
            tower_freeze: 240,
            tower_die: 120,
        }
    }
}

/// A flower's phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bloom {
    /// `flower_wait`: a bud.
    Closed,
    /// `flower_hit` playing forwards.
    Opening,
    /// `flower_fwait`: open.
    Open,
    /// `flower_hit` playing backwards: wilting.
    Closing,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Flower {
    pub pos: Vec3,
    pub phase: Bloom,
    /// Ticks in the phase.
    pub age: u32,
    /// Ticks open (`unk154`).
    open_for: u32,
}

/// A coin leaving a ring (`checkGenCoin`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CoinLaunch {
    pub pos: Vec3,
    pub vel: Vec3,
}

/// A group of flowers that pay together (`TSamboFlowerCoinUnit`).
#[derive(Debug, Clone)]
pub struct FlowerUnit {
    /// The coin marker (`unk4`): the ring's middle.
    pub center: Vec3,
    pub flowers: Vec<Flower>,
    /// Flowers still to open (`unk1C`, counting down as they bloom: the open chime's pitch).
    remaining: usize,
    /// Coins thrown (`unk160` false on the flowers, `unk18` null).
    pub paid: bool,
    /// Coins it throws; the disc gives one per flower.
    pub coins: usize,
}

/// A hit cylinder for a thing the sim owns.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Target {
    pub id: u64,
    pub pos: Vec3,
    pub radius: f32,
    pub height: f32,
}

/// What happened this tick.
#[derive(Debug, Clone, PartialEq)]
pub enum FlowerEvent {
    /// A flower opened (`bloom`): pollen (`ms_smb_kafun_ow`) and the open chime, pitched by how
    /// many flowers of its unit are still shut.
    Bloomed { unit: usize, flower: usize, pos: Vec3, still_shut: usize },
    /// A flower wilted.
    Wilted { unit: usize, flower: usize },
    /// A whole unit bloomed: throw these coins (`MSD_SE_SY_COIN_APPEAR` once).
    CoinRing { unit: usize, coins: Vec<CoinLaunch> },
    /// A tower burst out of the ground or sank (`ms_smb_ap_*`/`ms_smb_hd_*`).
    TowerAppeared { tower: usize, pos: Vec3 },
    TowerHid { tower: usize, pos: Vec3 },
    /// A tower bit: the host hurts Mario (a stand-in cylinder at the tower's head).
    TowerBite { tower: usize, from: Vec3, target: usize },
    /// A tower or head died: scatter its coins (`genRandomItem`).
    EnemyDied { id: u64, pos: Vec3 },
    /// A head's hop landed or started (the host plays its clip).
    HeadHopped { head: usize, pos: Vec3 },
    /// A head touched Mario.
    HeadBit { head: usize, from: Vec3, target: usize },
}

/// A tower's nerve (`TNerveHanaSambo*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TowerState {
    /// Under the ground (`Hide`, hidden).
    Hidden,
    Appearing,
    Waiting,
    /// `Attack` and which of its three clips.
    AttackStart,
    AttackLoop,
    AttackEnd,
    Hiding,
    /// `Freeze`: held by water.
    Frozen,
    Dying,
    Dead,
}

#[derive(Debug, Clone)]
pub struct Tower {
    pub pos: Vec3,
    pub yaw: f32,
    pub state: TowerState,
    /// Ticks in the state (`TSpine::getTime`).
    pub age: u32,
    since_attack: u32,
}

/// A head's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadState {
    /// Waiting for Mario to come near.
    Dormant,
    /// On the ground, waiting `mSLJumpPrepareTime` between hops.
    Ready,
    Hopping,
    /// Thrown off its feet by water.
    Knocked,
    Dead,
}

#[derive(Debug, Clone)]
pub struct Head {
    pub pos: Vec3,
    pub home: Vec3,
    pub vel: Vec3,
    pub yaw: f32,
    pub state: HeadState,
    pub age: u32,
}

/// The sambo things in a stage.
#[derive(Debug, Clone, Default)]
pub struct FlowerWorld {
    pub flower: FlowerParams,
    pub tower_params: TowerParams,
    pub head_params: HeadParams,
    pub clips: Clips,
    pub units: Vec<FlowerUnit>,
    pub towers: Vec<Tower>,
    pub heads: Vec<Head>,
    ticks: u32,
}

/// The nearest of `marios` to `at` on the floor plane, and its index.
fn nearest(marios: &[Vec3], at: Vec3) -> Option<(usize, Vec3)> {
    marios.iter().copied().enumerate().min_by(|a, b| {
        let d = |v: Vec3| (v.x - at.x).powi(2) + (v.z - at.z).powi(2);
        d(a.1).total_cmp(&d(b.1))
    })
}

fn rot_y(v: Vec3, degrees: f32) -> Vec3 {
    // MsMtxSetRotY then MTXMultVec: x' = x cos + z sin, z' = -x sin + z cos.
    let (s, c) = degrees.to_radians().sin_cos();
    Vec3::new(v.x * c + v.z * s, v.y, -v.x * s + v.z * c)
}

impl FlowerUnit {
    pub fn new(center: Vec3, spots: &[Vec3], coins: usize) -> FlowerUnit {
        FlowerUnit {
            center,
            flowers: spots
                .iter()
                .map(|&pos| Flower { pos, phase: Bloom::Closed, age: 0, open_for: 0 })
                .collect(),
            remaining: spots.len(),
            paid: false,
            coins,
        }
    }

    /// `n` flowers evenly on a circle `radius` around `center`; the coin ring pays out of the
    /// middle.
    pub fn ring(center: Vec3, n: usize, radius: f32) -> FlowerUnit {
        let spots: Vec<Vec3> = (0..n)
            .map(|i| center + rot_y(Vec3::new(0.0, 0.0, radius), 360.0 * i as f32 / n as f32))
            .collect();
        FlowerUnit::new(center, &spots, n)
    }

    /// A lone flower: a unit of one that pays one coin.
    pub fn single(pos: Vec3) -> FlowerUnit {
        FlowerUnit::new(pos, &[pos], 1)
    }

    pub fn all_open(&self) -> bool {
        self.flowers.iter().all(|f| f.phase == Bloom::Open)
    }
}

impl FlowerWorld {
    pub fn new() -> FlowerWorld {
        FlowerWorld {
            head_params: HeadParams::default(),
            ..Default::default()
        }
    }

    /// Reads the three `.prm` files' values (any may be absent).
    pub fn with_params(
        mut self,
        flower: Option<&Prm>,
        tower: Option<&Prm>,
        head: Option<&Prm>,
    ) -> FlowerWorld {
        if let Some(p) = flower {
            self.flower = FlowerParams::from_prm(p);
        }
        if let Some(p) = tower {
            self.tower_params = TowerParams::from_prm(p);
        }
        if let Some(p) = head {
            self.head_params = HeadParams::from_prm(p);
        }
        self
    }

    pub fn add_unit(&mut self, unit: FlowerUnit) -> usize {
        self.units.push(unit);
        self.units.len() - 1
    }

    pub fn add_tower(&mut self, pos: Vec3, yaw: f32) -> usize {
        self.towers.push(Tower { pos, yaw, state: TowerState::Hidden, age: 0, since_attack: 0 });
        self.towers.len() - 1
    }

    pub fn add_head(&mut self, pos: Vec3, yaw: f32) -> usize {
        self.heads.push(Head {
            pos,
            home: pos,
            vel: Vec3::ZERO,
            yaw,
            state: HeadState::Dormant,
            age: 0,
        });
        self.heads.len() - 1
    }

    /// Water hit the flower `flower` of `unit` (`TSamboFlower::receiveMessage`).
    pub fn spray_flower(&mut self, unit: usize, flower: usize, out: &mut Vec<FlowerEvent>) {
        let Some(u) = self.units.get_mut(unit) else { return };
        let Some(f) = u.flowers.get_mut(flower) else { return };
        if matches!(f.phase, Bloom::Opening | Bloom::Open) {
            return;
        }
        f.phase = Bloom::Opening;
        f.age = 0;
        f.open_for = 0;
        let pos = f.pos;
        let mut still_shut = 0;
        if !u.paid {
            u.remaining = u.remaining.saturating_sub(1);
            still_shut = u.remaining;
        }
        out.push(FlowerEvent::Bloomed { unit, flower, pos, still_shut });
    }

    /// The things water and Mario's feet can hit now: flowers (id `ID_BASE + unit*64 + i`, so at
    /// most 64 to a unit), towers (`ID_BASE + 0x10000 + i`) and heads (`ID_BASE + 0x20000 + i`).
    pub fn targets(&self, out: &mut Vec<Target>) {
        for (ui, u) in self.units.iter().enumerate() {
            for (fi, f) in u.flowers.iter().enumerate().take(64) {
                out.push(Target {
                    id: ID_BASE + (ui * 64 + fi) as u64,
                    pos: f.pos,
                    radius: self.flower.body_radius,
                    height: self.flower.head_height,
                });
            }
        }
        for (i, t) in self.towers.iter().enumerate() {
            if matches!(t.state, TowerState::Hidden | TowerState::Dead | TowerState::Dying) {
                continue;
            }
            out.push(Target {
                id: ID_BASE + 0x10000 + i as u64,
                pos: t.pos + Vec3::Y * self.tower_params.head_height,
                radius: self.tower_params.head_damage_radius,
                height: self.tower_params.head_damage_height,
            });
        }
        for (i, h) in self.heads.iter().enumerate() {
            if matches!(h.state, HeadState::Dormant | HeadState::Dead) {
                continue;
            }
            out.push(Target {
                id: ID_BASE + 0x20000 + i as u64,
                pos: h.pos,
                radius: 60.0 * self.head_params.body_scale,
                height: self.head_params.head_height,
            });
        }
    }

    /// The same list as [`Host::enemies`] wants. Flowers are in it too so water can reach them.
    pub fn enemies_near(&self, pos: Vec3, radius: f32, out: &mut Vec<Enemy>) {
        let mut all = Vec::new();
        self.targets(&mut all);
        for t in all {
            let d = t.pos - pos;
            if d.length() <= radius + t.radius + t.height {
                out.push(Enemy { id: t.id, pos: t.pos, radius: t.radius, height: t.height });
            }
        }
    }

    /// Whether `id` is one of this world's.
    pub fn owns(id: u64) -> bool {
        (ID_BASE..ID_BASE + 0x30000).contains(&id)
    }

    /// Mario's water hit `id` (a [`Host::enemies`] id).
    pub fn water_hit(&mut self, id: u64, out: &mut Vec<FlowerEvent>) {
        let Some(n) = id.checked_sub(ID_BASE) else { return };
        if n < 0x10000 {
            self.spray_flower((n / 64) as usize, (n % 64) as usize, out);
        } else if n < 0x20000 {
            // `THanaSambo::waterDamage`: only a waiting tower freezes.
            if let Some(t) = self.towers.get_mut((n - 0x10000) as usize) {
                if t.state == TowerState::Waiting {
                    t.state = TowerState::Frozen;
                    t.age = 0;
                }
            }
        } else if let Some(h) = self.heads.get_mut((n - 0x20000) as usize) {
            // `TNerveSamboHeadHitWater`: it leaps away.
            if matches!(h.state, HeadState::Ready | HeadState::Hopping) {
                let p = self.head_params;
                let away = Vec3::new(h.vel.x, 0.0, h.vel.z).try_normalize().unwrap_or(Vec3::X);
                h.vel = Vec3::new(away.x * p.hit_jump_sp_xz, p.hit_jump_sp_y, away.z * p.hit_jump_sp_xz);
                h.state = HeadState::Knocked;
                h.age = 0;
            }
        }
    }

    /// Mario stomped or pounded `id` (`THanaSamboHead::receiveMessage`: trample / hip drop).
    pub fn stomped(&mut self, id: u64, out: &mut Vec<FlowerEvent>) {
        let Some(n) = id.checked_sub(ID_BASE) else { return };
        if (0x10000..0x20000).contains(&n) {
            if let Some(t) = self.towers.get_mut((n - 0x10000) as usize) {
                if !matches!(t.state, TowerState::Dying | TowerState::Dead | TowerState::Hidden) {
                    t.state = TowerState::Dying;
                    t.age = 0;
                }
            }
        } else if n >= 0x20000 {
            let i = (n - 0x20000) as usize;
            if let Some(h) = self.heads.get_mut(i) {
                if h.state != HeadState::Dead {
                    h.state = HeadState::Dead;
                    out.push(FlowerEvent::EnemyDied { id, pos: h.pos });
                }
            }
        }
    }

    /// One 120 Hz tick. `marios` are the feet of every Mario in play: the enemies wake for, chase
    /// and bite the nearest (an event's `target` indexes this slice).
    pub fn step(&mut self, host: &dyn Host, marios: &[Vec3], out: &mut Vec<FlowerEvent>) {
        self.ticks = self.ticks.wrapping_add(1);
        self.step_units(host, out);
        self.step_towers(host, marios, out);
        self.step_heads(host, marios, out);
    }

    fn step_units(&mut self, host: &dyn Host, out: &mut Vec<FlowerEvent>) {
        // `TSamboFlower::moveObject` puts the flower on the floor; once a second here is plenty.
        let snap = self.ticks % 120 == 1;
        let (open_ticks, bloom_timer) = (self.clips.flower_open, self.flower.bloom_timer);
        let (r, vxz, vy) = (
            self.flower.coin_circle_r,
            self.flower.coin_vel_xz,
            self.flower.coin_vel_y,
        );
        for (ui, u) in self.units.iter_mut().enumerate() {
            if snap {
                let mut drop = None;
                for f in u.flowers.iter_mut() {
                    if let Some(g) = host.ground(f.pos + Vec3::Y * 100.0) {
                        drop.get_or_insert(g.height - f.pos.y);
                        f.pos.y = g.height;
                    }
                }
                u.center.y += drop.unwrap_or(0.0);
            }
            for (fi, f) in u.flowers.iter_mut().enumerate() {
                f.age += 1;
                match f.phase {
                    Bloom::Closed => {}
                    Bloom::Opening => {
                        if f.age >= open_ticks {
                            f.phase = Bloom::Open;
                            f.age = 0;
                        }
                    }
                    Bloom::Open => {
                        // `moveObject`: only a flower whose unit has not paid yet wilts.
                        if !u.paid {
                            f.open_for += 1;
                            if f.open_for > bloom_timer {
                                f.phase = Bloom::Closing;
                                f.age = 0;
                                u.remaining += 1;
                                out.push(FlowerEvent::Wilted { unit: ui, flower: fi });
                            }
                        }
                    }
                    Bloom::Closing => {
                        if f.age >= open_ticks {
                            f.phase = Bloom::Closed;
                            f.age = 0;
                        }
                    }
                }
            }
            if !u.paid && u.coins > 0 && u.all_open() {
                u.paid = true;
                let n = u.coins;
                let coins = (0..n)
                    .map(|i| {
                        let rate = i as f32 / n as f32;
                        let offset = rot_y(Vec3::new(0.0, 0.0, r), 360.0 * rate);
                        let dir = offset.normalize_or_zero();
                        CoinLaunch {
                            pos: u.center + offset,
                            vel: Vec3::new(dir.x * vxz, 8.0 * rate + vy, dir.z * vxz),
                        }
                    })
                    .collect();
                out.push(FlowerEvent::CoinRing { unit: ui, coins });
            }
        }
    }

    fn step_towers(&mut self, host: &dyn Host, marios: &[Vec3], out: &mut Vec<FlowerEvent>) {
        let (p, c) = (self.tower_params, self.clips);
        for (i, t) in self.towers.iter_mut().enumerate() {
            t.age += 1;
            // Nobody to wake for: a tower with no Mario about stays as it is.
            let (target, mario) = nearest(marios, t.pos).unwrap_or((usize::MAX, Vec3::splat(1.0e9)));
            let d = mario - t.pos;
            let flat = (d.x * d.x + d.z * d.z).sqrt();
            let in_reach = mario.y < t.pos.y + 100.0;
            if !matches!(t.state, TowerState::Dying | TowerState::Dead | TowerState::Hidden) {
                t.yaw = d.x.atan2(d.z).to_degrees();
            }
            match t.state {
                TowerState::Hidden => {
                    if flat < p.appear_dist {
                        t.state = TowerState::Appearing;
                        t.age = 0;
                        out.push(FlowerEvent::TowerAppeared { tower: i, pos: t.pos });
                    }
                }
                TowerState::Appearing => {
                    if t.age >= c.tower_appear {
                        t.state = TowerState::Waiting;
                        t.age = 0;
                        t.since_attack = 0;
                        // `TNerveHanaSamboAppear`: straight into a bite when Mario is on it.
                        if in_reach && flat < p.attack_dist {
                            t.state = TowerState::AttackStart;
                        }
                    }
                }
                TowerState::Waiting => {
                    if t.age > p.attack_interval && in_reach && flat < p.attack_dist {
                        t.state = TowerState::AttackStart;
                        t.age = 0;
                    } else if flat > p.hide_dist {
                        t.state = TowerState::Hiding;
                        t.age = 0;
                        out.push(FlowerEvent::TowerHid { tower: i, pos: t.pos });
                    }
                }
                TowerState::AttackStart => {
                    if t.age >= c.tower_attack_start {
                        t.state = TowerState::AttackLoop;
                        t.age = 0;
                    }
                }
                TowerState::AttackLoop => {
                    // The stand-in head: a cylinder `head_attack_radius` wide around the line
                    // from the base towards Mario, out to the attack distance.
                    if in_reach && flat < p.attack_dist + p.head_attack_radius && t.age % 30 == 1 {
                        out.push(FlowerEvent::TowerBite { tower: i, from: t.pos, target });
                    }
                    if t.age > p.attacking_time {
                        t.state = TowerState::AttackEnd;
                        t.age = 0;
                    }
                }
                TowerState::AttackEnd => {
                    if t.age >= c.tower_attack_end {
                        t.state = TowerState::Waiting;
                        t.age = 0;
                    }
                }
                TowerState::Hiding => {
                    if t.age >= c.tower_hide {
                        t.state = TowerState::Hidden;
                        t.age = 0;
                    }
                }
                TowerState::Frozen => {
                    if t.age >= c.tower_freeze {
                        t.state = TowerState::Waiting;
                        t.age = 0;
                    }
                }
                TowerState::Dying => {
                    if t.age >= c.tower_die {
                        t.state = TowerState::Dead;
                        out.push(FlowerEvent::EnemyDied {
                            id: ID_BASE + 0x10000 + i as u64,
                            pos: t.pos,
                        });
                    }
                }
                TowerState::Dead => {}
            }
            // `TSamboFlower` / enemy actors stay on the floor.
            if t.state != TowerState::Dead {
                if let Some(g) = host.ground(t.pos + Vec3::Y * 100.0) {
                    t.pos.y = g.height;
                }
            }
        }
    }

    fn step_heads(&mut self, host: &dyn Host, marios: &[Vec3], out: &mut Vec<FlowerEvent>) {
        let p = self.head_params;
        // Air time of one hop and the sideways speed that covers `move_dist` in it.
        let air = (2.0 * p.jump_sp / p.move_gravity).max(1.0);
        let hop_xz = p.move_dist / air;
        for (i, h) in self.heads.iter_mut().enumerate() {
            h.age += 1;
            let (target, mario) = nearest(marios, h.pos).unwrap_or((usize::MAX, Vec3::splat(1.0e9)));
            let d = mario - h.pos;
            let flat = (d.x * d.x + d.z * d.z).sqrt();
            match h.state {
                HeadState::Dormant => {
                    if flat < p.appear_dist {
                        h.state = HeadState::Ready;
                        h.age = 0;
                    }
                }
                HeadState::Ready => {
                    h.yaw = d.x.atan2(d.z).to_degrees();
                    if flat > p.hide_dist {
                        h.state = HeadState::Dormant;
                        h.age = 0;
                    } else if h.age > p.jump_prepare_time {
                        let dir = Vec3::new(d.x, 0.0, d.z).normalize_or_zero();
                        h.vel = Vec3::new(dir.x * hop_xz, p.jump_sp, dir.z * hop_xz);
                        h.state = HeadState::Hopping;
                        h.age = 0;
                        out.push(FlowerEvent::HeadHopped { head: i, pos: h.pos });
                    }
                }
                HeadState::Hopping | HeadState::Knocked => {
                    let g = if h.state == HeadState::Knocked { p.hit_jump_gravity } else { p.move_gravity };
                    h.vel.y -= g;
                    h.pos += h.vel;
                    let floor = host.ground(h.pos + Vec3::Y * 100.0).map(|g| g.height);
                    if h.vel.y < 0.0 && floor.is_some_and(|f| h.pos.y <= f) {
                        h.pos.y = floor.unwrap_or(h.pos.y);
                        h.vel = Vec3::ZERO;
                        h.state = HeadState::Ready;
                        h.age = 0;
                    }
                    // A touch (a hop onto Mario is its attack; `TSamboHead::attackToMario`).
                    let reach = 60.0 * p.body_scale;
                    let dm = mario - h.pos;
                    if dm.y.abs() < p.head_height && (dm.x * dm.x + dm.z * dm.z).sqrt() < reach
                        && h.state == HeadState::Hopping
                        && h.age % 30 == 1
                    {
                        out.push(FlowerEvent::HeadBit { head: i, from: h.pos, target });
                    }
                }
                HeadState::Dead => {}
            }
        }
    }
}

/// One thing of a snapshot ([`FlowerWorld::items`] / [`FlowerWorld::mirror`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Item {
    /// 0 flower, 1 tower, 2 head.
    pub kind: u8,
    /// The phase / [`TowerState`] / [`HeadState`] as its enum number.
    pub state: u8,
    /// The id's low part: flower `unit * 64 + flower`, else the index.
    pub handle: u32,
    pub age: u32,
    pub pos: Vec3,
    pub yaw: f32,
}

const BLOOMS: [Bloom; 4] = [Bloom::Closed, Bloom::Opening, Bloom::Open, Bloom::Closing];
const TOWERS: [TowerState; 10] = [
    TowerState::Hidden,
    TowerState::Appearing,
    TowerState::Waiting,
    TowerState::AttackStart,
    TowerState::AttackLoop,
    TowerState::AttackEnd,
    TowerState::Hiding,
    TowerState::Frozen,
    TowerState::Dying,
    TowerState::Dead,
];
const HEADS: [HeadState; 5] = [
    HeadState::Dormant,
    HeadState::Ready,
    HeadState::Hopping,
    HeadState::Knocked,
    HeadState::Dead,
];

impl FlowerWorld {
    /// Everything, for a snapshot (flowers in unit order, so `handle`s are increasing).
    pub fn items(&self) -> Vec<Item> {
        let mut v = Vec::new();
        for (ui, u) in self.units.iter().enumerate() {
            for (fi, f) in u.flowers.iter().enumerate() {
                v.push(Item {
                    kind: 0,
                    state: f.phase as u8,
                    handle: (ui * 64 + fi) as u32,
                    age: f.age,
                    pos: f.pos,
                    yaw: 0.0,
                });
            }
        }
        for (i, t) in self.towers.iter().enumerate() {
            v.push(Item { kind: 1, state: t.state as u8, handle: i as u32, age: t.age, pos: t.pos, yaw: t.yaw });
        }
        for (i, h) in self.heads.iter().enumerate() {
            v.push(Item { kind: 2, state: h.state as u8, handle: i as u32, age: h.age, pos: h.pos, yaw: h.yaw });
        }
        v
    }

    /// Becomes a copy of the server snapshot (a client world: it is not stepped, it is drawn and
    /// its targets take the spray and stomps the server decides on). Unknown states are ignored;
    /// a gap in a unit's flowers is filled by a copy of its neighbour.
    pub fn mirror(&mut self, items: &[Item]) {
        self.units.clear();
        self.towers.clear();
        self.heads.clear();
        for it in items {
            match it.kind {
                0 => {
                    let (ui, fi) = ((it.handle / 64) as usize, (it.handle % 64) as usize);
                    let Some(&phase) = BLOOMS.get(it.state as usize) else { continue };
                    if ui > 64 {
                        continue;
                    }
                    while self.units.len() <= ui {
                        self.units.push(FlowerUnit::new(it.pos, &[], 0));
                    }
                    let u = &mut self.units[ui];
                    let f = Flower { pos: it.pos, phase, age: it.age, open_for: 0 };
                    while u.flowers.len() < fi {
                        u.flowers.push(f);
                    }
                    u.flowers.push(f);
                    u.center = it.pos;
                }
                1 => {
                    let Some(&state) = TOWERS.get(it.state as usize) else { continue };
                    while self.towers.len() < (it.handle as usize).min(64) {
                        self.towers.push(Tower { pos: it.pos, yaw: 0.0, state: TowerState::Dead, age: 0, since_attack: 0 });
                    }
                    self.towers.push(Tower { pos: it.pos, yaw: it.yaw, state, age: it.age, since_attack: 0 });
                }
                2 => {
                    let Some(&state) = HEADS.get(it.state as usize) else { continue };
                    while self.heads.len() < (it.handle as usize).min(64) {
                        self.heads.push(Head { pos: it.pos, home: it.pos, vel: Vec3::ZERO, yaw: 0.0, state: HeadState::Dead, age: 0 });
                    }
                    self.heads.push(Head { pos: it.pos, home: it.pos, vel: Vec3::ZERO, yaw: it.yaw, state, age: it.age });
                }
                _ => {}
            }
        }
    }

    /// A hit reported for `id`: 0 water, anything else a stomp, pound or other attack.
    pub fn hit(&mut self, id: u64, kind: u8, out: &mut Vec<FlowerEvent>) {
        if kind == 0 {
            self.water_hit(id, out);
        } else {
            self.stomped(id, out);
        }
    }
}

/// What a stage's rare roll picks (`pick`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spawn {
    /// A circle of flowers that pay coins when all are open.
    FlowerRing { flowers: usize },
    /// One flower, one coin.
    SingleFlower,
    /// A circle of hopping flower enemies.
    EnemyRing { heads: usize },
    /// A lone tower enemy.
    Tower,
}

/// The weights of the rare roll, out of `chance_per_mille` a roll: tuning, not the disc's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpawnTable {
    /// Chance of any spawn from one roll, per thousand.
    pub chance_per_mille: u32,
    /// Relative weights: flower ring, single flower, enemy ring, tower.
    pub weights: [u32; 4],
}

impl Default for SpawnTable {
    fn default() -> Self {
        SpawnTable { chance_per_mille: 120, weights: [3, 5, 2, 2] }
    }
}

impl SpawnTable {
    /// `r` and `s` are uniform in [0, 1): `r` decides whether and what, `s` the ring's size.
    pub fn pick(&self, r: f32, s: f32) -> Option<Spawn> {
        let chance = self.chance_per_mille.min(1000) as f32 / 1000.0;
        if r >= chance {
            return None;
        }
        let total: u32 = self.weights.iter().sum();
        if total == 0 {
            return None;
        }
        let mut x = (r / chance * total as f32) as u32;
        let size = |lo: usize, hi: usize| lo + ((s * (hi - lo + 1) as f32) as usize).min(hi - lo);
        for (i, w) in self.weights.iter().enumerate() {
            if x < *w {
                return Some(match i {
                    0 => Spawn::FlowerRing { flowers: size(4, 8) },
                    1 => Spawn::SingleFlower,
                    2 => Spawn::EnemyRing { heads: size(3, 6) },
                    _ => Spawn::Tower,
                });
            }
            x -= *w;
        }
        None
    }
}

impl FlowerWorld {
    /// Puts a roll's pick at `at` (`yaw` turns the ring). Returns how many things it added.
    pub fn place(&mut self, spawn: Spawn, at: Vec3, yaw: f32) -> usize {
        match spawn {
            Spawn::FlowerRing { flowers } => {
                // Wide enough that the flowers do not overlap and the coins land inside it.
                let radius = (flowers as f32 * 45.0).max(self.flower.coin_circle_r + 100.0);
                self.add_unit(FlowerUnit::ring(at, flowers, radius));
                flowers
            }
            Spawn::SingleFlower => {
                self.add_unit(FlowerUnit::single(at));
                1
            }
            Spawn::EnemyRing { heads } => {
                let radius = (heads as f32 * 90.0).max(250.0);
                for i in 0..heads {
                    let spot = at + rot_y(Vec3::new(0.0, 0.0, radius), yaw + 360.0 * i as f32 / heads as f32);
                    self.add_head(spot, yaw);
                }
                heads
            }
            Spawn::Tower => {
                self.add_tower(at, yaw);
                1
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::FlatHost;

    fn run(w: &mut FlowerWorld, mario: Vec3, ticks: u32) -> Vec<FlowerEvent> {
        let marios = [mario];
        let host = FlatHost::default();
        let mut out = Vec::new();
        for _ in 0..ticks {
            w.step(&host, &marios, &mut out);
        }
        out
    }

    #[test]
    fn a_ring_pays_when_every_flower_is_open() {
        let mut w = FlowerWorld::new();
        let u = w.add_unit(FlowerUnit::ring(Vec3::ZERO, 5, 250.0));
        let mut ev = Vec::new();
        for i in 0..5 {
            w.spray_flower(u, i, &mut ev);
        }
        // The last flower opens a clip's length after the first, well inside the 300 ticks.
        let ev = run(&mut w, Vec3::splat(5000.0), 200);
        let ring = ev
            .iter()
            .find_map(|e| if let FlowerEvent::CoinRing { coins, .. } = e { Some(coins) } else { None })
            .expect("coins");
        assert_eq!(ring.len(), 5);
        // `checkGenCoin`: on the circle, thrown outward, rising 8 * rate more each.
        assert!((ring[0].pos - Vec3::new(0.0, 0.0, 100.0)).length() < 1e-3);
        assert!((ring[0].vel.y - 10.0).abs() < 1e-4 && ring[1].vel.y > ring[0].vel.y);
        assert!(w.units[0].paid);
        // Paid flowers stay open for good.
        run(&mut w, Vec3::splat(5000.0), 1000);
        assert!(w.units[0].all_open());
    }

    #[test]
    fn a_missed_flower_lets_the_rest_wilt() {
        let mut w = FlowerWorld::new();
        let u = w.add_unit(FlowerUnit::ring(Vec3::ZERO, 4, 250.0));
        let mut ev = Vec::new();
        w.spray_flower(u, 0, &mut ev);
        w.spray_flower(u, 1, &mut ev);
        let ev = run(&mut w, Vec3::splat(5000.0), 300 + 60 + 70);
        assert!(ev.iter().any(|e| matches!(e, FlowerEvent::Wilted { flower: 0, .. })));
        assert!(!w.units[0].paid);
        run(&mut w, Vec3::splat(5000.0), 100);
        assert!(w.units[0].flowers.iter().all(|f| f.phase == Bloom::Closed));
    }

    #[test]
    fn a_tower_rises_bites_and_dies_to_a_stomp() {
        let mut w = FlowerWorld::new();
        let t = w.add_tower(Vec3::ZERO, 0.0);
        let mario = Vec3::new(0.0, 0.0, 400.0);
        let ev = run(&mut w, mario, 5);
        assert!(ev.iter().any(|e| matches!(e, FlowerEvent::TowerAppeared { .. })));
        let ev = run(&mut w, mario, 600);
        assert!(ev.iter().any(|e| matches!(e, FlowerEvent::TowerBite { .. })));
        let mut ev = Vec::new();
        w.stomped(ID_BASE + 0x10000 + t as u64, &mut ev);
        let ev = run(&mut w, mario, 200);
        assert!(ev.iter().any(|e| matches!(e, FlowerEvent::EnemyDied { .. })));
        assert_eq!(w.towers[0].state, TowerState::Dead);
    }

    #[test]
    fn water_freezes_a_waiting_tower_and_it_recovers() {
        let mut w = FlowerWorld::new();
        let t = w.add_tower(Vec3::ZERO, 0.0);
        let mario = Vec3::new(0.0, 0.0, 700.0);
        run(&mut w, mario, 200);
        assert_eq!(w.towers[0].state, TowerState::Waiting);
        let mut ev = Vec::new();
        w.water_hit(ID_BASE + 0x10000 + t as u64, &mut ev);
        assert_eq!(w.towers[0].state, TowerState::Frozen);
        run(&mut w, mario, 300);
        assert_eq!(w.towers[0].state, TowerState::Waiting);
    }

    #[test]
    fn a_tower_sinks_when_mario_leaves() {
        let mut w = FlowerWorld::new();
        w.add_tower(Vec3::ZERO, 0.0);
        run(&mut w, Vec3::new(0.0, 0.0, 700.0), 200);
        let ev = run(&mut w, Vec3::new(0.0, 0.0, 3000.0), 400);
        assert!(ev.iter().any(|e| matches!(e, FlowerEvent::TowerHid { .. })));
        assert_eq!(w.towers[0].state, TowerState::Hidden);
    }

    #[test]
    fn a_head_hops_to_mario_and_lands() {
        let mut w = FlowerWorld::new();
        w.add_head(Vec3::ZERO, 0.0);
        let mario = Vec3::new(0.0, 0.0, 600.0);
        let ev = run(&mut w, mario, 400);
        assert!(ev.iter().any(|e| matches!(e, FlowerEvent::HeadHopped { .. })));
        run(&mut w, mario, 120);
        assert!(w.heads[0].pos.z > 100.0, "{:?}", w.heads[0].pos);
        assert!(w.heads[0].pos.y.abs() < 1.0);
    }

    #[test]
    fn rolls_are_rare_and_cover_every_kind() {
        let t = SpawnTable::default();
        let mut counts = [0usize; 4];
        let mut any = 0;
        for i in 0..10_000u32 {
            let r = (i as f32 + 0.5) / 10_000.0;
            let s = ((i * 37) % 100) as f32 / 100.0;
            if let Some(sp) = t.pick(r, s) {
                any += 1;
                counts[match sp {
                    Spawn::FlowerRing { flowers } => {
                        assert!((4..=8).contains(&flowers));
                        0
                    }
                    Spawn::SingleFlower => 1,
                    Spawn::EnemyRing { heads } => {
                        assert!((3..=6).contains(&heads));
                        2
                    }
                    Spawn::Tower => 3,
                }] += 1;
            }
        }
        assert!((1100..=1300).contains(&any), "{any}");
        assert!(counts.iter().all(|&c| c > 0), "{counts:?}");
    }

    #[test]
    fn a_snapshot_mirrors_into_an_identical_world() {
        let mut w = FlowerWorld::new();
        let u = w.add_unit(FlowerUnit::ring(Vec3::ZERO, 5, 250.0));
        w.add_unit(FlowerUnit::single(Vec3::new(900.0, 0.0, 0.0)));
        w.add_tower(Vec3::new(0.0, 0.0, 2000.0), 0.0);
        w.add_head(Vec3::new(500.0, 0.0, 500.0), 30.0);
        let mut ev = Vec::new();
        w.spray_flower(u, 2, &mut ev);
        run(&mut w, Vec3::new(0.0, 0.0, 1500.0), 50);
        let mut copy = FlowerWorld::new();
        copy.mirror(&w.items());
        assert_eq!(copy.items(), w.items());
        let (mut a, mut b) = (Vec::new(), Vec::new());
        copy.targets(&mut a);
        w.targets(&mut b);
        assert_eq!(a, b);
    }

    #[test]
    fn disc_values_load() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/params_enemy");
        let Ok(d) = std::fs::read(p.join("samboflower.prm")) else {
            eprintln!("SKIP: no extracted assets");
            return;
        };
        let f = FlowerParams::from_prm(&Prm::parse(&d).unwrap());
        assert_eq!(f, FlowerParams::default());
        let d = std::fs::read(p.join("hanasambo.prm")).unwrap();
        assert_eq!(TowerParams::from_prm(&Prm::parse(&d).unwrap()), TowerParams::default());
        let d = std::fs::read(p.join("sambohead.prm")).unwrap();
        let h = HeadParams::from_prm(&Prm::parse(&d).unwrap());
        assert_eq!(h.jump_prepare_time, 300);
    }
}
