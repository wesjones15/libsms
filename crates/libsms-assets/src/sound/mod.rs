//! Super Mario Sunshine's sound data (JAudio, the JSystem C++ generation): the init data with
//! the instrument and wave banks and the sound table, AFC-compressed waves, the BMS sequence
//! archive, the title stream and the sound names. Playing them is `libsms-audio`'s job; this
//! module only parses and decodes.
//! Animation cues (`bas/ma_<clip>.bas`) are exposed as [`AnimSoundTable`]; the
//! matching `bck/ma_<clip>.bck` stem is confirmed by `TMario::initModel` in MarioDraw.cpp.
//!
//! ```text
//! assets/AudioRes/msound.aaf          init data (nintendo.szs:audi/msound.aaf)
//! assets/AudioRes/mSound.asn          sound names
//! assets/AudioRes/Banks/*.aw          wave archives
//! assets/AudioRes/Seqs/sequence.arc   sequences
//! assets/AudioRes/Streams/*.afc       streams
//! ```

pub mod aaf;
pub mod adpcm;
pub mod bank;
pub mod barc;
pub mod bas;
pub mod names;
pub mod stream;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub use aaf::{CategoryInfo, InitData, SoundInfo, SoundTable};
pub use bank::{Ibnk, Inst, Osc, Perc, Program, Wave, WaveBank};
pub use barc::SeqArchive;
pub use bas::{AnimSound, AnimSoundTable};
pub use names::SoundNames;
pub use stream::Stream;

use crate::Error;

pub const INIT_DATA: &str = "AudioRes/msound.aaf";
pub const NAMES: &str = "AudioRes/mSound.asn";
pub const SEQ_ARC: &str = "AudioRes/Seqs/sequence.arc";
pub const BANKS_DIR: &str = "AudioRes/Banks";
pub const STREAMS_DIR: &str = "AudioRes/Streams";

/// Pikmin 2's files (`libsms::extract::p2::extract_audio`): the same JAudio layout, its init data, names and sequence
/// archive under other names.
pub const P2_INIT_DATA: &str = "AudioRes/PSound.aaf";
pub const P2_NAMES: &str = "AudioRes/PSound.asn";
pub const P2_SEQ_ARC: &str = "AudioRes/Seqs/Seq.arc";
/// The sequence that plays every sound effect.
pub const P2_SE_SEQUENCE: &str = "se.bms";

/// (wave bank, group, wave)
pub type WaveKey = (usize, usize, usize);

/// Everything the sound engine reads, plus lazily decoded waves.
pub struct SoundData {
    pub init: InitData,
    pub seqs: SeqArchive,
    pub names: SoundNames,
    dir: PathBuf,
    files: Mutex<HashMap<String, Arc<Vec<u8>>>>,
    pcm: Mutex<HashMap<WaveKey, Arc<Vec<i16>>>>,
    /// Pitch for the voice cues (SE category 7, Mario's `_MV` lines): 1.0 is the disc's own.
    /// Baked into the PCM once, with the cue's length kept (resample for the pitch, then a
    /// WSOLA pass for the duration), so a pitched-up voice is not faster.
    voice_pitch: Mutex<f32>,
    /// A voice cue was just started: the next one-shot wave decoded is its voice.
    voice_pending: AtomicBool,
}

impl SoundData {
    /// Loads from an extracted `assets/` folder.
    pub fn load(assets: &Path) -> Result<Self, Error> {
        let read = |rel: &str| {
            let p = assets.join(rel);
            std::fs::read(&p).map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => Error::NotFound(p),
                _ => Error::Io(e),
            })
        };
        let init = InitData::parse(&read(INIT_DATA)?)?;
        let seqs = SeqArchive::parse(&init.seq_header, read(SEQ_ARC)?)?;
        // The names are a convenience; a missing file only loses lookups by name.
        let names = read(NAMES)
            .ok()
            .and_then(|d| SoundNames::parse(&d).ok())
            .unwrap_or_default();
        Ok(SoundData {
            init,
            seqs,
            names,
            dir: assets.to_path_buf(),
            files: Mutex::default(),
            pcm: Mutex::default(),
            voice_pitch: Mutex::new(1.0),
            voice_pending: AtomicBool::new(false),
        })
    }

    /// Pikmin 2's sound data from the folder `assets` (`AudioRes/PSound.aaf`, `Seqs/Seq.arc`, `Banks/wScene_N.aw`).
    /// Sound effects are ids in the table as in Sunshine (`SoundID.h` in the decomp: `PSSE_PL_DAMAGE_LUI = 0x891`).
    pub fn load_pikmin2(assets: &Path) -> Result<Self, Error> {
        let read = |rel: &str| {
            let p = assets.join(rel);
            std::fs::read(&p).map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => Error::NotFound(p),
                _ => Error::Io(e),
            })
        };
        let init = InitData::parse(&read(P2_INIT_DATA)?)?;
        let arc = libsms::extract::rarc::Archive::parse(read(P2_SEQ_ARC)?)
            .map_err(|e| Error::Bad(format!("Seq.arc: {e}")))?;
        let seqs = SeqArchive::from_rarc(&arc, P2_SE_SEQUENCE)?;
        let names = read(P2_NAMES).ok().and_then(|d| SoundNames::parse(&d).ok()).unwrap_or_default();
        Ok(SoundData {
            init,
            seqs,
            names,
            dir: assets.to_path_buf(),
            files: Mutex::default(),
            pcm: Mutex::default(),
            voice_pitch: Mutex::new(1.0),
            voice_pending: AtomicBool::new(false),
        })
    }

    pub fn wave(&self, key: WaveKey) -> Option<&Wave> {
        self.init.wave_banks.get(key.0)?.bank.wave((key.1, key.2))
    }

    /// Pitch the voice cues (`_MV`, SE category 7) by `ratio`, keeping their length. Clears
    /// the decoded-wave cache so cues decoded before this call bake at the new pitch.
    pub fn set_voice_pitch(&self, ratio: f32) {
        *self.voice_pitch.lock().unwrap() = if ratio > 0.0 { ratio } else { 1.0 };
        self.pcm.lock().unwrap().clear();
    }

    pub fn voice_pitch(&self) -> f32 {
        *self.voice_pitch.lock().unwrap()
    }

    /// A voice cue is starting: the next one-shot wave decoded is its voice.
    pub fn mark_voice(&self) {
        self.voice_pending.store(true, Ordering::Relaxed);
    }

    /// A wave decoded to 16-bit PCM (cached).
    pub fn pcm(&self, key: WaveKey) -> Result<Arc<Vec<i16>>, Error> {
        if let Some(p) = self.pcm.lock().unwrap().get(&key) {
            return Ok(p.clone());
        }
        let bank = &self
            .init
            .wave_banks
            .get(key.0)
            .ok_or(Error::Bad(format!("no wave bank {}", key.0)))?
            .bank;
        let group = bank
            .groups
            .get(key.1)
            .ok_or(Error::Bad(format!("no wave group {}", key.1)))?;
        let w = group
            .waves
            .get(key.2)
            .ok_or(Error::Bad(format!("no wave {}", key.2)))?;
        let file = self.file(&group.file)?;
        let start = w.offset as usize;
        let bytes = file
            .get(start..start + w.length as usize)
            .ok_or(Error::Truncated(start))?;
        let pcm = adpcm::decode_wave(w.format, bytes, w.end as usize);
        // A voice cue's wave decoded right after `mark_voice`: bake the pitch in, same
        // length, so the line is not faster. Looping waves (music) never take this path.
        let ratio = self.voice_pitch();
        let pcm = if !w.looping && ratio != 1.0 && self.voice_pending.swap(false, Ordering::Relaxed)
        {
            shift_voice(&pcm, ratio)
        } else {
            pcm
        };
        let pcm = Arc::new(pcm);
        self.pcm.lock().unwrap().insert(key, pcm.clone());
        Ok(pcm)
    }

    fn file(&self, name: &str) -> Result<Arc<Vec<u8>>, Error> {
        let mut files = self.files.lock().unwrap();
        if let Some(f) = files.get(name) {
            return Ok(f.clone());
        }
        let p = self.dir.join(BANKS_DIR).join(name);
        let f = Arc::new(std::fs::read(&p).map_err(|_| Error::NotFound(p))?);
        files.insert(name.to_string(), f.clone());
        Ok(f)
    }

    /// A stream by file name (e.g. `title.afc`).
    pub fn stream(&self, name: &str) -> Result<Stream, Error> {
        let p = self.dir.join(STREAMS_DIR).join(name);
        Stream::decode(&std::fs::read(&p).map_err(|_| Error::NotFound(p))?)
    }
}

/// One voice cue, pitched by `ratio` with its length kept: resample by `ratio` (the pitch),
/// then a WSOLA overlap-add pass back to the original sample count (the duration). Plain
/// resampling alone would make the line `ratio` times faster, which is what the GameCube's
/// pitch parameter does; this decouples the two.
pub fn shift_voice(samples: &[i16], ratio: f32) -> Vec<i16> {
    if samples.len() < 2 || ratio <= 0.0 || ratio == 1.0 {
        return samples.to_vec();
    }
    let x: Vec<f32> = samples.iter().map(|s| *s as f32 / 32768.0).collect();
    let n = (x.len() as f32 / ratio).floor() as usize;
    let mut y = Vec::with_capacity(n);
    let mut t = 0.0f32;
    for _ in 0..n {
        let i = t as usize;
        if i + 1 >= x.len() {
            break;
        }
        let f = t - i as f32;
        y.push(x[i] * (1.0 - f) + x[i + 1] * f);
        t += ratio;
    }
    let out = wsola(&y, x.len());
    out.into_iter()
        .map(|v| (v.clamp(-1.0, 1.0) * 32767.0) as i16)
        .collect()
}

/// Overlap-add time stretch to `target` samples: 512-sample Hann windows at half overlap, the
/// analysis offset searched in a small window so successive grains stay in phase.
fn wsola(y: &[f32], target: usize) -> Vec<f32> {
    const WIN: usize = 512;
    const HOP: usize = WIN / 2;
    const SEARCH: usize = 96;
    if y.len() <= WIN {
        let mut out = y.to_vec();
        out.resize(target, 0.0);
        return out;
    }
    let win: Vec<f32> = (0..WIN)
        .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / WIN as f32).cos())
        .collect();
    let analyse_hop = HOP as f32 * (y.len() as f32 / target as f32);
    let mut out = vec![0.0f32; target + WIN];
    let mut wsum = vec![0.0f32; target + WIN];
    let mut pos = 0.0f32;
    let mut o = 0usize;
    let mut prev: Vec<f32> = Vec::new();
    while o + WIN <= target + WIN && (pos as usize) + SEARCH + WIN < y.len() {
        let mut best = 0i32;
        if !prev.is_empty() {
            let mut best_score = f32::NEG_INFINITY;
            for lag in -(SEARCH as i32)..=(SEARCH as i32) {
                let at = pos as i32 + lag;
                if at < 0 || (at as usize) + HOP >= y.len() {
                    continue;
                }
                let score: f32 = (0..HOP)
                    .map(|i| prev[i] * y[at as usize + i])
                    .sum();
                if score > best_score {
                    best_score = score;
                    best = lag;
                }
            }
        }
        let p = (pos as i32 + best).max(0) as usize;
        if p + WIN >= y.len() {
            break;
        }
        for i in 0..WIN {
            out[o + i] += y[p + i] * win[i];
            wsum[o + i] += win[i];
        }
        prev = y[p + HOP..p + HOP + HOP].to_vec();
        pos += analyse_hop;
        o += HOP;
    }
    let mut res: Vec<f32> = out
        .iter()
        .zip(&wsum)
        .map(|(s, w)| if *w > 0.25 { s / *w } else { 0.0 })
        .collect();
    res.truncate(target);
    res
}
