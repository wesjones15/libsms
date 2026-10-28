//! The sound effects Pikmin 2 plays around treasures, through the engine that plays Sunshine's (see
//! `pikmin2_sounds.rs`). Ids are `SoundID.h` in the decomp. Needs the user's extracted Pikmin 2 sound files
//! (`libsms/assets/p2/AudioRes`); skipped without them. The `.ast` streams (`PSSTR_TREASURE_GET` /
//! `_GLAD`, `ff_treasure*.ast`) are not read here: they are a separate stream path.

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

/// (name in `SoundID.h`, id, what plays it). `data.names` is checked against the id when it knows the name.
const TREASURE_SOUNDS: &[(&str, u32, &str)] = &[
    ("PSSE_EV_HOME_PELLET_BACUUM", 0x3804, "an Onion/ship sucks a pellet in (onyonMgr.cpp)"),
    ("PSSE_EV_HOME_PELLET_FINISH", 0x3805, "the pellet is gone into the Onion/ship"),
    ("PSSE_EV_TREASURE_RISE_UP", 0x3832, "a treasure rises out of its hiding place (itemTreasure.cpp)"),
    ("PSSE_EV_TREASURE_JUMP_OUT", 0x3833, "a treasure hops out"),
    ("PSSE_OTAKARA_COMPLETE", 0x1845, "the 'all treasures' jingle"),
    ("PSSE_DEMOG04_TREASURE_FX", 0x3870, "the treasure-shine demo effect"),
    ("PSSE_EV_ITEM_LAND_SOIL_S", 0x3808, "a small treasure lands on soil (Pellet::onBounce, pelletMgr.cpp: 0x3808 + size)"),
    ("PSSE_EV_ITEM_LAND_SOIL_M", 0x3809, "medium"),
    ("PSSE_EV_ITEM_LAND_SOIL_L", 0x380A, "large"),
    ("PSSE_EV_ITEM_LAND_WATER1_S", 0x380B, "lands in water (+ the TOtakaraDive splash)"),
    ("PSSE_EV_ITEM_LAND_WATER1_M", 0x380C, ""),
    ("PSSE_EV_ITEM_LAND_WATER1_L", 0x380D, ""),
    ("PSSE_EV_ONYON_BOUND_PLAYER", 0x3806, "a pellet bounces off the Onion (pelletState.cpp)"),
    ("PSSE_PK_VC_LIFT_TRY", 0x282C, "Pikmin strain to lift (Pikmin voice)"),
    ("PSSE_PK_VC_LIFT_SUCCESS", 0x282D, "Pikmin get it up"),
    ("PSSE_PK_VC_LIFT_MOVE", 0x282E, "Pikmin carry it along"),
    ("PSSE_PK_HAPPA_LIFT_TRY", 0x2870, "same, leaf/bud/flower Pikmin"),
    ("PSSE_PK_HAPPA_LIFT_SUCCESS", 0x2871, ""),
    ("PSSE_PK_HAPPA_LIFT_MOVE", 0x2872, ""),
    ("PSSE_SY_OTAKARA_ZOOMIN", 0x1082, "treasure collection screen zoom in"),
    ("PSSE_SY_OTAKARA_ZOOMOUT", 0x1083, ""),
    ("PSSE_SY_OTAKARA_SCROLL", 0x1084, ""),
    ("PSSE_SY_OTAKARA_NAME", 0x1885, "the name appears"),
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
fn treasure_sound_effects_make_sound() {
    let Some(dir) = assets() else { eprintln!("SKIP: no Pikmin 2 sound files"); return };
    let data = Arc::new(SoundData::load_pikmin2(&dir).expect("sound data"));
    let mut silent = Vec::new();
    for (name, id, what) in TREASURE_SOUNDS {
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

/// The treasure jingles are `.ast` streams (`PSSTR_TREASURE_GET` 0x1001, `_GLAD` 0x1027, and the "all collected"
/// one); the cache keeps them under `treasure/streams/`. Each decodes to a few seconds of stereo with real signal.
#[test]
fn treasure_jingles_decode() {
    let Some(dir) = assets() else { eprintln!("SKIP: no Pikmin 2 sound files"); return };
    let streams = dir.join("treasure/streams");
    if !streams.is_dir() {
        eprintln!("SKIP: no Pikmin 2 treasure streams");
        return;
    }
    for stem in ["ff_treasureget", "ff_treasureglad", "ff_treasurecomp"] {
        let bytes = std::fs::read(streams.join(format!("{stem}.ast"))).expect("stream");
        let s = libsms_assets::sound::Stream::decode_ast(&bytes).unwrap_or_else(|e| panic!("{stem}: {e}"));
        let frames = s.stereo.len() / 2;
        let secs = frames as f32 / s.header.sample_rate as f32;
        let peak = s.stereo.iter().map(|x| x.unsigned_abs()).max().unwrap_or(0);
        let rms = (s.stereo.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / s.stereo.len() as f64).sqrt();
        eprintln!("{stem}: {secs:.1} s at {} Hz, loop {} from {}, peak {peak}, rms {rms:.0}", s.header.sample_rate, s.header.looping, s.header.loop_start);
        assert_eq!(s.header.sample_rate, 32000, "{stem}");
        assert!(secs > 3.0 && secs < 60.0, "{stem} length {secs}");
        assert!(peak > 3000 && rms > 300.0, "{stem} is silent: peak {peak} rms {rms}");
        // The tail has sound too (the blocks all decoded, not just the first).
        let tail = &s.stereo[s.stereo.len() * 3 / 4..];
        assert!(tail.iter().any(|x| x.unsigned_abs() > 500), "{stem}: silent tail");
    }
}
