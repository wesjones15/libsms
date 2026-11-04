//! VR arms proof (`View::set_vr_arms`): each forearm and glove cut from Mario's own body
//! (`mario/bmd/ma_mdl1.bmd`, joints `jnt_arm_R2`/`jnt_hand_R`/`M_hand1_R` and the left's) by
//! skin weight, in its arm space. Headless; one row per pose (wait, run, jump, wet slide):
//!   1. the body from the front (model space), the right arm's triangles red, the left's blue;
//!   2. both arms in arm space from the side (+z right, +y up), axes through the origin;
//!   3. both arms in arm space from above (+x right, +z up).
//! `cargo run --release -p viewer --example vr_arms -- out.png`.

#[path = "support/png.rs"]
mod png;

use glam::{Vec2, Vec3};
use libsms::{FlatHost, Pad, Sim};
use libsms_present::View;
use libsms_present::view::VR_ARM_KEY;

const P: usize = 256;

struct Canvas {
    w: usize,
    px: Vec<u8>,
    depth: Vec<f32>,
}

impl Canvas {
    fn new(w: usize, h: usize) -> Canvas {
        let mut px = vec![0u8; w * h * 4];
        for p in px.chunks_exact_mut(4) {
            p.copy_from_slice(&[24, 26, 32, 255]);
        }
        Canvas { w, px, depth: vec![f32::INFINITY; w * h] }
    }

    fn dot(&mut self, x: i32, y: i32, c: [u8; 3]) {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.px.len() / 4 / self.w {
            return;
        }
        let i = (y as usize * self.w + x as usize) * 4;
        self.px[i..i + 3].copy_from_slice(&c);
    }

    /// A triangle already in pixels (x, y, depth), flat coloured.
    fn tri(&mut self, a: Vec3, b: Vec3, c: Vec3, col: [u8; 3]) {
        let lo = a.min(b).min(c);
        let hi = a.max(b).max(c);
        let area = (b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y);
        if area.abs() < 1e-6 {
            return;
        }
        for y in lo.y.floor().max(0.0) as i32..=hi.y.ceil() as i32 {
            for x in lo.x.floor().max(0.0) as i32..=hi.x.ceil() as i32 {
                let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                let w0 = ((b.x - p.x) * (c.y - p.y) - (c.x - p.x) * (b.y - p.y)) / area;
                let w1 = ((c.x - p.x) * (a.y - p.y) - (a.x - p.x) * (c.y - p.y)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                if x as usize >= self.w || y as usize >= self.depth.len() / self.w {
                    continue;
                }
                let z = a.z * w0 + b.z * w1 + c.z * w2;
                let i = y as usize * self.w + x as usize;
                if z < self.depth[i] {
                    self.depth[i] = z;
                    self.dot(x, y, col);
                }
            }
        }
    }
}

fn shade(base: [f32; 3], n: Vec3, toward: Vec3) -> [u8; 3] {
    let l = 0.35 + 0.65 * n.dot(toward).abs();
    [(base[0] * l * 255.0) as u8, (base[1] * l * 255.0) as u8, (base[2] * l * 255.0) as u8]
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "vr-arms.png".into());
    let dir = libsms::assets::find(None).expect("no assets/ (set SMS_ASSETS)");
    let a = libsms::Assets::load(&dir);
    let mut view = View::load(&dir).expect("Mario's models");
    view.set_vr_arms(true);
    let mut sim = Sim::new(a.params, a.anim_table, Vec3::ZERO, 0);
    let mut host = FlatHost::default();
    let dt = 1.0 / 120.0;
    let run = Pad { stick: Vec2::new(0.0, 1.0), ..Default::default() };
    // (name, presses each tick for that many ticks)
    let poses: [(&str, Vec<(Pad, u32)>); 4] = [
        ("wait", vec![(Pad::default(), 90)]),
        ("run", vec![(run, 70)]),
        ("jump", vec![(Pad { a: true, ..run }, 2), (run, 18)]),
        ("wet slide", vec![(run, 60), (Pad { b: true, ..run }, 2), (run, 70)]),
    ];
    let mut canvas = Canvas::new(P * 3, P * poses.len());
    for (row, (name, presses)) in poses.iter().enumerate() {
        for (pad, ticks) in presses {
            for _ in 0..*ticks {
                sim.advance(&mut host, pad, 0, dt);
                view.set_camera(Vec3::X, Vec3::Y);
                view.update(&mut sim, dt);
            }
        }
        let body = view.draws.iter().find(|d| d.key == 0).copied().expect("body draw");
        let arms: Vec<_> = view.draws.iter().filter(|d| d.model == 0 && d.key & 0xFF >= VR_ARM_KEY).copied().collect();
        assert_eq!(arms.len(), 2, "{name}: two arm draws");
        let count = view.draws.iter().filter(|d| d.model == 0).count();
        let n = (arms[1].first_vertex - arms[0].first_vertex) as usize;
        let mut kept = [0usize; 2];
        let mut ext = [(Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)); 2];
        // 1: the body from the front, arms coloured. Mario faces +z; look back down -z.
        let to = |p: Vec3, col: usize, row: usize, scale: f32, centre: Vec3, axes: (usize, usize)| {
            let q = p - centre;
            let (u, v) = (q[axes.0], q[axes.1]);
            Vec3::new(col as f32 * P as f32 + P as f32 * 0.5 + u * scale, row as f32 * P as f32 + P as f32 * 0.5 - v * scale, -q[3 - axes.0 - axes.1])
        };
        let pos = &view.positions;
        let nrm = &view.normals;
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for i in 0..n {
            let p = pos[body.first_vertex as usize + i];
            if p != Vec3::ZERO {
                lo = lo.min(p);
                hi = hi.max(p);
            }
        }
        let centre = (lo + hi) * 0.5;
        let body_scale = P as f32 * 0.9 / (hi - lo).max_element();
        for t in (0..n).step_by(3) {
            let i = body.first_vertex as usize + t;
            if pos[i] == Vec3::ZERO && pos[i + 1] == Vec3::ZERO && pos[i + 2] == Vec3::ZERO {
                continue;
            }
            let side = (0..2).find(|&s| pos[arms[s].first_vertex as usize + t] != Vec3::ZERO);
            let base = match side { Some(0) => [1.0, 0.25, 0.25], Some(_) => [0.3, 0.45, 1.0], None => [0.55, 0.55, 0.55] };
            let col = shade(base, nrm[i], Vec3::Z);
            canvas.tri(
                to(pos[i], 0, row, body_scale, centre, (0, 1)),
                to(pos[i + 1], 0, row, body_scale, centre, (0, 1)),
                to(pos[i + 2], 0, row, body_scale, centre, (0, 1)),
                col,
            );
        }
        // 2 and 3: the arms in arm space, both sides overlaid, with the origin's axes.
        let arm_scale = P as f32 * 0.45 / 40.0;
        for (panel, axes, toward) in [(1usize, (2usize, 1usize), Vec3::X), (2, (0, 2), Vec3::Y)] {
            for (s, arm) in arms.iter().enumerate() {
                for t in (0..n).step_by(3) {
                    let i = arm.first_vertex as usize + t;
                    if pos[i] == Vec3::ZERO && pos[i + 1] == Vec3::ZERO && pos[i + 2] == Vec3::ZERO {
                        continue;
                    }
                    if panel == 1 {
                        kept[s] += 1;
                        for k in 0..3 {
                            ext[s].0 = ext[s].0.min(pos[i + k]);
                            ext[s].1 = ext[s].1.max(pos[i + k]);
                        }
                    }
                    let base = if s == 0 { [1.0, 0.35, 0.35] } else { [0.4, 0.55, 1.0] };
                    let col = shade(base, nrm[i], toward);
                    canvas.tri(
                        to(pos[i], panel, row, arm_scale, Vec3::ZERO, axes),
                        to(pos[i + 1], panel, row, arm_scale, Vec3::ZERO, axes),
                        to(pos[i + 2], panel, row, arm_scale, Vec3::ZERO, axes),
                        col,
                    );
                }
            }
            let o = to(Vec3::ZERO, panel, row, arm_scale, Vec3::ZERO, axes);
            for k in -120..=120 {
                canvas.dot(o.x as i32 + k, o.y as i32, [90, 90, 90]);
                canvas.dot(o.x as i32, o.y as i32 + k, [90, 90, 90]);
            }
        }
        println!(
            "{name}: clip {}, {count} body-model draws, triangles kept right {} left {}, arm space bounds right {:.0?}..{:.0?} left {:.0?}..{:.0?}",
            sim.mario.anim.clip, kept[0], kept[1], ext[0].0, ext[0].1, ext[1].0, ext[1].1
        );
        assert!(kept[0] > 20 && kept[1] > 20, "{name}: an arm came out empty");
    }
    std::fs::write(&out, png::rgba((P * 3) as u32, (P * 4) as u32, &canvas.px)).unwrap();
    println!("wrote {out}: rows wait/run/jump/wet slide; columns body front (right arm red, left blue), arm space side (+z right), arm space top (+x right, +z up)");
}
