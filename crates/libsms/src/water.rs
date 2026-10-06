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
use crate::host::{Host, SurfaceFlags};
use crate::params::{Params, WaterParticle};

/// `SLOT_NUM`.
pub const SLOTS: usize = 256;

/// Where a particle is (`mParticleFlagSOA & 0xF`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Flying = 1,
    /// Stuck to the ground, running downhill.
    Ground = 2,
    /// Stuck to a wall, running down it.
    Wall = 3,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Particle {
    pub pos: Vec3,
    /// Units per frame.
    pub vel: Vec3,
    pub size: f32,
    /// Frames left (`mParticleLifetimeSOA`).
    pub life: f32,
    pub state: State,
    pub kind: usize,
    pub attack: i16,
    /// The surface it sticks to: normal and whether it is wet (`isWaterSlip`).
    surface: Option<(Vec3, bool)>,
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
    pub particles: Vec<Particle>,
    pub types: [WaterParticle; 17],
    pub events: Vec<WaterEvent>,
    seed: u32,
}

impl WaterManager {
    pub fn new(params: &Params) -> WaterManager {
        WaterManager {
            particles: Vec::with_capacity(SLOTS),
            types: params.water_particles.clone(),
            events: Vec::new(),
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
        let kind = (w.particle_type.max(0) as usize).min(16);
        let mut n = 0;
        while n < w.count && self.particles.len() < SLOTS {
            let tremble = Vec3::new(self.rand11(), self.rand11(), self.rand11()) * w.dir_tremble;
            let mut dir = w.dir + tremble;
            if dir == Vec3::ZERO {
                dir.y = -1.0;
            }
            let speed = self.rand11() * w.pow_tremble + w.power;
            let vel = dir.normalize() * speed + w.inherit;
            let size = self.rand11() * w.size_tremble + w.size;
            self.particles.push(Particle {
                pos: w.pos,
                vel,
                size,
                life: self.types[kind].alive,
                state: State::Flying,
                kind,
                attack: w.attack,
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

    /// `TModelWaterManager::move`: one frame.
    pub fn step(&mut self, host: &dyn Host, mario: Vec3) {
        // Fly, slide, fall; cap horizontal speed at the particle's size.
        for i in 0..self.particles.len() {
            let gravity = self.kind(i).gravity;
            let p = &mut self.particles[i];
            let d = mario - p.pos;
            if d.x * d.x + d.z * d.z > k::RANGE * k::RANGE {
                p.life = 0.0;
                continue;
            }
            let h2 = p.vel.x * p.vel.x + p.vel.z * p.vel.z;
            if h2 > p.size * p.size {
                let s = (p.size - 0.1) / h2.sqrt();
                p.vel.x *= s;
                p.vel.z *= s;
            }
            match p.state {
                State::Flying => {
                    p.vel.y += gravity;
                    p.pos += p.vel;
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
                    self.particles[i].life -= 1.0;
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
        self.particles.retain(|p| p.life > 0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        }
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

    #[test]
    fn slots_are_limited() {
        let mut w = WaterManager::new(&Params::default());
        let mut e = emit(Vec3::Z);
        e.count = 1000;
        assert_eq!(w.emit(&e) as usize, SLOTS);
    }
}
