//! Prints a texture-scroll animation's mode, length and each track's values over its length.
//!
//!     cargo run -p libsms-assets --example btkdump -- enemy/kagemario/kagemario_scroll.btk

use libsms_assets::btk::TexSrt;

fn main() {
    let rel = std::env::args().nth(1).unwrap_or_else(|| "enemy/kagemario/kagemario_scroll.btk".into());
    let dir = libsms::assets::find(None).expect("no assets/ folder");
    let data = std::fs::read(dir.join(&rel)).expect("read");
    let s = TexSrt::parse(&data).expect("parse");
    println!("{rel}: mode {} frames {} tracks {}", s.mode, s.frames, s.tracks.len());
    for t in &s.tracks {
        println!("  {} mtx {} center {:?}", t.material, t.matrix, t.center);
        let n = s.frames as i32;
        for f in [0, 1, n / 2, n - 1, n, n + 1] {
            let v = t.values(f as f32);
            println!("    f{f:4}: scale ({:.4},{:.4}) rot {:.4} trans ({:.4},{:.4})", v[0], v[3], v[7], v[2], v[5]);
        }
    }
}
