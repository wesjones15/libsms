//! Samples a clip's joints at a frame: `cargo run -p libsms-assets --example clip -- <bck> <bmd> [frame]`.

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let dir = libsms::assets::find(None).expect("no assets/");
    let clip = libsms::Clip::parse(&std::fs::read(dir.join(&a[0])).unwrap()).expect("clip");
    let m = libsms_assets::Bmd::parse(&std::fs::read(dir.join(&a[1])).unwrap()).unwrap();
    let f: f32 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    for (j, jt) in m.joints.iter().enumerate() {
        let s = clip.sample(j, f);
        println!(
            "{j} {} bind rot {:.3} t {:.1} | clip {:?}",
            jt.name,
            jt.rotation,
            jt.translation,
            s.map(|s| (s.scale, s.rot, s.trans))
        );
    }
}
