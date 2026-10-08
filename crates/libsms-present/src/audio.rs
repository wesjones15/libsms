//! The game's sounds for the sim, on the default output device. CPAL owns the device; the
//! game thread posts animation clocks and events to `libsms-audio` after each 120 Hz tick
//! (`Sim::advance_observed`). All positions and the listener matrix use decomp units.
//!
//! Hosts with their own soundtrack start it without music.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use glam::{Mat4, Vec3};
use libsms::{Event, JumpKind, Mario, NozzleKind, SoundCue};
use libsms_assets::sound::{AnimSoundTable, SoundData};
use libsms_audio::{SoundHandle, SoundSystem, animation::AnimationPlayer};
use std::{
    collections::HashMap,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
};

const MARIO: u64 = 0x4D41_5249;
const FLUDD: u64 = 0x464C_5544;
const WATER: u64 = 0x5741_5452;
const YOSHI: u64 = 0x594F_5348;

pub struct Audio {
    scene: SceneAudio,
    sys: Arc<Mutex<SoundSystem>>,
    /// Output gain 0..1, as `f32` bits.
    volume: Arc<AtomicU32>,
    _stream: cpal::Stream,
    pub rate: u32,
    pub channels: u16,
}

/// Kept separate from the device so integration tests can render the same cues headlessly.
struct SceneAudio {
    tables: HashMap<String, AnimSoundTable>,
    animation: AnimationPlayer,
    clip: Option<&'static str>,
    /// Yoshi's body and eating-head clips' sound keys (`mBodyAnmSound`, `mTongueAnmSound`;
    /// `yoshi/bas/yoshi_<clip>.bas`).
    yoshi_tables: HashMap<String, AnimSoundTable>,
    yoshi: [(AnimationPlayer, Option<&'static str>); 2],
    /// Held sounds (ids without `0x800`) the sim re-requests each tick, by name.
    held: HashMap<&'static str, SoundHandle>,
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
        let mut yoshi_tables = HashMap::new();
        for entry in std::fs::read_dir(dir.join("yoshi/bas"))
            .into_iter()
            .flatten()
            .flatten()
        {
            let path = entry.path();
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if let Some(t) = std::fs::read(&path)
                .ok()
                .and_then(|b| AnimSoundTable::parse(&b).ok())
            {
                yoshi_tables.insert(stem.to_owned(), t);
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
            yoshi_tables,
            yoshi: Default::default(),
            held: HashMap::new(),
            tracks,
            track: None,
            ticks: 0,
        }
    }

    fn reset(&mut self, sys: &mut SoundSystem) {
        self.animation.stop(sys);
        for (p, c) in &mut self.yoshi {
            p.stop(sys);
            *c = None;
        }
        for (_, h) in self.held.drain() {
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
        self.yoshi_tick(sys, mario);
        self.ticks += 1;
        // JAI runs at 30 Hz, independently of the display refresh rate.
        if self.ticks.is_multiple_of(4) {
            sys.frame();
        }
    }

    /// `TYoshi::calcAnim`'s `animeLoop`s: his body clip, and the eating head.
    fn yoshi_tick(&mut self, sys: &mut SoundSystem, mario: &Mario) {
        let y = &mario.yoshi;
        let shown = mario.yoshi_look().is_some();
        let pos = if mario.on_yoshi() { mario.pos } else { y.pos };
        let clips = [
            shown.then(|| {
                (
                    libsms::yoshi::CLIPS[y.clip as usize],
                    y.ctrl.frame,
                    y.ctrl.rate,
                )
            }),
            y.eat
                .filter(|_| shown)
                .map(|(c, f)| (libsms::yoshi::CLIPS[c as usize], f.frame, f.rate)),
        ];
        for ((player, cur), clip) in self.yoshi.iter_mut().zip(clips) {
            let name = clip.map(|c| c.0);
            if *cur != name {
                player.reset(sys);
                *cur = name;
            }
            if let Some((name, frame, rate)) = clip
                && let Some(table) = self.yoshi_tables.get(name)
            {
                player.update(sys, table, frame, rate, pos, YOSHI);
            }
        }
    }

    fn events(&mut self, sys: &mut SoundSystem, events: &[Event], mario: &Mario) {
        sys.player = mario.pos;
        for event in events {
            let name = match event {
                // `TMario::setStatusToJumping` (MarioMove.cpp) starts the voice separately
                // from the animation's takeoff effects. `.bas` alone is not enough.
                Event::Jump(kind) => {
                    let voice = match kind {
                        JumpKind::Normal => "MSD_SE_MV21_JUMP_SMALL_01",
                        JumpKind::Second | JumpKind::Wall | JumpKind::Long => {
                            "MSD_SE_MV22_JUMP_MID_01"
                        }
                        JumpKind::Triple | JumpKind::Back | JumpKind::UTurn | JumpKind::Spin => {
                            "MSD_SE_MV23_JUMP_LARGE_01"
                        }
                        JumpKind::Dive => "MSD_SE_MV13_ACTION_SMALL_01",
                        JumpKind::Forced => "MSD_SE_MV24_JUMP_SPECIAL_01",
                    };
                    sys.mario_voice(voice);
                    continue;
                }
                // `TMario::hipAttacking` / `soundProcess` and `jumping` (MarioJump.cpp).
                Event::GroundPound { .. } => "MSD_SE_MA_HIP_ATTACK",
                Event::Land { strong: true } => "MSD_SE_MA_FALL_AFTER",
                // `MSound::getWallSound`: the sample course uses common hard walls.
                // The event does not retain impact speed, so use its hard-impact variant.
                Event::WallBump => "MSD_SE_MA_WALL_COL_CMN_H",
                // The rocket launch call; the trigger's own sound comes as a `Sound` event.
                Event::TriggerFired(NozzleKind::Rocket) => {
                    sys.mario_voice("MSD_SE_MV24_JUMP_SPECIAL_01");
                    continue;
                }
                // The sim's `startSoundActor` points (FLUDD's water, triggers, pumping,
                // nozzle selection).
                Event::Sound(cue) => {
                    self.cue(sys, cue);
                    continue;
                }
                _ => continue,
            };
            sys.play(name, Some(mario.pos), MARIO);
        }
    }

    /// One `startSoundActor(WithInfo)`. Held sounds keep their handle while re-requested
    /// (the game requests them once a frame; the sim reports every tick).
    fn cue(&mut self, sys: &mut SoundSystem, cue: &SoundCue) {
        // Mario's voice goes through `startMarioVoice`'s variant selection.
        if cue.name.starts_with("MSD_SE_MV") {
            sys.mario_voice(cue.name);
            return;
        }
        let Some(id) = sys.id(cue.name) else {
            return;
        };
        if id & 0xC00 != 0 {
            match cue.info {
                Some(v) => sys.play_with_info(cue.name, Some(cue.pos), FLUDD, v),
                None => sys.play(cue.name, Some(cue.pos), FLUDD),
            };
            return;
        }
        if let Some(&h) = self.held.get(cue.name)
            && sys.refresh(h, cue.pos)
        {
            if let Some(v) = cue.info {
                sys.apply_info(h, cue.name, v);
            }
            return;
        }
        let h = match cue.info {
            Some(v) => sys.play_with_info(cue.name, Some(cue.pos), FLUDD, v),
            None => sys.play(cue.name, Some(cue.pos), FLUDD),
        };
        match h {
            Some(h) => {
                self.held.insert(cue.name, h);
            }
            None => {
                self.held.remove(cue.name);
            }
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
    /// Failure leaves a usable silent host, with a diagnostic. `music` starts Delfino Plaza's
    /// theme (hosts with their own soundtrack leave it off).
    pub fn start(dir: &Path, music: bool) -> Option<Self> {
        match Self::open(dir, music) {
            Ok(audio) => Some(audio),
            Err(e) => {
                eprintln!("sound disabled: {e}");
                None
            }
        }
    }

    pub fn open(dir: &Path, music: bool) -> Result<Self, Box<dyn std::error::Error>> {
        let mut sys = SoundSystem::new(Arc::new(SoundData::load(dir)?));
        let mut scene = SceneAudio::load(dir, &sys);
        if music
            && let Some(i) = scene.tracks.iter().position(|(n, _)| n == "MSD_BGM_DOLPIC")
            && sys.play_bgm(scene.tracks[i].1)
        {
            scene.track = Some(i);
        }
        let sys = Arc::new(Mutex::new(sys));
        let volume = Arc::new(AtomicU32::new(1.0f32.to_bits()));
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("no output device")?;
        let config = device.default_output_config()?;
        let (rate, channels) = (config.sample_rate().0, config.channels());
        let v = volume.clone();
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => output::<f32>(&device, &config.into(), sys.clone(), v)?,
            cpal::SampleFormat::I16 => output::<i16>(&device, &config.into(), sys.clone(), v)?,
            cpal::SampleFormat::U16 => output::<u16>(&device, &config.into(), sys.clone(), v)?,
            format => return Err(format!("unsupported output sample format {format}").into()),
        };
        stream.play()?;
        Ok(Self {
            scene,
            sys,
            volume,
            _stream: stream,
            rate,
            channels,
        })
    }

    /// Output gain, 0..1.
    pub fn set_volume(&self, v: f32) {
        self.volume
            .store(v.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    /// The listener as a world-to-camera matrix (camera looking down -Z) and its position.
    pub fn set_listener(&self, view: Mat4, eye: Vec3) {
        if let Ok(mut sys) = self.sys.lock() {
            sys.set_listener(view, eye);
        }
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
    /// `TModelWaterManager`'s sounds for one frame of water: splashes on the ground
    /// (`splashSound`, `MSD_SE_WT_GND`), drops into water, and stuck water drying up.
    pub fn water(&mut self, events: &[libsms::water::WaterEvent]) {
        use libsms::water::WaterEvent;
        let mut sys = self.sys.lock().unwrap();
        let mut dried = false;
        for e in events {
            match *e {
                WaterEvent::SplashSound { pos, .. } => {
                    sys.play("MSD_SE_WT_GND", Some(pos), WATER);
                }
                WaterEvent::IntoWater(pos) => {
                    sys.play("MSD_SE_WT_INTO_WATER", Some(pos), WATER);
                }
                WaterEvent::DryUp(pos) if !dried => {
                    dried = true;
                    sys.play("MSD_SE_PO_WT_DRY_UP", Some(pos), WATER);
                }
                _ => {}
            }
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
    volume: Arc<AtomicU32>,
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
            let gain = f32::from_bits(volume.load(Ordering::Relaxed));
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
                    *sample = T::from_sample((value * gain).clamp(-1.0, 1.0));
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
        let audio = Audio::open(&assets.dir, true).expect("open output device");
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
    fn jump_events_render_voice_without_animation_or_music() {
        let Some((assets, _)) = fixture() else {
            return;
        };
        let data = Arc::new(SoundData::load(&assets.dir).unwrap());
        let mario = Mario::new(assets.params, assets.anim_table, Vec3::ZERO, 0);
        for kind in [
            JumpKind::Normal,
            JumpKind::Second,
            JumpKind::Triple,
            JumpKind::Wall,
            JumpKind::Spin,
            JumpKind::Back,
            JumpKind::UTurn,
            JumpKind::Long,
            JumpKind::Dive,
            JumpKind::Forced,
        ] {
            let mut sys = SoundSystem::new(data.clone());
            let mut scene = SceneAudio::load(&assets.dir, &sys);
            scene.events(&mut sys, &[Event::Jump(kind)], &mario);
            assert!(sys.active_sounds() > 0, "no voice registered for {kind:?}");
            let mut peak = 0.0f32;
            let mut buffer = [0.0; 1600];
            for _ in 0..30 {
                sys.frame();
                sys.render(&mut buffer, 24000.0);
                assert!(buffer.iter().all(|v| v.is_finite()));
                peak = peak.max(buffer.iter().map(|v| v.abs()).fold(0.0, f32::max));
            }
            assert!(peak > 0.001, "silent {kind:?} voice: {peak}");
        }
    }

    #[test]
    fn pound_and_spin_sounds_are_audible() {
        let Some((assets, _)) = fixture() else {
            return;
        };
        let data = Arc::new(SoundData::load(&assets.dir).unwrap());
        let mario = Mario::new(assets.params, assets.anim_table, Vec3::ZERO, 0);
        let cue = |name, info| {
            Event::Sound(SoundCue {
                name,
                pos: Vec3::ZERO,
                info,
            })
        };
        let cases = [
            (
                "ground pound",
                Event::GroundPound {
                    super_: true,
                    drop: 900.0,
                },
            ),
            ("spin voice", cue("MSD_SE_MV24_JUMP_SPECIAL_01", None)),
            ("spin whoosh", cue("MSD_SE_MA_ROLL_JUMP", None)),
            ("empty tank", cue("MSD_SE_PO_ACTION_ON_EMPTY", None)),
            ("full tank", cue("MSD_SE_SY_PO_WATER_FULL", None)),
        ];
        for (what, event) in cases {
            let mut sys = SoundSystem::new(data.clone());
            sys.set_listener(Mat4::IDENTITY, Vec3::ZERO);
            let mut scene = SceneAudio::load(&assets.dir, &sys);
            let mut peak = 0.0f32;
            let mut buffer = [0.0; 1600];
            for i in 0..30 {
                if i == 0 {
                    scene.events(&mut sys, &[event], &mario);
                }
                sys.frame();
                sys.render(&mut buffer, 24000.0);
                peak = peak.max(buffer.iter().map(|v| v.abs()).fold(0.0, f32::max));
            }
            assert!(peak > 0.001, "silent {what}: {peak}");
        }
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
        let event = Event::Sound(SoundCue {
            name: "MSD_SE_PO_HOVER",
            pos: Vec3::ZERO,
            info: None,
        });
        scene.events(&mut sys, &[event], &mario);
        let handle = *scene
            .held
            .get("MSD_SE_PO_HOVER")
            .expect("hover sound registered");
        for _ in 0..8 {
            scene.events(&mut sys, &[event], &mario);
        }
        assert_eq!(
            scene.held.get("MSD_SE_PO_HOVER"),
            Some(&handle),
            "repeated emission restarted sound"
        );
        let mut buffer = [0.0; 1600];
        for _ in 0..20 {
            scene.events(&mut sys, &[event], &mario);
            sys.frame();
            sys.render(&mut buffer, 24000.0);
        }
        assert_eq!(scene.held.get("MSD_SE_PO_HOVER"), Some(&handle));
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
