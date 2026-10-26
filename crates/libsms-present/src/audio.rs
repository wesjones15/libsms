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
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
};

const MARIO: u64 = 0x4D41_5249;
const FLUDD: u64 = 0x464C_5544;
const WATER: u64 = 0x5741_5452;
const YOSHI: u64 = 0x594F_5348;
const COIN: u64 = 0x434F_494E;
const PORTAL: u64 = 0x504F_5254;

/// Cache RoR2's short arrival/death stings from the user's extracted sound banks, without an
/// output device. This runs on the host's first-run worker, never the Unity thread. Existing
/// cached WAVs need neither sound-bank loading nor the original disc.
pub fn render_stings(dir: &Path) -> Result<(), String> {
    let out = dir.join("ror2_audio");
    // `death.wav` is the miss jingle ("Too Bad!"); the game-over song and the blue coin's
    // pickup sound are cached beside it, and the Shine Get fanfare (`MSD_BGM_GET_SHINE`: about
    // four and a half seconds, then silence) and the "puzzle solved" chime a host may use for
    // finding a secret (`MSD_SE_SY_PROBLEM_SOLVED`: under two seconds).
    let stings = [
        ("spawn.wav", "MSD_BGM_DOLPIC", 75),
        ("death.wav", "MSD_BGM_MISS", 138),
        ("gameover.wav", "MSD_BGM_GAMEOVER", 250),
        ("bluecoin.wav", "MSD_SE_SY_BLUE_COIN_GET", 36),
        ("shineget.wav", "MSD_BGM_GET_SHINE", 150),
        ("secret.wav", "MSD_SE_SY_PROBLEM_SOLVED", 66),
    ];
    // A cache from before the left ear was connected (right channel only) is rendered again.
    let marker = out.join(STINGS_MARKER);
    let current = std::fs::read_to_string(&marker).is_ok_and(|v| v.trim() == STINGS_VERSION);
    if current && stings.iter().all(|(file, _, _)| out.join(file).is_file()) {
        return Ok(());
    }
    let data = Arc::new(SoundData::load(dir).map_err(|e| e.to_string())?);
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    for (file, cue, frames) in stings {
        let path = out.join(file);
        if current && path.is_file() {
            continue;
        }
        let wav = sting_wav(data.clone(), cue, frames)?;
        // Publish only complete files; an interrupted first run can retry the temp file.
        let temp = path.with_extension("wav.tmp");
        std::fs::write(&temp, wav).map_err(|e| e.to_string())?;
        std::fs::rename(temp, path).map_err(|e| e.to_string())?;
    }
    std::fs::write(&marker, STINGS_VERSION).map_err(|e| e.to_string())?;
    Ok(())
}

/// The cached stings' marker file, and its content: bumped when they must be rendered again
/// (2: both ears; 1 was right only). Hosts that decide whether to render check it too.
pub const STINGS_MARKER: &str = "stings.version";
pub const STINGS_VERSION: &str = "2";

fn sting_wav(data: Arc<SoundData>, cue: &str, frames: usize) -> Result<Vec<u8>, String> {
    let mut sys = SoundSystem::new(data.clone());
    let started = sys.play(cue, None, 0);
    // Music is a sequence of its own; a sound effect hands back its handle.
    if started.is_none() && sys.bgm_playing().is_none() {
        return Err(format!("could not start {cue}"));
    }
    // Match the existing offline `play` tool: JAI at 30 Hz, stereo PCM16 at 32 kHz.
    let mut samples = Vec::new();
    let mut buf = vec![0.0; (32000 / 30) * 2];
    for _ in 0..frames {
        sys.frame();
        sys.render(&mut buf, 32000.0);
        samples.extend(buf.iter().map(|v| (v * 32767.0) as i16));
    }
    let len = (samples.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&[1, 0, 2, 0]);
    wav.extend_from_slice(&32000u32.to_le_bytes());
    wav.extend_from_slice(&128000u32.to_le_bytes());
    wav.extend_from_slice(&[4, 0, 16, 0]);
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&len.to_le_bytes());
    for sample in samples {
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    Ok(wav)
}

pub struct Audio {
    scene: SceneAudio,
    sys: Arc<Mutex<SoundSystem>>,
    /// Output gain 0..1, as `f32` bits.
    volume: Arc<AtomicU32>,
    /// While set the stream outputs silence and the sound system does not advance.
    paused: Arc<AtomicBool>,
    _stream: cpal::Stream,
    pub rate: u32,
    pub channels: u16,
}

/// Kept separate from the device so integration tests can render the same cues headlessly.
struct SceneAudio {
    tables: HashMap<String, AnimSoundTable>,
    animation: AnimationPlayer,
    portal: AnimationPlayer,
    clip: Option<&'static str>,
    /// Yoshi's body and eating-head clips' sound keys (`mBodyAnmSound`, `mTongueAnmSound`;
    /// `yoshi/bas/yoshi_<clip>.bas`).
    yoshi_tables: HashMap<String, AnimSoundTable>,
    yoshi: [(AnimationPlayer, Option<&'static str>); 2],
    /// Held sounds (ids without `0x800`) the sim re-requests each tick, by name.
    held: HashMap<&'static str, SoundHandle>,
    /// Held sounds of host-owned objects ([`Audio::hold_at`]), by name.
    hums: HashMap<String, SoundHandle>,
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
            portal: AnimationPlayer::default(),
            clip: None,
            yoshi_tables,
            yoshi: Default::default(),
            held: HashMap::new(),
            hums: HashMap::new(),
            tracks,
            track: None,
            ticks: 0,
        }
    }

    fn reset(&mut self, sys: &mut SoundSystem) {
        self.animation.stop(sys);
        self.portal.stop(sys);
        for (p, c) in &mut self.yoshi {
            p.stop(sys);
            *c = None;
        }
        for (_, h) in self.held.drain().chain(self.hums.drain().map(|(_, h)| ("", h))) {
            sys.stop(h, 0);
        }
        self.clip = None;
    }

    /// MarioDraw loads this BAS beside the warp-out BCK. Its nine Y_ROLL cues
    /// (0x1963 down to 0x195B) are the original particle-to-Mario portal sound.
    fn portal_exit(&mut self, sys: &mut SoundSystem, frame: f32, pos: Vec3) {
        if let Some(table) = self.tables.get("demo_gate_out_appear_get") {
            // Compress cue timing, but retain each sample's authored pitch.
            self.portal.update(sys, table, frame, 1., pos, PORTAL);
        }
    }

    fn tick(&mut self, sys: &mut SoundSystem, mario: &Mario) {
        sys.player = mario.pos;
        if self.clip != Some(mario.anim.clip) {
            self.animation.reset(sys);
            self.clip = Some(mario.anim.clip);
        }
        // The corkscrew calls out once (a cue of the sim's) and goes quiet: the spin clip's own cues would
        // repeat with every turn.
        let corkscrew = mario.model_pitch == 0x4000;
        if let Some(table) = self.tables.get(mario.anim.clip).filter(|_| !corkscrew) {
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

    /// A host object's held sound (`startSoundActor` asked for again every frame, as the Shine's
    /// hum is): started the first time, kept alive while asked for, and let fade when it isn't.
    fn hold(&mut self, sys: &mut SoundSystem, name: &str, pos: Vec3) -> bool {
        if let Some(&h) = self.hums.get(name)
            && sys.refresh(h, pos)
        {
            return true;
        }
        match sys.play(name, Some(pos), COIN) {
            Some(h) => {
                self.hums.insert(name.to_owned(), h);
                true
            }
            None => {
                self.hums.remove(name);
                false
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
    pub fn portal_exit(&mut self, frame: f32, pos: Vec3) {
        if let Ok(mut sys) = self.sys.lock() { self.scene.portal_exit(&mut sys, frame, pos); }
    }
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
        let paused = Arc::new(AtomicBool::new(false));
        let p = paused.clone();
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => output::<f32>(&device, &config.into(), sys.clone(), v, p)?,
            cpal::SampleFormat::I16 => output::<i16>(&device, &config.into(), sys.clone(), v, p)?,
            cpal::SampleFormat::U16 => output::<u16>(&device, &config.into(), sys.clone(), v, p)?,
            format => return Err(format!("unsupported output sample format {format}").into()),
        };
        stream.play()?;
        Ok(Self {
            scene,
            sys,
            volume,
            paused,
            _stream: stream,
            rate,
            channels,
        })
    }

    /// Holds every sound where it is (a paused game): silence out, voices and envelopes
    /// frozen, and they carry on from the same place when unpaused.
    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    /// Output gain, 0..1.
    pub fn set_volume(&self, v: f32) {
        self.volume
            .store(v.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    /// Pitches Mario's voice cues by `ratio` with their length unchanged: 1.0 is the disc's
    /// voice, and a skin can use it to tell itself apart (Luigi is Mario's voice up a little).
    pub fn set_voice_pitch(&self, ratio: f32) {
        if let Ok(mut sys) = self.sys.lock() {
            sys.set_voice_pitch(ratio);
        }
    }

    /// The listener as a world-to-camera matrix (camera looking down -Z) and its position.
    pub fn set_listener(&self, view: Mat4, eye: Vec3) {
        if let Ok(mut sys) = self.sys.lock() {
            sys.set_listener(view, eye);
        }
    }

    /// Mario says `name` now (`MSound::startMarioVoice`): one voice at a time, so it cuts off
    /// the one before. For a host's own moments (a lobby intro's "wahoo").
    pub fn mario_voice(&self, name: &str) {
        if let Ok(mut sys) = self.sys.lock() {
            sys.mario_voice(name);
        }
    }

    /// TCoin::taken: the positional pickup sound, on its own actor group.
    pub fn coin_get(&self, pos: Vec3) {
        if let Ok(mut sys) = self.sys.lock() {
            sys.play("MSD_SE_SY_COIN", Some(pos), COIN);
        }
    }

    /// A host-owned object's sound effect (`MSD_SE_*`, as `startSoundActor` plays it) at `pos`, on
    /// its own actor group; false for a name the game doesn't have. The Shine's appearance is one.
    pub fn play_at(&self, name: &str, pos: Vec3) -> bool {
        self.sys
            .lock()
            .ok()
            .is_some_and(|mut sys| sys.play(name, Some(pos), COIN).is_some())
    }

    /// A host object's held sound (an `MSD_SE_*` the game re-requests every frame, like
    /// `MSD_SE_SHINE_EXIST`, the Shine's hum) at `pos`: call it every frame the object makes it. It
    /// starts on the first call and fades when the calls stop. False for a name the game lacks.
    pub fn hold_at(&mut self, name: &str, pos: Vec3) -> bool {
        match self.sys.lock() {
            Ok(mut sys) => self.scene.hold(&mut sys, name, pos),
            Err(_) => false,
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
        let mut sys = self.sys.lock().unwrap();
        water_sounds(&mut sys, events);
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

fn water_sounds(sys: &mut SoundSystem, events: &[libsms::water::WaterEvent]) {
    use libsms::water::WaterEvent;
    let mut dried = false;
    let mut splashed = false;
    for e in events {
        match *e {
            // MSSetSoundTL::startSoundSetDyna's frame gate allows one start per set
            // per 30 Hz frame, not a restart for every water particle that lands.
            WaterEvent::SplashSound { pos, .. } if !splashed => {
                splashed = true;
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

fn output<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    sys: Arc<Mutex<SoundSystem>>,
    volume: Arc<AtomicU32>,
    paused: Arc<AtomicBool>,
) -> Result<cpal::Stream, cpal::BuildStreamError> {
    let channels = config.channels as usize;
    let rate = config.sample_rate.0 as f32;
    let mut scratch = Vec::new();
    device.build_output_stream(
        config,
        move |out: &mut [T], _| {
            if paused.load(Ordering::Relaxed) {
                out.fill(T::from_sample(0.0));
                return;
            }
            scratch.resize(out.len() / channels * 2, 0.0);
            scratch.fill(0.0);
            let gain = f32::from_bits(volume.load(Ordering::Relaxed));
            if let Ok(mut sys) = sys.lock() {
                // Turning down already saturated PCM only makes the distortion quieter.
                // Apply the host's master/SFX gain to the mix before that saturation.
                sys.engine.output_gain = gain;
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
    fn arrival_and_death_stings_are_audible_stereo_pcm16() {
        let Some(dir) = libsms::assets::find(None) else {
            return;
        };
        let data = Arc::new(SoundData::load(&dir).unwrap());
        for (cue, frames) in [("MSD_BGM_DOLPIC", 75), ("MSD_BGM_MISS", 138)] {
            let wav = sting_wav(data.clone(), cue, frames).unwrap();
            assert_eq!(&wav[..4], b"RIFF");
            assert_eq!(&wav[8..16], b"WAVEfmt ");
            assert_eq!(&wav[20..24], &[1, 0, 2, 0]);
            assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 32000);
            assert_eq!(&wav[34..40], b"\x10\x00data");
            let len = frames * (32000 / 30) * 4;
            assert_eq!(wav.len(), 44 + len);
            assert_eq!(
                u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize,
                len
            );
            assert!(
                wav[44..]
                    .chunks_exact(2)
                    .any(|p| i16::from_le_bytes([p[0], p[1]]).abs() > 100),
                "silent {cue}"
            );
        }
    }

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
            ("yellow coin pickup", cue("MSD_SE_SY_COIN", None)),
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
    fn original_portal_exit_bas_plays_its_nine_cues_at_compressed_timing() {
        let Some((assets, _)) = fixture() else { eprintln!("SKIP: Sunshine audio absent"); return; };
        let mut sys = SoundSystem::new(Arc::new(SoundData::load(&assets.dir).unwrap()));
        let mut scene = SceneAudio::load(&assets.dir, &sys);
        let table = scene.tables.get("demo_gate_out_appear_get").expect("original exit BAS");
        assert_eq!(table.sounds.iter().map(|k| k.id).collect::<Vec<_>>(), (0x195b..=0x1963).rev().collect::<Vec<_>>());
        let mut peak = 0f32;
        let mut buffer = [0.;1600];
        for i in 0..60 {
            let time = i as f32 / 30.;
            scene.portal_exit(&mut sys, (time / crate::portal_cache::REVEAL * 89.).min(89.), Vec3::ZERO);
            sys.frame(); sys.render(&mut buffer,24000.);
            assert!(buffer.iter().all(|x|x.is_finite()));
            peak = buffer.iter().map(|x|x.abs()).fold(peak,f32::max);
        }
        assert!(peak > 0.001,"portal exit cue sequence is silent");
    }

    #[test]
    fn held_fludd_output_has_headroom_at_device_rates() {
        let Some((assets, _)) = fixture() else {
            return;
        };
        let data = Arc::new(SoundData::load(&assets.dir).unwrap());
        for rate in [32000, 48000] {
            let mut sys = SoundSystem::new(data.clone());
            let mut scene = SceneAudio::load(&assets.dir, &sys);
            let mario = Mario::new(
                assets.params.clone(),
                assets.anim_table.clone(),
                Vec3::ZERO,
                0,
            );
            let cue = Event::Sound(SoundCue {
                name: "MSD_SE_PO_HOVER",
                pos: Vec3::ZERO,
                info: None,
            });
            let mut buffer = vec![0.0; rate / 30 * 2];
            let mut peak = 0.0f32;
            let mut clipped = 0;
            for _ in 0..120 {
                scene.events(&mut sys, &[cue], &mario);
                sys.frame();
                sys.render(&mut buffer, rate as f32);
                assert!(buffer.iter().all(|v| v.is_finite()));
                peak = peak.max(buffer.iter().map(|v| v.abs()).fold(0.0, f32::max));
                clipped += buffer.iter().filter(|v| v.abs() >= 0.999).count();
            }
            eprintln!("hover {rate} Hz: peak {peak}, clipped {clipped}");
            assert!(peak > 0.001, "silent hover loop");
            assert_eq!(clipped, 0, "hover loop clips before host volume is applied");
        }
    }

    #[test]
    fn simulated_fludd_mix_has_headroom() {
        use libsms::{Host, Pad, Sim, water::WaterManager};
        struct Floor(Vec<Event>);
        impl Host for Floor {
            fn ground(&self, p: Vec3) -> Option<libsms::GroundHit> {
                Host::ground(&libsms::FlatHost::default(), p)
            }
            fn event(&mut self, e: Event) {
                self.0.push(e);
            }
        }
        let Some((assets, _)) = fixture() else {
            return;
        };
        let data = Arc::new(SoundData::load(&assets.dir).unwrap());
        for nozzle in [NozzleKind::Spray, NozzleKind::Hover] {
            let mut sys = SoundSystem::new(data.clone());
            let mut scene = SceneAudio::load(&assets.dir, &sys);
            let mut sim = Sim::new(
                assets.params.clone(),
                assets.anim_table.clone(),
                Vec3::ZERO,
                0,
            );
            // Shared RoR2 output reference (-12 dB) at full game master/SFX.
            // No per-nozzle gain or treble filter is applied.
            sys.engine.output_gain = 10.0f32.powf(-12.0 / 20.0);
            sim.mario.give_fludd();
            let params = sim.mario.params.clone();
            let fludd = sim.mario.fludd.as_mut().unwrap();
            fludd.change_nozzle(nozzle, false, &params);
            fludd.refill(&params);
            let mut water = WaterManager::new(&params);
            let mut host = Floor(Vec::new());
            let mut peak = 0.0f32;
            let mut clipped = 0;
            let mut emits = 0;
            let mut buffer = [0.0; 3200];
            for frame in 0..180 {
                let pad = Pad {
                    r_analog: 200,
                    a: nozzle == NozzleKind::Hover && frame < 5,
                    ..Pad::default()
                };
                sim.advance_observed(&mut host, &pad, 0, 1.0 / 30.0, |mario, host| {
                    let events = std::mem::take(&mut host.0);
                    for e in &events {
                        if let Event::WaterEmit(w) = e {
                            water.emit(w);
                            emits += 1;
                        }
                    }
                    scene.events(&mut sys, &events, mario);
                    scene.tick(&mut sys, mario);
                });
                water.step(&host, sim.mario.pos);
                water_sounds(&mut sys, &water.events);
                water.events.clear();
                sys.render(&mut buffer, 48000.0);
                assert!(buffer.iter().all(|v| v.is_finite()));
                peak = peak.max(buffer.iter().map(|v| v.abs()).fold(0.0, f32::max));
                clipped += buffer.iter().filter(|v| v.abs() >= 0.999).count();
            }
            eprintln!("full {nozzle:?}: emits {emits}, peak {peak}, clipped {clipped}");
            assert!(emits > 0 && peak > 0.001, "no audible nozzle activity");
            assert_eq!(clipped, 0, "full {nozzle:?} mix clips before host gain");
        }
    }

    #[test]
    fn a_host_objects_hum_is_started_once_kept_while_asked_for_and_fades() {
        let Some((assets, mut sys)) = fixture() else {
            return;
        };
        let mut scene = SceneAudio::load(&assets.dir, &sys);
        let at = Vec3::new(0.0, 100.0, 0.0);
        assert!(scene.hold(&mut sys, "MSD_SE_SHINE_EXIST", at), "the game has the Shine's hum");
        let handle = scene.hums["MSD_SE_SHINE_EXIST"];
        let mut buffer = [0.0; 1600];
        let mut loud = 0.0f32;
        for _ in 0..40 {
            assert!(scene.hold(&mut sys, "MSD_SE_SHINE_EXIST", at));
            sys.frame();
            sys.render(&mut buffer, 24000.0);
            loud = loud.max(buffer.iter().fold(0.0, |m, v| m.max(v.abs())));
        }
        assert_eq!(scene.hums["MSD_SE_SHINE_EXIST"], handle, "asking again kept the same sound");
        assert!(loud > 0.001, "it hums: {loud}");
        for _ in 0..60 {
            sys.frame();
            sys.render(&mut buffer, 24000.0);
        }
        assert!(!sys.is_playing(handle), "it fades once nobody asks for it");
        assert!(!scene.hold(&mut sys, "MSD_SE_NOT_A_SOUND", at));
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
