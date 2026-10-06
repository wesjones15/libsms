//! Prints each texture's size and alpha coverage: `cargo run -p libsms-assets --example texstat -- <bmd>`.

fn main() {
    let rel = std::env::args().nth(1).expect("bmd path");
    let dir = libsms::assets::find(None).expect("no assets/");
    let m = libsms_assets::Bmd::parse(&std::fs::read(dir.join(&rel)).unwrap()).unwrap();
    for (i, t) in m.textures.iter().enumerate() {
        let px = &t.image.rgba;
        let n = (px.len() / 4).max(1);
        let opaque = px.chunks(4).filter(|p| p[3] > 128).count();
        let sum: u64 = px.chunks(4).map(|p| p[0] as u64 + p[1] as u64 + p[2] as u64).sum();
        println!(
            "{i} {} fmt {} {}x{} opaque {:.2} mean rgb {:.0} filter {}/{}",
            t.name,
            t.format,
            t.image.width,
            t.image.height,
            opaque as f32 / n as f32,
            sum as f32 / (3 * n) as f32,
            t.min_filter,
            t.mag_filter
        );
    }
}
