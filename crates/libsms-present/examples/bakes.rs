//! Writes every baked texture to PNG and lists each model's batches, to eyeball the TEV bake:
//! `cargo run -p libsms-present --example bakes -- <out dir> [shadow|shine]` (`DUMP=1`: the materials too).

use std::io::Write;

use libsms_present::{Look, View};

fn main() {
    let out = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "bakes".into()));
    std::fs::create_dir_all(&out).unwrap();
    let dir = libsms::assets::find(None).expect("no assets/");
    let look = match std::env::args().nth(2).as_deref() {
        Some("shadow") => Look::Shadow,
        Some("shine") => Look::Shine,
        _ => Look::Mario,
    };
    let mut v = View::load_look(&dir, look).expect("models");
    // `SKINS=1` also bakes the same-rig skins' bodies (Piantissimo).
    if std::env::var_os("SKINS").is_some() {
        for skin in libsms_present::view::Skin::ALL {
            println!("skin {skin:?}: loaded {}", v.load_skin(skin, None));
        }
    }
    for (mi, m) in v.models.iter().enumerate() {
        for (bi, b) in m.batches.iter().enumerate() {
            let mat = &m.bmd.materials[b.material];
            println!(
                "model {mi} batch {bi} shape {} material {} ({} stages): texture {} blend {} lit {} colour {:?}",
                b.shape,
                mat.name,
                mat.stages.len(),
                m.batch_texture[bi],
                m.batch_blend[bi],
                m.batch_lit[bi],
                m.batch_color_of(bi),
            );
        }
    }
    if std::env::var("DUMP").is_ok() {
        if look == Look::Shine {
            for m in &v.models[0].bmd.materials {
                dump(m);
            }
        } else {
            dump(&v.models[5].bmd.materials[0]);
            dump(&v.models[6].bmd.materials[0]);
        }
    }
    for (i, t) in v.textures.list.iter().enumerate() {
        let path = out.join(format!("bake{i:03}.png"));
        std::fs::write(&path, png(t.width, t.height, &t.rgba)).unwrap();
    }
    println!(
        "{} textures written to {}",
        v.textures.list.len(),
        out.display()
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
