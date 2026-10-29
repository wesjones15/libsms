//! Draws FLUDD's four HUD nozzle pictures (`Hud::nozzle_icon`: Spray, Hover, Rocket, Turbo)
//! side by side over a dark backdrop, to a PNG scaled up 4x:
//! `cargo run -p libsms-present --example nozzle_icons -- out.png`.
//!
//! The radial menu uses exactly these pixels through `sms_view_nozzle_icon_*`; this is the
//! headless proof that it is the disc's own icon rather than a stand-in.

use std::io::Write;

use libsms_present::hud::Hud;

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "nozzle_icons.png".into());
    let dir = libsms::assets::find(None).expect("no assets/");
    let hud = Hud::load(&dir).expect("the HUD's files (re-extract the assets)");
    let icons: Vec<_> = (0..4)
        .map(|i| hud.nozzle_icon(i).expect("nozzle icon"))
        .collect();
    const S: usize = 4;
    let cell = icons
        .iter()
        .map(|i| i.width.max(i.height) as usize)
        .max()
        .unwrap();
    let gap = 8;
    let cw = (cell * S + gap * S) * icons.len();
    let ch = cell * S + gap * S;
    let mut canvas = vec![0u8; cw * ch * 4];
    for y in 0..ch {
        for x in 0..cw {
            let d = &mut canvas[(y * cw + x) * 4..][..4];
            d.copy_from_slice(&[24, 26, 34, 255]);
        }
    }
    for (i, icon) in icons.iter().enumerate() {
        let ox = i * (cell * S + gap * S) + gap * S / 2;
        let oy = gap * S / 2;
        for y in 0..icon.height as usize * S {
            for x in 0..icon.width as usize * S {
                let s = &icon.rgba[((y / S) * icon.width as usize + x / S) * 4..][..4];
                let a = s[3] as f32 / 255.0;
                let d = &mut canvas[((oy + y) * cw + ox + x) * 4..][..4];
                for c in 0..3 {
                    d[c] = (s[c] as f32 * a + d[c] as f32 * (1.0 - a)) as u8;
                }
                d[3] = 255;
            }
        }
    }
    std::fs::write(&out, png(cw as u32, ch as u32, &canvas)).unwrap();
    println!(
        "{out}: Spray, Hover, Rocket, Turbo at {}x{} each, {}x scale",
        cell, cell, S
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
