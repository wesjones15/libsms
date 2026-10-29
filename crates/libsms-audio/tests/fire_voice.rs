//! The burning Mario's lines (`fireDowning`, `fireJumping`: `MSD_SE_MV05_DAMAGE_FIRE_01`; the hit's
//! `MSD_SE_MA_DAMAGE_FIRE`; the lie-down's `MSD_SE_MV07_DAMAGE_REACT_01`) are in the banks and play.
//! Needs the user's extracted `assets/`; skipped without them.

use std::sync::Arc;

use glam::{Mat4, Vec3};
use libsms_assets::sound::SoundData;
use libsms_audio::{SoundSystem, VoiceSet};

#[test]
fn the_fire_yell_and_the_fire_sound_are_in_the_banks_and_play() {
    let Some(dir) = libsms::assets::find(None) else { eprintln!("SKIP: no assets/"); return };
    let data = Arc::new(SoundData::load(&dir).unwrap());
    for name in [
        "MSD_SE_MV05_DAMAGE_FIRE_01",
        "MSD_SE_MV05_DAMAGE_FIRE_02",
        "MSD_SE_MA_DAMAGE_FIRE",
        "MSD_SE_MV07_DAMAGE_REACT_01",
    ] {
        assert!(data.names.id(name).is_some(), "{name} is not in mSound.asn");
    }
    for cue in ["MSD_SE_MV05_DAMAGE_FIRE_01", "MSD_SE_MV05_DAMAGE_FIRE_02"] {
        let mut s = SoundSystem::new(data.clone());
        s.set_listener(Mat4::IDENTITY, Vec3::ZERO);
        s.set_voice_set(VoiceSet::Mario);
        assert!(s.mario_voice(cue).is_some(), "{cue} did not start");
        let mut energy = 0f64;
        let mut buf = vec![0f32; (32000 / 30) * 2];
        for _ in 0..60 {
            s.frame();
            s.render(&mut buf, 32000.0);
            energy += buf.iter().map(|x| (*x as f64).powi(2)).sum::<f64>();
        }
        assert!(energy > 0.0, "{cue} is silent");
    }
}
