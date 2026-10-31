//! Enemy water-hit feedback, shared by the viewer and native hosts.
//!
//! `Enemy/chuuhana.cpp::TChuuHana::receiveMessage` and
//! `Enemy/smallEnemy.cpp::TSmallEnemy::receiveMessage`, water-message branch:
//! JPA cue 0xE7 (`ms_enm_wathit.jpa`) at the droplet, sound 0x6802 at the enemy.
//! Host water hits carry the droplet contact position, used for both here.

use glam::Mat4;
use libsms::water::WaterEvent;
use libsms_audio::{SoundHandle, SoundSystem};

use crate::particles::Particles;

pub const EFFECT: &str = "ms_enm_wathit";
pub const SOUND: &str = "MSD_SE_EN_COMMON_W_HIT_OK";

/// Each accepted droplet gets an independent burst with the authored scale and lifetime.
/// Ground/wall splashes do not get the enemy-hit mark.
pub fn effects(fx: &mut Particles, events: &[WaterEvent]) {
    for event in events {
        if let WaterEvent::Hit { pos, .. } = *event {
            fx.emit(EFFECT, Mat4::from_translation(pos), 1.0);
        }
    }
}

/// `MSSetSound::init` (MSoundStruct.cpp), set 6: minimum interval 10 game frames.
/// Keep this gate separate from the ordinary water splash set. Spatial aggregation,
/// random shift and continuous volume/pitch modulation of MSSetSound are not reproduced.
#[derive(Default)]
pub struct HitSound {
    cooldown: u8,
}

impl HitSound {
    /// Called once per 30 Hz water frame, including frames without hits.
    pub fn step(&mut self, sys: &mut SoundSystem, events: &[WaterEvent]) -> Option<SoundHandle> {
        self.cooldown = self.cooldown.saturating_sub(1);
        if self.cooldown != 0 {
            return None;
        }
        let pos = events.iter().find_map(|event| match *event {
            WaterEvent::Hit { pos, .. } => Some(pos),
            _ => None,
        })?;
        self.hit(sys, pos)
    }

    /// Replay one contact without advancing the 30 Hz water clock.
    pub fn hit(&mut self, sys: &mut SoundSystem, pos: glam::Vec3) -> Option<SoundHandle> {
        if self.cooldown != 0 {
            return None;
        }
        let handle = sys.play(SOUND, Some(pos), 0x5748_4954)?;
        self.cooldown = 10;
        Some(handle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use libsms_assets::sound::SoundData;
    use std::sync::Arc;

    fn hit(id: u64, pos: Vec3) -> WaterEvent {
        WaterEvent::Hit {
            id,
            pos,
            attack: 1,
            burst: false,
            spin: false,
        }
    }

    #[test]
    fn disc_hit_bursts_are_yellow_world_space_contacts_and_expire() {
        let Some(dir) = libsms::assets::find(None) else {
            eprintln!("SKIP: Sunshine cache absent");
            return;
        };
        if !dir.join("particle/ms_enm_wathit.jpa").is_file() {
            eprintln!("SKIP: ms_enm_wathit.jpa absent");
            return;
        }
        let mut fx = Particles::load(&dir, &[EFFECT]);
        fx.set_world_space(EFFECT, true);
        effects(&mut fx, &[WaterEvent::Splash(Vec3::ZERO)]);
        assert!(
            !fx.active(),
            "terrain splash must not create an enemy-hit mark"
        );
        // A normal host enemy and a native flower enemy go through the same presentation.
        effects(
            &mut fx,
            &[
                hit(7, Vec3::new(-500., 200., 0.)),
                hit(libsms::flower::ID_BASE, Vec3::new(500., 200., 0.)),
            ],
        );
        fx.advance(0);
        fx.build(Vec3::X, Vec3::Y);
        assert!(!fx.batches.is_empty());
        assert!(fx.vertices.iter().all(|v| v.pos.is_finite()));
        assert!(fx.vertices.iter().any(|v| v.pos.x < -400.));
        assert!(fx.vertices.iter().any(|v| v.pos.x > 400.));
        for batch in &fx.batches {
            assert!(fx.texture_world_space(batch.texture));
        }
        let yellow = fx.batches.iter().any(|batch| {
            let tex = &fx.textures[batch.texture as usize];
            fx.vertices
                [batch.first_vertex as usize..(batch.first_vertex + batch.vertex_count) as usize]
                .iter()
                .any(|v| {
                    tex.rgba.chunks_exact(4).any(|p| {
                        let c = std::array::from_fn::<_, 4, _>(|i| p[i] as f32 / 255. * v.color[i]);
                        c[0] > 0.7 && c[1] > 0.58 && c[2] < c[1] && c[3] > 0.1
                    })
                })
        });
        assert!(yellow, "0xE7 must retain the disc's yellow hit colour");
        for frame in 1..=90 {
            fx.advance(frame * 4);
        }
        fx.build(Vec3::X, Vec3::Y);
        assert!(
            !fx.active() && fx.vertices.is_empty(),
            "hit burst must expire"
        );
    }

    #[test]
    fn disc_enemy_hit_sound_is_audible_and_gated_separately_from_splashes() {
        let Some(dir) = libsms::assets::find(None) else {
            eprintln!("SKIP: Sunshine cache absent");
            return;
        };
        if !dir.join(libsms_assets::sound::INIT_DATA).is_file() {
            eprintln!("SKIP: Sunshine audio absent");
            return;
        }
        let data = Arc::new(SoundData::load(&dir).unwrap());
        let mut sys = SoundSystem::new(data);
        assert_eq!(sys.id(SOUND), Some(0x6802));
        let mut gate = HitSound::default();
        assert!(
            gate.step(
                &mut sys,
                &[WaterEvent::SplashSound {
                    pos: Vec3::ZERO,
                    size: 5.
                }]
            )
            .is_none()
        );
        let hits = [hit(7, Vec3::ZERO), hit(8, Vec3::X)];
        let handle = gate.step(&mut sys, &hits).expect("enemy-hit cue starts");
        assert_eq!(sys.sound_id(handle), Some(0x6802));
        for _ in 0..50 {
            assert!(
                gate.hit(&mut sys, Vec3::ZERO).is_none(),
                "contacts must not advance the sound clock"
            );
        }
        let mut peak = 0f32;
        let mut pcm = [0f32; 1600];
        for frame in 0..30 {
            sys.frame();
            sys.render(&mut pcm, 24000.);
            assert!(pcm.iter().all(|v| v.is_finite()));
            peak = pcm.iter().fold(peak, |p, v| p.max(v.abs()));
            if frame < 9 {
                assert!(gate.step(&mut sys, &hits).is_none());
            } else if frame == 9 {
                assert!(gate.step(&mut sys, &hits).is_some());
            } else {
                gate.step(&mut sys, &[]);
            }
        }
        assert!(peak > 0.001, "silent enemy water-hit cue: {peak}");
    }
}
