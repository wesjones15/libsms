//! The male Pianta's speech lines (`MSD_SE_NPC_VM_MONTM_*`), which Olimar's Pianta skin speaks with
//! in libpikmin-present (`pianta::Voice`): each is in the banks and makes sound when played like any
//! sound effect. Needs the user's extracted `assets/`; skipped without them.

use std::sync::Arc;

use glam::{Mat4, Vec3};
use libsms_assets::sound::SoundData;
use libsms_audio::SoundSystem;

const CUES: [&str; 4] = [
    "MSD_SE_NPC_VM_MONTM_PROUD",
    "MSD_SE_NPC_VM_MONTM_INVITN",
    "MSD_SE_NPC_VM_MONTM_SURPRS",
    "MSD_SE_NPC_VM_MONTM_REGRET",
];

#[test]
fn the_pianta_lines_are_in_the_banks_and_make_sound() {
    let Some(dir) = libsms::assets::find(None) else { eprintln!("SKIP: no assets/"); return };
    let data = Arc::new(SoundData::load(&dir).expect("sound data"));
    for name in CUES {
        assert!(data.names.id(name).is_some(), "{name} is not in mSound.asn");
        let mut s = SoundSystem::new(data.clone());
        s.set_listener(Mat4::IDENTITY, Vec3::ZERO);
        assert!(s.play(name, Some(Vec3::new(0.0, 0.0, 100.0)), 1).is_some(), "{name} did not start");
        let mut energy = 0f64;
        let mut buf = vec![0f32; (32000 / 30) * 2];
        for _ in 0..60 {
            s.frame();
            s.render(&mut buf, 32000.0);
            energy += buf.iter().map(|x| (*x as f64).powi(2)).sum::<f64>();
        }
        assert!(energy > 1e-6, "{name} was silent");
    }
}
