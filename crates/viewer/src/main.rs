//! A running sample: libsms driving Mario through a small 3D obstacle course.
//!
//! The course is the host: `World` implements `libsms::Host` with boxes and two ramps and turns
//! FLUDD `WaterEmit` events into droplets. Everything about how Mario moves comes from the library.
//!
//! Controls
//!   WASD            move (Left Alt = gentle push)        Arrows   orbit camera
//!   Space           jump (A)    hold for full height     F        B: dive, grab, put, throw
//!   Z               ground pound in the air (L)          Left Ctrl  crouch (R click)
//!   Right mouse     aim along the camera (cyan line)
//!   Left Shift      spray (R, half press)                X        swap nozzle (Spray <-> second)
//!   1 2 3 4         fit Spray / Hover / Rocket / Turbo   G        refill tank
//!   R               respawn                              H        toggle help
//!   T               Shine Shirt on/off                  M        cycle music (includes off)
//!
//! Tuning, animation clips and the skeleton come from your own disc: run
//! `sms-viewer --iso <your disc image>` once (or `cargo run -p libsms --bin sms-extract -- <iso>`,
//! or `python tools/extract_assets.py <iso>`) to build the git-ignored `assets/` folder; a path
//! argument or `SMS_ASSETS` overrides where it is looked for. Without it the decomp's
//! code defaults and a plain capsule are used, and the HUD says so. `--demo` replays a script.

mod coin;
mod fludd;
mod fx;
mod helmet;
mod j3d;
mod luigi;
mod mario_model;
mod melee_peach;
mod peach;
mod pianta;
mod props;
mod stu;
mod water;
mod yoshi_model;
mod yoshi_skin;

use std::path::Path;

use libsms::math::to_angle;
use libsms::water::WaterManager;
use libsms::{
    AnimTable, Event, GroundHit, Host, NozzleKind, Pad, Params, RoofHit, Sim, Status, SurfaceFlags,
    TICK_HZ, Vec2 as SV2, Vec3 as V3, WallHit, WaterEmit,
};
use macroquad::prelude::*;

/// Decomp units (about cm) to scene metres.
const S: f32 = 0.01;
// The helmet's per-skin fit on the head joint (helmet.rs); Mario's is the identity. Tuned by eye
// against the contact sheet (`SMS_HELMET_ADJ`): up is -y and forward is -z in the head frame.
use ::glam::Vec3 as V3g;
const LUIGI_FIT: helmet::Fit = helmet::Fit { offset: V3g::new(8.0, -22.0, 0.0), scale: 0.95, tilt: 0.0 };
const PIANTA_FIT: helmet::Fit = helmet::Fit { offset: V3g::new(0.0, -12.0, -12.0), scale: 1.15, tilt: 0.0 };
// Yoshi's neck leans forward, so the bowl is turned back upright about the side axis.
const YOSHI_FIT: helmet::Fit = helmet::Fit { offset: V3g::new(0.0, -12.0, -4.0), scale: 1.5, tilt: -45.0 };

fn mq(v: V3) -> Vec3 {
    vec3(v.x * S, v.y * S, v.z * S)
}

struct Block {
    min: V3,
    max: V3,
    color: Color,
}

struct Ramp {
    x0: f32,
    x1: f32,
    z0: f32,
    z1: f32,
    /// Rise per unit of Z.
    slope: f32,
    color: Color,
}

struct World {
    half: f32,
    blocks: Vec<Block>,
    ramps: Vec<Ramp>,
    emits: Vec<WaterEmit>,
    body_drips: Vec<(V3, V3)>,
    /// Fruit to pick up and throw (indices are their take ids).
    fruits: Vec<props::Fruit>,
    /// Strollin' Stus for Yoshi to eat (indices are their enemy ids).
    stus: Vec<stu::Stu>,
    /// Coins to heal with.
    coins: Vec<coin::Coin>,
    sounds: Vec<Event>,
    log: Vec<String>,
}

impl World {
    fn course() -> World {
        let b = |min: [f32; 3], max: [f32; 3], c: Color| Block {
            min: V3::from(min),
            max: V3::from(max),
            color: c,
        };
        let step = Color::new(0.75, 0.62, 0.35, 1.0);
        let wall = Color::new(0.55, 0.6, 0.7, 1.0);
        let stone = Color::new(0.5, 0.5, 0.55, 1.0);
        let blocks = vec![
            // Staircase of ledges along +X: 100, 200, 300, 450 high.
            b([-1500.0, 0.0, 400.0], [-1200.0, 100.0, 700.0], step),
            b([-1200.0, 0.0, 400.0], [-900.0, 200.0, 700.0], step),
            b([-900.0, 0.0, 400.0], [-600.0, 300.0, 700.0], step),
            b([-600.0, 0.0, 400.0], [-300.0, 450.0, 700.0], step),
            // A wall-kick shaft: two tall walls 200 apart.
            b([600.0, 0.0, -200.0], [700.0, 1500.0, 200.0], wall),
            b([900.0, 0.0, -200.0], [1000.0, 1500.0, 200.0], wall),
            // A high platform for hover and rocket.
            b([-300.0, 0.0, 1500.0], [300.0, 900.0, 2100.0], stone),
            // A low bridge you can run under and jump onto.
            b([-400.0, 250.0, -1100.0], [400.0, 330.0, -800.0], stone),
            // A pillar to bump into.
            b([-100.0, 0.0, 1000.0], [100.0, 400.0, 1200.0], wall),
        ];
        let ramps = vec![
            // Gentle 25 degree ramp.
            Ramp {
                x0: -300.0,
                x1: 300.0,
                z0: -1800.0,
                z1: -1300.0,
                slope: 0.466,
                color: Color::new(0.4, 0.7, 0.4, 1.0),
            },
            // Steep 70 degree ramp: slippery.
            Ramp {
                x0: 1200.0,
                x1: 1700.0,
                z0: -300.0,
                z1: 100.0,
                slope: 2.747,
                color: Color::new(0.8, 0.35, 0.3, 1.0),
            },
        ];
        World {
            half: 3000.0,
            blocks,
            ramps,
            emits: Vec::new(),
            body_drips: Vec::new(),
            fruits: vec![
                props::Fruit::new(props::Kind::Papaya, V3::new(150.0, 0.0, 200.0)),
                props::Fruit::new(props::Kind::Coconut, V3::new(-200.0, 0.0, 150.0)),
                props::Fruit::new(props::Kind::Pine, V3::new(300.0, 0.0, -150.0)),
                props::Fruit::new(props::Kind::Banana, V3::new(0.0, 0.0, 350.0)),
                props::Fruit::new(props::Kind::Durian, V3::new(-300.0, 0.0, -250.0)),
            ],
            stus: vec![stu::Stu::new(V3::new(450.0, 0.0, 450.0), 150.0)],
            coins: {
                // A row of gold coins, a red one on the top step, a blue one on the platform.
                let mut c: Vec<coin::Coin> = (0..6)
                    .map(|i| {
                        coin::Coin::new(
                            libsms::Coin::Gold,
                            V3::new(-600.0 - 160.0 * i as f32, 40.0, -450.0),
                        )
                    })
                    .collect();
                c.push(coin::Coin::new(
                    libsms::Coin::Red,
                    V3::new(-450.0, 490.0, 550.0),
                ));
                c.push(coin::Coin::new(
                    libsms::Coin::Blue,
                    V3::new(0.0, 940.0, 1800.0),
                ));
                c
            },
            sounds: Vec::new(),
            log: Vec::new(),
        }
    }

    /// One 120 Hz tick of the loose fruit.
    fn tick_stus(&mut self) {
        for s in &mut self.stus {
            s.tick();
        }
        for c in &mut self.coins {
            c.tick();
        }
    }

    /// The hit check between Mario and the world's things, after the frame's ticks: coins he
    /// touches are his (`TCoin::taken`), a Stu that walks into him hurts him
    /// (`HIT_MESSAGE_ATTACK` with `DmgHamukuri.prm`), unless he lands on top of it.
    fn touch(&mut self, m: &mut libsms::Mario) {
        let (r, h) = if m.on_yoshi() {
            (80.0, 200.0)
        } else {
            (50.0, 160.0)
        };
        for c in &mut self.coins {
            if c.touches(m.pos, r, h) {
                c.take();
                m.get_coin(c.kind);
            }
        }
        for s in self.stus.iter().filter(|s| s.loose()) {
            let d = V3::new(s.pos.x - m.pos.x, 0.0, s.pos.z - m.pos.z).length();
            let above = m.pos.y > s.pos.y + stu::HEIGHT * 0.5;
            if d < stu::RADIUS + r && m.pos.y < s.pos.y + stu::HEIGHT && !above {
                m.damage(s.pos, libsms::DamageKind::Hamukuri);
            }
        }
    }

    fn tick_fruits(&mut self) {
        for i in 0..self.fruits.len() {
            if self.fruits[i].held {
                continue;
            }
            let p = self.fruits[i].pos + V3::Y * 30.0;
            let floor = self.ground(p).map_or(-1.0e9, |g| g.height);
            self.fruits[i].tick(|_| floor);
        }
    }

    fn note(&mut self, s: String) {
        self.log.push(s);
        if self.log.len() > 6 {
            self.log.remove(0);
        }
    }
}

impl Host for World {
    fn ground(&self, p: V3) -> Option<GroundHit> {
        if std::env::var_os("SMS_PORTRAIT").is_some() {
            return Some(GroundHit { height: 0.0, normal: V3::Y,
                flags: if std::env::var("SMS_PORTRAIT").as_deref() == Ok("slide") { SurfaceFlags(SurfaceFlags::WET) } else { SurfaceFlags::NONE } });
        }
        let mut best: Option<GroundHit> = None;
        let mut consider = |h: f32, n: V3| {
            if h <= p.y && best.is_none_or(|b| h > b.height) {
                best = Some(GroundHit {
                    height: h,
                    normal: n,
                    flags: SurfaceFlags::NONE,
                });
            }
        };
        if p.x.abs() < self.half && p.z.abs() < self.half {
            consider(0.0, V3::Y);
        }
        for b in &self.blocks {
            if p.x >= b.min.x && p.x <= b.max.x && p.z >= b.min.z && p.z <= b.max.z {
                consider(b.max.y, V3::Y);
            }
        }
        for r in &self.ramps {
            if p.x >= r.x0 && p.x <= r.x1 && p.z >= r.z0 && p.z <= r.z1 {
                let n = V3::new(0.0, 1.0, -r.slope).normalize();
                consider((p.z - r.z0) * r.slope, n);
            }
        }
        best
    }

    fn roof(&self, p: V3) -> Option<RoofHit> {
        if std::env::var_os("SMS_PORTRAIT").is_some() { return None; }
        let mut best: Option<f32> = None;
        for b in &self.blocks {
            if b.min.y > p.y
                && p.x >= b.min.x
                && p.x <= b.max.x
                && p.z >= b.min.z
                && p.z <= b.max.z
                && best.is_none_or(|h| b.min.y < h)
            {
                best = Some(b.min.y);
            }
        }
        best.map(|height| RoofHit {
            height,
            flags: SurfaceFlags::NONE,
        })
    }

    fn walls(&self, center: V3, radius: f32, max: usize, out: &mut Vec<WallHit>) -> V3 {
        if std::env::var_os("SMS_PORTRAIT").is_some() { return center; }
        let mut c = center;
        let mut hits: Vec<(f32, WallHit)> = Vec::new();
        for b in &self.blocks {
            if c.y < b.min.y || c.y >= b.max.y {
                continue;
            }
            let nx = c.x.clamp(b.min.x, b.max.x);
            let nz = c.z.clamp(b.min.z, b.max.z);
            let (dx, dz) = (c.x - nx, c.z - nz);
            let dist = (dx * dx + dz * dz).sqrt();
            if dist >= radius {
                continue;
            }
            let (normal, push) = if dist > 1e-4 {
                (V3::new(dx / dist, 0.0, dz / dist), radius - dist)
            } else {
                // Centre inside the box: leave through the nearest face.
                let faces = [
                    (c.x - b.min.x, V3::new(-1.0, 0.0, 0.0)),
                    (b.max.x - c.x, V3::new(1.0, 0.0, 0.0)),
                    (c.z - b.min.z, V3::new(0.0, 0.0, -1.0)),
                    (b.max.z - c.z, V3::new(0.0, 0.0, 1.0)),
                ];
                let (d, n) = faces
                    .iter()
                    .copied()
                    .fold(faces[0], |a, f| if f.0 < a.0 { f } else { a });
                // Like the game's wall triangles: only a face within the radius touches.
                if d >= radius {
                    continue;
                }
                (n, d + radius)
            };
            c.x += normal.x * push;
            c.z += normal.z * push;
            hits.push((
                dist,
                WallHit {
                    normal,
                    flags: SurfaceFlags::NONE,
                },
            ));
        }
        hits.sort_by(|a, b| a.0.total_cmp(&b.0));
        out.extend(hits.into_iter().take(max).map(|h| h.1));
        c
    }

    fn takeables(&self, pos: V3, radius: f32, out: &mut Vec<libsms::host::Takeable>) {
        for (i, f) in self.fruits.iter().enumerate() {
            let t = f.takeable(i);
            if f.loose() && (t.pos - pos).length() < radius + t.radius {
                out.push(t);
            }
        }
    }

    fn enemies(&self, pos: V3, radius: f32, out: &mut Vec<libsms::Enemy>) {
        for (i, s) in self.stus.iter().enumerate() {
            let d = s.pos - pos;
            if s.loose() && V3::new(d.x, 0.0, d.z).length() < radius + stu::RADIUS {
                out.push(s.enemy(i));
            }
        }
    }

    fn take_enemy(&mut self, id: u64) -> bool {
        match self.stus.get_mut(id as usize) {
            Some(s) if s.loose() => {
                s.held = true;
                true
            }
            _ => false,
        }
    }

    fn take(&mut self, id: u64) -> bool {
        match self.fruits.get_mut(id as usize) {
            Some(f) if f.loose() => {
                f.held = true;
                f.vel = V3::ZERO;
                // `TMario::soundProcess`: fruit kinds take with MA_TAKE_DRIAN.
                let pos = f.pos;
                self.sounds.push(Event::Sound(libsms::SoundCue {
                    name: "MSD_SE_MA_TAKE_DRIAN",
                    pos,
                    info: None,
                }));
                true
            }
            _ => false,
        }
    }

    fn event(&mut self, e: Event) {
        self.sounds.push(e);
        match e {
            Event::Jump(k) => self.note(format!("jump: {k:?}")),
            Event::Land { strong: true } => self.note("hard landing".into()),
            Event::GroundPound { super_, drop } => {
                self.note(format!("ground pound (super: {super_}, fell {drop:.0})"))
            }
            Event::WallBump => self.note("wall bump".into()),
            Event::TriggerFired(n) => self.note(format!("{n:?} fired")),
            Event::NozzleChanged(n) => self.note(format!("nozzle: {n:?}")),
            Event::Fell => {}
            Event::WaterEmit(w) => self.emits.push(w),
            Event::BodyDrip { pos, velocity } => self.body_drips.push((pos, velocity)),
            Event::Put { id, pos, yaw } => {
                if let Some(f) = self.fruits.get_mut(id as usize) {
                    f.put(pos, yaw);
                    self.sounds.push(Event::Sound(libsms::SoundCue {
                        name: "MSD_SE_MA_PUT_DRIAN",
                        pos,
                        info: None,
                    }));
                }
            }
            Event::Damaged {
                damage,
                health,
                yoshi,
            } => self.note(if yoshi {
                "Yoshi took the hit".into()
            } else {
                format!("hit: -{damage}, health {health}")
            }),
            Event::Down => self.note("Mario is down: R to start again".into()),
            Event::EnemyHit { id, kind } => {
                self.note(format!("enemy {id}: {kind:?}"));
                // The stand-in Stu goes down to a stomp or a pound (and comes back later).
                if kind != libsms::EnemyHitKind::Punch
                    && let Some(s) = self.stus.get_mut(id as usize)
                {
                    s.eaten();
                }
            }
            Event::EnemyEaten { id } => {
                if let Some(s) = self.stus.get_mut(id as usize) {
                    s.eaten();
                }
            }
            Event::Eaten { id } => {
                if let Some(f) = self.fruits.get_mut(id as usize) {
                    f.eaten();
                }
            }
            Event::Throw {
                id,
                pos,
                yaw,
                power,
                vel,
            } => {
                if let Some(f) = self.fruits.get_mut(id as usize) {
                    f.throw(pos, yaw, power, vel);
                }
            }
            Event::Land { .. } | Event::Sound(_) | Event::PoundStart | Event::FallImpact { .. } => {
            }
            // The portable host view consumes particle cues; this viewer has its own renderer.
            Event::Particle(_) | Event::AirNozzle { .. } | Event::AirJump | Event::WetSlide => {}
            Event::ShineTouch { .. } | Event::ShineDemo { .. } => {}
        }
    }
}

fn nozzle_color(n: NozzleKind) -> Color {
    match n {
        NozzleKind::Spray => Color::new(0.3, 0.55, 0.95, 1.0),
        NozzleKind::Hover => Color::new(0.3, 0.8, 0.5, 1.0),
        NozzleKind::Rocket => Color::new(0.9, 0.3, 0.3, 1.0),
        NozzleKind::Turbo => Color::new(0.95, 0.8, 0.2, 1.0),
        _ => GRAY,
    }
}

fn draw_world(w: &World) {
    draw_grid(
        60,
        1.0,
        Color::new(0.3, 0.3, 0.35, 1.0),
        Color::new(0.2, 0.22, 0.25, 1.0),
    );
    draw_cube(
        vec3(0.0, -0.05, 0.0),
        vec3(w.half * 2.0 * S, 0.1, w.half * 2.0 * S),
        None,
        Color::new(0.16, 0.18, 0.2, 1.0),
    );
    for b in &w.blocks {
        let size = mq(b.max - b.min);
        let center = mq((b.min + b.max) * 0.5);
        draw_cube(center, size, None, b.color);
        draw_cube_wires(center, size, Color::new(0.0, 0.0, 0.0, 0.5));
    }
    for r in &w.ramps {
        let (h1, c) = ((r.z1 - r.z0) * r.slope, r.color);
        let v = |x: f32, y: f32, z: f32| Vertex::new(x * S, y * S, z * S, 0.0, 0.0, c);
        let mesh = Mesh {
            vertices: vec![
                v(r.x0, 0.0, r.z0),
                v(r.x1, 0.0, r.z0),
                v(r.x1, h1, r.z1),
                v(r.x0, h1, r.z1),
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
            texture: None,
        };
        draw_mesh(&mesh);
        let (a, b2) = (
            vec3(r.x0 * S, 0.0, r.z0 * S),
            vec3(r.x0 * S, h1 * S, r.z1 * S),
        );
        draw_line_3d(a, b2, BLACK);
        draw_line_3d(
            vec3(r.x1 * S, 0.0, r.z0 * S),
            vec3(r.x1 * S, h1 * S, r.z1 * S),
            BLACK,
        );
        draw_line_3d(b2, vec3(r.x1 * S, h1 * S, r.z1 * S), BLACK);
    }
}

/// Water without the disc's textures: plain spheres.
fn draw_water_plain(water: &WaterManager) {
    for p in &water.particles {
        draw_sphere(
            mq(p.pos),
            (p.size * S * 0.5).clamp(0.02, 0.3),
            None,
            Color::new(0.45, 0.7, 1.0, 0.8),
        );
    }
}

/// The round shadow under Mario.
fn draw_shadow(sim: &Sim, alpha: f32) {
    let m = &sim.mario;
    let pos = mq(m.prev_pos.lerp(m.pos, alpha));
    draw_cylinder(
        vec3(pos.x, m.floor_y() * S + 0.01, pos.z),
        0.3,
        0.3,
        0.005,
        None,
        Color::new(0.0, 0.0, 0.0, 0.45),
    );
}

/// The game's rig and clips, if the user's extracted data has them.
struct Rig {
    skeleton: libsms::Skeleton,
    clips: libsms::ClipLibrary,
}

impl Rig {
    fn from_assets(a: &libsms::Assets) -> Option<Rig> {
        let skeleton = a.skeleton.clone()?;
        (!a.clips.is_empty()).then(|| Rig {
            skeleton,
            clips: a.clips.clone(),
        })
    }
}

/// Draws a bone as a string of small spheres, so it reads as a limb from any angle.
fn draw_bone(a: Vec3, b: Vec3, radius: f32, color: Color) {
    let len = (b - a).length();
    let n = ((len / (radius * 1.2)).ceil() as i32).max(1);
    for i in 0..=n {
        draw_sphere(a.lerp(b, i as f32 / n as f32), radius, None, color);
    }
}

/// A neutral mannequin driven by the game's real animation: the skeleton from the disc, posed
/// each frame from the clip the sim is playing. Without the disc rig it falls back to a plain
/// capsule so the sample still runs.
fn draw_mario(sim: &Sim, rig: Option<&Rig>, alpha: f32) {
    let m = &sim.mario;
    let pos = mq(m.prev_pos.lerp(m.pos, alpha));
    let floor = vec3(pos.x, m.floor_y() * S, pos.z);
    let ang = m.model_yaw as f32 * std::f32::consts::PI / 32768.0;
    let fwd = vec3(ang.sin(), 0.0, ang.cos());

    // Shadow on the floor under it.
    draw_cylinder(
        floor + vec3(0.0, 0.01, 0.0),
        0.3,
        0.3,
        0.005,
        None,
        Color::new(0.0, 0.0, 0.0, 0.45),
    );

    let bone = Color::new(0.82, 0.84, 0.88, 1.0);
    let joint = Color::new(0.95, 0.55, 0.2, 1.0);
    let marker = Color::new(0.2, 0.85, 0.5, 1.0);

    // Model space (+Z forward, +Y up, centimetres) to the scene, turned by the model yaw.
    let (sy, cy) = ang.sin_cos();
    let to_world = |p: libsms::Vec3| -> Vec3 {
        vec3(p.x * cy + p.z * sy, p.y, -p.x * sy + p.z * cy) * S + pos
    };

    let posed = rig.map(|r| {
        let world = match r.clips.get(m.anim.clip) {
            Some(clip) => r.skeleton.pose(clip, m.anim.frame),
            None => r.skeleton.bind_world(),
        };
        (r, world)
    });

    if let Some((r, world)) = posed {
        let pts: Vec<Vec3> = world
            .iter()
            .map(|w| to_world(w.w_axis.truncate()))
            .collect();
        for (j, parent) in r.skeleton.parents.iter().enumerate() {
            let Some(p) = *parent else { continue };
            // `M_*` joints are mesh attachment points, not limbs.
            if r.skeleton.names[j].starts_with("M_") || r.skeleton.names[j].starts_with("eff_") {
                continue;
            }
            if p == 0 {
                continue;
            }
            draw_bone(pts[p], pts[j], 0.035, bone);
        }
        for (j, p) in pts.iter().enumerate() {
            if r.skeleton.names[j].starts_with("M_") || j < 2 {
                continue;
            }
            draw_sphere(*p, 0.05, None, joint);
        }
        if let Some(h) = r.skeleton.find("jnt_head") {
            // The head joint sits at the neck; the head itself rises from it.
            let up = vec3(0.0, 0.2, 0.0);
            draw_sphere(pts[h] + up, 0.17, None, bone);
            draw_sphere(pts[h] + up + fwd * 0.14, 0.05, None, marker);
        }
        if let (Some(f), Some(c)) = (&m.fludd, r.skeleton.find("jnt_chest")) {
            let back = pts[c] - fwd * 0.2 - vec3(0.0, 0.1, 0.0);
            draw_cylinder(back, 0.13, 0.13, 0.4, None, Color::new(0.8, 0.8, 0.85, 1.0));
            draw_cylinder(
                back + vec3(0.0, 0.4, 0.0),
                0.0,
                0.13,
                0.1,
                None,
                nozzle_color(f.nozzle),
            );
        }
    } else {
        // Fallback: a capsule-ish mannequin.
        let squat = if matches!(m.status, Status::Squat) {
            0.65
        } else {
            1.0
        };
        draw_cylinder(pos, 0.2, 0.22, 0.9 * squat, None, bone);
        let head = pos + vec3(0.0, 1.1 * squat, 0.0);
        draw_sphere(head, 0.2, None, bone);
        draw_sphere(head + fwd * 0.17, 0.05, None, marker);
        if let Some(f) = &m.fludd {
            let back = pos - fwd * 0.28 + vec3(0.0, 0.45 * squat, 0.0);
            draw_cylinder(
                back,
                0.15,
                0.15,
                0.5 * squat,
                None,
                Color::new(0.8, 0.8, 0.85, 1.0),
            );
            draw_cylinder(
                back + vec3(0.0, 0.5 * squat, 0.0),
                0.0,
                0.15,
                0.1,
                None,
                nozzle_color(f.nozzle),
            );
        }
    }
}

fn hud(sim: &Sim, data: &str, rig: Option<&Rig>, show_help: bool) {
    let m = &sim.mario;
    let mut rows: Vec<String> = vec![
        format!(
            "{:?}   speed {:.1}   pos ({:.0}, {:.0}, {:.0})",
            m.status,
            m.speed(),
            m.pos.x,
            m.pos.y,
            m.pos.z
        ),
        format!(
            "clip {} frame {:.0}/{:.0}   {} Hz sim",
            m.anim.clip,
            m.anim.frame,
            m.anim.length(),
            TICK_HZ
        ),
    ];
    if let Some(f) = &m.fludd {
        rows.push(format!(
            "FLUDD {:?}  tank {:.0}%  trigger {:?}  upper {:?}",
            f.nozzle,
            f.level(&m.params) * 100.0,
            f.trigger_state(),
            m.upper
        ));
    }
    if m.yoshi.is_hatched() {
        rows.push(format!(
            "Yoshi {:?}  juice {:.0} s  colour {}  clip {}",
            m.yoshi.state,
            m.yoshi.juice as f32 / TICK_HZ as f32,
            m.yoshi.color,
            libsms::yoshi::CLIPS[m.yoshi.clip as usize],
        ));
    } else if let Some(e) = m.eggs.first() {
        rows.push(format!("egg {:?}, wants {:?}", e.state, e.wants));
    }
    rows.push(format!("data: {data}"));
    rows.push(match rig {
        Some(r) => format!(
            "rig: {} joints, {} clips from the disc",
            r.skeleton.len(),
            r.clips.len()
        ),
        None => "rig: none (needs assets/), showing a plain capsule".to_owned(),
    });
    for (i, r) in rows.iter().enumerate() {
        draw_text(r, 14.0, 24.0 + 22.0 * i as f32, 22.0, WHITE);
    }
    health_meter(m);
    if let Some(f) = &m.fludd {
        let y = 24.0 + 22.0 * rows.len() as f32;
        let lvl = f.level(&m.params).clamp(0.0, 1.0);
        draw_rectangle(14.0, y, 220.0, 12.0, Color::new(0.0, 0.0, 0.0, 0.5));
        draw_rectangle(14.0, y, 220.0 * lvl, 12.0, nozzle_color(f.nozzle));
    }
    if show_help {
        let h = screen_height();
        draw_text(
            "WASD move  Space jump  F dive/grab  Z pound  Ctrl crouch  Shift spray  X swap  1-4 nozzle  RMB aim",
            14.0,
            h - 34.0,
            20.0,
            LIGHTGRAY,
        );
        draw_text(
            "Arrows camera  G refill  R respawn  T shirt  K get hurt  H hide help   On Yoshi: F tongue  X off",
            14.0,
            h - 12.0,
            20.0,
            LIGHTGRAY,
        );
    }
}

/// The life meter: `mHPMax` wedges, lit up to `mHealth`, and the coin count. Flashes while
/// Mario is invincible.
fn health_meter(m: &libsms::Mario) {
    let (cx, cy, r) = (screen_width() - 70.0, 70.0, 46.0);
    let max = m.params.mario.hp_max.max(1) as usize;
    let lit = if m.health >= 6 {
        Color::new(0.25, 0.55, 1.0, 1.0)
    } else if m.health >= 3 {
        Color::new(1.0, 0.8, 0.1, 1.0)
    } else {
        Color::new(1.0, 0.25, 0.25, 1.0)
    };
    let flash = m.invincible > 0 && (m.ticks / 8).is_multiple_of(2);
    draw_circle(cx, cy, r + 4.0, Color::new(0.0, 0.0, 0.0, 0.5));
    for i in 0..max {
        let a0 = std::f32::consts::TAU * i as f32 / max as f32 - std::f32::consts::FRAC_PI_2;
        let a1 = a0 + std::f32::consts::TAU / max as f32 * 0.9;
        let c = if (i as i16) < m.health && !flash {
            lit
        } else {
            Color::new(0.25, 0.25, 0.3, 1.0)
        };
        let p = |a: f32| Vec2::new(cx + r * a.cos(), cy + r * a.sin());
        draw_triangle(Vec2::new(cx, cy), p(a0), p(a1), c);
    }
    draw_circle(cx, cy, r * 0.35, Color::new(0.1, 0.1, 0.15, 1.0));
    draw_text(format!("{}", m.health), cx - 7.0, cy + 8.0, 26.0, WHITE);
    draw_text(
        format!("coins {}", m.coins),
        cx - 40.0,
        cy + r + 26.0,
        22.0,
        Color::new(1.0, 0.85, 0.2, 1.0),
    );
}

/// A looping script that shows off the clips: run, jump, dive, run, jump + pound, spray, crouch.
fn demo_pad(t: f32) -> Pad {
    let t = t % 16.0;
    let run = SV2::new(0.0, 1.0);
    let mut p = Pad::default();
    match t {
        t if t < 2.5 => p.stick = run,
        t if t < 3.0 => {
            p.stick = run;
            p.a = true;
        }
        t if t < 3.4 => {
            p.stick = run;
            p.a = true;
            p.b = t > 3.3;
        }
        t if t < 5.0 => p.stick = SV2::ZERO,
        t if t < 6.5 => p.stick = SV2::new(1.0, 0.0),
        t if t < 7.0 => {
            p.stick = SV2::new(1.0, 0.0);
            p.a = true;
        }
        t if t < 8.0 => {
            p.a = t < 7.6;
            p.l = t > 7.45 && t < 7.5;
        }
        t if t < 11.0 => {
            p.stick = SV2::new(0.0, 0.3);
            p.r_analog = 150;
        }
        t if t < 13.0 => {
            p.r = true;
            p.r_analog = 255;
        }
        _ => p.stick = SV2::new(-1.0, 0.0),
    }
    p
}

fn window_conf() -> Conf {
    // `SMS_SQUARE=1`: a square window, for skill-portrait reference captures.
    let square = std::env::var_os("SMS_SQUARE").is_some();
    Conf {
        window_title: if std::env::args().any(|a| a == "--yoshi-mario") {
            "Sunshine Yoshi — Mario movement demo".to_owned()
        } else if std::env::args().any(|a| a == "--pianta") {
            "Pianta — Mario movement demo".to_owned()
        } else if std::env::args().any(|a| a.starts_with("--peach")) {
            "Sunshine Peach — Mario movement demo".to_owned()
        } else { "libsms viewer".to_owned() },
        window_width: if square { 720 } else { 1280 },
        window_height: 720,
        ..Default::default()
    }
}

fn spawn() -> Sim {
    Sim::new(
        Params::default(),
        AnimTable::default(),
        V3::new(0.0, 0.0, 0.0),
        0,
    )
}

#[macroquad::main(window_conf)]
async fn main() {
    let demo = std::env::args().any(|a| a == "--demo");
    // The user's own disc files, extracted by `tools/extract_assets.py` into `assets/` (git-ignored).
    // `--iso <disc image>` builds `assets/` from your disc first if it is not there yet.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let iso = args
        .iter()
        .position(|a| a == "--iso")
        .and_then(|i| args.get(i + 1));
    let explicit = args
        .iter()
        .enumerate()
        .find(|(i, a)| !a.starts_with("--") && (*i == 0 || args[i - 1] != "--iso"))
        .map(|(_, a)| a.clone());
    // First run: a complete `assets/`, else extract from `--iso`, `SMS_DISC` or a Sunshine
    // image found next to the workspace.
    let ws = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut disc_dirs: Vec<std::path::PathBuf> = vec![
        ws.clone(),
        ws.join("../iso backups"),
        ws.join("../sms-main/orig/GMSP01"),
        ws.join("../sms-main/orig/GMSE01"),
        std::path::PathBuf::from("."),
    ];
    if let Some(iso) = iso {
        // SAFETY: single-threaded, before anything reads the environment.
        unsafe { std::env::set_var("SMS_DISC", iso) };
        disc_dirs.clear();
    }
    let assets = libsms::Assets::discover_or_extract(
        explicit.as_deref().map(Path::new),
        &disc_dirs,
        &mut |m| println!("{m}"),
    );
    let rig = assets.as_ref().and_then(Rig::from_assets);
    let mut audio = assets
        .as_ref()
        .and_then(|a| libsms_present::Audio::start(&a.dir, true));
    // `--luigi`: draw Luigi with his own FLUDD instead of Mario.
    let use_luigi = std::env::args().any(|s| s == "--luigi" || s == "--luigi-beside");
    let compare_peach = args.iter().any(|s| s == "--peach-compare");
    let use_peach = compare_peach || args.iter().any(|s| s == "--peach");
    let use_melee_peach = compare_peach || args.iter().any(|s| s == "--peach-melee");
    let mut show_melee_peach = args.iter().any(|s| s == "--peach-melee");
    // `--pianta`: the male Pianta, the female Pianta and Piantissimo wearing Mario's motion
    // (Tab cycles; `SMS_PIANTA=0|1|2` picks the first).
    let use_pianta = args.iter().any(|s| s == "--pianta");
    let use_yoshi_skin = args.iter().any(|s|s=="--yoshi-mario");
    let mut pianta_index: usize = std::env::var("SMS_PIANTA").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let luigi_beside = std::env::args().any(|s| s == "--luigi-beside");
    // Luigi's voice: Mario's, pitched up with the lines' length kept (the Melee trick).
    // `SMS_VOICE_PITCH=<ratio>` overrides it; 1.0 leaves Mario's voice alone.
    if use_luigi {
        let ratio = std::env::var("SMS_VOICE_PITCH")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(1.14);
        if let Some(a) = &audio {
            a.set_voice_pitch(ratio);
        }
    }
    // The disc's Mario: body, hands, cap, FLUDD and nozzles with their materials.
    let mut mario_model = assets.as_ref().and_then(|a| {
        mario_model::MarioModel::load(&a.dir)
            .map_err(|e| eprintln!("Mario's model: {e}"))
            .ok()
    });
    // `--helmet` / `SMS_HELMET=1`: the diving helmet on whichever skin is drawn (helmet.rs).
    let mut helmet = (args.iter().any(|s| s == "--helmet") || std::env::var_os("SMS_HELMET").is_some())
        .then(|| assets.as_ref().and_then(|a| helmet::Helmet::load(&a.dir)
            .map_err(|e| eprintln!("Diving helmet: {e}")).ok()))
        .flatten();
    // Luigi owns a FLUDD of his own; it follows his chest, not Mario's.
    let mut luigi_fludd = if use_luigi || use_peach || use_melee_peach || use_pianta || use_yoshi_skin {
        assets.as_ref().and_then(|a| {
            fludd::Fludd::load(&a.dir)
                .map_err(|e| eprintln!("Luigi's FLUDD: {e}"))
                .ok()
        })
    } else {
        None
    };
    // Yoshi, his tongue and his egg.
    let mut luigi_model = if use_luigi {
        assets.as_ref().and_then(|a| {
            luigi::Luigi::load(&luigi::melee_dir(), &a.dir, 0)
                .map_err(|e| eprintln!("Luigi.s model: {e}"))
                .ok()
        })
    } else {
        None
    };
    let mut peach_model = if use_peach {
        let a = assets.as_ref().expect("Peach demo requires the Sunshine assets");
        peach::ensure_assets(&a.dir, &disc_dirs).expect("extract Peach from your Sunshine disc");
        Some(peach::Peach::load(&a.dir, &rig.as_ref().expect("Mario rig").skeleton)
            .expect("load the exact Sunshine Peach model"))
    } else { None };
    let mut melee_peach_model = if use_melee_peach {
        let a = assets.as_ref().expect("Peach demo requires Sunshine assets");
        let cache = melee_peach::ensure_assets(&a.dir, &disc_dirs).expect("extract Melee Peach from your disc");
        Some(melee_peach::MeleePeach::load(&cache,&rig.as_ref().expect("Mario rig").skeleton)
            .expect("load Melee Peach's exact model"))
    } else { None };
    let mut pianta_models = if use_pianta {
        let a = assets.as_ref().expect("Pianta demo requires the Sunshine assets");
        pianta::ensure_assets(&a.dir, &disc_dirs).expect("extract Piantissimo from your Sunshine disc");
        pianta::Pianta::load_all(&a.dir, &rig.as_ref().expect("Mario rig").skeleton)
            .expect("load the Pianta models")
    } else { Vec::new() };
    pianta_index = pianta_index.min(pianta_models.len().saturating_sub(1));
    let capture_pose = std::env::var("SMS_POSE").ok().map(|p| {
        let (name, frame) = p.split_once(',').expect("SMS_POSE=clip,frame");
        let frame = frame.parse::<f32>().expect("SMS_POSE frame must be a number");
        assert!(rig.as_ref().is_some_and(|r| r.clips.get(name).is_some()), "unknown SMS_POSE clip: {name}");
        (name.to_string(), frame)
    });
    // `SMS_JOINTS=<file>`: a Melee animation ported onto this rig, one 4x4
    // column-major matrix per Sunshine joint (the file the libssbm
    // `mario_sms_pose` example writes). Used for the Mario A-attack captures.
    let external_joints = std::env::var("SMS_JOINTS").ok().map(|p| {
        let text = std::fs::read_to_string(&p).expect("SMS_JOINTS file");
        let mut values = text
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
            .flat_map(|l| l.split_whitespace())
            .map(|v| v.parse::<f32>().expect("SMS_JOINTS number"));
        let mut joints = Vec::new();
        while let Some(m) = values.next() {
            let mut c = [0.0f32; 16];
            c[0] = m;
            for slot in c.iter_mut().skip(1) {
                *slot = values.next().expect("SMS_JOINTS needs 16 floats per joint");
            }
            joints.push(::glam::Mat4::from_cols_array(&c));
        }
        let want = rig.as_ref().map_or(0, |r| r.skeleton.len());
        assert_eq!(joints.len(), want, "SMS_JOINTS joint count");
        joints
    });
    let mut yoshi_model = assets.as_ref().and_then(|a| {
        yoshi_model::YoshiModel::load(&a.dir)
            .map_err(|e| eprintln!("Yoshi's model: {e}"))
            .ok()
    });
    let yoshi_skin = if use_yoshi_skin {
        let a=assets.as_ref().expect("Yoshi demo requires Sunshine disc assets");
        assert!(yoshi_model.is_some(),"Yoshi model failed to load");
        Some(yoshi_skin::YoshiSkin::load(&a.dir,&rig.as_ref().expect("Mario rig").skeleton)
            .expect("load Yoshi's exact 38-joint rig"))
    } else { None };
    let (params, anims, data_label) = match &assets {
        Some(a) => (
            a.params.clone(),
            a.anim_table.clone(),
            format!("{} ({} .prm files)", a.dir.display(), a.prm_files),
        ),
        None => (
            Params::default(),
            AnimTable::default(),
            "no assets/: run with --iso <disc> (or sms-extract); using code defaults".to_owned(),
        ),
    };
    let mk = || {
        let mut s = spawn();
        s.mario.params = params.clone();
        s.mario.anims = anims.clone();
        s.mario.give_fludd();
        s.mario.options.turn_to_aim=!use_yoshi_skin;
        // `SMS_NOFLUDD=1`: the RoR2 radial's "No FLUDD", for skin captures.
        if std::env::var_os("SMS_NOFLUDD").is_some() {
            s.mario.fludd = None;
        }
        // The skin's flutter (TYoshi::thinkHoldOut without riding), for the demo and captures.
        s.mario.options.yoshi_skin=use_yoshi_skin;
        // `SMS_SPRAY_DIVE=1`: the host extension the RoR2 mod uses â€” the Spray nozzle may fire
        // during a B dive and the belly slide (`Options::spray_during_dive`).
        s.mario.options.spray_during_dive = std::env::var_os("SMS_SPRAY_DIVE").is_some();
        s.mario.anim = libsms::AnimState::new(libsms::anim::clip::WAIT, &s.mario.anims);
        // Yoshi's egg on the far side of the start, facing it (`eggYoshi`).
        s.mario.add_egg(V3::new(0.0, 0.0, -700.0), 0);
        // `SMS_YOSHI=1` starts riding an orange Yoshi, `SMS_YOSHI=2` with him standing by
        // (for screenshots).
        match std::env::var("SMS_YOSHI").as_deref() {
            Ok("1") => {
                s.mario
                    .spawn_yoshi(s.mario.pos, s.mario.face_yaw, Some(libsms::Fruit::Papaya));
                s.mario.mount_yoshi();
            }
            Ok(_) => s.mario.spawn_yoshi(
                V3::new(-250.0, 0.0, 0.0),
                0x4000,
                Some(libsms::Fruit::Papaya),
            ),
            Err(_) => {}
        }
        s
    };

    let mut sim = mk();
    // `SMS_HANG=1` starts hanging from the 300-high step (for screenshots).
    if std::env::var("SMS_HANG").is_ok() {
        let m = &mut sim.mario;
        m.pos = V3::new(-750.0, 300.0, 410.0);
        m.prev_pos = m.pos;
        m.face_yaw = 0;
        m.model_yaw = 0;
        m.change_status(Status::Hanging, 0, true);
    }
    let mut world = World::course();
    // `SMS_HOLD=1` starts carrying the papaya (for screenshots).
    if std::env::var("SMS_HOLD").is_ok() {
        world.fruits[0].held = true;
        sim.mario.held = Some(0);
        sim.mario.upper = libsms::UpperState::HoldingObject;
    }
    let mut water = WaterManager::new(&params);
    let mut juice_water=use_yoshi_skin.then(||WaterManager::new(&params));
    // Viewer-first host tuning: same procedural water and disc type, faster flight only.
    water.spray_speed = std::env::var("SMS_WATER_SPEED").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
    water.reach = std::env::var("SMS_WATER_REACH").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
    if std::env::var_os("SMS_WATER_SPEED").is_some() {
        eprintln!("Spray: NozzleDeform.prm mType={}, speed={}x, reach={}x; WaterGun.cpp::TNozzleDeform::emit -> ModelWaterManager.cpp::makeEmit/move/calcDrawVtx",
            params.nozzle(NozzleKind::Spray).particle_type, water.spray_speed, water.reach);
    }
    let mut water_view = assets.as_ref().and_then(|a| water::WaterView::load(&a.dir));
    let mut juice_view=if use_yoshi_skin {assets.as_ref().and_then(|a|water::WaterView::load(&a.dir))}else{None};
    // The game's particle effects (nozzle mist, smoke, exhaust), run by libsms-present's View.
    let mut fx_layer = assets.as_ref().and_then(|a| fx::FxLayer::load(&a.dir));
    let mut water_ticks = 0u32;
    let mut props = assets.as_ref().map(|a| props::Props::load(&a.dir));
    let mut coin_model = assets.as_ref().and_then(|a| {
        coin::CoinModel::load(&a.dir)
            .map_err(|e| eprintln!("coins: {e}"))
            .ok()
    });
    let mut stu_model = assets.as_ref().and_then(|a| {
        stu::StuModel::load(&a.dir)
            .map_err(|e| eprintln!("Strollin' Stu: {e}"))
            .ok()
    });
    let (mut cam_a, mut cam_p) = (std::f32::consts::PI, 0.35f32);
    let mut show_help = true;
    let focus_yoshi = std::env::var("SMS_FOCUS").as_deref() == Ok("yoshi");
    let mut cam_target = if focus_yoshi {
        mq(sim.mario.yoshi.pos) + vec3(0.0, 1.3, 0.0)
    } else {
        mq(sim.mario.pos) + vec3(0.0, 1.0, 0.0)
    };
    let mut frame_no = 0u32;
    let mut sim_secs = 0.0f32;
    // `SMS_PORTRAIT=fludd|pound|slide|air`: hold a skill portrait's pose for reference captures.
    let portrait = std::env::var("SMS_PORTRAIT").ok();
    if portrait.as_deref() == Some("pound") {
        sim.mario.pos.y = 20000.0;
        sim.mario.prev_pos = sim.mario.pos;
        sim.mario.portrait_pose(Status::HipDrop, libsms::anim::clip::HIP_FALL, 22.0, 3, false);
    }
    if portrait.as_deref() == Some("slide") {
        sim.mario.options.slide_fx_scale = std::env::var("SMS_SLIDE_FX_SCALE").ok().and_then(|v| v.parse().ok()).unwrap_or(1.4);
        sim.mario.limits.wet_slide = Some(libsms::WetSlide::timed(240, true));
        sim.mario.launch(V3::new(0.0, 0.0, 45.0));
        sim.mario.portrait_pose(Status::Catch, libsms::anim::clip::DIVE, 50.0, 1, true);
        sim.mario.vel = V3::new(0.0, 0.0, 45.0);
    }
    // `SMS_CLEAN=1`: no course, water or HUD; only Mario and his effects on a keyable green.
    let clean = std::env::var_os("SMS_CLEAN").is_some();
    let prof = std::env::var("SMS_PROF").is_ok();
    let mut prof_acc = [0.0f64; 5];
    // `SMS_CAM=yaw,pitch,dist` overrides the starting camera.
    let mut cam_dist = 6.0f32;
    if let Some(v) = std::env::var("SMS_CAM").ok().map(|s| {
        s.split(',')
            .filter_map(|x| x.parse::<f32>().ok())
            .collect::<Vec<_>>()
    }) && v.len() == 3
    {
        (cam_a, cam_p, cam_dist) = (v[0], v[1], v[2]);
    }
    // Luigi hangs a little lower and further off the wall than Mario's anchor
    // (`SMS_HANG_OFFSET=<down>,<back>`, decomp cm).
    let hang_offset = std::env::var("SMS_HANG_OFFSET")
        .ok()
        .map(|v| {
            let mut it = v.split(',').filter_map(|x| x.trim().parse::<f32>().ok());
            (it.next().unwrap_or(0.0), it.next().unwrap_or(0.0))
        })
        .unwrap_or((8.0, 6.0));

    loop {
        if is_key_pressed(KeyCode::Escape) { break; }
        if compare_peach && is_key_pressed(KeyCode::Tab) { show_melee_peach = !show_melee_peach; }
        if use_pianta && is_key_pressed(KeyCode::Tab) { pianta_index = (pianta_index + 1) % pianta_models.len(); }
        let dt = if portrait.is_some() && std::env::var_os("SMS_SHOT").is_some() {
            1.0 / 120.0
        } else { get_frame_time().min(0.1) };
        sim_secs += dt;

        // `SMS_LOOP=slide`: with `SMS_PORTRAIT=slide`, start the slide again as soon as Mario
        // gets up, so the slide spray (and FLUDD's pump) can be watched looping.
        if std::env::var("SMS_LOOP").as_deref() == Ok("slide")
            && portrait.as_deref() == Some("slide")
            && sim.mario.status != Status::Catch
        {
            sim = mk();
            sim.mario.limits.wet_slide = Some(libsms::WetSlide::timed(240, true));
            sim.mario.launch(V3::new(0.0, 0.0, 45.0));
            sim.mario
                .portrait_pose(Status::Catch, libsms::anim::clip::DIVE, 50.0, 1, true);
            sim.mario.vel = V3::new(0.0, 0.0, 45.0);
            water.particles.clear();
        }

        // Camera.
        if is_key_down(KeyCode::Left) {
            cam_a -= 1.8 * dt;
        }
        if is_key_down(KeyCode::Right) {
            cam_a += 1.8 * dt;
        }
        if is_key_down(KeyCode::Up) {
            cam_p = (cam_p + 1.0 * dt).min(1.3);
        }
        if is_key_down(KeyCode::Down) {
            cam_p = (cam_p - 1.0 * dt).max(-0.2);
        }
        let camera_yaw = to_angle(cam_a * 32768.0 / std::f32::consts::PI);

        // Input.
        let k = is_key_down;
        let mut stick = SV2::new(
            (k(KeyCode::D) as i32 - k(KeyCode::A) as i32) as f32,
            (k(KeyCode::W) as i32 - k(KeyCode::S) as i32) as f32,
        );
        if stick.length() > 1.0 {
            stick = stick.normalize();
        }
        if k(KeyCode::LeftAlt) {
            stick *= 0.35;
        }
        let crouch = k(KeyCode::LeftControl);
        let pad = Pad {
            stick,
            a: k(KeyCode::Space),
            b: k(KeyCode::F),
            l: k(KeyCode::Z),
            r: crouch,
            l_analog: 0,
            r_analog: if crouch {
                255
            } else if k(KeyCode::LeftShift) {
                150
            } else {
                0
            },
            x: k(KeyCode::X),
        };

        // `--demo` replays a scripted loop (run, jump, dive, pound, spray) instead of the keys.
        let mut pad = if demo {
            demo_pad(get_time() as f32)
        } else {
            pad
        };
        // `SMS_R=<0-255>` holds the trigger (for screenshots of spraying).
        if let Some(r) = std::env::var("SMS_R").ok().and_then(|v| v.parse().ok()) {
            pad.r_analog = r;
            // All the way in is the click that crouches him (Ctrl): what the Rocket charges from.
            pad.r = r >= 255;
        }
        if portrait.as_deref() == Some("fludd") {
            pad.r_analog = 255;
            pad.r = true;
        }
        // `SMS_FLUTTER=1` scripts a double jump: take off, release, press A again in the air, so
        // the Yoshi skin flutters; for captures. Sim ticks, so the timing does not depend on the
        // render rate.
        if std::env::var_os("SMS_FLUTTER").is_some() {
            let t = sim.mario.ticks;
            pad.a = (60..80).contains(&t) || (95..900).contains(&t);
        }
        // `SMS_B=<frame>` presses B on that frame (for screenshots of the tongue).
        if std::env::var("SMS_B")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            == Some(frame_no)
        {
            pad.b = true;
        }
        if is_key_pressed(KeyCode::R) || sim.mario.pos.y < -2500.0 {
            sim = mk();
            water.particles.clear();
            if let Some(juice)=&mut juice_water {juice.particles.clear();}
            if let Some(audio) = &mut audio {
                audio.reset();
            }
        }
        if is_key_pressed(KeyCode::H) {
            show_help = !show_help;
        }
        if is_key_pressed(KeyCode::M)
            && let Some(audio) = &mut audio
        {
            audio.cycle();
        }
        if let Some(model) = mario_model.as_mut()
            && (is_key_pressed(KeyCode::T) || (frame_no == 0 && std::env::var("SMS_SHIRT").is_ok()))
        {
            model.shirt = !model.shirt;
        }
        if let Some(f) = sim.mario.fludd.as_mut() {
            let p = sim.mario.params.clone();
            for (key, n) in [
                (KeyCode::Key1, NozzleKind::Spray),
                (KeyCode::Key2, NozzleKind::Hover),
                (KeyCode::Key3, NozzleKind::Rocket),
                (KeyCode::Key4, NozzleKind::Turbo),
            ] {
                if is_key_pressed(key) {
                    if f.nozzle != n {
                        world.sounds.push(Event::Sound(libsms::SoundCue {
                            name: "MSD_SE_SY_SELECT_POMP",
                            pos: sim.mario.pos,
                            info: None,
                        }));
                    }
                    f.change_nozzle(n, false, &p);
                    f.refill(&p);
                }
            }
            if is_key_pressed(KeyCode::G) {
                f.refill(&p);
            }
            // `SMS_NOZZLE=<1-4>` starts with that nozzle (Spray, Hover, Rocket, Turbo), for screenshots.
            if frame_no == 0
                && let Some(n) = std::env::var("SMS_NOZZLE").ok().and_then(|v| v.parse::<u32>().ok())
            {
                let kind = match n {
                    1 => Some(NozzleKind::Spray),
                    2 => Some(NozzleKind::Hover),
                    3 => Some(NozzleKind::Rocket),
                    4 => Some(NozzleKind::Turbo),
                    _ => None,
                };
                if let Some(kind) = kind {
                    f.change_nozzle(kind, false, &p);
                    f.refill(&p);
                }
            }
        }

        // Simulate. Water events are drained into droplets right after each frame's ticks.
        // Host aim, the way the RoR2 adapter reports it: `MarioDriver.Aim` sends
        // `SetAim(1, crosshair)` while `AimAtCrosshair` is on, the spray trigger is down and
        // the fitted nozzle is the Spray (`step.r_analog > 0 && fitted.nozzle == 1`), and
        // `SetAim(0, ..)` otherwise. The viewer mirrors that condition with the camera as the
        // crosshair. `SMS_AIM=<yaw>[,<pitch>]` overrides it in absolute degrees for captures.
        let aim_env = std::env::var("SMS_AIM").ok().map(|v| {
            let mut it = v.split(',').filter_map(|x| x.trim().parse::<f32>().ok());
            (it.next().unwrap_or(0.0), it.next().unwrap_or(0.0))
        });
        let pi = std::f32::consts::PI;
        let aim = if let Some((deg, pitch)) = aim_env {
            Some((
                to_angle(deg.to_radians() * 32768.0 / pi),
                to_angle(pitch.to_radians() * 32768.0 / pi),
            ))
        } else if pad.r_analog > 0
            && sim.mario.fludd.as_ref().is_some_and(|f| f.nozzle == NozzleKind::Spray)
        {
            Some((
                to_angle((cam_a + pi) * 32768.0 / pi),
                to_angle(-cam_p * 32768.0 / pi),
            ))
        } else {
            None
        };
        sim.mario.aim = aim.map(|(yaw, pitch)| libsms::Aim { yaw, pitch });
        let t0 = std::time::Instant::now();
        sim.mario.water_underfoot = water.on_ground_near(sim.mario.pos);
        let mut routed_emits=0;
        let ticks = sim.advance_observed(&mut world, &pad, camera_yaw, dt, |mario, world| {
            if let Some(skin)=&yoshi_skin {
                for w in world.emits.iter_mut().skip(routed_emits) {skin.route_emit(mario,w);}
                routed_emits=world.emits.len();
            }
            world.tick_fruits();
            world.tick_stus();
            if let Some(audio) = &mut audio {
                audio.events(&world.sounds, mario);
                audio.tick(mario);
            }
            if let Some(fx) = &mut fx_layer {
                fx.events(&world.sounds, mario);
            }
            world.sounds.clear();
        });
        if portrait.is_none() { world.touch(&mut sim.mario); }
        // Reference captures: hold the pose each skill portrait uses (the FFI's
        // sms_sim_portrait). The generated icons use the same poses.
        if let Some(p) = portrait.as_deref() {
            use libsms::anim::clip;
            match p {
                "air" => sim.mario.portrait_pose(Status::Wait, clip::WAIT, 30.0, 0, false),
                _ => {}
            }
        }
        // `K` hurts Mario from in front (for trying the knock-backs without a Stu);
        // `SMS_HIT=<frame>` does it on that frame (for screenshots).
        let hit_frame = std::env::var("SMS_HIT")
            .ok()
            .and_then(|v| v.parse::<u32>().ok());
        if is_key_pressed(KeyCode::K) || hit_frame == Some(frame_no) {
            let m = &mut sim.mario;
            let ahead = m.pos
                + V3::new(
                    (m.face_yaw as f32 * std::f32::consts::PI / 32768.0).sin(),
                    0.0,
                    (m.face_yaw as f32 * std::f32::consts::PI / 32768.0).cos(),
                ) * 100.0;
            m.damage(ahead, libsms::DamageKind::EnemyCommon);
        }
        // Fire proof in the full Rust renderer, using the same host ignition path as RoR2.
        // SMS_FIRE=<frame>; SMS_SHOT_TIME chooses the burn hop or recovery capture time.
        if std::env::var("SMS_FIRE").ok().and_then(|v| v.parse::<u32>().ok()) == Some(frame_no) {
            sim.mario.options.host_fire_reaction = true;
            sim.mario.ignite();
        }
        // FLUDD's water (`TModelWaterManager`): emitted every tick, moved once a frame.
        for e in std::mem::take(&mut world.emits) {
            if e.nozzle==NozzleKind::Yoshi && use_yoshi_skin {
                if let Some(juice)=&mut juice_water {juice.emit_carried(&e);}
            } else {water.emit(&e);}
        }
        for (pos, velocity) in std::mem::take(&mut world.body_drips) { water.emit_body_drip(pos, velocity); }
        water_ticks += ticks;
        while water_ticks >= 4 {
            water_ticks -= 4;
            water.step(&world, sim.mario.pos);
            if let Some(juice)=&mut juice_water {
                juice.step(&world,sim.mario.pos);
                if let Some(v)=&mut juice_view {v.step(&juice.events);}
                if let Some(a)=&mut audio {a.water(&juice.events);}
                juice.events.clear();
            }
            if sim.mario.wet_sliding() && portrait.is_none() {
                water.slide_splashes(sim.mario.pos);
            }
            if let Some(v) = water_view.as_mut() {
                v.step(&water.events);
            }
            if let Some(audio) = &mut audio {
                audio.water(&water.events);
            }
            water.events.clear();
        }

        let t1 = std::time::Instant::now();
        // Render.
        clear_background(if clean {
            if portrait.as_deref() == Some("air") { Color::new(0.78, 0.32, 0.07, 1.0) }
            else if portrait.as_deref() == Some("fludd") { Color::new(0.06, 0.26, 0.56, 1.0) }
            else if portrait.as_deref() == Some("slide") { Color::new(0.06, 0.30, 0.13, 1.0) }
            else if portrait.as_deref() == Some("pound") { Color::from_rgba(116, 70, 158, 255) }
            else { Color::new(0.035, 0.045, 0.065, 1.0) }
        } else {
            Color::new(0.45, 0.65, 0.9, 1.0)
        });
        let focus_y = std::env::var("SMS_TARGET_Y").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0f32);
        let offset = std::env::var("SMS_TARGET_Z").ok().and_then(|v| v.parse().ok()).unwrap_or(0.0);
        let right_offset = std::env::var("SMS_TARGET_X").ok().and_then(|v| v.parse().ok()).unwrap_or(0.0);
        let target = if portrait.as_deref() == Some("air") { vec3(0.0, 0.0, 0.0) } else {
            mq(sim.mario.prev_pos.lerp(sim.mario.pos, sim.alpha())) + vec3(right_offset, focus_y, offset)
        };
        // `SMS_FOCUS=yoshi` follows Yoshi instead (for screenshots).
        let target = if focus_yoshi {
            mq(sim.mario.yoshi.pos) + vec3(0.0, 1.3, 0.0)
        } else {
            target
        };
        cam_target = if portrait.is_some() { target } else { cam_target.lerp(target, (dt * 8.0).min(1.0)) };
        let eye = cam_target
            + vec3(
                cam_a.sin() * cam_p.cos(),
                cam_p.sin(),
                cam_a.cos() * cam_p.cos(),
            ) * cam_dist;
        if let Some(audio) = &audio {
            let to_units = |v: Vec3| V3::new(v.x, v.y, v.z) / S;
            audio.listener(to_units(eye), to_units(cam_target));
        }
        let camera = Camera3D {
            position: eye,
            target: cam_target,
            up: vec3(0.0, 1.0, 0.0),
            fovy: 0.9,
            ..Default::default()
        };
        set_camera(&camera);
        // The host aim vector (cyan) and the facing it is measured against (white), so the
        // side of the aim can be read at a glance.
        if let Some(aim) = sim.mario.aim {
            let base = mq(sim.mario.prev_pos.lerp(sim.mario.pos, sim.alpha()))
                + vec3(0.0, 1.25, 0.0);
            let yaw = aim.yaw as f32 * std::f32::consts::PI / 32768.0;
            let dir = vec3(yaw.sin(), 0.0, yaw.cos());
            draw_line_3d(base, base + dir * 2.5, Color::new(0.2, 1.0, 1.0, 1.0));
            let face = sim.mario.model_yaw as f32 * std::f32::consts::PI / 32768.0;
            let fwd = vec3(face.sin(), 0.0, face.cos());
            draw_line_3d(base, base + fwd * 1.5, Color::new(1.0, 1.0, 1.0, 0.8));
        }
        if !clean {
            draw_world(&world);
        }
        match (&mut mario_model, &rig) {
            (Some(model), Some(r)) => {
                let m = &sim.mario;
                // The body clip, with the pumping arm over the chest while pumping.
                let upper = m
                    .upper_layer()
                    .and_then(|(c, f)| Some((r.clips.get(c)?, f, r.skeleton.find("jnt_chest")?)));
                let upper=upper.filter(|_|!use_yoshi_skin || !yoshi_skin::SprayRoute::for_mario(m).mouth
                    || !matches!(m.upper,libsms::UpperState::Pumping|libsms::UpperState::HoldingPump));
                let mut joints = r
                    .skeleton
                    .pose_layered(r.clips.get(m.anim.clip).map(|c| (c, m.anim.frame)), upper);
                // Deterministic multi-frame retarget captures; ordinary play uses the sim's pose.
                if let Some((name, frame)) = &capture_pose {
                    joints = r.skeleton.pose(r.clips.get(name).expect("validated capture clip"), *frame);
                }
                if let Some(ported) = &external_joints {
                    joints = ported.clone();
                }
                let pose_name = capture_pose.as_ref().map_or(m.anim.clip,|(n,_)|n.as_str());
                let pose_frame = capture_pose.as_ref().map_or(m.anim.frame,|(_,f)|*f);
                let pose_phase = r.clips.get(pose_name).map_or(0.,|c|pose_frame.rem_euclid(c.frames.max(1.))/c.frames.max(1.));
                let p = mq(m.prev_pos.lerp(m.pos, sim.alpha()));
                let yaw = m.model_yaw as f32 * std::f32::consts::PI / 32768.0;
                let root = mario_model::root_matrix(::glam::Vec3::new(p.x, p.y, p.z), yaw, S);
                let g = |v: Vec3| ::glam::Vec3::new(v.x, v.y, v.z);
                let view = ::glam::Mat4::look_at_rh(g(eye), g(cam_target), ::glam::Vec3::Y);
                let proj = ::glam::Mat4::perspective_rh_gl(
                    0.9,
                    screen_width() / screen_height(),
                    0.01,
                    10000.0,
                );
                if !clean {
                    draw_shadow(&sim, sim.alpha());
                }
                let mut muzzle_mtx: Vec<::glam::Mat4> = if portrait.as_deref() == Some("air") {
                    model.draw_nozzle_group(view, proj);
                    Vec::new()
                } else if (use_luigi && !luigi_beside) || use_peach || use_melee_peach || use_pianta || use_yoshi_skin {
                    Vec::new()
                } else {
                    let mario_root = if luigi_beside {
                        root * ::glam::Mat4::from_translation(::glam::Vec3::new(120.0, 0.0, 0.0))
                    } else {
                        root
                    };
                    let muzzles = model.draw(&sim, &joints, mario_root, view, proj, get_time() as f32);
                    if let (Some(h), Some(j)) = (helmet.as_mut(), r.skeleton.find("jnt_head").and_then(|j| joints.get(j))) {
                        h.draw(*j, helmet::Fit::from_env(helmet::Fit::MARIO), mario_root, view, proj);
                    }
                    muzzles
                };
                if let (Some(skin),Some(ym))=(&yoshi_skin,&mut yoshi_model) {
                    let route=yoshi_skin::SprayRoute::for_mario(m);
                    let active=m.fludd.as_ref().is_some_and(|f|f.spray_amount()>0. && f.water>0 && pad.r_analog>0);
                    let (native,frame)=skin.choose(m,pose_name,pose_phase);
                    let base=match native {
                        Some(name)=>skin.pose_native(name,frame),
                        None=>skin.pose(&joints),
                    };
                    let mut world=skin.spray_world(base,route,active);
                    let offset=model.lowest(&joints)-ym.lowest(&world);
                    for joint in &mut world { joint.w_axis.y+=offset; }
                    let mouth=skin.remember_mouth(&world);
                    ym.draw_mario_pose(&world,root,view,proj,get_time() as f32);
                    if let Some(h)=helmet.as_mut() {
                        h.draw(skin.head_world(&world),helmet::Fit::from_env(YOSHI_FIT),root,view,proj);
                    }
                    if m.fludd.is_some() {
                        if let Some(f)=&mut luigi_fludd {
                            f.belt_scale=1.5;
                            let deployed=!route.mouth && m.fludd.as_ref().is_some_and(|f|f.housed<0.5);
                            let muzzles=f.draw(skin.fludd_look(m,route),deployed,false,dt,
                                skin.chest(&world),root,view,proj,(true,true));
                            muzzle_mtx=if route.mouth {vec![mouth]}else{muzzles};
                        }
                    }
                }
                if let Some(peach) = peach_model.as_mut().filter(|_| !show_melee_peach) {
                    let world = peach.pose(&joints, model.lowest(&joints),pose_name,pose_phase);
                    peach.draw(&world, root, view, proj, get_time() as f32);
                    if let Some(f) = &mut luigi_fludd {
                        f.belt_scale=0.8;
                        let deployed = m.fludd.as_ref().is_some_and(|f| f.housed < 0.5);
                        muzzle_mtx = f.draw(m.fludd_look(), deployed, true, dt,
                            peach.chest(&world), root, view, proj, (true, true));
                    }
                }
                if let Some(peach) = melee_peach_model.as_mut().filter(|_| show_melee_peach) {
                    let world = peach.pose(&joints, model.lowest(&joints),pose_name,pose_phase);
                    peach.draw(&world, root, view, proj);
                    if let Some(f) = &mut luigi_fludd {
                        f.belt_scale=0.8;
                        let deployed = m.fludd.as_ref().is_some_and(|f| f.housed < 0.5);
                        muzzle_mtx = f.draw(m.fludd_look(), deployed, true, dt,
                            peach.chest(&world), root, view, proj, (true, true));
                    }
                }
                if let Some(pianta) = pianta_models.get_mut(pianta_index) {
                    let world = pianta.pose(&joints, model.lowest(&joints));
                    pianta.draw(&world, root, view, proj);
                    if let Some(h) = helmet.as_mut() {
                        h.draw(pianta.head(&world), helmet::Fit::from_env(PIANTA_FIT), root, view, proj);
                    }
                    if let Some(f) = &mut luigi_fludd {
                        let deployed = m.fludd.as_ref().is_some_and(|f| f.housed < 0.5);
                        muzzle_mtx = f.draw(m.fludd_look(), deployed, true, dt,
                            pianta.chest(&world), root, view, proj, (true, true));
                    }
                }
                if let Some(l) = &mut luigi_model {
                    let name = sim.mario.anim.clip;
                    let frame = sim.mario.anim.frame;
                    if let Some(mut world) = l.pose(name, frame) {
                        // Luigi's rig carries Mario's clip as rotations only, so it loses the
                        // root motion the game's clips carry (the dive's drop, the hang's drop
                        // below the grab, the idle's bob). Anchor his lowest point to the drawn
                        // Mario's: feet to feet, belly to belly.
                        let anchor = (model.lowest(&joints) - l.lowest(&world) * 10.0) * 0.1;
                        for m in &mut world {
                            m.w_axis.y += anchor;
                        }
                        // Hanging, he sits a touch lower and further off the wall than the
                        // anchor puts him (`SMS_HANG_OFFSET=<down>,<back>` in decomp cm).
                        if sim.mario.status == Status::Hanging {
                            for m in &mut world {
                                m.w_axis.y -= hang_offset.0 * 0.1;
                                m.w_axis.z -= hang_offset.1 * 0.1;
                            }
                        }
                        l.draw(&world, root, view, proj);
                        if let Some(h) = helmet.as_mut() {
                            h.draw(l.head(&world), helmet::Fit::from_env(LUIGI_FIT), root, view, proj);
                        }
                        // Luigi's own FLUDD, on his chest: the sim's character wears it, so its
                        // muzzles are the ones the water and the mist leave.
                        if let Some(f) = &mut luigi_fludd {
                            let chest = l.chest(&world);
                            let look = sim.mario.fludd_look();
                            let deployed =
                                sim.mario.fludd.as_ref().is_some_and(|f| f.housed < 0.5);
                            muzzle_mtx = f.draw(
                                look, deployed, true, dt, chest, root, view, proj, (true, true),
                            );
                        }
                    }
                }
                let muzzles: Vec<(::glam::Vec3, ::glam::Vec3)> = muzzle_mtx
                    .iter()
                    .map(|m| (m.w_axis.truncate(), m.x_axis.truncate()))
                    .collect();
                if let Some(fx) = &mut fx_layer {
                    fx.set_muzzles(&muzzle_mtx);
                }
                if let Some(f) = &sim.mario.fludd {
                    let kind = f.nozzle;
                    sim.mario.set_emitters(kind, &muzzles);
                }
                // Yoshi under Mario (his root is Mario's while ridden) and the eggs. The
                // posed mouth goes back to the sim for the tongue and the juice.
                if let Some(ym) = yoshi_model.as_mut().filter(|_| portrait.is_none()) {
                    if let Some(look) = sim.mario.yoshi_look() {
                        let at = look.mounted.then_some(root);
                        if let Some((p, x)) = ym.draw(&look, at, view, proj) {
                            let g = |v: ::glam::Vec3| V3::new(v.x, v.y, v.z);
                            sim.mario.set_yoshi_mouth(g(p), g(x));
                        }
                    }
                    for e in &sim.mario.eggs {
                        if let Some(l) = e.look() {
                            ym.draw_egg(&l, view, proj, get_time() as f32);
                        }
                    }
                }
                // Fruit; Mario's rides his right hand (`getTakingMtx`), Yoshi's the tongue
                // tip, shrinking as it comes in.
                if let Some(pr) = props.as_mut().filter(|p| !clean && p.loaded()) {
                    let hand = r.skeleton.find("jnt_hand_R").and_then(|h| joints.get(h));
                    let tongue = match sim.mario.yoshi.tongue.held {
                        Some(libsms::tongue::Held::Object { id, .. }) => Some(id),
                        _ => None,
                    };
                    for (i, f) in world.fruits.iter().enumerate().filter(|f| f.1.gone == 0) {
                        let at = match (f.held, hand) {
                            _ if f.held && tongue == Some(i as u64) => {
                                let t = &sim.mario.yoshi.tongue;
                                let k = t.held_scale;
                                ::glam::Mat4::from_translation(
                                    ::glam::Vec3::new(t.tip.x, t.tip.y, t.tip.z) * S,
                                ) * ::glam::Mat4::from_scale(::glam::Vec3::splat(S * k))
                            }
                            (true, Some(h)) => {
                                let lift = f.kind.held_offset() * S;
                                ::glam::Mat4::from_translation(::glam::Vec3::Y * lift) * root * *h
                            }
                            _ => props::place(f.kind, f.pos, f.yaw, S),
                        };
                        pr.draw(f.kind, at, view, proj);
                    }
                }
                if let Some(cm) = coin_model.as_mut().filter(|_| !clean) {
                    cm.draw(&world.coins, sim.mario.ticks, S, view, proj);
                }
                // Stus walk their circle; one on the tongue rides the tip, shrinking in.
                if let Some(sm) = stu_model.as_mut().filter(|_| !clean) {
                    let t = &sim.mario.yoshi.tongue;
                    let on_tongue = match t.held {
                        Some(libsms::tongue::Held::Enemy { id }) => Some(id),
                        _ => None,
                    };
                    for (i, s) in world.stus.iter().enumerate().filter(|s| s.1.gone == 0) {
                        let at = if s.held && on_tongue == Some(i as u64) {
                            let lift = V3::Y * (0.5 * stu::HEIGHT * t.held_scale);
                            stu::place(t.tip - lift, s.yaw, S)
                                * ::glam::Mat4::from_scale(::glam::Vec3::splat(t.held_scale))
                        } else {
                            stu::place(s.pos, s.yaw, S)
                        };
                        sm.draw(s, at, view, proj);
                    }
                }
            }
            _ => draw_mario(&sim, rig.as_ref(), sim.alpha()),
        }
        let t2 = std::time::Instant::now();
        if let Some(fx) = fx_layer.as_mut() {
            let fwd = (camera.target - camera.position).normalize_or_zero();
            let right = fwd.cross(vec3(0.0, 1.0, 0.0)).normalize_or_zero();
            fx.update(&mut sim, dt, right, right.cross(fwd));
            fx.draw(S);
        }
        match water_view.as_mut() {
            Some(v) => v.draw(&water, &camera, sim.mario.juice_color()),
            None if !clean => draw_water_plain(&water),
            None => {}
        }
        if let (Some(juice),Some(view))=(&juice_water,&mut juice_view) {
            view.draw(juice,&camera,1); // Sunshine's orange juice palette; old drops retain their colour.
        }
        let t3 = std::time::Instant::now();
        if clean {
            // No 2D overlay; the capture is Mario and his effects alone.
        } else {
            set_default_camera();
            hud(&sim, &data_label, rig.as_ref(), show_help);
        }
        if show_help && !clean {
            let music = audio
                .as_ref()
                .map_or("audio unavailable", |a| a.music_name());
            draw_text(
                format!("M cycle music: {music}"),
                16.0,
                screen_height() - 12.0,
                18.0,
                WHITE,
            );
        }
        for (i, l) in world.log.iter().rev().filter(|_| !clean).enumerate() {
            draw_text(
                l,
                screen_width() - 300.0,
                170.0 + 22.0 * i as f32,
                22.0,
                Color::new(1.0, 1.0, 1.0, 1.0 - i as f32 * 0.14),
            );
        }

        // `SMS_SHOT=<png>` saves a screenshot after `SMS_SHOT_FRAME` frames (default 90) and
        // quits; for checking the renderer without a person at the window.
        frame_no += 1;
        if use_yoshi_skin {
            let pose=std::env::var("SMS_POSE").unwrap_or_else(|_|format!("{}, {:.1}",sim.mario.anim.clip,sim.mario.anim.frame));
            let route=yoshi_skin::SprayRoute::for_mario(&sim.mario);
            let mode=if route.mouth {"MOUTH JUICE / FLUDD REST"}else{"FLUDD"};
            draw_rectangle(0.,screen_height()-88.,screen_width(),24.,Color::new(0.04,0.04,0.08,0.85));
            draw_text(&format!("MARI-YOSHI | {mode} | Yoshi.cpp null_tongue | NozzleYoshiMouth.prm type 12 | Mario: {pose} | Esc: quit"),
                12.,screen_height()-71.,16.,WHITE);
        }
        if let Some(p) = pianta_models.get(pianta_index) {
            let pose = std::env::var("SMS_POSE").unwrap_or_else(|_| format!("{}, {:.1}", sim.mario.anim.clip, sim.mario.anim.frame));
            draw_rectangle(0.0, screen_height() - 88.0, screen_width(), 24.0, Color::new(0.04,0.04,0.08,0.85));
            draw_text(&format!("{} | Mario: {pose} | Tab: next | Esc: quit", p.label),
                12.0, screen_height() - 71.0, 16.0, WHITE);
        }
        if use_peach || use_melee_peach {
            let pose = std::env::var("SMS_POSE").unwrap_or_else(|_| format!("{}, {:.1}", sim.mario.anim.clip, sim.mario.anim.frame));
            draw_rectangle(0.0, screen_height() - 88.0, screen_width(), 24.0, Color::new(0.04,0.04,0.08,0.85));
            let identity = if show_melee_peach { "MELEE PEACH | PlPeNr.dat:PlyPeach5K_Share_joint | ftPeach/ftpeach.c" }
                else { "SUNSHINE PEACH | 26 joints | airport0.szs:peach/peach_model.bmd | TPeachManager::createModelData" };
            draw_text(&format!("{identity} | Mario: {pose} | Tab: swap Peach | Esc: quit"),
                12.0, screen_height() - 71.0, 16.0, WHITE);
        }
        if let Ok(path) = std::env::var("SMS_SHOT") {
            if std::env::var_os("SMS_WATER_SPEED").is_some() {
                draw_rectangle(0.0, screen_height() - 60.0, screen_width(), 60.0, Color::new(0.04, 0.05, 0.08, 0.95));
                let label = format!("Spray mType {} | flight {:.2}x | reach {:.2}x", params.nozzle(NozzleKind::Spray).particle_type, water.spray_speed, water.reach);
                draw_text(&label, 12.0, screen_height() - 40.0, 20.0, WHITE);
                draw_text("WaterGun.cpp::TNozzleDeform::emit -> ModelWaterManager.cpp::makeEmit/move/calcDrawVtx", 12.0, screen_height() - 16.0, 16.0, WHITE);
            }
            let at = std::env::var("SMS_SHOT_FRAME")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(90u32);
            // `SMS_SHOT_TIME=<seconds>` waits for that much sim time instead (frames run as fast as
            // the machine draws them, so a frame count isn't a moment in the game).
            let due = match std::env::var("SMS_SHOT_TIME").ok().and_then(|v| v.parse::<f32>().ok()) {
                Some(t) => sim_secs >= t,
                None => frame_no == at,
            };
            if due {
                eprintln!("portrait {:?}: {:?}, position {:?}, wet={}, pounding={}", portrait, sim.mario.status, sim.mario.pos, sim.mario.wet_sliding(), sim.mario.pounding());
                if let Some(fx) = fx_layer.as_ref() { fx.capture_report(); }
                get_screen_data().export_png(&path);
                break;
            }
        }

        // `SMS_PROF=1` prints where each second's frame time went.
        if prof {
            prof_acc[0] += (t1 - t0).as_secs_f64();
            prof_acc[1] += (t2 - t1).as_secs_f64();
            prof_acc[2] += (t3 - t2).as_secs_f64();
            prof_acc[3] += dt as f64;
            prof_acc[4] += 1.0;
            if prof_acc[3] >= 1.0 {
                let n = prof_acc[4];
                eprintln!(
                    "prof: {:.0} fps  sim+audio {:.2} ms  scene {:.2} ms  water {:.2} ms  drops {}",
                    n / prof_acc[3],
                    prof_acc[0] / n * 1e3,
                    prof_acc[1] / n * 1e3,
                    prof_acc[2] / n * 1e3,
                    water.particles.len()
                );
                prof_acc = [0.0; 5];
            }
        }
        next_frame().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sim_on_course() -> Sim {
        let mut s = spawn();
        if let Some(a) = libsms::Assets::discover(None) {
            s.mario.params = a.params;
            s.mario.anims = a.anim_table;
        }
        s
    }

    #[test]
    fn ground_and_walls_match_the_course() {
        let w = World::course();
        // On the first step (100 high), and beside it on the floor.
        assert_eq!(
            w.ground(V3::new(-1350.0, 500.0, 500.0)).unwrap().height,
            100.0
        );
        assert_eq!(
            w.ground(V3::new(-1350.0, 500.0, 100.0)).unwrap().height,
            0.0
        );
        // Off the edge of the world is a void.
        assert!(w.ground(V3::new(5000.0, 0.0, 0.0)).is_none());
        // A cylinder overlapping the tall wall is pushed out of it, normal facing away.
        let mut hits = Vec::new();
        let c = w.walls(V3::new(580.0, 50.0, 0.0), 50.0, 4, &mut hits);
        assert_eq!(hits.len(), 1);
        assert!(
            hits[0].normal.x < -0.9 && c.x <= 550.0 + 1e-3,
            "{c:?} {:?}",
            hits[0].normal
        );
        // Under the bridge there is a ceiling at 250.
        assert_eq!(w.roof(V3::new(0.0, 80.0, -900.0)).unwrap().height, 250.0);
    }

    #[test]
    fn mario_jumps_onto_a_ledge_and_stands_on_it() {
        if libsms::assets::find(None).is_none() {
            eprintln!("skipped: needs assets/ from tools/extract_assets.py");
            return;
        }
        let mut w = World::course();
        let mut s = sim_on_course();
        s.mario.pos = V3::new(-1350.0, 0.0, 100.0);
        s.mario.prev_pos = s.mario.pos;
        // A gentle push (about a third of the stick) keeps the jump short enough for a 300-deep ledge.
        let run = Pad {
            stick: SV2::new(0.0, 0.35),
            ..Pad::default()
        };
        let step = |s: &mut Sim, w: &mut World, pad: &Pad, n: u32| {
            for _ in 0..n {
                s.advance(w, pad, i16::MIN, 1.0 / TICK_HZ as f32);
            }
        };
        for _ in 0..2000 {
            if s.mario.pos.z > 335.0 {
                break;
            }
            step(&mut s, &mut w, &run, 1);
        }
        let jump = Pad { a: true, ..run };
        step(&mut s, &mut w, &jump, 1);
        for _ in 0..600 {
            step(&mut s, &mut w, &jump, 1);
            if s.mario.status != Status::Jump && s.mario.pos.y > 50.0 {
                break;
            }
        }
        // Let go and stand still on whatever he landed on.
        step(&mut s, &mut w, &Pad::default(), 300);
        let m = &s.mario;
        assert!(
            (m.pos.y - 100.0).abs() < 2.0,
            "not on the ledge: {:?} {:?}",
            m.pos,
            m.status
        );
        assert!(m.pos.z > 400.0 && m.pos.z < 700.0, "{:?}", m.pos);
    }

    #[test]
    fn spraying_makes_droplets() {
        let mut w = World::course();
        let mut s = sim_on_course();
        s.mario.give_fludd();
        let mut water = WaterManager::new(&s.mario.params);
        let spray = Pad {
            r_analog: 150,
            ..Pad::default()
        };
        for _ in 0..TICK_HZ {
            s.advance(&mut w, &spray, i16::MIN, 1.0 / TICK_HZ as f32);
        }
        assert!(!w.emits.is_empty(), "no water events");
        for e in std::mem::take(&mut w.emits) {
            water.emit(&e);
        }
        assert!(!water.particles.is_empty());
        for _ in 0..5 {
            water.step(&w, s.mario.pos);
        }
        // Droplets travel forward (+Z, Mario faces 0).
        assert!(water.particles.iter().any(|d| d.pos.z > 100.0));
    }
}







