//! Super Mario Sunshine's sound data (JAudio, the JSystem C++ generation): the init data with
//! the instrument and wave banks and the sound table, AFC-compressed waves, the BMS sequence
//! archive, the title stream and the sound names. Playing them is `libsms-audio`'s job; this
//! module only parses and decodes.
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
pub mod names;
pub mod stream;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub use aaf::{CategoryInfo, InitData, SoundInfo, SoundTable};
pub use bank::{Ibnk, Inst, Osc, Perc, Program, Wave, WaveBank};
pub use barc::SeqArchive;
pub use names::SoundNames;
pub use stream::Stream;

use crate::Error;

pub const INIT_DATA: &str = "AudioRes/msound.aaf";
pub const NAMES: &str = "AudioRes/mSound.asn";
pub const SEQ_ARC: &str = "AudioRes/Seqs/sequence.arc";
pub const BANKS_DIR: &str = "AudioRes/Banks";
pub const STREAMS_DIR: &str = "AudioRes/Streams";

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
        })
    }

    pub fn wave(&self, key: WaveKey) -> Option<&Wave> {
        self.init.wave_banks.get(key.0)?.bank.wave((key.1, key.2))
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
        let pcm = Arc::new(adpcm::decode_wave(w.format, bytes, w.end as usize));
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
