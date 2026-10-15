//! Prints a `.bti` picture's channel histograms: `cargo run -p libsms-assets --example btistat -- in.bti`.

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let d = std::fs::read(&a[0]).expect("read");
    let t = libsms_assets::texture::read_bti(&d, a[0].clone()).expect("bti");
    for (name, ch) in [("r", 0), ("g", 1), ("b", 2), ("a", 3)] {
        let mut hist = [0u32; 8];
        for p in t.image.rgba.chunks_exact(4) {
            hist[(p[ch] as usize * 8 / 256).min(7)] += 1;
        }
        println!("{name}: {hist:?}");
    }
}
