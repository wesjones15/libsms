//! The sound effects the Candypop Buds borrow from Pikmin 2's own list (`SoundID.h`), through the engine that plays
//! Sunshine's (see `pikmin2_treasure_sounds.rs`). The Pom enemy has no sound of its own in the decomp (its sounds come from
//! its animation's sequence), so these are the Onion and sprout sounds that mean the same: opening, taking a Pikmin in,
//! closing, spitting seeds, the seeds landing, withering. Needs the extracted Pikmin 2 sound files; skipped without them.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::{Mat4, Vec3};
use libsms_assets::sound::SoundData;
use libsms_audio::SoundSystem;

fn assets() -> Option<PathBuf> {
    let dir = std::env::var_os("SMS_P2_ASSETS")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/p2"));
    dir.join("AudioRes/PSound.aaf").is_file().then_some(dir)
}

/// (name in `SoundID.h`, id, which moment of the bud it plays at).
const BUD_SOUNDS: &[(&str, u32, &str)] = &[
    ("PSSE_PK_SE_ONY_SAKU", 0x281B, "the petals open (the sprout blooming)"),
    ("PSSE_PK_SE_ONY_ENTER", 0x281D, "a Pikmin goes in"),
    ("PSSE_PK_SE_ONY_TSUBOMI", 0x281A, "the petals close (becoming a bud)"),
    ("PSSE_PK_SE_ONY_SEED_EJECT", 0x2816, "the seeds are spat out"),
    ("PSSE_PK_SE_ONY_SEED_GROUND", 0x2817, "a seed lands"),
    ("PSSE_PK_SE_ONY_KARERU", 0x281C, "the bud withers"),
    ("PSSE_PK_SE_ONY_HATSUGA", 0x2818, "a seed takes root"),
    ("PSSE_EV_ONYON_APPEAR", 0x3814, "a bud appears"),
];

fn render(data: &Arc<SoundData>, id: u32, seconds: usize) -> Option<(f64, f32, f32)> {
    let mut s = SoundSystem::new(data.clone());
    s.set_listener(Mat4::IDENTITY, Vec3::ZERO);
    s.play_se(id, Some(Vec3::new(0.0, 0.0, 100.0)), 1)?;
    let (mut energy, mut peak, mut live) = (0f64, 0f32, 0u32);
    let mut buf = vec![0f32; (32000 / 30) * 2];
    for _ in 0..seconds * 30 {
        s.frame();
        s.render(&mut buf, 32000.0);
        let e: f64 = buf.iter().map(|x| (*x as f64).powi(2)).sum();
        energy += e;
        peak = buf.iter().fold(peak, |p, x| p.max(x.abs()));
        if e > 1e-8 {
            live += 1;
        }
    }
    Some((energy, peak, live as f32 / 30.0))
}

#[test]
fn the_buds_borrowed_sounds_make_sound() {
    let Some(dir) = assets() else { eprintln!("SKIP: no Pikmin 2 sound files"); return };
    let data = Arc::new(SoundData::load_pikmin2(&dir).expect("sound data"));
    let mut silent = Vec::new();
    for (name, id, what) in BUD_SOUNDS {
        if let Some(named) = data.names.id(name) {
            assert_eq!(named & !0x800, *id & !0x800, "{name}");
        } else {
            eprintln!("{name} is not in PSound.asn");
        }
        match render(&data, *id, 3) {
            Some((energy, peak, live)) => {
                eprintln!("{name} {id:#x}: energy {energy:.4} peak {peak:.3} sound for {live:.2} s  ({what})");
                if !(energy > 1.0 && peak > 0.02) {
                    silent.push(*name);
                }
            }
            None => {
                eprintln!("{name} {id:#x}: not started");
                silent.push(*name);
            }
        }
    }
    assert!(silent.is_empty(), "silent: {silent:?}");
}
