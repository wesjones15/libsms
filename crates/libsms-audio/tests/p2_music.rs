use libsms_assets::sound::{
    SoundData,
    p2music::{self, SONGS},
};
use libsms_audio::SoundSystem;
use std::{path::PathBuf, sync::Arc};
fn assets() -> Option<PathBuf> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../libpikmin/assets/p2");
    if !p.join("music/trackMap_Seq_W.txt").is_file() {
        eprintln!("SKIP: extracted Pikmin 2 track maps absent");
        return None;
    }
    Some(p)
}
#[test]
fn source_director_maps_select_distinct_activity_layers() {
    let Some(dir) = assets() else { return };
    let maps = p2music::load(&dir).unwrap();
    for &name in SONGS {
        let m = &maps[name];
        let base = m.levels(16);
        let active = m.levels(31);
        assert_ne!(base, active, "no dynamic layers for {name}");
        assert!(active.iter().all(|&v| v == 0.0 || v == 1.0));
    }
}
#[test]
fn every_selected_p2_bgm_is_audible_and_the_director_changes_pcm() {
    let Some(dir) = assets() else { return };
    let data = Arc::new(SoundData::load_pikmin2(&dir).unwrap());
    let maps = p2music::load(&dir).unwrap();
    for &name in SONGS {
        let mut sys = SoundSystem::new(data.clone());
        assert!(sys.play_bgm_named(name), "cannot start {name}");
        let mut energy = 0.0f64;
        let mut peak = 0.0f32;
        let mut buf = vec![0.0; 2206];
        for frame in 0..150 {
            sys.frame();
            sys.directed_bgm_layers(&maps[name].levels(if frame < 75 { 16 } else { 31 }));
            sys.render(&mut buf, 33090.0);
            for &v in &buf {
                assert!(v.is_finite());
                energy += (v as f64).powi(2);
                peak = peak.max(v.abs());
            }
        }
        eprintln!("{name}: energy {energy}, peak {peak}");
        assert!(energy > 0.01, "silent {name}");
    }
    let render = |flags| {
        let mut sys = SoundSystem::new(data.clone());
        assert!(sys.play_bgm_named("forest.bms"));
        let mut pcm = Vec::new();
        let mut buf = vec![0.0; 2206];
        for _ in 0..100 {
            sys.frame();
            sys.directed_bgm_layers(&maps["forest.bms"].levels(flags));
            sys.render(&mut buf, 33090.0);
            pcm.extend_from_slice(&buf);
        }
        pcm
    };
    let base = render(16);
    let active = render(31);
    assert!(
        base.iter().zip(active).any(|(a, b)| (a - b).abs() > 0.001),
        "activity never changes music PCM"
    );
}

#[test]
fn added_enemy_presentation_cues_produce_finite_audio() {
    let Some(dir) = assets() else { return };
    let data = Arc::new(SoundData::load_pikmin2(&dir).unwrap());
    for id in [
        0x3055, 0x5805, 0x5844, 0x5848, 0x5849, 0x584a, 0x584b, 0x50b4, 0x58b5,
    ] {
        let mut sys = SoundSystem::new(data.clone());
        sys.set_listener(glam::Mat4::IDENTITY, glam::Vec3::ZERO);
        let mut energy = 0.0f64;
        let mut buf = vec![0.0; 2206];
        for frame in 0..60 {
            if frame == 0 || id & 0x800 == 0 {
                assert!(
                    sys.play_se(id, Some(glam::Vec3::new(0., 0., 100.)), 1)
                        .is_some(),
                    "cannot start {id:x}"
                );
            }
            sys.frame();
            sys.render(&mut buf, 33090.0);
            for &v in &buf {
                assert!(v.is_finite());
                energy += (v as f64).powi(2);
            }
        }
        eprintln!("presentation cue {id:x}: energy {energy}");
        assert!(energy > 0.01, "silent cue {id:x}");
    }
}
