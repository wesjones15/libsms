//! A running sample: libsms driving Mario through a small 3D obstacle course.
//!
//! The course is the host: `World` implements `libsms::Host` with boxes and two ramps and turns
//! FLUDD `WaterEmit` events into droplets. Everything about how Mario moves comes from the library.
//!
//! Controls
//!   WASD            move (Left Alt = gentle push)        Arrows   orbit camera
//!   Space           jump (A)    hold for full height     F        dive (B)
//!   Z               ground pound in the air (L)          Left Ctrl  crouch/aim (R click)
//!   Left Shift      spray (R, half press)                X        swap nozzle (Spray <-> second)
//!   1 2 3 4         fit Spray / Hover / Rocket / Turbo   G        refill tank
//!   R               respawn                              H        toggle help
//!
//! Tuning, animation clips and the skeleton come from your own disc: run
//! `sms-viewer --iso <your disc image>` once (or `cargo run -p libsms --bin sms-extract -- <iso>`,
//! or `python tools/extract_assets.py <iso>`) to build the git-ignored `assets/` folder; a path
//! argument or `SMS_ASSETS` overrides where it is looked for. Without it the decomp's
//! code defaults and a plain capsule are used, and the HUD says so. `--demo` replays a script.

use std::path::Path;

use libsms::math::to_angle;
use libsms::{
    AnimTable, Event, GroundHit, Host, NozzleKind, Pad, Params, RoofHit, Sim, Status, SurfaceFlags,
    TICK_HZ, Vec2 as SV2, Vec3 as V3, WallHit, WaterEmit,
};
use macroquad::prelude::*;

/// Decomp units (about cm) to scene metres.
const S: f32 = 0.01;

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

struct Drop {
    pos: V3,
    vel: V3,
    life: i32,
}

struct World {
    half: f32,
    blocks: Vec<Block>,
    ramps: Vec<Ramp>,
    drops: Vec<Drop>,
    emits: Vec<WaterEmit>,
    log: Vec<String>,
    seed: u32,
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
            drops: Vec::new(),
            emits: Vec::new(),
            log: Vec::new(),
            seed: 0x1234_5678,
        }
    }

    fn rand(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed % 20_000) as f32 / 10_000.0 - 1.0
    }

    fn note(&mut self, s: String) {
        self.log.push(s);
        if self.log.len() > 6 {
            self.log.remove(0);
        }
    }

    fn spawn_water(&mut self, w: &WaterEmit, mario_vel: V3) {
        for _ in 0..w.count.min(8) {
            let jitter = V3::new(self.rand(), self.rand(), self.rand()) * 0.03 * w.power;
            self.drops.push(Drop {
                pos: w.pos,
                vel: w.dir * w.power + mario_vel * 0.125 + jitter,
                life: 240,
            });
        }
        if self.drops.len() > 4000 {
            self.drops.drain(..self.drops.len() - 4000);
        }
    }

    /// Droplets use the same conventions as Mario: `pos += vel / 4` per tick, light gravity.
    fn step_drops(&mut self, ticks: u32) {
        for _ in 0..ticks {
            let mut drops = std::mem::take(&mut self.drops);
            drops.retain_mut(|d| {
                d.pos += d.vel * 0.25;
                d.vel.y -= 0.3;
                d.life -= 1;
                let floor = self
                    .ground(d.pos + V3::Y * 1.0)
                    .map(|g| g.height)
                    .unwrap_or(-1.0e9);
                d.life > 0 && d.pos.y > floor
            });
            self.drops = drops;
        }
    }
}

impl Host for World {
    fn ground(&self, p: V3) -> Option<GroundHit> {
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

    fn event(&mut self, e: Event) {
        match e {
            Event::Jump(k) => self.note(format!("jump: {k:?}")),
            Event::Land { strong: true } => self.note("hard landing".into()),
            Event::GroundPound { super_ } => self.note(format!("ground pound (super: {super_})")),
            Event::WallBump => self.note("wall bump".into()),
            Event::TriggerFired(n) => self.note(format!("{n:?} fired")),
            Event::NozzleChanged(n) => self.note(format!("nozzle: {n:?}")),
            Event::Fell => {}
            Event::WaterEmit(w) => self.emits.push(w),
            Event::Land { .. } => {}
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
    for d in &w.drops {
        draw_sphere(mq(d.pos), 0.045, None, Color::new(0.45, 0.7, 1.0, 0.9));
    }
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
    if let Some(f) = &m.fludd {
        let y = 24.0 + 22.0 * rows.len() as f32;
        let lvl = f.level(&m.params).clamp(0.0, 1.0);
        draw_rectangle(14.0, y, 220.0, 12.0, Color::new(0.0, 0.0, 0.0, 0.5));
        draw_rectangle(14.0, y, 220.0 * lvl, 12.0, nozzle_color(f.nozzle));
    }
    if show_help {
        let h = screen_height();
        draw_text(
            "WASD move  Space jump  F dive  Z pound  Ctrl crouch  Shift spray  X swap  1-4 nozzle",
            14.0,
            h - 34.0,
            20.0,
            LIGHTGRAY,
        );
        draw_text(
            "Arrows camera  G refill  R respawn  H hide help",
            14.0,
            h - 12.0,
            20.0,
            LIGHTGRAY,
        );
    }
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
    Conf {
        window_title: "libsms viewer".to_owned(),
        window_width: 1280,
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
    let mut assets = libsms::Assets::discover(explicit.as_deref().map(Path::new));
    if assets.is_none() {
        if let Some(iso) = iso {
            println!("no assets/ yet: extracting from {iso}");
            match libsms::Assets::ensure(Path::new(iso), Path::new("assets"), &mut |m| {
                println!("{m}")
            }) {
                Ok(a) => assets = Some(a),
                Err(e) => eprintln!("extraction failed: {e}"),
            }
        }
    }
    let rig = assets.as_ref().and_then(Rig::from_assets);
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
        s.mario.anim = libsms::AnimState::new(libsms::anim::clip::WAIT, &s.mario.anims);
        s
    };

    let mut sim = mk();
    let mut world = World::course();
    let (mut cam_a, mut cam_p) = (std::f32::consts::PI, 0.35f32);
    let mut show_help = true;
    let mut cam_target = mq(sim.mario.pos) + vec3(0.0, 1.0, 0.0);

    loop {
        let dt = get_frame_time().min(0.1);

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
            (k(KeyCode::A) as i32 - k(KeyCode::D) as i32) as f32,
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
        let pad = if demo {
            demo_pad(get_time() as f32)
        } else {
            pad
        };
        if is_key_pressed(KeyCode::R) || sim.mario.pos.y < -2500.0 {
            sim = mk();
            world.drops.clear();
        }
        if is_key_pressed(KeyCode::H) {
            show_help = !show_help;
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
                    f.change_nozzle(n, false, &p);
                    f.refill(&p);
                }
            }
            if is_key_pressed(KeyCode::G) {
                f.refill(&p);
            }
        }

        // Simulate. Water events are drained into droplets right after each frame's ticks.
        let ticks = sim.advance(&mut world, &pad, camera_yaw, dt);
        let emits = std::mem::take(&mut world.emits);
        let vel = V3::new(sim.mario.vel.x, sim.mario.vel.y, sim.mario.vel.z);
        for e in &emits {
            world.spawn_water(e, vel);
        }
        world.step_drops(ticks);

        // Render.
        clear_background(Color::new(0.45, 0.65, 0.9, 1.0));
        let target = mq(sim.mario.prev_pos.lerp(sim.mario.pos, sim.alpha())) + vec3(0.0, 1.0, 0.0);
        cam_target = cam_target.lerp(target, (dt * 8.0).min(1.0));
        let eye = cam_target
            + vec3(
                cam_a.sin() * cam_p.cos(),
                cam_p.sin(),
                cam_a.cos() * cam_p.cos(),
            ) * 6.0;
        set_camera(&Camera3D {
            position: eye,
            target: cam_target,
            up: vec3(0.0, 1.0, 0.0),
            fovy: 0.9,
            ..Default::default()
        });
        draw_world(&world);
        draw_mario(&sim, rig.as_ref(), sim.alpha());
        set_default_camera();
        hud(&sim, &data_label, rig.as_ref(), show_help);
        for (i, l) in world.log.iter().rev().enumerate() {
            draw_text(
                l,
                screen_width() - 300.0,
                24.0 + 22.0 * i as f32,
                22.0,
                Color::new(1.0, 1.0, 1.0, 1.0 - i as f32 * 0.14),
            );
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
        let spray = Pad {
            r_analog: 150,
            ..Pad::default()
        };
        for _ in 0..TICK_HZ {
            s.advance(&mut w, &spray, i16::MIN, 1.0 / TICK_HZ as f32);
        }
        assert!(!w.emits.is_empty(), "no water events");
        let vel = V3::ZERO;
        for e in std::mem::take(&mut w.emits) {
            w.spawn_water(&e, vel);
        }
        assert!(!w.drops.is_empty());
        w.step_drops(20);
        // Droplets travel forward (+Z, Mario faces 0).
        assert!(w.drops.iter().any(|d| d.pos.z > 100.0));
    }
}
