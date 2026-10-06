//! `AudioRes/msound.aaf`: JAudio's init data (`JAIBasic::checkInitDataOnMemory`,
//! JAIBasic.cpp). A list of u32 commands, each followed by its arguments, ending in 0:
//!
//! | cmd | args | contents |
//! |-----|------|----------|
//! | 1 | (offset, size, flag) | sound table(s); flag 0 = one table for SE, sequences and streams |
//! | 2 | (offset, size, wave bank)* 0 | instrument banks (IBNK) and the wave bank each uses |
//! | 3 | (offset, size, load timing)* 0 | wave banks (WSYS); timing 0/1 = resident, 2 = per scene |
//! | 4 | (offset, size, _) | the sequence archive's `BARC` header |
//! | 5 | (offset, size, _) | the stream list |
//! | 6 | (offset, size, _) | sound scenes: per SE category, how many play at once |
//! | 7 | (offset, size, _) | effect (reverb) scenes |
//! | 8 | (offset, size, _) | unused by Sunshine's code |
//!
//! Offsets are from the start of the file.

use crate::Error;
use crate::reader::Be;
use crate::sound::bank::{Ibnk, WaveBank};

/// `JAISoundInfo` (JAIData.hpp): one entry of the sound table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoundInfo {
    /// `JAISeSwBit` / `JAISeqSwBit` flags.
    pub sw_bit: u32,
    pub priority: u8,
    /// SE: unused. Sequences: which `mDefaultSeqHandle` slot plays it.
    pub track: u8,
    /// Indirect ids (format bit 0) play `id & !0x3FF | offset_no` instead.
    pub offset_no: u16,
    pub pitch: f32,
    pub volume: u8,
    pub fxmix: u8,
}

/// `JAISoundID_*` (JAISound.hpp).
pub mod id {
    pub const INDEX_MASK: u32 = 0x3FF;
    pub const TYPE_MASK: u32 = 0xC000_0000;
    pub const TYPE_SE: u32 = 0;
    pub const TYPE_SEQUENCE: u32 = 0x8000_0000;
    pub const TYPE_STREAM: u32 = 0xC000_0000;
    /// SE ids with either bit keep playing until stopped (`mSoundID & 0xC00`).
    pub const SUSTAIN_MASK: u32 = 0xC00;

    /// SE category: bits 12-19 (`(u8)(sound_id >> 12)`).
    pub fn category(id: u32) -> u8 {
        (id >> 12) as u8
    }
}

/// The sound table (`JAIData::initInfoDataWork`): 18 categories, 0-15 for SE, 16 sequences,
/// 17 streams.
#[derive(Debug, Clone)]
pub struct SoundTable {
    /// Info formats for SE, sequences and streams (`getInfoFormat`); bit 0 = indirect ids.
    pub formats: [u8; 3],
    pub categories: Vec<Vec<SoundInfo>>,
}

impl SoundTable {
    fn parse(d: &[u8]) -> Result<Self, Error> {
        let r = Be(d);
        let mut categories = Vec::with_capacity(18);
        for c in 0..18 {
            let count = r.u16(c * 4 + 6)? as usize;
            let first = r.u16(c * 4 + 8)? as usize;
            categories.push(
                (first..first + count)
                    .map(|i| {
                        let o = 0x50 + i * 16;
                        Ok(SoundInfo {
                            sw_bit: r.u32(o)?,
                            priority: r.u8(o + 4)?,
                            track: r.u8(o + 5)?,
                            offset_no: r.u16(o + 6)?,
                            pitch: r.f32(o + 8)?,
                            volume: r.u8(o + 0xC)?,
                            fxmix: r.u8(o + 0xD)?,
                        })
                    })
                    .collect::<Result<_, Error>>()?,
            );
        }
        Ok(SoundTable {
            formats: [r.u8(0)?, r.u8(1)?, r.u8(2)?],
            categories,
        })
    }

    /// `JAIData::getInfoPointer`.
    pub fn info(&self, sound_id: u32) -> Option<&SoundInfo> {
        let category = match sound_id & id::TYPE_MASK {
            id::TYPE_SEQUENCE => 16,
            id::TYPE_STREAM => 17,
            0 => id::category(sound_id) as usize,
            _ => return None,
        };
        self.categories
            .get(category)?
            .get((sound_id & id::INDEX_MASK) as usize)
    }

    /// `getInfoFormat`.
    pub fn format(&self, sound_id: u32) -> u8 {
        match sound_id & id::TYPE_MASK {
            id::TYPE_SEQUENCE => self.formats[1],
            id::TYPE_STREAM => self.formats[2],
            _ => self.formats[0],
        }
    }

    /// How many SE categories are in use (`mCategoryMax`).
    pub fn se_categories(&self) -> usize {
        (0..16)
            .rev()
            .find(|&c| !self.categories[c].is_empty())
            .map_or(0, |c| c + 1)
    }
}

/// `JAICategoryInfo`: per SE category in a sound scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CategoryInfo {
    /// SE tracks the category gets (`mMaxPlaying`).
    pub max_playing: u8,
    pub max_same: u8,
}

/// `DSPInterface::FxlineConfig_`: one echo line of an effect scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FxLine {
    pub enabled: bool,
    /// Two destinations: DSP buffer id (`SEND_TABLE`, JASDSPInterface.cpp) and volume.
    pub out: [(u16, i16); 2],
    /// Delay length in 80-sample DSP frames.
    pub frames: u32,
    pub fir: [i16; 8],
}

/// `DSPInterface::SEND_TABLE`
const SEND_TABLE: [u16; 12] = [
    0x0D00, 0x0D60, 0x0DC0, 0x0E20, 0x0E80, 0x0EE0, 0x0CA0, 0x0F40, 0x0FA0, 0x0B00, 0x09A0, 0x0000,
];

#[derive(Debug, Clone)]
pub struct BankEntry {
    pub bank: Ibnk,
    /// Index into [`InitData::wave_banks`] (`BankMgr::assignWaveBank`).
    pub wave_bank: usize,
}

#[derive(Debug, Clone)]
pub struct WaveBankEntry {
    pub bank: WaveBank,
    /// 0 = first resident load, 1 = second resident load, 2 = loaded per scene.
    pub load_timing: u32,
}

#[derive(Debug, Clone)]
pub struct InitData {
    pub sound_table: SoundTable,
    pub banks: Vec<BankEntry>,
    pub wave_banks: Vec<WaveBankEntry>,
    /// The `BARC` header for `Seqs/sequence.arc`.
    pub seq_header: Vec<u8>,
    /// Stream file names under `AudioRes/Streams/`.
    pub streams: Vec<String>,
    /// Per sound scene, per SE category.
    pub scenes: Vec<Vec<CategoryInfo>>,
    /// Effect scenes (`FabricatedFxSceneTable`): four echo lines each. A line whose buffer
    /// size (`mBufferMax`) is 0 is off.
    pub fx_scenes: Vec<[FxLine; 4]>,
}

impl InitData {
    pub fn parse(d: &[u8]) -> Result<Self, Error> {
        let r = Be(d);
        let chunk =
            |off: u32, size: u32| -> Result<&[u8], Error> { r.bytes(off as usize, size as usize) };
        let mut sound_table = None;
        let mut banks = Vec::new();
        let mut wave_banks = Vec::new();
        let mut seq_header = Vec::new();
        let mut streams = Vec::new();
        let mut scenes = Vec::new();
        let mut fx_scenes = Vec::new();
        let mut i = 0usize;
        let word = |i: usize| r.u32(i * 4);
        loop {
            let cmd = word(i)?;
            i += 1;
            match cmd {
                0 => break,
                1 => {
                    sound_table = Some(SoundTable::parse(chunk(word(i)?, word(i + 1)?)?)?);
                    // A separate seq/stream table pair follows when the flag is set; Sunshine
                    // uses one table.
                    i += if word(i + 2)? != 0 { 6 } else { 3 };
                }
                2 | 3 => {
                    while word(i)? != 0 {
                        let data = chunk(word(i)?, word(i + 1)?)?;
                        let extra = word(i + 2)?;
                        if cmd == 2 {
                            banks.push(BankEntry {
                                bank: Ibnk::parse(data)?,
                                wave_bank: extra as usize,
                            });
                        } else {
                            wave_banks.push(WaveBankEntry {
                                bank: WaveBank::parse(data)?,
                                load_timing: extra,
                            });
                        }
                        i += 3;
                    }
                    i += 1;
                }
                4 => {
                    seq_header = chunk(word(i)?, word(i + 1)?)?.to_vec();
                    i += 3;
                }
                5 => {
                    // `FabricatedStreamEntry`: 0x30 bytes, the file name at +0x10.
                    let list = Be(chunk(word(i)?, word(i + 1)?)?);
                    let mut o = 0;
                    while o + 0x30 <= list.0.len() {
                        let name = list.cstr(o + 0x10)?;
                        if !name.is_empty() {
                            streams.push(name);
                        }
                        o += 0x30;
                    }
                    i += 3;
                }
                6 => {
                    // `FabricatedSoundSceneTable`: count, then table-relative offsets to
                    // `JAICategoryInfo` arrays.
                    let t = Be(chunk(word(i)?, word(i + 1)?)?);
                    let n = t.u32(0)? as usize;
                    let cats = sound_table.as_ref().map_or(16, SoundTable::se_categories);
                    for s in 0..n {
                        let o = t.u32(4 + s * 4)? as usize;
                        scenes.push(
                            (0..cats)
                                .map(|c| {
                                    Ok(CategoryInfo {
                                        max_playing: t.u8(o + c * 2)?,
                                        max_same: t.u8(o + c * 2 + 1)?,
                                    })
                                })
                                .collect::<Result<_, Error>>()?,
                        );
                    }
                    i += 3;
                }
                7 => {
                    // `FabricatedFxSceneTable`: count, four buffer sizes, then table-relative
                    // offsets to four `FxlineConfig_`s per scene.
                    let t = Be(chunk(word(i)?, word(i + 1)?)?);
                    let n = t.u32(0)? as usize;
                    let sizes: Vec<u32> =
                        (0..4).map(|k| t.u32(4 + k * 4)).collect::<Result<_, _>>()?;
                    for sc in 0..n {
                        let o = t.u32(0x14 + sc * 4)? as usize;
                        let mut lines = [FxLine {
                            enabled: false,
                            out: [(0, 0); 2],
                            frames: 0,
                            fir: [0; 8],
                        }; 4];
                        for (k, l) in lines.iter_mut().enumerate() {
                            let c = o + k * 0x20;
                            let send = |at: usize| -> Result<u16, Error> {
                                Ok(SEND_TABLE.get(t.u16(at)? as usize).copied().unwrap_or(0))
                            };
                            *l = FxLine {
                                enabled: t.u8(c)? != 0 && sizes[k] != 0,
                                out: [(send(c + 2)?, t.i16(c + 4)?), (send(c + 6)?, t.i16(c + 8)?)],
                                frames: t.u32(c + 0xC)?,
                                fir: std::array::from_fn(|j| t.i16(c + 0x10 + j * 2).unwrap_or(0)),
                            };
                        }
                        fx_scenes.push(lines);
                    }
                    i += 3;
                }
                8 => i += 3,
                _ => {
                    // Unknown: skip to the next zero word.
                    while word(i)? != 0 {
                        i += 1;
                    }
                    i += 1;
                }
            }
        }
        Ok(InitData {
            sound_table: sound_table.ok_or(Error::Bad("no sound table".into()))?,
            banks,
            wave_banks,
            seq_header,
            streams,
            scenes,
            fx_scenes,
        })
    }

    /// `BankMgr::getPhysicalNumber`: the bank registered with a virtual id.
    pub fn bank_physical(&self, virtual_id: u16) -> Option<usize> {
        self.banks
            .iter()
            .position(|b| b.bank.virtual_id == virtual_id as u32)
    }
}
