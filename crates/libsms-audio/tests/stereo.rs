//! Both ears: a sound reaches the left and right outputs, in the proportion its position says.
//! Needs the user's extracted `assets/`; skipped without them.

use std::sync::Arc;

use glam::{Mat4, Vec3};
use libsms_assets::sound::SoundData;
use libsms_audio::SoundSystem;

/// Left and right level of `name` played at `pos` for a second, the listener at the origin looking
/// down -Z with +X on its right.
fn levels(data: &Arc<SoundData>, name: &str, pos: Option<Vec3>, voice: bool) -> (f64, f64) {
    let mut s = SoundSystem::new(data.clone());
    s.set_listener(Mat4::IDENTITY, Vec3::ZERO);
    if voice {
        assert!(s.mario_voice(name).is_some(), "{name}");
    } else {
        assert!(s.play(name, pos, 1).is_some(), "{name}");
    }
    let (mut l, mut r) = (0f64, 0f64);
    let mut buf = vec![0f32; (32000 / 30) * 2];
    for _ in 0..30 {
        s.frame();
        s.render(&mut buf, 32000.0);
        for c in buf.chunks_exact(2) {
            l += (c[0] as f64).powi(2);
            r += (c[1] as f64).powi(2);
        }
    }
    (l.sqrt(), r.sqrt())
}

fn data() -> Option<Arc<SoundData>> {
    let dir = libsms::assets::find(None)?;
    SoundData::load(&dir).ok().map(Arc::new)
}

#[test]
fn a_sound_at_the_listener_is_in_both_ears_alike() {
    let Some(data) = data() else { return };
    // FLUDD's hover, a coin, and Mario's voice (which plays where Mario stands, the listener's spot).
    for (name, voice) in [("MSD_SE_PO_HOVER", false), ("MSD_SE_SY_COIN", false), ("MSD_SE_MV23_JUMP_LARGE_01", true)] {
        let (l, r) = levels(&data, name, Some(Vec3::ZERO), voice);
        assert!(l > 0.0 && r > 0.0, "{name}: left {l}, right {r}");
        assert!((l / r - 1.0).abs() < 0.1, "{name}: left {l}, right {r}");
    }
}

#[test]
fn a_sound_to_one_side_is_louder_in_that_ear() {
    let Some(data) = data() else { return };
    let (ll, lr) = levels(&data, "MSD_SE_PO_HOVER", Some(Vec3::new(-500.0, 0.0, 0.0)), false);
    let (rl, rr) = levels(&data, "MSD_SE_PO_HOVER", Some(Vec3::new(500.0, 0.0, 0.0)), false);
    assert!(ll > 2.0 * lr, "left source: left {ll}, right {lr}");
    assert!(rr > 2.0 * rl, "right source: left {rl}, right {rr}");
}
