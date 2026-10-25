//! Headless viewer proof of the exact 05_gate01 material and Plaza statue.
//! cargo run --release -p viewer --example portal -- assets ../output/mario-portal
//! Source: sms-main/src/MoveBG/ModelGate.cpp::loadAfter, perform.
use glam::Vec3;
use libsms_assets::Bmd;
use libsms_present::portal::Portal;
use std::path::PathBuf;

fn save(path: &std::path::Path, w: usize, h: usize, rgba: Vec<u8>) {
    // macroquad exports framebuffer rows bottom-first; our CPU images are top-first.
    let rgba = rgba.chunks_exact(w * 4).rev().flatten().copied().collect();
    macroquad::texture::Image {
        bytes: rgba,
        width: w as u16,
        height: h as u16,
    }
    .export_png(path.to_str().unwrap());
}
fn main() {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().unwrap_or_else(|| "assets".into()));
    let out = PathBuf::from(
        args.next()
            .unwrap_or_else(|| "../output/mario-portal".into()),
    );
    std::fs::create_dir_all(&out).unwrap();
    let plaza = Bmd::parse(&std::fs::read(dir.join("portal/plaza.bmd")).unwrap()).unwrap();
    let batches = plaza.batches().unwrap();
    for b in &batches {
        if plaza.materials[b.material].name.contains("dokan") {
            let min = b
                .vertices
                .iter()
                .map(|v| v.position)
                .fold(Vec3::splat(f32::INFINITY), Vec3::min);
            let max = b
                .vertices
                .iter()
                .map(|v| v.position)
                .fold(Vec3::splat(f32::NEG_INFINITY), Vec3::max);
            println!("PIPE {} {min:?} {max:?}", plaza.materials[b.material].name);
        }
        if plaza.materials[b.material].name.contains("king") {
            let min = b
                .vertices
                .iter()
                .map(|v| v.position)
                .fold(Vec3::splat(f32::INFINITY), Vec3::min);
            let max = b
                .vertices
                .iter()
                .map(|v| v.position)
                .fold(Vec3::splat(f32::NEG_INFINITY), Vec3::max);
            println!(
                "STATUE shape {} material {} min {min:?} max {max:?}",
                b.shape, plaza.materials[b.material].name
            );
        }
    }
    // The scene's JellyGate/Gate actor is at (0.02115,300,4941.47), facing +Z.
    let still = libsms_present::plaza::plaza_still(&dir, 256).unwrap();
    save(
        &out.join("delfino-plaza-still.png"),
        256,
        256,
        still.rgba.clone(),
    );
    let detail = libsms_present::plaza::plaza_still(&dir, 768).unwrap();
    save(&out.join("delfino-plaza-detail.png"), 768, 768, detail.rgba);
    let pipe = libsms_present::plaza::render(
        &[(&plaza, glam::Mat4::IDENTITY)],
        Vec3::new(700., 3000., 3000.),
        Vec3::new(0., 2500., 1100.),
        512,
        512,
        |_| true,
    );
    save(&out.join("rooftop-pipe-check.png"), 512, 512, pipe.rgba);
    let statue = libsms_present::plaza::render_statue(
        &plaza,
        Vec3::new(1700., 1400., 7300.),
        Vec3::new(0., 1100., 4500.),
        512,
        640,
    );
    save(&out.join("plaza-statue-shape23.png"), 512, 640, statue.rgba);
    let mut portal = Portal::load(&dir).unwrap();
    portal.set_interior(&still);
    let frames = [0., 30., 60., 90.];
    let size = 256;
    let mut sheet = vec![255u8; size * 4 * size * 3 * 4];
    for (row, active) in [0., 0.5, 1.].into_iter().enumerate() {
        for (col, frame) in frames.into_iter().enumerate() {
            let image = portal.image(frame, active);
            println!(
                "frame {frame} active {active} {}x{} maxalpha {}",
                image.width,
                image.height,
                image.rgba.chunks_exact(4).map(|p| p[3]).max().unwrap()
            );
            for y in 0..size {
                for x in 0..size {
                    // The gate's authored UVs run upward from its feet: place the
                    // material on that exact quad, rather than displaying UV space.
                    let i = ((size - 1 - y) * size + x) * 4;
                    let c = &image.rgba[i..i + 4];
                    let a = c[3] as f32 / 255.;
                    let j = ((row * size + y) * size * 4 + col * size + x) * 4;
                    for k in 0..3 {
                        sheet[j + k] = (c[k] as f32 * a + [25., 29., 40.][k] * (1. - a)) as u8;
                    }
                }
            }
            let posed = image
                .rgba
                .chunks_exact(size * 4)
                .rev()
                .flatten()
                .copied()
                .collect();
            save(
                &out.join(format!("05_gate01_active{active}_f{frame}.png")),
                image.width as usize,
                image.height as usize,
                posed,
            );
        }
    }
    save(
        &out.join("05_gate01_ModelGate_frames.png"),
        size * 4,
        size * 3,
        sheet,
    );
    std::fs::write(out.join("SOURCE.txt"),"Exact asset: dolpic1.szs:map/map/gate/05_gate01.bmd + .btk + .brk.\nSource: sms-main/src/MoveBG/ModelGate.cpp::loadAfter / perform.\nRows: BRK activation 0, .5, 1. Columns: BTK frames 0,30,60,90.\nIndirect map P_ms_indwp1_ia, mask P_gate_msk_m2.\nStatue: dolpic1.szs:map/map/map.bmd shape23, _king_storn_1.\nInterior: still rendered from the disc's Delfino Plaza map; no video.\n").unwrap();
}
