//! Draws the spin jump's swirl and the ground pound's smear (the game's own particles) to a
//! PNG with a small software rasteriser, for Mario or for the Luigi alt-skin:
//! `cargo run -p libsms-present --example smear -- out.png [luigi] [frames]`.
//!
//! Exact assets: `TMario::emitBlurSpinJump` (MarioParticle.cpp:564) emits `ms_m_blur2`
//! (`PARTICLE_MS_M_BLUR2`) and `ms_m_blur2sp`; `TMario::emitBlurHipDrop` (:546) emits
//! `ms_m_blur3` (`PARTICLE_MS_M_BLUR3`). Their textures are grey, so all their colour is the
//! JPA's prm/env: the smear's red prm and the spark's red table key are what Luigi turns green.

use std::io::Write;

use glam::{Mat4, Vec3};
use libsms_present::View;
use libsms_present::particles::blend;
use libsms_present::view::Skin;

const W: usize = 512;
const H: usize = 512;
const FRAME_TICKS: u64 = 4;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = args
        .first()
        .cloned()
        .unwrap_or_else(|| "smear.png".into());
    let luigi = args.iter().any(|a| a == "luigi");
    let smear_only = args.iter().any(|a| a == "smear");
    let spin_only = args.iter().any(|a| a == "spin");
    let frames: u64 = args.iter().find_map(|a| a.parse().ok()).unwrap_or(14);
    let dir = libsms::assets::find(None).expect("no assets/");
    let mut view = View::load(&dir).expect("Mario's models");
    view.set_skin(if luigi { Skin::Luigi } else { Skin::Mario });
    // Mario's centre, a little over a metre up: the effects are bound to getCenterAnmMtx.
    let at = Mat4::from_translation(Vec3::new(0.0, 58.0, 0.0));
    for frame in 0..frames {
        if !smear_only {
            view.fx.request(1, "ms_m_blur2", at);
            view.fx.request(2, "ms_m_blur2sp", at);
        }
        if !spin_only {
            view.fx.request(3, "ms_m_blur3", at);
        }
        view.fx.advance(frame * FRAME_TICKS);
    }
    view.fx.build(Vec3::X, Vec3::Y);
    if args.iter().any(|a| a == "debug") {
        for (i, b) in view.fx.batches.iter().enumerate() {
            let verts =
                &view.fx.vertices[b.first_vertex as usize..(b.first_vertex + b.vertex_count) as usize];
            let mut a = (f32::MAX, f32::MIN);
            let mut c = [0f32; 3];
            for q in verts {
                a = (a.0.min(q.color[3]), a.1.max(q.color[3]));
                for k in 0..3 {
                    c[k] = c[k].max(q.color[k]);
                }
            }
            eprintln!(
                "batch {i}: blend {} tex {} {} corners, alpha {:.2}..{:.2}, rgb max {c:?}",
                b.blend,
                b.texture,
                b.vertex_count,
                a.0,
                a.1
            );
        }
    }
    // A three-quarter view (the swirl is a ring, the smear runs along the motion), fitted to the
    // effects with a margin. `top` looks straight down, where the spin's ring is a ring.
    let top = args.iter().any(|a| a == "top");
    let turn = if top {
        Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2)
    } else {
        Mat4::from_rotation_y(0.6)
    };
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for v in &view.fx.vertices {
        let q = turn.transform_point3(v.pos);
        lo = lo.min(q);
        hi = hi.max(q);
    }
    let centre = (lo + hi) * 0.5;
    let extent = (hi - lo).max(Vec3::splat(8.0));
    let scale = (W.min(H) as f32 * 0.8) / extent.x.max(extent.y);
    eprintln!("effects bounds {lo:?}..{hi:?}, {scale:.2} px a unit");
    let to_px = |p: Vec3| -> (f32, f32) {
        let q = turn.transform_point3(p) - centre;
        (W as f32 / 2.0 + q.x * scale, H as f32 / 2.0 - q.y * scale)
    };
    let mut img = vec![[0.05f32, 0.06, 0.1]; W * H];
    let fx = &view.fx;
    for b in &fx.batches {
        let tex = &fx.textures[b.texture as usize];
        let verts =
            &fx.vertices[b.first_vertex as usize..(b.first_vertex + b.vertex_count) as usize];
        for q in verts.chunks_exact(4) {
            for tri in [[0, 1, 2], [0, 2, 3]] {
                raster(
                    &mut img,
                    [q[tri[0]], q[tri[1]], q[tri[2]]],
                    tex,
                    b.blend,
                    &to_px,
                );
            }
        }
    }
    let mut orange = 0u32;
    let mut green = 0u32;
    let mut blue = 0u32;
    for p in &img {
        let (r, g, b) = (p[0], p[1], p[2]);
        if r > 0.2 && g < r * 0.7 && b < 0.2 {
            orange += 1;
        } else if g > 0.2 && r < g * 0.7 && b < 0.3 {
            green += 1;
        } else if b > 0.2 && b > r * 1.5 && b > g * 1.2 {
            blue += 1;
        }
    }
    eprintln!(
        "{} batches, {} corners; orange {orange}, green {green}, blue {blue} pixels",
        fx.batches.len(),
        fx.vertices.len()
    );
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
            let s = tex.sample_nearest(u, v);
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
