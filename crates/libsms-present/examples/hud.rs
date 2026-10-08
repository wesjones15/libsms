//! Draws FLUDD's water gauge (`hud.rs`, the game's HUD tank) at a row of tank levels, a
//! charging and a fully charged trigger nozzle, over a grey backdrop, to a PNG scaled up 3x:
//! `cargo run -p libsms-present --example hud -- out.png`.

use std::io::Write;

use libsms::NozzleKind;
use libsms_present::hud::{Gauge, Hud};

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "hud.png".into());
    let dir = libsms::assets::find(None).expect("no assets/");
    let mut hud = Hud::load(&dir).expect("the HUD's files (re-extract the assets)");
    let g = |fill, nozzle, pressure| Gauge {
        fill,
        nozzle,
        pressure,
    };
    let shots = [
        g(0.0, NozzleKind::Spray, None),
        g(0.25, NozzleKind::Spray, None),
        g(0.5, NozzleKind::Hover, None),
        g(0.75, NozzleKind::Rocket, Some((40.0, 100.0))),
        g(1.0, NozzleKind::Turbo, Some((100.0, 100.0))),
    ];
    const S: usize = 3;
    let (w, h) = (hud.image.width as usize, hud.image.height as usize);
    let (cw, ch) = (w * S * shots.len(), h * S);
    let mut canvas = vec![0u8; cw * ch * 4];
    for (i, shot) in shots.iter().enumerate() {
        // Several frames in, so the full charge is mid-flash.
        hud.update(shot, 11);
        for y in 0..h * S {
            for x in 0..w * S {
                let s = &hud.image.rgba[((y / S) * w + x / S) * 4..][..4];
                let a = s[3] as f32 / 255.0;
                let d = &mut canvas[(y * cw + i * w * S + x) * 4..][..4];
                for c in 0..3 {
                    d[c] = (s[c] as f32 * a + 96.0 * (1.0 - a)) as u8;
                }
                d[3] = 255;
            }
        }
    }
    std::fs::write(&out, png(cw as u32, ch as u32, &canvas)).unwrap();
    println!(
        "{out}: {} levels, gauge {w}x{h} at {:?} of 600x480",
        shots.len(),
        hud.origin
    );
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
