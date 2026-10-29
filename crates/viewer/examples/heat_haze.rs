//! Headless audit / proof for Map/Shimmer.cpp::TShimmer (no window).
use std::path::Path;
fn main() {
    let iso = std::env::args().nth(1).expect("Sunshine ISO path");
    let disc = libsms::extract::Disc::open(Path::new(&iso)).unwrap();
    let path = disc
        .paths_under("")
        .into_iter()
        .find(|p| p.ends_with("scene/dolpic1.szs"))
        .unwrap();
    let bytes = disc.read_file(&path).unwrap();
    let bytes = libsms::extract::yaz0::decompress(&bytes).unwrap();
    let arc = libsms::extract::rarc::Archive::parse(bytes).unwrap();
    let cache = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/heat_haze");
    std::fs::create_dir_all(&cache).unwrap();
    for name in ["shimmerlow.bmd", "shimmerlow.btk"] {
        std::fs::write(
            cache.join(name),
            arc.file(&format!("mapobj/{name}")).unwrap(),
        )
        .unwrap();
    }
    let bmd = libsms_assets::Bmd::parse(arc.file("mapobj/shimmerlow.bmd").unwrap()).unwrap();
    let batches = bmd.batches().unwrap();
    for b in &batches {
        let (lo, hi) = b.vertices.iter().fold(
            (
                glam::Vec3::splat(f32::INFINITY),
                glam::Vec3::splat(f32::NEG_INFINITY),
            ),
            |(lo, hi), v| (lo.min(v.position), hi.max(v.position)),
        );
        println!(
            "shape {} material {} bounds {lo:?}..{hi:?}, vertices {}",
            b.shape,
            b.material,
            b.vertices.len()
        );
        println!(
            "UVs {:?}",
            b.vertices.iter().map(|v| v.uv[0]).collect::<Vec<_>>()
        );
    }
    for m in &bmd.materials {
        println!(
            "material {} textures {:?}, indirect {:?}, texgens {:?}, texmtx {:?}",
            m.name, m.textures, m.indirect, m.tex_gens, m.tex_mtx
        );
    }
    for t in &bmd.textures {
        println!(
            "texture {} {}x{} {:?}",
            t.name, t.image.width, t.image.height, t.format
        );
    }
    let btk =
        libsms_assets::btk::TexSrt::parse(arc.file("mapobj/shimmerlow.btk").unwrap()).unwrap();
    println!("BTK {btk:?}");
    for path in arc.paths().filter(|p| {
        p.ends_with(".btk")
            || p.to_lowercase().contains("shimmer")
            || p.to_lowercase().contains("yura")
    }) {
        println!("{path}");
    }
    let data = arc.file("map/scene.bin").unwrap();
    for i in 0..data.len().saturating_sub(80) {
        let size = u32::from_be_bytes(data[i..i + 4].try_into().unwrap()) as usize;
        let n = u16::from_be_bytes(data[i + 6..i + 8].try_into().unwrap()) as usize;
        if size < 60
            || size > 250
            || i + size > data.len()
            || n != 7
            || data.get(i + 8..i + 15) != Some(b"Shimmer")
        {
            continue;
        }
        let at = i + 15;
        let len = u16::from_be_bytes(data[at + 2..at + 4].try_into().unwrap()) as usize;
        let start = at + 4 + len;
        let values: Vec<_> = (0..9)
            .map(|j| f32::from_be_bytes(data[start + j * 4..start + j * 4 + 4].try_into().unwrap()))
            .collect();
        println!(
            "Shimmer record {i} size {size}, transform {values:?}, tail {:?}",
            &data[start + 36..i + size]
        );
    }
    let dir = cache.parent().unwrap();
    let mut effect = libsms_present::heat_haze::HeatHaze::load(dir).unwrap();
    let (w, h) = (1200usize, 600usize);
    let background = |x: f32, y: f32| -> [u8; 4] {
        let grid = (x as i32 / 22 + y as i32 / 22) % 2 == 0;
        if grid {
            [202, 211, 225, 255]
        } else {
            [62, 89, 120, 255]
        }
    };
    let mut image = vec![0u8; w * h * 4];
    for row in 0..2 {
        for col in 0..3 {
            for y in 0..300 {
                for x in 0..400 {
                    let uv = glam::Vec2::new(x as f32 / 400., y as f32 / 300.);
                    let near = (150..250).contains(&x) && (90..270).contains(&y);
                    let offset = if row == 1 && !near {
                        effect.offset(uv, col as f32 * 0.25)
                    } else {
                        glam::Vec2::ZERO
                    };
                    let color = if near {
                        [210, 74, 40, 255]
                    } else {
                        background(x as f32 + offset.x * 400., y as f32 + offset.y * 300.)
                    };
                    let i = ((row * 300 + y) * w + col * 400 + x) * 4;
                    image[i..i + 4].copy_from_slice(&color);
                }
            }
        }
    }
    let out = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "target/heat-haze-shimmerlow.png".into());
    let image = image.chunks_exact(w * 4).rev().flatten().copied().collect();
    macroquad::prelude::Image {
        bytes: image,
        width: w as u16,
        height: h as u16,
    }
    .export_png(&out);
    std::fs::write(format!("{out}.txt"),"TShimmer / ShimmerLow.bmd + ShimmerLow.btk / B_kagero_s. Source: Map/Shimmer.cpp::load,loadAfter,perform. Texel-to-UV conversion: MarioUtil/ScreenUtil.cpp::TScreenTexture::load, System/Resolution.cpp (320x224 copy).\nTop: disabled. Bottom: animated source indirect refraction at 0,0.25,0.5 seconds; orange foreground remains sharp (depth 10m vs sheet 24.25m).\n").unwrap();
    println!("Headless capture: {out}");
}
