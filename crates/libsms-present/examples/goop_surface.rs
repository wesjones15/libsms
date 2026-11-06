//! Writes each goop kind's tile, the disc's stamps and Bowser Jr.'s sign to PNG:
//! `cargo run -p libsms-present --example goop_surface -- <out dir>`.
use libsms::goop::GoopKind;
use libsms_present::goop_surface::{GoopSurface, sign_mask, stamp_masks};

fn main() {
    let out = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "goop".into()));
    std::fs::create_dir_all(&out).unwrap();
    let dir = libsms::assets::find(None).expect("no assets/");
    let s = GoopSurface::load(&dir);
    for k in GoopKind::ALL {
        if let Some(t) = s.tile(k) {
            let px = t.image.rgba.as_chunks::<4>().0;
            let n = px.len() as f32;
            let avg: Vec<f32> = (0..4).map(|c| px.iter().map(|p| p[c] as f32).sum::<f32>() / n).collect();
            println!("{k:?}: {} {}x{} avg {avg:?}", t.layer, t.image.width, t.image.height);
            std::fs::write(out.join(format!("tile_{k:?}.png")), png(t.image.width, t.image.height, &t.image.rgba)).unwrap();
        } else {
            println!("{k:?}: no tile");
        }
    }
    let gray = |m: &libsms::goop::Mask| m.alpha.iter().flat_map(|&a| [a, a, a, 255]).collect::<Vec<u8>>();
    match stamp_masks(&dir) {
        Some((p, c)) => {
            for (name, m) in [("pollute", &p), ("clean", &c)] {
                let mn = m.alpha.iter().min().unwrap();
                let mx = m.alpha.iter().max().unwrap();
                println!("{name}: {} min {mn} max {mx}", m.size);
                std::fs::write(out.join(format!("{name}.png")), png(m.size as u32, m.size as u32, &gray(m))).unwrap();
            }
        }
        None => println!("no stamps"),
    }
    if let Some((m, half)) = sign_mask(&dir, 128) {
        println!("sign half {half}");
        std::fs::write(out.join("sign.png"), png(128, 128, &gray(&m))).unwrap();
    }
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
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |kind: &[u8], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut c = kind.to_vec();
        c.extend_from_slice(data);
        out.extend_from_slice(&c);
        out.extend_from_slice(&crc(&c).to_be_bytes());
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
    let mut c = 0xffff_ffffu32;
    for &b in d {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}
