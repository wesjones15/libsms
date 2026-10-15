//! Writes a `.bti` picture over a checkerboard to a PNG: `cargo run -p libsms-assets --example btipng -- in.bti out.png`.

use std::io::Write;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let d = std::fs::read(&a[0]).expect("read");
    let t = libsms_assets::texture::read_bti(&d, a[0].clone()).expect("bti");
    let (w, h) = (t.image.width as usize, t.image.height as usize);
    println!("{w}x{h} format {} wrap {:?}/{:?}", t.format, t.wrap_s, t.wrap_t);
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let p = &t.image.rgba[(y * w + x) * 4..][..4];
            let back = if (x / 8 + y / 8) % 2 == 0 { 200.0 } else { 120.0 };
            let al = p[3] as f32 / 255.0;
            for c in 0..3 {
                out.push((p[c] as f32 * al + back * (1.0 - al)) as u8);
            }
            out.push(255);
        }
    }
    std::fs::write(&a[1], png(w as u32, h as u32, &out)).unwrap();
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
