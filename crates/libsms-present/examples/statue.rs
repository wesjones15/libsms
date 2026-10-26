//! What the statue selection actually contains: per batch (shape, material, joint)
//! triangle counts and *skinned* bounds in the entrance's translated space, plus
//! headless renders of the selection so non-statue geometry is visible.
use glam::{Mat4, Vec3};
use libsms_assets::{Bmd, Image, Pose};
use std::io::Write;
use std::path::Path;

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "assets".into());
    let out = std::path::PathBuf::from(
        std::env::args()
            .nth(2)
            .unwrap_or_else(|| "output/statue".into()),
    );
    std::fs::create_dir_all(&out).unwrap();
    let bmd =
        Bmd::parse(&std::fs::read(Path::new(&dir).join("portal/plaza.bmd")).unwrap()).unwrap();
    let shift = Mat4::from_translation(Vec3::new(0., -300., -4941.47));
    let pose = Pose::new(&bmd, shift, &bmd.bind_world());
    let batches = libsms_present::plaza::statue_batches(&bmd).unwrap();
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    let (mut plo, mut phi) = (lo, hi);
    for (i, b) in batches.iter().enumerate() {
        let (mut l, mut h) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for v in &b.vertices {
            let p = pose.skin(v).0;
            l = l.min(p);
            h = h.max(p);
        }
        let name = &bmd.materials[b.material].name;
        println!(
            "batch {i:2} shape {:3} {name:22} tris {:5}  x {:.0}..{:.0}  y {:.0}..{:.0}  z {:.0}..{:.0}",
            b.shape,
            b.indices.len() / 3,
            l.x,
            h.x,
            l.y,
            h.y,
            l.z,
            h.z
        );
        lo = lo.min(l);
        hi = hi.max(h);
        if b.shape != 23 {
            plo = plo.min(l);
            phi = phi.max(h);
        }
    }
    println!(
        "pedestal box (shapes != 23) x {:.0}..{:.0}  y {:.0}..{:.0}  z {:.0}..{:.0}",
        plo.x, phi.x, plo.y, phi.y, plo.z, phi.z
    );
    println!(
        "whole selection           x {:.0}..{:.0}  y {:.0}..{:.0}  z {:.0}..{:.0}",
        lo.x, hi.x, lo.y, hi.y, lo.z, hi.z
    );

    let item = [(&bmd, Mat4::IDENTITY)];
    let save = |name: &str, image: Image| {
        std::fs::write(out.join(name), png(image.width, image.height, &image.rgba)).unwrap();
    };
    for (name, eye, target) in [
        (
            "front",
            Vec3::new(0., 1100., 7600.),
            Vec3::new(0., 900., 4300.),
        ),
        (
            "back",
            Vec3::new(0., 1100., 2000.),
            Vec3::new(0., 900., 4300.),
        ),
        (
            "left",
            Vec3::new(-4500., 1100., 4500.),
            Vec3::new(0., 900., 4500.),
        ),
        (
            "top",
            Vec3::new(300., 7000., 4600.),
            Vec3::new(0., 300., 4300.),
        ),
    ] {
        save(
            &format!("sel_{name}.png"),
            libsms_present::plaza::render_statue(&bmd, eye, target, 800, 700),
        );
    }
    save(
        "statue_only_front.png",
        libsms_present::plaza::render(
            &item,
            Vec3::new(0., 1100., 7600.),
            Vec3::new(0., 900., 4300.),
            800,
            700,
            |s| s == 23,
        ),
    );
    // The RoR2 pod camera: `MarioStatuePod.PlaceCamera` sits 5.5 m left, 3.8 m up and
    // 11.5 m in front of the statue, looking at the M gate at 1.8 m.
    save(
        "pod_camera.png",
        libsms_present::plaza::render_statue(
            &bmd,
            Vec3::new(-550., 380., 1150.) + Vec3::new(0., 300., 4941.47),
            Vec3::new(0., 180., 140.) + Vec3::new(0., 300., 4941.47),
            800,
            700,
        ),
    );
    println!("wrote {}", out.display());
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
