//! Sample host audio. CPAL owns the device; the game thread posts animation clocks and
//! events to `libsms-audio`. All positions and the listener matrix use decomp units.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use glam::{Mat4, Vec3};
use libsms::{Event, JumpKind, Mario, NozzleKind};
use libsms_assets::sound::{AnimSoundTable, SoundData};
use libsms_audio::{SoundHandle, SoundSystem, animation::AnimationPlayer};
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
};

const MARIO: u64 = 0x4D41_5249;
const FLUDD: u64 = 0x464C_5544;

pub struct Audio {
    scene: SceneAudio,
    sys: Arc<Mutex<SoundSystem>>,
    _stream: cpal::Stream,
}

/// Kept separate from the device so integration tests can render the same cues headlessly.
struct SceneAudio {
    tables: HashMap<String, AnimSoundTable>,
    animation: AnimationPlayer,
    clip: Option<&'static str>,
    water: Option<SoundHandle>,
    tracks: Vec<(String, u32)>,
    track: Option<usize>,
    ticks: u32,
}

impl SceneAudio {
    fn load(dir: &Path, sys: &SoundSystem) -> Self {
        let mut tables = HashMap::new();
        // `TMario::initModel` (MarioDraw.cpp) loads ma_<stem>.bas with ma_<stem>.bck.
        if let Ok(entries) = std::fs::read_dir(dir.join("bas")) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) != Some("bas") {
                    continue;
                }
                let Some(stem) = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .and_then(|s| s.strip_prefix("ma_"))
                else {
                    continue;
                };
                match std::fs::read(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|b| AnimSoundTable::parse(&b).map_err(|e| e.to_string()))
                {
                    Ok(table) => {
                        tables.insert(stem.to_owned(), table);
                    }
                    Err(e) => eprintln!("sound: {}: {e}", path.display()),
                }
            }
        }
        let mut tracks: Vec<_> = sys
            .data()
            .names
            .iter()
            .filter(|(n, id)| n.starts_with("MSD_BGM_") && id & 0xC000_0000 == 0x8000_0000)
            .map(|(n, id)| (n.to_owned(), id))
            .collect();
        tracks.sort_by(|a, b| a.0.cmp(&b.0));
        Self {
            tables,
            animation: AnimationPlayer::default(),
            clip: None,
            water: None,
            tracks,
            track: None,
            ticks: 0,
        }
    }

    fn reset(&mut self, sys: &mut SoundSystem) {
        self.animation.stop(sys);
        if let Some(h) = self.water.take() {
            sys.stop(h, 0);
        }
        self.clip = None;
    }

    fn tick(&mut self, sys: &mut SoundSystem, mario: &Mario) {
        sys.player = mario.pos;
        if self.clip != Some(mario.anim.clip) {
            self.animation.reset(sys);
            self.clip = Some(mario.anim.clip);
        }
        if let Some(table) = self.tables.get(mario.anim.clip) {
            self.animation.update(
                sys,
                table,
                mario.anim.frame,
                mario.anim.rate * 0.5,
                mario.pos,
                MARIO,
            );
        }
        self.ticks += 1;
        // JAI runs at 30 Hz, independently of the display refresh rate.
        if self.ticks.is_multiple_of(4) {
            sys.frame();
        }
    }

    fn events(&mut self, sys: &mut SoundSystem, events: &[Event], mario: &Mario) {
        sys.player = mario.pos;
        for event in events {
            let name = match event {
                // `TMario::soundProcess` handles forced jumps explicitly. Ordinary jump
                // voices and takeoff/landing sounds are already in the animation tables.
                Event::Jump(JumpKind::Forced) => {
                    sys.mario_voice("MSD_SE_MV24_JUMP_SPECIAL_01");
                    continue;
                }
                // `TMario::hipAttacking` / `soundProcess` and `jumping` (MarioJump.cpp).
                Event::GroundPound { .. } => "MSD_SE_MA_HIP_ATTACK",
                Event::Land { strong: true } => "MSD_SE_MA_FALL_AFTER",
                // `MSound::getWallSound`: the sample course uses common hard walls.
                // The event does not retain impact speed, so use its hard-impact variant.
                Event::WallBump => "MSD_SE_MA_WALL_COL_CMN_H",
                // TNozzleTrigger::movement and TWaterGun's trigger setup (WaterGun.cpp).
                Event::TriggerFired(NozzleKind::Rocket) => "MSD_SE_PO_ROCKET_TRIGGER",
                Event::TriggerFired(NozzleKind::Turbo) => "MSD_SE_PO_SNIPER_TRIGGER",
                // TWaterGun::changeNozzle: front vs back selection sound.
                Event::NozzleChanged(NozzleKind::Spray) => "MSD_SE_SY_SELECT_POMP",
                Event::NozzleChanged(_) => "MSD_SE_SY_SELECT_POMP_BACK",
                Event::WaterEmit(w) => {
                    // TWaterGun::emit: held sounds, refreshed while water is emitted.
                    let name = match w.nozzle {
                        NozzleKind::Spray => "MSD_SE_PO_NORMAL_NOZZLE_IMI",
                        NozzleKind::Hover => "MSD_SE_PO_HOVER",
                        NozzleKind::Turbo => "MSD_SE_PO_WATER_HI",
                        _ => continue,
                    };
                    let id = sys.id(name);
                    // Different nozzle loops must not inherit the previous handle.
                    if self.water.is_some_and(|h| sys.sound_id(h) != id)
                        && let Some(h) = self.water.take()
                    {
                        sys.stop(h, 0);
                    }
                    if !self.water.is_some_and(|h| sys.refresh(h, w.pos)) {
                        self.water = sys.play(name, Some(w.pos), FLUDD);
                    }
                    continue;
                }
                _ => continue,
            };
            sys.play(name, Some(mario.pos), MARIO);
        }
    }

    fn cycle(&mut self, sys: &mut SoundSystem) {
        let next = self.track.map_or(0, |i| i + 1);
        sys.stop_bgm(0);
        self.track = None;
        for i in next..self.tracks.len() {
            if sys.play_bgm(self.tracks[i].1) {
                self.track = Some(i);
                println!("music: {}", self.tracks[i].0);
                return;
            }
        }
        println!("music: off");
    }
}

impl Audio {
    /// Failure leaves a usable silent viewer, with a diagnostic.
    pub fn start(dir: &Path) -> Option<Self> {
        match Self::open(dir) {
            Ok(audio) => Some(audio),
            Err(e) => {
                eprintln!("sound disabled: {e}");
                None
            }
        }
    }

    fn open(dir: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let mut sys = SoundSystem::new(Arc::new(SoundData::load(dir)?));
        let mut scene = SceneAudio::load(dir, &sys);
        if let Some(i) = scene.tracks.iter().position(|(n, _)| n == "MSD_BGM_DOLPIC")
            && sys.play_bgm(scene.tracks[i].1)
        {
            scene.track = Some(i);
        }
        let sys = Arc::new(Mutex::new(sys));
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("no output device")?;
        let config = device.default_output_config()?;
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => output::<f32>(&device, &config.into(), sys.clone())?,
            cpal::SampleFormat::I16 => output::<i16>(&device, &config.into(), sys.clone())?,
            cpal::SampleFormat::U16 => output::<u16>(&device, &config.into(), sys.clone())?,
            format => return Err(format!("unsupported output sample format {format}").into()),
        };
        stream.play()?;
        println!(
            "sound: device active, {} Mario animation tables",
            scene.tables.len()
        );
        Ok(Self {
            scene,
            sys,
            _stream: stream,
        })
    }

    pub fn tick(&mut self, mario: &Mario) {
        if let Ok(mut sys) = self.sys.lock() {
            self.scene.tick(&mut sys, mario);
        }
    }
    pub fn events(&mut self, events: &[Event], mario: &Mario) {
        if let Ok(mut sys) = self.sys.lock() {
            self.scene.events(&mut sys, events, mario);
        }
    }
    pub fn listener(&self, eye: Vec3, target: Vec3) {
        if let Ok(mut sys) = self.sys.lock() {
            sys.set_listener(Mat4::look_at_rh(eye, target, Vec3::Y), eye);
        }
    }
    pub fn reset(&mut self) {
        if let Ok(mut sys) = self.sys.lock() {
            self.scene.reset(&mut sys);
        }
    }
    pub fn cycle(&mut self) {
        if let Ok(mut sys) = self.sys.lock() {
            self.scene.cycle(&mut sys);
        }
    }
    pub fn music_name(&self) -> &str {
        self.scene
            .track
            .map_or("off", |i| self.scene.tracks[i].0.as_str())
    }
}

fn output<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    sys: Arc<Mutex<SoundSystem>>,
) -> Result<cpal::Stream, cpal::BuildStreamError> {
    let channels = config.channels as usize;
    let rate = config.sample_rate.0 as f32;
    let mut scratch = Vec::new();
    device.build_output_stream(
        config,
        move |out: &mut [T], _| {
            scratch.resize(out.len() / channels * 2, 0.0);
            scratch.fill(0.0);
            if let Ok(mut sys) = sys.lock() {
                sys.render(&mut scratch, rate);
            }
            for (stereo, frame) in scratch
                .as_chunks::<2>()
                .0
                .iter()
                .zip(out.chunks_exact_mut(channels))
            {
                for (c, sample) in frame.iter_mut().enumerate() {
                    let value = if channels == 1 {
                        (stereo[0] + stereo[1]) * 0.5
                    } else if c < 2 {
                        stereo[c]
                    } else {
                        0.0
                    };
                    *sample = T::from_sample(value.clamp(-1.0, 1.0));
                }
            }
        },
        |e| eprintln!("sound output: {e}"),
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires an output device and plays a short sound"]
    fn output_device_smoke() {
        let assets = libsms::Assets::discover(None).expect("assets required");
        let audio = Audio::open(&assets.dir).expect("open output device");
        {
            let mut sys = audio.sys.lock().unwrap();
            let id = audio.scene.tables["jump"].sounds[0].id;
            sys.play_anim_sound(id, Vec3::ZERO, MARIO)
                .expect("jump sound");
            sys.frame();
        }
        for _ in 0..15 {
            std::thread::sleep(std::time::Duration::from_millis(33));
            audio.sys.lock().unwrap().frame();
        }
        assert!(
            audio.sys.lock().unwrap().active_voices() > 0,
            "device did not render music"
        );
    }

    fn fixture() -> Option<(libsms::Assets, SoundSystem)> {
        let assets = libsms::Assets::discover(None)?;
        let data = SoundData::load(&assets.dir).expect("extracted sound data");
        Some((assets, SoundSystem::new(Arc::new(data))))
    }

    #[test]
    fn animation_cues_render_audible_pcm_and_reset() {
        let Some((assets, mut sys)) = fixture() else {
            return;
        };
        let mut scene = SceneAudio::load(&assets.dir, &sys);
        assert!(scene.tables.contains_key("jump"));
        let mut mario = Mario::new(assets.params, assets.anim_table, Vec3::ZERO, 0);
        mario.anim.set("jump", 1.0, &mario.anims);
        sys.set_listener(Mat4::IDENTITY, Vec3::ZERO);
        let mut peak = 0.0f32;
        let mut buffer = [0.0; 1600];
        for _ in 0..60 {
            for _ in 0..4 {
                mario.anim.advance();
                scene.tick(&mut sys, &mario);
            }
            sys.render(&mut buffer, 24000.0);
            assert!(buffer.iter().all(|v| v.is_finite()));
            peak = peak.max(buffer.iter().map(|v| v.abs()).fold(0.0, f32::max));
        }
        assert!(peak > 0.001, "jump animation produced silence: {peak}");
        scene.reset(&mut sys);
        assert!(scene.clip.is_none());
    }

    #[test]
    fn held_water_keeps_its_handle_then_stops_and_music_cycles() {
        let Some((assets, mut sys)) = fixture() else {
            return;
        };
        let mut scene = SceneAudio::load(&assets.dir, &sys);
        let mario = Mario::new(assets.params, assets.anim_table, Vec3::ZERO, 0);
        let event = Event::WaterEmit(libsms::WaterEmit {
            nozzle: NozzleKind::Hover,
            pos: Vec3::ZERO,
            dir: Vec3::Y,
            power: 1.0,
            count: 1,
            attack: 0,
            hit_radius: 0.0,
            hit_height: 0.0,
        });
        scene.events(&mut sys, &[event], &mario);
        let handle = scene.water.expect("hover sound registered");
        for _ in 0..8 {
            scene.events(&mut sys, &[event], &mario);
        }
        assert_eq!(
            scene.water,
            Some(handle),
            "repeated emission restarted sound"
        );
        let mut buffer = [0.0; 1600];
        for _ in 0..20 {
            scene.events(&mut sys, &[event], &mario);
            sys.frame();
            sys.render(&mut buffer, 24000.0);
        }
        assert_eq!(scene.water, Some(handle));
        for _ in 0..20 {
            sys.frame();
            sys.render(&mut buffer, 24000.0);
        }
        assert!(
            !sys.is_playing(handle),
            "water loop survived without refresh"
        );
        scene.cycle(&mut sys);
        assert!(sys.bgm_playing().is_some());
        scene.track = scene.tracks.len().checked_sub(1);
        scene.cycle(&mut sys);
        assert!(sys.bgm_playing().is_none());
    }
}
