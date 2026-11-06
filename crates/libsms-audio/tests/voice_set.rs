//! Piantissimo's voice (`MSound::startMarioVoice`, voice status 6): the five jump cues become
//! his own, and the rest fall back to Mario's lines or, in the strict set, fall silent.
//! Needs the user's extracted `assets/`; skipped without them.

use std::sync::Arc;

use glam::{Mat4, Vec3};
use libsms_assets::sound::SoundData;
use libsms_audio::{SoundSystem, VoiceSet};

fn data() -> Option<Arc<SoundData>> {
    let dir = libsms::assets::find(None)?;
    SoundData::load(&dir).ok().map(Arc::new)
}

/// One second of `name` as a voice under `set`, rendered; None when it did not start.
fn voice(data: &Arc<SoundData>, set: VoiceSet, name: &str) -> Option<Vec<f32>> {
    let mut s = SoundSystem::new(data.clone());
    s.set_listener(Mat4::IDENTITY, Vec3::ZERO);
    s.set_voice_set(set);
    s.mario_voice(name)?;
    let mut out = Vec::new();
    let mut buf = vec![0f32; (32000 / 30) * 2];
    for _ in 0..30 {
        s.frame();
        s.render(&mut buf, 32000.0);
        out.extend_from_slice(&buf);
    }
    Some(out)
}

fn energy(v: &[f32]) -> f64 {
    v.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt()
}

#[test]
fn piantissimos_jump_cue_is_in_the_banks_and_replaces_marios_jump_voices() {
    let Some(data) = data() else { eprintln!("SKIP: no assets/"); return };
    assert!(data.names.id("MSD_SE_NPC_VA_MTMAN_JUMP1").is_some(), "his cue should be in mSound.asn");
    for name in [
        "MSD_SE_MV21_JUMP_SMALL_01",
        "MSD_SE_MV22_JUMP_MID_01",
        "MSD_SE_MV23_JUMP_LARGE_01",
        "MSD_SE_MV24_JUMP_SPECIAL_01",
        "MSD_SE_MV25A_JUMP_HUGE_01",
    ] {
        let mario = voice(&data, VoiceSet::Mario, name).expect("Mario's jump voice");
        let mut pian = Vec::new();
        for set in [VoiceSet::Piantissimo, VoiceSet::PiantissimoStrict] {
            let v = voice(&data, set, name).unwrap_or_else(|| panic!("{set:?} {name}"));
            assert!(energy(&v) > 0.0, "{set:?} {name} is silent");
            pian.push(v);
        }
        assert_eq!(pian[0].len(), pian[1].len());
        assert!(energy(&pian[0]) > 0.0);
        // Not Mario's line: the two waves differ.
        let diff: f64 = mario.iter().zip(&pian[0]).map(|(a, b)| ((a - b) as f64).powi(2)).sum::<f64>().sqrt();
        assert!(diff > 0.05 * energy(&mario).max(energy(&pian[0])), "{name} still sounds like Mario");
    }
}

#[test]
fn other_voices_fall_back_to_marios_or_stay_silent_when_strict() {
    let Some(data) = data() else { eprintln!("SKIP: no assets/"); return };
    for name in ["MSD_SE_MV13_ACTION_SMALL_01", "MSD_SE_MV01_DAMAGE_BIG_01", "MSD_SE_MV08A_DOWN_01"] {
        assert!(voice(&data, VoiceSet::Piantissimo, name).is_some(), "{name}: fall back to Mario's line");
        assert!(voice(&data, VoiceSet::PiantissimoStrict, name).is_none(), "{name}: the game plays nothing");
    }
}

/// Shadow Mario (voice status 2): Mario's own line, raised by `setPitch(1.2f)`, so it is
/// heard but is not Mario's rendering of it.
#[test]
fn shadow_marios_voice_is_marios_line_pitched_up() {
    let Some(data) = data() else { eprintln!("SKIP: no assets/"); return };
    let name = "MSD_SE_MV15_EXERT_INST_01";
    let mario = voice(&data, VoiceSet::Mario, name).expect("Mario's line");
    let shadow = voice(&data, VoiceSet::Shadow, name).expect("Shadow Mario's line");
    assert!(energy(&shadow) > 0.0, "it should be heard");
    let diff: f64 = mario.iter().zip(&shadow).map(|(a, b)| ((a - b) as f64).powi(2)).sum::<f64>().sqrt();
    assert!(diff > 0.05 * energy(&mario), "pitched up, it should not render as Mario's");
}
