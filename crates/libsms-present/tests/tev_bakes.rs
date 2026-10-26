//! Every baked material of the user's extracted models, pinned pixel for pixel before the TEV
//! arithmetic moved to libcommon (skipped when `assets/` isn't there).

use libsms_present::{Look, View};

fn fnv(h: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(h, |h, &b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

#[test]
fn baked_textures_match_baseline() {
    let Some(dir) = libsms::assets::find(None) else {
        eprintln!("SKIP: no extracted assets");
        return;
    };
    let mut hash = 0xcbf29ce484222325u64;
    let mut count = 0;
    for look in [Look::Mario, Look::Shadow] {
        let view = View::load_look(&dir, look).expect("models load");
        for t in &view.textures.list {
            hash = fnv(hash, &t.width.to_le_bytes());
            hash = fnv(hash, &t.height.to_le_bytes());
            hash = fnv(hash, &t.wrap);
            hash = fnv(hash, &t.rgba);
            count += 1;
        }
    }
    println!("{count} textures, hash {hash:#x}");
    assert!(count > 0);
    assert_eq!((count, hash), BASELINE);
}

// Recorded from the evaluator before it moved to libcommon::tev.
const BASELINE: (usize, u64) = (58, 0xa3a709833753420a);
