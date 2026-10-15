//! Draws FLUDD's nozzle effects (the game's own particles, `particles.rs`) while spraying or hovering,
//! to a PNG with the small software rasteriser of `flames.rs`, from the side, Mario's skinned body in grey
//! for scale: `cargo run -p libsms-present --example nozzle -- out.png [1 spray|2 hover] [seconds]`.

use std::io::Write;

use glam::Vec3;
use libsms::{Event, Host, NozzleKind, Pad, Sim};
use libsms_present::View;
use libsms_present::particles::blend;

const W: usize = 512;
const H: usize = 512;

/// Flat ground, and the sim's events kept for the view.
struct Collect(Vec<Event>);

impl Host for Collect {
    fn ground(&self, p: Vec3) -> Option<libsms::GroundHit> {
        Host::ground(&libsms::FlatHost::default(), p)
    }
    fn event(&mut self, e: Event) {
        self.0.push(e);
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().unwrap_or_else(|| "nozzle.png".into());
    let which: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1);
    let secs: f32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1.5);
    let dir = libsms::assets::find(None).expect("no assets/");
    let a = libsms::Assets::load(&dir);
    let mut view = View::load(&dir).expect("Mario's models");
    let mut sim = Sim::new(a.params, a.anim_table, Vec3::ZERO, 0);
    sim.mario.give_fludd();
    let nozzle = if which == 2 { NozzleKind::Hover } else { NozzleKind::Spray };
    {
        let p = sim.mario.params.clone();
        let f = sim.mario.fludd.as_mut().unwrap();
        f.change_nozzle(nozzle, false, &p);
        f.refill(&p);
    }
    let mut host = Collect(Vec::new());
    let dt = 1.0 / 120.0;
    let mut pad = Pad { r_analog: 200, ..Pad::default() };
    if nozzle == NozzleKind::Hover {
        pad.a = true;
    }
    for t in 0..(secs * 120.0) as u32 {
        if t == 20 {
            pad.a = false;
        }
        sim.advance_observed(&mut host, &pad, 0, dt, |mario, h| {
            let events = std::mem::take(&mut h.0);
            view.events(&events, mario);
        });
        view.set_camera(Vec3::X, Vec3::Y);
        view.update(&mut sim, dt);
    }
    let fx = &view.fx;
    eprintln!(
        "{nozzle:?} after {secs} s: {} batches, {} corners, {} textures; Mario at {:?}",
        fx.batches.len(),
        fx.vertices.len(),
        fx.textures.len(),
        sim.mario.pos
    );
    // Side view: x right, y up, centred a metre above Mario's feet; 4 units a pixel.
    let centre = sim.mario.pos + Vec3::new(0.0, 100.0, 0.0);
    let scale: f32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(0.9);
    let to_px = |p: Vec3| -> (f32, f32) {
        (
            W as f32 / 2.0 + (p.x - centre.x) * scale,
            H as f32 / 2.0 - (p.y - centre.y) * scale,
        )
    };
    let mut img = vec![[0.05f32, 0.06, 0.1]; W * H];
    for d in &view.draws {
        let n = view.models[d.model as usize].vertex_count;
        for v in &view.positions[d.first_vertex as usize..d.first_vertex as usize + n] {
            let p = d.pos + d.rot * *v;
            let (x, y) = to_px(p);
            if (0.0..W as f32).contains(&x) && (0.0..H as f32).contains(&y) {
                img[y as usize * W + x as usize] = [0.45, 0.45, 0.5];
            }
        }
    }
    for b in &fx.batches {
        let tex = &fx.textures[b.texture as usize];
        let verts =
            &fx.vertices[b.first_vertex as usize..(b.first_vertex + b.vertex_count) as usize];
        for q in verts.chunks_exact(4) {
            for tri in [[0, 1, 2], [0, 2, 3]] {
                raster(&mut img, [q[tri[0]], q[tri[1]], q[tri[2]]], tex, b.blend, &to_px);
            }
        }
    }
    let rgba: Vec<u8> = img
        .iter()
        .flat_map(|c| {
            [c[0], c[1], c[2]]
                .map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8)
                .into_iter()
                .chain([255])
        })
        .collect();
    std::fs::write(&out, png(W as u32, H as u32, &rgba)).unwrap();
    eprintln!("wrote {out}");
}

fn raster(
    img: &mut [[f32; 3]],
    t: [libsms_present::particles::FxVertex; 3],
    tex: &libsms_present::particles::FxTexture,
    mode: u32,
    to_px: &dyn Fn(Vec3) -> (f32, f32),
) {
    let p: Vec<(f32, f32)> = t.iter().map(|v| to_px(v.pos)).collect();
    let area = (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[2].0 - p[0].0) * (p[1].1 - p[0].1);
    if area.abs() < 1e-6 {
        return;
    }
    let (x0, x1) = (
        p.iter().map(|q| q.0).fold(f32::MAX, f32::min).max(0.0) as usize,
        p.iter()
            .map(|q| q.0)
            .fold(f32::MIN, f32::max)
            .min(W as f32 - 1.0) as usize,
    );
    let (y0, y1) = (
        p.iter().map(|q| q.1).fold(f32::MAX, f32::min).max(0.0) as usize,
        p.iter()
            .map(|q| q.1)
            .fold(f32::MIN, f32::max)
            .min(H as f32 - 1.0) as usize,
    );
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let w0 = ((p[1].0 - px) * (p[2].1 - py) - (p[2].0 - px) * (p[1].1 - py)) / area;
            let w1 = ((p[2].0 - px) * (p[0].1 - py) - (p[0].0 - px) * (p[2].1 - py)) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            let u = w0 * t[0].u + w1 * t[1].u + w2 * t[2].u;
            let v = w0 * t[0].v + w1 * t[1].v + w2 * t[2].v;
            let c: Vec<f32> = (0..4)
                .map(|k| w0 * t[0].color[k] + w1 * t[1].color[k] + w2 * t[2].color[k])
                .collect();
            let wrap = |f: f32, n: u32| -> usize {
                let i = (f * n as f32).floor() as i64;
                if tex.repeat {
                    i.rem_euclid(n as i64) as usize
                } else {
                    i.clamp(0, n as i64 - 1) as usize
                }
            };
            let o = (wrap(v, tex.height) * tex.width as usize + wrap(u, tex.width)) * 4;
            let s = &tex.rgba[o..o + 4];
            let a = s[3] as f32 / 255.0 * c[3];
            let px = &mut img[y * W + x];
            for k in 0..3 {
                let src = s[k] as f32 / 255.0 * c[k];
                px[k] = if mode == blend::ADDITIVE {
                    px[k] + src * a
                } else {
                    px[k] * (1.0 - a) + src * a
                };
            }
        }
    }
}

/// A minimal PNG: RGBA8, zlib "stored" blocks (no compression).
fn png(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let mut raw = Vec::new();
    for row in rgba.chunks_exact(w as usize * 4) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (i, b) in blocks.iter().enumerate() {
        z.push((i + 1 == blocks.len()) as u8);
        z.extend_from_slice(&(b.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(b.len() as u16)).to_le_bytes());
        z.extend_from_slice(b);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&(b << 16 | a).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut chunk = |tag: &[u8], data: &[u8]| {
        out.write_all(&(data.len() as u32).to_be_bytes()).unwrap();
        let mut c = tag.to_vec();
        c.extend_from_slice(data);
        out.write_all(&c).unwrap();
        out.write_all(&crc(&c).to_be_bytes()).unwrap();
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    out
}

fn crc(d: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in d {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}
