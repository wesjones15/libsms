//! Pikmin 2's sound effects through the same engine that plays Sunshine's (JAudio's JAS/JAI are the one system):
//! Louie's voice (`PSSE_PL_*_LUI` / `_LUGI`, SoundID.h in the decomp) is in the banks and makes sound.
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

/// Plays sound `id` on a fresh system for `seconds` and returns (energy, peak, seconds with sound).
fn render(data: &Arc<SoundData>, id: u32, seconds: usize) -> (f64, f32, f32) {
    let mut s = SoundSystem::new(data.clone());
    s.set_listener(Mat4::IDENTITY, Vec3::ZERO);
    let started = s.play_se(id, Some(Vec3::new(0.0, 0.0, 100.0)), 1).is_some();
    if !started {
        return (-1.0, 0.0, 0.0);
    }
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
    (energy, peak, live as f32 / 30.0)
}

/// The ids `libpikmin_present::louie::voice_id` plays for Louie, with the names `SoundID.h` gives them.
const LOUIE_VOICE: [(&str, u32); 6] = [
    ("PSSE_PL_THROW", 0x806),
    ("PSSE_PL_DAMAGE_LUI", 0x891),
    ("PSSE_PL_DOOM_LUGI", 0x871),
    ("PSSE_PL_SYUGO_LUI", 0x89F),
    ("PSSE_PL_KAISAN_LUI", 0x8A1),
    ("PSSE_PL_PUNCH_LUI", 0x88E),
];

#[test]
fn louies_voice_lines_make_sound() {
    let Some(dir) = assets() else { eprintln!("SKIP: no Pikmin 2 sound files"); return };
    let data = Arc::new(SoundData::load_pikmin2(&dir).expect("sound data"));
    for (name, id) in LOUIE_VOICE {
        // The disc's own name table agrees with SoundID.h. A name without the 0x800 bit (`PSSE_PL_SYUGO_LUI`, 0x9F) is the
        // held flavour the game re-requests every frame; with the bit it plays through once, which is how a host that
        // asks once has to ask.
        if let Some(named) = data.names.id(name) {
            assert_eq!(named & !0x800, id & !0x800, "{name}");
        } else {
            eprintln!("{name} is not in PSound.asn");
        }
        let (energy, peak, live) = render(&data, id, 3);
        eprintln!("{name} {id:#x}: energy {energy:.4} peak {peak:.3} sound for {live:.2} s");
        assert!(energy > 1.0 && peak > 0.05, "{name} ({id:#x}) was silent or never started");
        assert!(peak <= 1.0, "{name} clips");
    }
}

/// Louie's lines are not Olimar's: the same moment's sound differs between the two.
#[test]
fn louie_and_olimar_sound_different() {
    let Some(dir) = assets() else { eprintln!("SKIP: no Pikmin 2 sound files"); return };
    let data = Arc::new(SoundData::load_pikmin2(&dir).expect("sound data"));
    for (louie, olimar) in [(0x891, 0x890), (0x88E, 0x88D)] {
        let a = render(&data, louie, 3);
        let b = render(&data, olimar, 3);
        assert!(a.0 != b.0 || a.1 != b.1 || a.2 != b.2, "{louie:#x} and {olimar:#x} rendered alike: {a:?}");
    }
}
