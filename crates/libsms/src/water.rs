//! FLUDD's water particles (`TModelWaterManager`, Player/ModelWaterManager.cpp): what happens
//! to the water after a [`WaterEmit`] leaves the nozzle. Each particle flies under its type's
//! gravity, and on hitting the ground or a wall either splashes away or sticks and runs along
//! the surface for a while before drying up.
//!
//! The sim reports emission; a host that wants the game's water feeds those events into a
//! [`WaterManager`] and steps it once per 30 Hz frame. Particle velocities are per frame, like
//! Mario's `mVel`. Collision goes through the same [`Host`] probes Mario uses.

use glam::Vec3;

use crate::event::WaterEmit;
use crate::host::{Enemy, Host, SurfaceFlags};
use crate::params::{Params, WaterParticle};

/// `SLOT_NUM`.
pub const SLOTS: usize = 256;

/// A ground mark left by FLUDD. Surface samples keep the mark on slopes and stop it
/// crossing a ledge. Geometry and slide eligibility share this footprint.
#[derive(Debug, Clone)]
pub struct WetPatch {
    pub pos: Vec3,
    pub normal: Vec3,
    pub radius: f32,
    /// 30 Hz frames remaining, independently of the visible airborne droplets.
    pub life: f32,
    pub duration: f32,
    /// A 3x3 grid over the mark; absent samples are off the receiving surface.
    pub surface: [Option<Vec3>; 9],
}

impl WetPatch {
    pub fn axes(&self) -> (Vec3, Vec3) {
        let a = self.normal.any_orthonormal_vector();
        (a, self.normal.cross(a))
    }

    pub fn cell(&self, x: usize, y: usize) -> Option<[Vec3; 4]> {
        let i = y * 3 + x;
        Some([
            self.surface[i]?,
            self.surface[i + 1]?,
            self.surface[i + 4]?,
            self.surface[i + 3]?,
        ])
    }

    pub fn contains(&self, pos: Vec3) -> bool {
        let d = pos - self.pos;
        let (a, b) = self.axes();
        let (x, y) = (d.dot(a), d.dot(b));
        self.life > 0.0
            && d.dot(self.normal).abs() < 25.0
            && x * x + y * y < self.radius * self.radius
            && self
                .cell(usize::from(x >= 0.0), usize::from(y >= 0.0))
                .is_some()
    }
}

/// Where a particle is (`mParticleFlagSOA & 0xF`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Flying = 1,
    /// Stuck to the ground, running downhill.
    Ground = 2,
    /// Stuck to a wall, running down it.
    Wall = 3,
}

/// How many times as long the Rocket's precision slug flies as its water type's own lifetime (host extension).
pub const PRECISE_LIFE: f32 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Particle {
    /// Body runoff is not a fresh spray contact: it must not re-wet its emitter forever.
    pub body_drip: bool,
    pub pos: Vec3,
    /// Where it left the nozzle: a host drawing it never shows any of it behind this.
    pub spawn: Vec3,
    /// Units per original trajectory frame; use [`Self::velocity`] for world motion.
    pub vel: Vec3,
    /// Host-carried motion exempt from the droplet's own flight-speed cap.
    carry: Vec3,
    pub size: f32,
    /// Frames left (`mParticleLifetimeSOA`).
    pub life: f32,
    pub state: State,
    pub kind: usize,
    pub attack: i16,
    /// [`WaterManager::reach`] when it left the nozzle (1 for all but Spray's).
    pub reach: f32,
    /// Flight time scale captured from the host tuning (Spray's stream or all other water);
    /// ignored on surfaces.
    pub travel_speed: f32,
    /// From a light press's burst ([`WaterEmit::burst`]).
    pub burst: bool,
    /// From a spin's spray ([`WaterEmit::spin`]).
    pub spin: bool,
    /// From the Rocket's precision slug ([`WaterEmit::precise`]).
    pub precise: bool,
    /// The surface it sticks to: normal and whether it is wet (`isWaterSlip`).
    surface: Option<(Vec3, bool)>,
}

impl Particle {
    /// World velocity per 30 Hz frame, including the host's faster Spray flight.
    /// Renderers use it for interpolation and streak length to preserve stream coverage.
    pub fn velocity(&self) -> Vec3 {
        self.vel * self.flight_step()
    }

    fn flight_step(&self) -> f32 {
        if self.state == State::Flying { self.travel_speed } else { 1.0 }
    }
}

/// Something the host may show or play.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WaterEvent {
    /// `gpSplashManager->newSplash(pos, 5)`.
    Splash(Vec3),
    /// `splashSound`: `MSD_SE_WT_GND` at a volume of the particle's size.
    SplashSound { pos: Vec3, size: f32 },
    /// Fell into a body of water (`MSD_SE_WT_INTO_WATER`, a tiny ripple).
    IntoWater(Vec3),
    /// A stuck particle drying (`MSD_SE_PO_WT_DRY_UP`).
    DryUp(Vec3),
    /// A flying particle hit the enemy `id` (`HIT_MESSAGE_SPRAYED_BY_WATER`) with its
    /// `attack` (`getParticleAttack`: hit points, at least 1 to the game's small enemies) and
    /// was spent on it. What that does to the enemy is the host's.
    Hit {
        id: u64,
        pos: Vec3,
        attack: i16,
        /// The particle came from a light press's burst ([`WaterEmit::burst`]).
        burst: bool,
        /// The particle came from a spin jump's spray ([`WaterEmit::spin`]).
        spin: bool,
    },
}

/// The manager's constants (`TModelWaterManager::load`, `gWaterManagerPlaneInfo`).
mod k {
    /// `unk5D88[0]`: chance a particle sticks to the ground instead of vanishing.
    pub const GROUND_STICK: f32 = 0.5;
    /// `unk5D88[2]`: the same for walls.
    pub const WALL_STICK: f32 = 0.5;
    /// `unk5D88[6]`: stuck particles closer than this times their sizes merge.
    pub const MERGE: f32 = 2.5;
    /// `unk5D88[7]`: frames before a new particle is drawn.
    pub const DRAW_DELAY: f32 = 1.0;
    /// `unk5D88[8]`: chance a particle spent on an actor splashes.
    pub const ACTOR_SPLASH: f32 = 0.3;
    /// `mStaticHitActor.initHitActor(..., 50, 80, ...)`: a particle's attack cylinder.
    pub const HIT_RADIUS: f32 = 50.0;
    pub const HIT_HEIGHT: f32 = 80.0;
    /// `unk5D88[9]`, `[10]`: splash chances.
    pub const GROUND_SPLASH: f32 = 0.5;
    pub const WALL_SPLASH: f32 = 0.8;
    /// `unk5D68` / `unk5D6C`: life cap for stuck particles.
    pub const STUCK_LIFE: f32 = 256.0;
    /// `unk5E08`: particles this far from Mario (XZ) vanish.
    pub const RANGE: f32 = 5000.0;
    /// Friction, fall and dry rate on wet (`isWaterSlip`) and other surfaces.
    pub const FRICTION: (f32, f32) = (0.93, 0.7);
    pub const FALL: (f32, f32) = (-3.0, -1.0);
    pub const VANISH: (f32, f32) = (2.0, 1.0);
}

#[derive(Debug, Clone)]
pub struct WaterManager {
    wet_emit: crate::params::WetWaterEmit,
    /// Actual droplet sweeps from the most recent water frame, clipped by world contacts.
    /// Hosts may test these against hazards without changing the water simulation.
    pub contacts: Vec<(Vec3, Vec3, f32)>,
    pub particles: Vec<Particle>,
    pub wet_patches: Vec<WetPatch>,
    pub types: [WaterParticle; 17],
    pub events: Vec<WaterEvent>,
    /// Where the water washed goop off since the host last took them: `(pos, radius)`, a
    /// ground impact's `mCleanSize * 10` (`splashGround`) or a wall's `mCleanSize * 32`
    /// (`splashWall`). The host cleans its [`crate::goop::GoopField`] there and clears this.
    pub cleans: Vec<(Vec3, f32)>,
    /// Host tuning: the Spray nozzle's water leaves at this times its speed, and its
    /// horizontal speed cap is this times its size, so a spray lands about this many times
    /// further. 1: the game's. The other nozzles' water is the game's.
    pub reach: f32,
    /// Spray-only flight speed, independent of reach. Accelerates its trajectory and
    /// airborne lifetime together, preserving its arc and range. 1: the game's.
    pub spray_speed: f32,
    /// Flight time scale for every water particle that is not the Spray's stream: body runoff
    /// (the drips off a wet Mario) and the other FLUDD nozzles' water and splashes. Accelerates
    /// each trajectory and its airborne lifetime together, preserving its arc and range. 1: the
    /// game's. Yoshi's juice keeps its own rate.
    pub water_speed: f32,
    /// A light press's burst has been emitted: lay its wet circles at Mario's feet on the next step,
    /// facing this way (horizontal) with this particle type.
    stamp: Option<(Vec3, usize)>,
    seed: u32,
}

/// A light press's burst wets this many circles, from Mario's feet forward in his aim.
pub const BURST_CIRCLES: usize = 6;
/// Each circle's radius is this share of a drop's ground coverage (`mCleanSize * 10`), and the next
/// circle is this many radii on, so the six overlap into one trail from his feet.
const BURST_RADIUS: f32 = 0.5;
const BURST_SPACING: f32 = 0.8;

impl WaterManager {
    pub fn new(params: &Params) -> WaterManager {
        WaterManager {
            wet_emit: params.wet_emit.clone(),
            contacts: Vec::with_capacity(SLOTS),
            particles: Vec::with_capacity(SLOTS),
            wet_patches: Vec::with_capacity(SLOTS),
            types: params.water_particles.clone(),
            events: Vec::new(),
            cleans: Vec::new(),
            reach: 1.0,
            spray_speed: 1.0,
            water_speed: 1.0,
            stamp: None,
            seed: 0x2545_F491,
        }
    }

    /// `MsRandF`: 0..1.
    fn rand(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed >> 8) as f32 / (1u32 << 24) as f32
    }

    /// `rand11`: -1..1.
    fn rand11(&mut self) -> f32 {
        self.rand() * 2.0 - 1.0
    }

    fn kind(&self, i: usize) -> &WaterParticle {
        &self.types[self.particles[i].kind.min(16)]
    }

    /// `emitRequest` / `makeEmit`: adds up to `w.count` particles; returns how many fit.
    pub fn emit(&mut self, w: &WaterEmit) -> u32 {
        self.emit_inner(w,Vec3::ZERO)
    }

    /// `TMario::checkWet` -> `emitRequest(*unk158)`: model-water type from the disc,
    /// with its own lifetime and full 0.3 * Mario velocity, unaffected by Spray tuning.
    pub fn emit_body_drip(&mut self, pos: Vec3, velocity: Vec3) -> u32 {
        let p = &self.wet_emit;
        let w = WaterEmit {
            nozzle: crate::NozzleKind::Underwater, pos, dir: p.dir, power: p.power,
            count: p.count.max(0) as u32, attack: p.attack,
            hit_radius: p.hit_radius, hit_height: p.hit_height, inherit: velocity,
            size: p.size, size_tremble: p.size_tremble, dir_tremble: p.dir_tremble,
            pow_tremble: p.pow_tremble, particle_type: p.particle_type,
            burst: false, spin: false, precise: false,
        };
        let first = self.particles.len();
        let count = self.emit_carried(&w);
        for p in &mut self.particles[first..] {
            p.life = self.wet_emit.alive.max(0) as f32;
            p.body_drip = true;
        }
        count
    }

    /// Emit while retaining the emitter's full carried motion. The speed cap applies
    /// to the launched fluid relative to the emitter, not to its inherited movement.
    pub fn emit_carried(&mut self,w:&WaterEmit)->u32 {
        self.emit_inner(w,w.inherit)
    }

    fn emit_inner(&mut self,w:&WaterEmit,carry:Vec3)->u32 {
        let kind = (w.particle_type.max(0) as usize).min(16);
        if w.burst && w.count > 0 {
            let flat = Vec3::new(w.dir.x, 0.0, w.dir.z);
            self.stamp = Some((flat.normalize_or(Vec3::Z), kind));
        }
        let mut n = 0;
        while n < w.count && self.particles.len() < SLOTS {
            let tremble = Vec3::new(self.rand11(), self.rand11(), self.rand11()) * w.dir_tremble;
            let mut dir = w.dir + tremble;
            if dir == Vec3::ZERO {
                dir.y = -1.0;
            }
            let reach = if w.nozzle == crate::params::NozzleKind::Spray {
                self.reach
            } else {
                1.0
            };
            let travel_speed = if w.nozzle == crate::params::NozzleKind::Spray && self.spray_speed.is_finite() {
                self.spray_speed.clamp(1.0, 2.0)
            } else if w.nozzle != crate::params::NozzleKind::Yoshi && self.water_speed.is_finite() {
                self.water_speed.clamp(1.0, 2.0)
            } else {
                1.0
            };
            let speed = (self.rand11() * w.pow_tremble + w.power) * reach;
            let vel = dir.normalize() * speed + w.inherit;
            let size = self.rand11() * w.size_tremble + w.size;
            self.particles.push(Particle {
                body_drip: false,
                pos: w.pos,
                spawn: w.pos,
                vel,
                carry,
                size,
                // A sniper's shot carries far: the slug lives `PRECISE_LIFE` times as long.
                life: self.types[kind].alive * if w.precise { PRECISE_LIFE } else { 1.0 },
                state: State::Flying,
                kind,
                attack: w.attack,
                reach,
                travel_speed,
                burst: w.burst,
                spin: w.spin,
                precise: w.precise,
                surface: None,
            });
            n += 1;
        }
        n
    }

    /// Should a particle be drawn as flying water yet (`calcDrawVtx`)?
    pub fn drawn(&self, p: &Particle) -> bool {
        p.state == State::Flying && p.life < self.types[p.kind.min(16)].alive - k::DRAW_DELAY
    }

    /// The type's stretch along the velocity when drawn (`mExtension`).
    pub fn extension(&self, p: &Particle) -> f32 {
        self.types[p.kind.min(16)].extension
    }

    /// `TObjHitCheck::checkWater` (Strategic/ObjHitCheck.cpp) and the top of `move`: a
    /// flying particle whose attack cylinder reaches an enemy is spent on it, with a splash
    /// sound and sometimes a splash. Mario's own water never hits him. One [`Host::enemies`]
    /// query covers all the flying particles.
    fn hit_enemies(&mut self, host: &dyn Host) {
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for p in self.particles.iter().filter(|p| p.state == State::Flying) {
            lo = lo.min(p.pos);
            hi = hi.max(p.pos);
        }
        if lo.x > hi.x {
            return;
        }
        let mut found: Vec<Enemy> = Vec::new();
        let reach = (hi - lo).length() * 0.5 + k::HIT_RADIUS + k::HIT_HEIGHT;
        host.enemies((lo + hi) * 0.5, reach, &mut found);
        if found.is_empty() {
            return;
        }
        for i in 0..self.particles.len() {
            let p = self.particles[i];
            if p.state != State::Flying || p.life <= 0.0 {
                continue;
            }
            let Some(e) = found
                .iter()
                .find(|e| e.reached_by(p.pos, k::HIT_RADIUS, k::HIT_HEIGHT))
            else {
                continue;
            };
            self.events.push(WaterEvent::Hit {
                id: e.id,
                pos: p.pos,
                attack: p.attack,
                burst: p.burst,
                spin: p.spin,
            });
            self.events.push(WaterEvent::SplashSound {
                pos: p.pos,
                size: p.size,
            });
            if self.rand() < k::ACTOR_SPLASH {
                self.events.push(WaterEvent::Splash(p.pos));
            }
            self.particles[i].life = 0.0;
        }
    }

    /// `TModelWaterManager::move`: one frame.
    /// Ground wetness left by impacts. Unlike the old 300-unit particle proximity check
    /// (`askHitWaterParticleOnGround`), this uses the footprint the host actually draws.
    pub fn on_ground_near(&self, pos: Vec3) -> bool {
        self.wet_patches.iter().any(|p| p.contains(pos))
    }

    /// ModelWaterManager.cpp, `splashGround`: ground coverage uses mCleanSize * 10.
    /// RoR2 has no Sunshine pollution map, so keep bounded, drying surface marks instead.
    fn wet_ground(&mut self, host: &dyn Host, pos: Vec3, normal: Vec3, kind: usize, burst: bool) {
        self.wet_ground_sized(host, pos, normal, kind, if burst { BURST_RADIUS } else { 1.0 });
    }

    /// As [`Self::wet_ground`], at `scale` times a drop's coverage.
    fn wet_ground_sized(&mut self, host: &dyn Host, pos: Vec3, normal: Vec3, kind: usize, scale: f32) {
        let radius = (self.types[kind].clean_size * 10.0 * scale).clamp(20.0, 400.0);
        if self.cleans.len() < 256 {
            self.cleans.push((pos, radius));
        }
        let duration = self.types[kind].alive.clamp(30.0, k::STUCK_LIFE);
        let normal = normal.normalize_or(Vec3::Y);
        // Refresh nearby marks without building an opaque stack of overlapping decals.
        if let Some(p) = self.wet_patches.iter_mut().find(|p| {
            p.normal.dot(normal) > 0.95
                && (pos - p.pos).dot(p.normal).abs() < 20.0
                && pos.distance(p.pos) < radius.min(p.radius) * 0.75
                && p.radius >= radius
        }) {
            p.life = duration;
            p.duration = duration;
            return;
        }
        let a = normal.any_orthonormal_vector();
        let b = normal.cross(a);
        let surface = std::array::from_fn(|i| {
            let q = pos + a * ((i % 3) as f32 - 1.0) * radius + b * ((i / 3) as f32 - 1.0) * radius;
            host.ground(q + Vec3::Y * 25.0)
                .filter(|g| {
                    !g.flags
                        .has(SurfaceFlags::ILLEGAL | SurfaceFlags::WATER_SURFACE)
                        && (g.height - q.y).abs() < 40.0
                        && g.normal.dot(normal) > 0.8
                })
                .map(|g| Vec3::new(q.x, g.height, q.z) + g.normal * 1.0)
        });
        if self.wet_patches.len() == SLOTS {
            let oldest = self
                .wet_patches
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| a.life.total_cmp(&b.life))
                .map(|(i, _)| i)
                .unwrap();
            self.wet_patches.swap_remove(oldest);
        }
        self.wet_patches.push(WetPatch {
            pos,
            normal,
            radius,
            life: duration,
            duration,
            surface,
        });
    }

    /// A stand-in for a belly slide over water, from before the game's own `frontSlipEffect`
    /// spray (`MS_M_WATSLIDE_*`, now a particle effect: `Mario::front_slip_effect`) was ported:
    /// splashes thrown off either side of `pos`. Hosts leave it off (`SmsTuning::slide_splashes`).
    pub fn slide_splashes(&mut self, pos: Vec3) {
        for _ in 0..2 {
            let off = Vec3::new(self.rand() * 120.0 - 60.0, 10.0, self.rand() * 120.0 - 60.0);
            self.events.push(WaterEvent::Splash(pos + off));
        }
    }

    /// A light press's burst wets [`BURST_CIRCLES`] circles in a trail from Mario's feet forward, each on the
    /// ground there (a ledge ends the trail), so the ground he is about to belly slide on is wet whatever
    /// the drops do.
    fn lay_burst(&mut self, host: &dyn Host, mario: Vec3, forward: Vec3, kind: usize) {
        let radius = (self.types[kind].clean_size * 10.0 * BURST_RADIUS).clamp(20.0, 400.0);
        for i in 0..BURST_CIRCLES {
            let at = mario + forward * (i as f32 * radius * BURST_SPACING);
            let Some(g) = host.ground(Vec3::new(at.x, mario.y + 25.0, at.z)) else { break };
            if g.flags.has(SurfaceFlags::ILLEGAL | SurfaceFlags::WATER_SURFACE) || (g.height - mario.y).abs() > 80.0 {
                break;
            }
            self.wet_ground_sized(host, Vec3::new(at.x, g.height, at.z), g.normal, kind, BURST_RADIUS);
        }
    }

    pub fn step(&mut self, host: &dyn Host, mario: Vec3) {
        self.contacts.clear();
        if let Some((forward, kind)) = self.stamp.take() {
            self.lay_burst(host, mario, forward, kind);
        }
        for p in &mut self.wet_patches {
            p.life -= 1.0;
        }
        self.wet_patches
            .retain(|p| p.life > 0.0 && p.pos.distance_squared(mario) < k::RANGE * k::RANGE);
        self.hit_enemies(host);
        let starts: Vec<_> = self.particles.iter()
            .map(|p| (p.pos, p.size, p.life > 0.0)).collect();
        // Fly, slide, fall; cap horizontal speed at the particle's size.
        for i in 0..self.particles.len() {
            // The Rocket's precision slug flies straight: no gravity, no horizontal speed cap (the cap is the
            // particle's size, and the slug is narrow and fast).
            let gravity = if self.particles[i].precise { 0.0 } else { self.kind(i).gravity };
            let p = &mut self.particles[i];
            let d = mario - p.pos;
            if d.x * d.x + d.z * d.z > k::RANGE * k::RANGE {
                p.life = 0.0;
                continue;
            }
            let carry=if p.state==State::Flying {p.carry}else{Vec3::ZERO};
            let relative=p.vel-carry;
            let h2 = relative.x * relative.x + relative.z * relative.z;
            let cap = p.size * p.reach;
            if h2 > cap * cap && !p.precise {
                let s = (cap - 0.1) / h2.sqrt();
                p.vel.x=relative.x*s+carry.x;
                p.vel.z=relative.z*s+carry.z;
            }
            match p.state {
                State::Flying => {
                    // Host time scaling of ModelWaterManager.cpp::move. The half-step
                    // correction follows the same discrete-gravity parabola at fractional
                    // ages: x(t)=v*t, y(t)=v_y*t+g*t*(t+1)/2. At speed 1 this is the game.
                    let dt = p.flight_step();
                    p.pos += p.vel * dt + Vec3::Y * (gravity * dt * (dt + 1.0) * 0.5);
                    p.vel.y += gravity * dt;
                }
                State::Ground => {
                    let (n, wet) = p.surface.unwrap_or((Vec3::Y, false));
                    let f = if wet { k::FRICTION.0 } else { k::FRICTION.1 };
                    p.vel = Vec3::new(p.vel.x * f + n.x, gravity, p.vel.z * f + n.z);
                    p.pos += p.vel;
                }
                State::Wall => {
                    let wet = p.surface.is_some_and(|s| s.1);
                    p.pos.y += if wet { k::FALL.0 } else { k::FALL.1 };
                }
            }
        }

        // Collisions.
        let mut walls = Vec::new();
        for i in 0..self.particles.len() {
            if self.particles[i].life <= 0.0 {
                continue;
            }
            let (gravity, magnify) = (self.kind(i).gravity, self.kind(i).magnify);
            let p = self.particles[i];
            let mut floor = None;
            if p.vel.y < 0.0 {
                let probe = Vec3::new(p.pos.x, p.pos.y - p.vel.y, p.pos.z);
                if let Some(g) = host.ground(probe)
                    && !g.flags.has(SurfaceFlags::ILLEGAL)
                {
                    floor = Some(g.height);
                    if p.pos.y <= g.height + 1.0 {
                        if g.flags.has(SurfaceFlags::WATER_SURFACE) {
                            self.particles[i].life = 0.0;
                            self.events.push(WaterEvent::IntoWater(p.pos));
                            continue;
                        }
                        self.particles[i].pos.y = g.height - gravity;
                        match p.state {
                            State::Wall => {
                                self.particles[i].life = 0.0;
                                continue;
                            }
                            State::Flying => {
                                // Every ground impact wets it, including drops whose visible
                                // particle splashes away instead of sticking.
                                self.wet_ground(
                                    host,
                                    Vec3::new(p.pos.x, g.height, p.pos.z),
                                    g.normal,
                                    p.kind,
                                    p.burst,
                                );
                                if self.rand() < k::GROUND_SPLASH {
                                    self.events.push(WaterEvent::Splash(p.pos));
                                }
                                self.events.push(WaterEvent::SplashSound {
                                    pos: p.pos,
                                    size: p.size,
                                });
                                self.particles[i].size *= magnify;
                                if self.rand() < k::GROUND_STICK {
                                    self.particles[i].state = State::Ground;
                                } else {
                                    self.particles[i].life = 0.0;
                                    continue;
                                }
                            }
                            State::Ground => {}
                        }
                        self.particles[i].surface =
                            Some((g.normal, g.flags.has(SurfaceFlags::WET)));
                        continue;
                    }
                }
            }
            walls.clear();
            let at = Vec3::new(p.pos.x, p.pos.y + p.size * 0.5, p.pos.z);
            let moved = host.walls(at, p.size, 1, &mut walls);
            if let Some(w) = walls.first().copied() {
                if p.state == State::Ground {
                    self.particles[i].life = 0.0;
                    continue;
                }
                if floor.is_some_and(|f| p.pos.y < p.size * magnify + f) {
                    self.particles[i].life = 0.0;
                    continue;
                }
                // Onto the wall plane.
                self.particles[i].pos.x = moved.x - w.normal.x * p.size;
                self.particles[i].pos.z = moved.z - w.normal.z * p.size;
                if p.state == State::Flying {
                    let at = self.particles[i].pos + w.normal * p.size;
                    if self.cleans.len() < 256 {
                        self.cleans.push((at, self.types[p.kind].clean_size * 32.0));
                    }
                    if self.rand() < k::WALL_SPLASH {
                        self.events.push(WaterEvent::Splash(at));
                    }
                    self.events.push(WaterEvent::SplashSound {
                        pos: p.pos,
                        size: p.size,
                    });
                    self.particles[i].size *= magnify;
                    if self.rand() < k::WALL_STICK {
                        self.particles[i].state = State::Wall;
                    } else {
                        self.particles[i].life = 0.0;
                        continue;
                    }
                }
                self.particles[i].surface = Some((w.normal, w.flags.has(SurfaceFlags::WET)));
            } else {
                if p.vel.y >= 0.0
                    && let Some(r) = host.roof(p.pos)
                    && p.pos.y + p.size > r.height
                {
                    let q = &mut self.particles[i];
                    q.pos.y = r.height - p.size;
                    q.vel = Vec3::ZERO;
                }
                self.particles[i].state = State::Flying;
            }
        }

        // Age; stuck particles dry and neighbours merge (`unk5D60 & 0x40`).
        let (mut last_ground, mut last_wall): (Option<usize>, Option<usize>) = (None, None);
        for i in 0..self.particles.len() {
            let p = self.particles[i];
            if p.life <= 0.0 {
                continue;
            }
            let wet = p.surface.is_some_and(|s| s.1);
            let vanish = if wet { k::VANISH.0 } else { k::VANISH.1 };
            let last = match p.state {
                State::Flying => {
                    self.particles[i].life -= p.flight_step();
                    continue;
                }
                State::Ground => {
                    self.events.push(WaterEvent::DryUp(p.pos));
                    &mut last_ground
                }
                State::Wall => &mut last_wall,
            };
            let q = &mut self.particles[i];
            q.life = q.life.min(k::STUCK_LIFE) - vanish;
            match *last {
                Some(j) => {
                    let o = self.particles[j];
                    if (p.pos - o.pos).length() < k::MERGE * (p.size + o.size) {
                        self.particles[j].size = p.size;
                        self.particles[j].life = self.particles[i].life;
                        self.particles[i].life = 0.0;
                    } else {
                        *last = Some(i);
                    }
                }
                None => *last = Some(i),
            }
        }
        for ((from, size, alive), p) in starts.into_iter().zip(&self.particles) {
            if alive {
                if !p.body_drip {
                    self.contacts.push((from, p.pos, size.max(0.0)));
                }
            }
        }
        self.particles.retain(|p| p.life > 0.0);
    }

    /// Whether a real water sweep touches a capsule segment. No synthetic wet-slide
    /// stamp is used: spraying must reach the flame itself.
    pub fn touches_segment(&self, a: Vec3, b: Vec3, radius: f32) -> bool {
        if !a.is_finite() || !b.is_finite() || !radius.is_finite() || radius < 0.0 {
            return false;
        }
        self.contacts.iter().any(|&(from, to, size)| {
            segment_distance_squared(a, b, from, to) <= (radius + size).powi(2)
        })
    }
}

/// Closest points of two finite line segments, including parallel and point segments.
fn segment_distance_squared(a: Vec3, b: Vec3, c: Vec3, d: Vec3) -> f32 {
    let (u, v, w) = (b - a, d - c, a - c);
    let (aa, bb, cc, dd, ee) = (u.dot(u), u.dot(v), v.dot(v), u.dot(w), v.dot(w));
    if aa <= 1e-8 && cc <= 1e-8 { return w.length_squared(); }
    if aa <= 1e-8 { return (w - v * (ee / cc).clamp(0.0, 1.0)).length_squared(); }
    if cc <= 1e-8 { return (w + u * (-dd / aa).clamp(0.0, 1.0)).length_squared(); }
    let denom = aa * cc - bb * bb;
    let mut s = if denom > 1e-8 { ((bb * ee - cc * dd) / denom).clamp(0.0, 1.0) } else { 0.0 };
    let mut t = (bb * s + ee) / cc;
    if t < 0.0 { t = 0.0; s = (-dd / aa).clamp(0.0, 1.0); }
    else if t > 1.0 { t = 1.0; s = ((bb - dd) / aa).clamp(0.0, 1.0); }
    (w + u * s - v * t).length_squared()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runoff_uses_disc_emit_parameters_and_ignores_spray_bonuses() {
        let Some(dir) = crate::assets::find(None) else { eprintln!("SKIP: no disc cache"); return; };
        let (params, _) = Params::from_dir(&dir.join("params_mario"));
        let mut water = WaterManager::new(&params);
        water.reach = 8.0;
        water.spray_speed = 2.0;
        assert_eq!(params.wet_emit.count, 3);
        assert_eq!(params.wet_emit.alive, 128);
        assert_eq!(params.wet_emit.particle_type, 1);
        assert_eq!(water.emit_body_drip(Vec3::Y * 200.0, Vec3::X * 3.0), 3);
        assert!(water.particles.iter().all(|p| p.body_drip && p.kind == 1 && p.life == 128.0
            && p.reach == 1.0 && p.travel_speed == 1.0));
        water.step(&crate::FlatHost::default(), Vec3::ZERO);
        assert!(water.contacts.is_empty(), "runoff must not continually re-wet Mario");
    }

    #[test]
    fn water_sweeps_touch_only_intersected_fire_segments() {
        let mut water = WaterManager::new(&Params::default());
        // A fast drop crosses a flame between water frames; neither endpoint is inside it.
        water.contacts.push((Vec3::new(-100., 20., 0.), Vec3::new(100., 20., 0.), 2.));
        assert!(water.touches_segment(Vec3::ZERO, Vec3::Y * 40., 3.));
        assert!(!water.touches_segment(Vec3::Z * 10., Vec3::Z * 10. + Vec3::Y * 40., 3.));
        assert!(water.touches_segment(Vec3::new(0., 20., 5.), Vec3::new(0., 20., 5.), 3.));
        assert!(!water.touches_segment(Vec3::splat(f32::NAN), Vec3::ZERO, 1.));
        // Parallel and degenerate segments must not produce NaNs or false intersections.
        assert_eq!(segment_distance_squared(Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::X + Vec3::Y), 1.);
        assert_eq!(segment_distance_squared(Vec3::ZERO, Vec3::ZERO, Vec3::X, Vec3::X), 1.);
        water.step(&crate::FlatHost::default(), Vec3::ZERO);
        assert!(!water.touches_segment(Vec3::ZERO, Vec3::Y * 40., 3.), "old sweeps survived an empty water frame");
    }
    use crate::host::FlatHost;
    use crate::params::NozzleKind;

    fn emit(dir: Vec3) -> WaterEmit {
        WaterEmit {
            nozzle: NozzleKind::Spray,
            pos: Vec3::new(0.0, 150.0, 0.0),
            dir,
            power: 40.0,
            count: 1,
            attack: 1,
            hit_radius: 50.0,
            hit_height: 80.0,
            inherit: Vec3::ZERO,
            size: 50.0,
            size_tremble: 0.0,
            dir_tremble: 0.0,
            pow_tremble: 0.0,
            particle_type: 0,
            burst: false,
            spin: false,
            precise: false,
        }
    }

    /// The Rocket's precision slug (`WaterEmit::precise`) flies like a sniper's shot: straight and at its full
    /// speed even though it is narrow (the flight-speed cap is a particle's size), where the same shot without
    /// the flag crawls and sags.
    #[test]
    fn a_precision_slug_flies_straight_and_fast_though_narrow() {
        let fly = |precise: bool| {
            let mut water = WaterManager::new(&Params::default());
            let mut shot = emit(Vec3::Z);
            shot.nozzle = NozzleKind::Rocket;
            shot.power = 100.0;
            shot.size = 30.0;
            shot.precise = precise;
            water.emit(&shot);
            for _ in 0..15 {
                water.step(&FlatHost::default(), Vec3::new(0.0, 150.0, 0.0));
            }
            let p = water.particles.first().copied();
            p.map(|p| (p.pos.z, p.pos.y))
        };
        let (z, y) = fly(true).expect("the slug vanished");
        let (z_plain, y_plain) = fly(false).unwrap_or((0.0, 0.0));
        assert!((y - 150.0).abs() < 0.01, "the slug sagged to {y}");
        assert!(z > 3.0 * z_plain.max(1.0), "the slug ({z}) is not much faster than the plain shot ({z_plain})");
        assert!(y_plain < 150.0 || z_plain < 100.0, "the plain shot didn't show the cap or the drop: z {z_plain} y {y_plain}");
    }

    #[test]
    fn carried_juice_keeps_running_velocity_after_the_flight_speed_cap() {
        let mut water=WaterManager::new(&Params::default());
        let mut shot=emit(Vec3::Z);
        shot.nozzle=NozzleKind::Yoshi;shot.particle_type=12;
        shot.pos.y=1000.;shot.power=45.;shot.size=35.;shot.inherit=Vec3::new(0.,0.,80.);
        water.emit_carried(&shot);
        let start=shot.pos;
        for frame in 1..=4 {
            water.step(&FlatHost::default(),Vec3::new(0.,1000.,80.*frame as f32));
            let p=&water.particles[0];
            assert!(p.velocity().z>80.,"juice must overtake its running emitter");
            assert!(p.pos.z-start.z>80.*frame as f32,"juice fell behind its emitter");
        }
        // The ordinary water path retains its original total-speed cap.
        let mut ordinary=WaterManager::new(&Params::default());ordinary.emit(&shot);
        ordinary.step(&FlatHost::default(),Vec3::ZERO);
        assert!(ordinary.particles[0].velocity().z<80.);
    }

    #[test]
    fn ground_impacts_leave_wetness_after_droplets_disappear_then_dry() {
        let mut w = WaterManager::new(&Params::default());
        let mut drop = emit(-Vec3::Y);
        drop.pos.y = 5.0;
        w.emit(&drop);
        w.step(&FlatHost::default(), Vec3::ZERO);
        let at = w.wet_patches[0].pos;
        w.particles.clear();
        assert!(w.on_ground_near(at));
        assert!(!w.on_ground_near(at + Vec3::X * 500.0));
        assert!(!w.on_ground_near(at + Vec3::Y * 100.0));
        for _ in 0..256 {
            w.step(&FlatHost::default(), Vec3::ZERO);
        }
        assert!(!w.on_ground_near(at));
        assert!(w.wet_patches.is_empty());
    }

    #[test]
    fn a_light_burst_wets_six_circles_from_his_feet_forward() {
        let mut w = WaterManager::new(&Params::default());
        let mut drop = emit(Vec3::new(0.0, -0.2, 1.0));
        drop.pos.y = 150.0;
        drop.burst = true;
        w.emit(&drop);
        // Before the drops land: the six circles, at his feet and out ahead in his aim, not off to a side.
        w.step(&FlatHost::default(), Vec3::ZERO);
        let radius = w.types[0].clean_size * 10.0 * BURST_RADIUS;
        let mut trail: Vec<_> = w.wet_patches.iter().filter(|p| p.pos.x.abs() < 1.0).map(|p| p.pos.z).collect();
        trail.sort_by(f32::total_cmp);
        assert!(trail.len() >= BURST_CIRCLES, "{} circles", trail.len());
        assert!(trail[0].abs() < 1.0, "none at his feet");
        for pair in trail.windows(2).take(BURST_CIRCLES - 1) {
            assert!((pair[1] - pair[0] - radius * BURST_SPACING).abs() < 1.0, "spacing {:?}", pair);
        }
        assert!(w.on_ground_near(Vec3::ZERO));
        assert!(w.on_ground_near(Vec3::new(0.0, 0.0, radius * BURST_SPACING * 5.0)));
        assert!(!w.on_ground_near(Vec3::new(0.0, 0.0, radius * BURST_SPACING * 5.0 + radius * 1.5)));
        assert!(!w.on_ground_near(Vec3::new(radius * 1.5, 0.0, 0.0)));
        // A plain spray lays no trail of its own.
        let mut normal = WaterManager::new(&Params::default());
        drop.burst = false;
        normal.emit(&drop);
        normal.step(&FlatHost::default(), Vec3::ZERO);
        assert!(!normal.on_ground_near(Vec3::ZERO));
    }

    #[test]
    fn a_burst_trail_stops_at_a_ledge() {
        struct Ledge;
        impl Host for Ledge {
            fn ground(&self, p: Vec3) -> Option<crate::GroundHit> {
                FlatHost::default().ground(p).filter(|_| p.z < 100.0)
            }
            fn roof(&self, _p: Vec3) -> Option<crate::RoofHit> {
                None
            }
            fn walls(&self, c: Vec3, _r: f32, _m: usize, _out: &mut Vec<crate::WallHit>) -> Vec3 {
                c
            }
            fn event(&mut self, _e: crate::Event) {}
        }
        let mut w = WaterManager::new(&Params::default());
        let mut drop = emit(Vec3::new(0.0, -0.2, 1.0));
        drop.burst = true;
        w.emit(&drop);
        w.step(&Ledge, Vec3::ZERO);
        assert!(w.wet_patches.iter().all(|p| p.pos.z < 100.0 + 1.0), "wet past the ledge");
        assert!(!w.wet_patches.is_empty());
    }

    #[test]
    fn wet_marks_follow_slopes_and_do_not_bridge_ledges() {
        struct Slope;
        impl Host for Slope {
            fn ground(&self, pos: Vec3) -> Option<crate::host::GroundHit> {
                (pos.z < 60.0).then_some(crate::host::GroundHit {
                    height: pos.x * 0.3,
                    normal: Vec3::new(-0.3, 1.0, 0.0).normalize(),
                    flags: SurfaceFlags::NONE,
                })
            }
        }
        let mut w = WaterManager::new(&Params::default());
        let n = Slope.ground(Vec3::ZERO).unwrap().normal;
        w.wet_ground(&Slope, Vec3::ZERO, n, 0, true);
        let p = &w.wet_patches[0];
        for q in p.surface.iter().flatten() {
            assert!((q.y - q.x * 0.3 - (n.y - n.x * 0.3)).abs() < 0.001);
        }
        assert!(p.surface.iter().any(Option::is_none));
        assert!(!p.contains(Vec3::new(0.0, 0.0, 100.0)));
    }

    #[test]
    fn repeated_spray_refreshes_wetness_and_marks_stay_bounded() {
        let mut w = WaterManager::new(&Params::default());
        for _ in 0..100 {
            w.wet_ground(&FlatHost::default(), Vec3::ZERO, Vec3::Y, 0, false);
        }
        assert_eq!(w.wet_patches.len(), 1);
        for i in 1..400 {
            w.wet_ground(
                &FlatHost::default(),
                Vec3::X * i as f32 * 400.0,
                Vec3::Y,
                0,
                false,
            );
        }
        assert_eq!(w.wet_patches.len(), SLOTS);
    }

    #[test]
    fn a_shot_arcs_down_and_lands() {
        let mut w = WaterManager::new(&Params::default());
        assert_eq!(w.emit(&emit(Vec3::Z)), 1);
        let host = FlatHost::default();
        let mut frames = 0;
        while w
            .particles
            .first()
            .is_some_and(|p| p.state == State::Flying)
            && frames < 200
        {
            w.step(&host, Vec3::ZERO);
            frames += 1;
        }
        // Horizontal speed is capped at the size, so it travels about 50 a frame.
        let landed = w
            .events
            .iter()
            .find_map(|e| match e {
                WaterEvent::SplashSound { pos, .. } => Some(*pos),
                _ => None,
            })
            .expect("splashed");
        assert!(landed.z > 300.0 && landed.y < 10.0, "{landed}");
    }

    /// A floor, and one enemy standing in the spray's path.
    struct EnemyAhead;

    impl Host for EnemyAhead {
        fn ground(&self, p: Vec3) -> Option<crate::host::GroundHit> {
            FlatHost::default().ground(p)
        }
        fn enemies(&self, _pos: Vec3, _radius: f32, out: &mut Vec<Enemy>) {
            out.push(Enemy {
                id: 7,
                pos: Vec3::new(0.0, 0.0, 200.0),
                radius: 60.0,
                height: 200.0,
            });
        }
    }

    #[test]
    fn a_shot_into_an_enemy_is_spent_on_it() {
        let mut w = WaterManager::new(&Params::default());
        let mut e = emit(Vec3::Z);
        e.count = 8;
        e.attack = 3;
        w.emit(&e);
        let mut hits = Vec::new();
        for _ in 0..30 {
            w.step(&EnemyAhead, Vec3::ZERO);
            hits.extend(w.events.drain(..).filter_map(|e| match e {
                WaterEvent::Hit {
                    id, pos, attack, ..
                } => Some((id, pos, attack)),
                _ => None,
            }));
        }
        assert_eq!(hits.len(), 8, "{hits:?}");
        assert!(
            hits.iter()
                .all(|&(id, pos, attack)| id == 7 && attack == 3 && pos.z < 200.0)
        );
        assert!(w.particles.is_empty());
    }

    #[test]
    fn faster_spray_traverses_the_same_arc_and_lifetime_in_less_time() {
        let params = Params::default();
        let mut normal = WaterManager::new(&params);
        let mut fast = WaterManager::new(&params);
        normal.reach = 2.0;
        fast.reach = 2.0;
        fast.spray_speed = 1.2;
        let mut drop = emit(Vec3::new(0.0, 0.2, 1.0).normalize());
        drop.pos.y = 1500.0;
        normal.emit(&drop);
        fast.emit(&drop);
        let host = FlatHost::default();
        // Every five accelerated frames reaches the same trajectory age as six normal frames.
        for _ in 0..4 {
            for _ in 0..6 { normal.step(&host, Vec3::ZERO); }
            for _ in 0..5 { fast.step(&host, Vec3::ZERO); }
            let (a, b) = (normal.particles[0], fast.particles[0]);
            assert!(a.pos.distance(b.pos) < 0.01, "arc changed: {:?} vs {:?}", a.pos, b.pos);
            assert!((a.life - b.life).abs() < 0.01);
            assert!(a.vel.distance(b.vel) < 0.001);
            assert!(b.velocity().distance(a.velocity() * 1.2) < 0.001);
        }
    }

    #[test]
    fn faster_spray_lands_sooner_at_the_same_effective_range() {
        let landing = |speed| {
            let mut water = WaterManager::new(&Params::default());
            water.reach = 2.0;
            water.spray_speed = speed;
            let mut drop = emit(Vec3::Z);
            drop.pos.y = 180.0;
            water.emit(&drop);
            for frame in 1..180 {
                water.step(&FlatHost::default(), Vec3::ZERO);
                if let Some(pos) = water.events.iter().find_map(|e| match e {
                    WaterEvent::SplashSound { pos, .. } => Some(*pos),
                    _ => None,
                }) { return (frame, pos.z); }
            }
            panic!("water never landed");
        };
        let (normal_time, normal_range) = landing(1.0);
        let (fast_time, fast_range) = landing(1.2);
        assert!(fast_time < normal_time);
        assert!((normal_time as f32 / fast_time as f32 - 1.2).abs() < 0.06);
        // Ground collision is sampled at 30 Hz: allow one accelerated frame of travel.
        assert!((normal_range - fast_range).abs() <= 96.0, "range {normal_range} -> {fast_range}");
    }

    #[test]
    fn spray_speed_keeps_damage_count_and_other_nozzles_unchanged() {
        for nozzle in [NozzleKind::Spray, NozzleKind::Hover, NozzleKind::Rocket, NozzleKind::Turbo, NozzleKind::Yoshi] {
            let mut normal = WaterManager::new(&Params::default());
            let mut fast = WaterManager::new(&Params::default());
            fast.spray_speed = 1.2;
            let mut drop = emit(Vec3::Z);
            drop.nozzle = nozzle;
            drop.count = 8;
            drop.attack = 3;
            assert_eq!(normal.emit(&drop), fast.emit(&drop));
            for _ in 0..30 {
                normal.step(&EnemyAhead, Vec3::ZERO);
                fast.step(&EnemyAhead, Vec3::ZERO);
            }
            let hits = |w: &WaterManager| w.events.iter().filter_map(|e| match e {
                WaterEvent::Hit { id, attack, .. } => Some((*id, *attack)),
                _ => None,
            }).collect::<Vec<_>>();
            assert_eq!(hits(&normal), vec![(7, 3); 8]);
            assert_eq!(hits(&normal), hits(&fast));
            if nozzle != NozzleKind::Spray {
                assert_eq!(normal.particles, fast.particles);
                assert_eq!(normal.events, fast.events);
            }
        }
    }

    #[test]
    fn water_speed_scales_body_runoff_and_other_nozzles_but_not_spray() {
        let params = Params::default();
        let mut fast = WaterManager::new(&params);
        fast.water_speed = 1.2;
        // Runoff (the drips off a wet Mario) takes the general water scale.
        let drips = fast.emit_body_drip(Vec3::Y * 200.0, Vec3::X * 3.0);
        assert!(drips > 0);
        assert_eq!(fast.particles.len(), drips as usize);
        assert!(fast.particles.iter().all(|p| p.travel_speed == 1.2));
        // Spray keeps its own stream scale, whatever the general one is.
        fast.emit(&emit(Vec3::Z));
        assert_eq!(fast.particles.last().unwrap().travel_speed, 1.0);
        // The other nozzles do not.
        let mut other = emit(Vec3::Z);
        other.nozzle = NozzleKind::Hover;
        fast.emit(&other);
        assert_eq!(fast.particles.last().unwrap().travel_speed, 1.2);
        // Yoshi's juice is not FLUDD water and keeps its rate.
        other.nozzle = NozzleKind::Yoshi;
        fast.emit(&other);
        assert_eq!(fast.particles.last().unwrap().travel_speed, 1.0);
    }

    #[test]
    fn slots_are_limited() {
        let mut w = WaterManager::new(&Params::default());
        let mut e = emit(Vec3::Z);
        e.count = 1000;
        assert_eq!(w.emit(&e) as usize, SLOTS);
    }
}
