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

#[test]
fn fire_cries_finish_before_recovery_or_movement_voices_replace_them() {
    let Some(dir) = libsms::assets::find(None) else { eprintln!("SKIP: no assets/"); return; };
    let data = Arc::new(SoundData::load(&dir).unwrap());
    for cue in ["MSD_SE_MV05_DAMAGE_FIRE_01", "MSD_SE_MV05_DAMAGE_FIRE_02"] {
        let mut clean = SoundSystem::new(data.clone());
        let mut interrupted = SoundSystem::new(data.clone());
        let a = clean.mario_voice(cue).unwrap();
        let b = interrupted.mario_voice(cue).unwrap();
        let mut reference = vec![0f32; (32000 / 30) * 2];
        let mut actual = reference.clone();
        let mut blocked = 0;
        for frame in 0..180 {
            if frame >= 8 && clean.is_playing(a) {
                let other = match frame % 3 {
                    0 => "MSD_SE_MV07_DAMAGE_REACT_01",
                    1 => "MSD_SE_MV21_JUMP_SMALL_01",
                    _ => "MSD_SE_MV13_ACTION_SMALL_01",
                };
                interrupted.mario_voice(other);
                blocked += 1;
                assert!(interrupted.is_playing(b), "{other} cut off {cue} at frame {frame}");
            }
            clean.frame();
            interrupted.frame();
            clean.render(&mut reference, 32000.0);
            interrupted.render(&mut actual, 32000.0);
            assert_eq!(actual, reference, "{cue} PCM changed at frame {frame}");
        }
        assert!(blocked > 0, "no voice interruption was exercised");
        assert!(!clean.is_playing(a), "fire must eventually finish naturally");
        assert!(interrupted.mario_voice("MSD_SE_MV21_JUMP_SMALL_01").is_some(), "normal voices must resume");
    }
}

#[test]
fn death_can_still_interrupt_a_fire_cry() {
    let Some(dir) = libsms::assets::find(None) else { eprintln!("SKIP: no assets/"); return; };
    let data = Arc::new(SoundData::load(&dir).unwrap());
    let mut s = SoundSystem::new(data);
    let fire = s.mario_voice("MSD_SE_MV05_DAMAGE_FIRE_02").unwrap();
    s.frame();
    assert!(s.is_playing(fire));
    let death = s.mario_voice("MSD_SE_MV08A_DOWN_01").expect("death voice must take precedence");
    assert!(!s.is_playing(fire), "death must stop the fire cry");
    assert!(s.is_playing(death));
}
