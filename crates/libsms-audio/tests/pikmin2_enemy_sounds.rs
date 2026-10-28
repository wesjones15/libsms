//! Pikmin 2's enemy sound effects (`PSSE_EN_*`, SoundID.h in the decomp) through the same engine: the ids the Kogane
//! family asks for (`libpikmin_teki::p2::sound_id`) are in the banks and make sound.
//! Needs the user's extracted Pikmin 2 sound files (`libsms/assets/p2/AudioRes`); skipped without them.

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

/// Plays sound `id` on a fresh system for `seconds` and returns (energy, peak); (-1, 0) when it did not start.
fn render(data: &Arc<SoundData>, id: u32, seconds: usize) -> (f64, f32) {
    let mut s = SoundSystem::new(data.clone());
    s.set_listener(Mat4::IDENTITY, Vec3::ZERO);
    if s.play_se(id, Some(Vec3::new(0.0, 0.0, 100.0)), 1).is_none() {
        return (-1.0, 0.0);
    }
    let (mut energy, mut peak) = (0f64, 0f32);
    let mut buf = vec![0f32; (32000 / 30) * 2];
    for _ in 0..seconds * 30 {
        s.frame();
        s.render(&mut buf, 32000.0);
        energy += buf.iter().map(|x| (*x as f64).powi(2)).sum::<f64>();
        peak = buf.iter().fold(peak, |p, x| p.max(x.abs()));
    }
    (energy, peak)
}

/// (`SoundID.h` name, id) of what the Kogane family plays, and whether it plays once (the 0x800 bit) or is held.
const KOGANE: [(&str, u32); 9] = [
    ("PSSE_EN_ENEMY_LOOSE_ITEM", 0x5810),
    ("PSSE_EN_KOGANE_HIT", 0x5873),
    ("PSSE_EN_OOGANE_HIT", 0x5880),
    ("PSSE_EN_TAMAGOMUSHI_DIVE", 0x58A9),
    ("PSSE_EN_TAMAGOMUSHI_APPEAR", 0x5915),
    ("PSSE_EN_FART_GAS", 0x5960),
    ("PSSE_EN_FART_HIT", 0x5961),
    ("PSSE_EN_FART_BUZZ", 0x5162),
    ("PSSE_EN_OTAKARA_ATK_GAS", 0x5164),
];

#[test]
fn the_kogane_familys_sounds_make_sound() {
    let Some(dir) = assets() else { eprintln!("SKIP: no Pikmin 2 sound files"); return };
    let data = Arc::new(SoundData::load_pikmin2(&dir).expect("sound data"));
    for (name, id) in KOGANE {
        if let Some(named) = data.names.id(name) {
            assert_eq!(named & !0x800, id & !0x800, "{name}");
        } else {
            eprintln!("{name} is not in PSound.asn");
        }
        // A held id (no 0x800 bit) is what the game re-requests every frame; as a once-only sound the same id has the bit.
        let once = id | 0x800;
        let (energy, peak) = render(&data, id, 2);
        let (e_once, p_once) = render(&data, once, 2);
        eprintln!("{name} {id:#x}: energy {energy:.4} peak {peak:.3}; once {once:#x}: energy {e_once:.4} peak {p_once:.3}");
        assert!((energy > 1.0 && peak > 0.02) || (e_once > 1.0 && p_once > 0.02), "{name} ({id:#x}) was silent or never started");
    }
}
