//! Draws the losing screens (`screenfx.rs`) over a grey checker "scene" to a PNG: "TOO BAD!" at a
//! few frames, then GAME OVER's: `cargo run -p libsms-present --example screenfx -- out.png`.

use std::io::Write;

use libsms_present::screenfx::{Frame, Kind, ScreenFx, FPS, SCREEN};

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "screenfx.png".into());
    let dir = libsms::assets::find(None).expect("no assets/");
    let mut fx = ScreenFx::load(&dir).expect("the pictures");
    let mut shots: Vec<Vec<u8>> = Vec::new();
    let (w, h) = (SCREEN[0] as usize / 2, SCREEN[1] as usize / 2);
    for (kind, frames) in [
        (Kind::TooBad, vec![25u32, 40, 70, 110, 200]),
        (Kind::GameOver, vec![20, 40, 60, 85, 110, 180]),
    ] {
        fx.start(kind);
        let mut at = 0;
        for target in frames {
            while at < target {
                fx.step(1.0 / FPS);
                at += 1;
            }
            shots.push(render(&fx, fx.frame(), w, h));
        }
    }
    let per_row = 3;
    let rows = shots.len().div_ceil(per_row);
    let (cw, ch) = (w * per_row, h * rows);
    let mut canvas = vec![255u8; cw * ch * 4];
    for (i, s) in shots.iter().enumerate() {
        let (ox, oy) = ((i % per_row) * w, (i / per_row) * h);
        for y in 0..h {
            canvas[((oy + y) * cw + ox) * 4..][..w * 4].copy_from_slice(&s[y * w * 4..][..w * 4]);
        }
    }
    std::fs::write(&out, png(cw as u32, ch as u32, &canvas)).unwrap();
    println!("{out}: {} frames", shots.len());
}

/// A bright "scene" (a ramp and a checker) with the frame over it, at half the screen's size.
fn render(fx: &ScreenFx, f: &Frame, w: usize, h: usize) -> Vec<u8> {
    let mut px = vec![0f32; w * h * 3];
    for y in 0..h {
        for x in 0..w {
            let c = if (x / 20 + y / 20) % 2 == 0 { 0.85 } else { 0.6 };
            let t = y as f32 / h as f32;
            px[(y * w + x) * 3..][..3].copy_from_slice(&[c * (1.0 - t * 0.3), c * 0.8, c]);
        }
    }
    let blend = |px: &mut [f32], i: usize, rgb: [f32; 3], a: f32| {
        for k in 0..3 {
            px[i * 3 + k] = rgb[k] * a + px[i * 3 + k] * (1.0 - a);
        }
    };
    for i in 0..w * h {
        blend(&mut px, i, [0.0; 3], f.fill);
    }
    if f.card.color[3] > 0.0 {
        let sx = w as f32 / SCREEN[0];
        let r = f.card.rect;
        for y in (r[1] * sx) as usize..((r[1] + r[3]) * sx) as usize {
            for x in (r[0] * sx) as usize..((r[0] + r[2]) * sx) as usize {
                blend(&mut px, y * w + x, [f.card.color[0], f.card.color[1], f.card.color[2]], f.card.color[3]);
            }
        }
    }
    if let Some(o) = &f.overlay {
        for y in 0..h {
            for x in 0..w {
                let (ox, oy) = (x * o.width as usize / w, y * o.height as usize / h);
                let a = o.rgba[(oy * o.width as usize + ox) * 4 + 3] as f32 / 255.0;
                blend(&mut px, y * w + x, [0.0; 3], a);
            }
        }
    }
    let scale = w as f32 / SCREEN[0];
    for s in &f.sprites {
        let Some(img) = fx.picture(s.picture) else { continue };
        let (sw, sh) = (s.size[0] * scale, s.size[1] * scale);
        let (sin, cos) = s.rotation.sin_cos();
        let (cx, cy) = (s.center[0] * scale, s.center[1] * scale);
        let r = (sw.max(sh) * 1.5) as i32;
        for dy in -r..=r {
            for dx in -r..=r {
                let (x, y) = (cx as i32 + dx, cy as i32 + dy);
                if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
                    continue;
                }
                let (lx, ly) = (dx as f32 * cos + dy as f32 * sin, -dx as f32 * sin + dy as f32 * cos);
                let (u, v) = (lx / sw + 0.5, ly / sh + 0.5);
                if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                    continue;
                }
                let t = &img.rgba[((v * img.height as f32) as usize * img.width as usize + (u * img.width as f32) as usize) * 4..][..4];
                let a = t[3] as f32 / 255.0 * s.color[3];
                let rgb = [0, 1, 2].map(|k| t[k] as f32 / 255.0 * s.color[k]);
                blend(&mut px, y as usize * w + x as usize, rgb, a);
            }
        }
    }
    px.chunks_exact(3)
        .flat_map(|c| [(c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8, 255])
        .collect()
}

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

#[allow(dead_code)]
fn dump(m: &libsms_assets::bmd::Material) {
    println!(
        "{} chans {:?} mat {:?} amb {:?} konst {:?} regs {:?} swap {:?} gens {:?}",
        m.name,
        m.chans,
        m.mat_colors,
        m.amb_colors,
        m.konst,
        m.tev_colors,
        m.swap_tables,
        m.tex_gens
    );
    for s in &m.stages {
        println!("  {:?}", s);
    }
}
