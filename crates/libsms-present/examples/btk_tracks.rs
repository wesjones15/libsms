//! Lists a btk's tracks and the matrices it moves on a model: `-- <btk> <bmd>`.
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let dir = libsms::assets::find(None).expect("no assets/");
    let srt = libsms_assets::btk::TexSrt::parse(&std::fs::read(dir.join(&a[0])).unwrap()).unwrap();
    let mut bmd = libsms_assets::Bmd::parse(&std::fs::read(dir.join(&a[1])).unwrap()).unwrap();
    println!("frames {} mode {}", srt.frames, srt.mode);
    for t in &srt.tracks {
        println!("track {} matrix {}", t.material, t.matrix);
    }
    for f in [0.0, srt.frames * 0.5, srt.frames - 1.0] {
        srt.apply(&mut bmd, f);
        for m in &bmd.materials {
            let ts: Vec<_> = m.tex_mtx.iter().map(|t| t.as_ref().map(|t| (t.translation, t.scale, t.rotation))).collect();
            println!("f {f} {} gens {:?} mtx {:?}", m.name, m.tex_gens.iter().map(|g| g.matrix).collect::<Vec<_>>(), &ts[..3]);
        }
    }
}
