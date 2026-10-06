//! Loads the user's extracted files. Skips when `assets/` isn't there.

use libsms_assets::{Bmd, btp::TexPattern, paths};

fn walk(dir: &std::path::Path, ext: &str, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, ext, out);
        } else if p.extension().and_then(|s| s.to_str()) == Some(ext) {
            out.push(p);
        }
    }
}

#[test]
fn every_model_and_pattern_loads() {
    let Some(dir) = libsms::assets::find(None) else {
        eprintln!("skipped: no assets/");
        return;
    };
    let mut files = Vec::new();
    walk(&dir, "bmd", &mut files);
    assert!(files.len() > 20, "{files:?}");
    for f in &files {
        let m = Bmd::parse(&std::fs::read(f).unwrap())
            .unwrap_or_else(|e| panic!("{}: {e}", f.display()));
        let batches = m
            .batches()
            .unwrap_or_else(|e| panic!("{}: {e}", f.display()));
        assert!(
            batches.iter().any(|b| !b.indices.is_empty()),
            "{}",
            f.display()
        );
        for b in &batches {
            assert!(b.indices.iter().all(|&i| (i as usize) < b.vertices.len()));
        }
    }
    let mut files = Vec::new();
    walk(&dir, "btp", &mut files);
    for f in &files {
        TexPattern::parse(&std::fs::read(f).unwrap())
            .unwrap_or_else(|e| panic!("{}: {e}", f.display()));
    }
}

#[test]
fn mario_stands_about_160_tall_in_bind_pose() {
    let Some(dir) = libsms::assets::find(None) else {
        eprintln!("skipped: no assets/");
        return;
    };
    let m = Bmd::parse(&std::fs::read(dir.join(paths::BODY)).unwrap()).unwrap();
    let pose = libsms_assets::Pose::new(&m, glam::Mat4::IDENTITY, &m.bind_world());
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for b in m.batches().unwrap() {
        for v in &b.vertices {
            let y = pose.skin(v).0.y;
            lo = lo.min(y);
            hi = hi.max(y);
        }
    }
    assert!(lo > -5.0 && lo < 10.0, "feet at {lo}");
    assert!(hi > 120.0 && hi < 200.0, "top at {hi}");
}
