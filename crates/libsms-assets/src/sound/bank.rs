//! Instrument banks (IBNK) and wave banks (WSYS), as embedded in `msound.aaf`.
//!
//! - `BNKParser::createBasicBank` (JASBNKParser.cpp): 128 instrument slots at 0x24, 12
//!   percussion sets at 0x3B4 (programs 0xE4-0xEF); pointers are offsets from the chunk start,
//!   zero for none. The bank's virtual id at +8 is what sequences select it by
//!   (`BankMgr::registBankBNK`, JASBankMgr.cpp).
//! - `WSParser::createBasicWaveBank` (JASWSParser.cpp): `WINF` lists one wave archive (`.aw`
//!   file) per group, `WBCT` one control scene per group whose `C-DF` list gives each wave's id
//!   (the low half of its first word). A note's velocity region names a wave by that id; the
//!   bank's wave bank maps it to the archive and wave (`TBasicWaveBank::incWaveTable`).

use std::collections::HashMap;

use crate::Error;
use crate::reader::Be;

/// A chunk-relative pointer at `at` (`None` for null).
fn ptr(r: Be, at: usize) -> Result<Option<usize>, Error> {
    let p = r.u32(at)? as usize;
    if p == 0 {
        return Ok(None);
    }
    if p >= r.0.len() {
        return Err(Error::Bad(format!(
            "pointer {p:#x} at {at:#x} outside chunk"
        )));
    }
    Ok(Some(p))
}

fn magic(r: Be, at: usize, want: &[u8; 4]) -> Result<(), Error> {
    if r.bytes(at, 4)? == want {
        Ok(())
    } else {
        Err(Error::Bad(format!(
            "expected {} at {at:#x}",
            String::from_utf8_lossy(want)
        )))
    }
}

// ---------------------------------------------------------------- WSYS

/// A wave bank: wave archives (groups) and an id -> wave table.
#[derive(Debug, Clone)]
pub struct WaveBank {
    pub groups: Vec<WaveGroup>,
    /// Wave id -> every (group, wave) that defines it, in group order.
    ids: HashMap<u16, Vec<(usize, usize)>>,
}

#[derive(Debug, Clone)]
pub struct WaveGroup {
    /// The archive's file under `AudioRes/Banks/`.
    pub file: String,
    pub waves: Vec<Wave>,
    /// `C-DF` ids, parallel to `waves`.
    pub ids: Vec<u16>,
}

/// `TWave` / `Driver::Wave_` (JASWSParser.hpp).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Wave {
    /// 0 = 4-bit ADPCM, 1 = 2-bit ADPCM, 2 = PCM8, 3 = PCM16.
    pub format: u8,
    /// The key it was recorded at (`gateOn`: `key + 0x3C - mKey`).
    pub key: u8,
    pub sample_rate: f32,
    /// Byte offset and length inside the archive file.
    pub offset: u32,
    pub length: u32,
    pub looping: bool,
    /// Loop start in samples and the decoder history there.
    pub loop_start: u32,
    pub loop_hist: [i16; 2],
    /// Loop end, or the sample count when not looping.
    pub end: u32,
}

impl WaveBank {
    pub fn parse(d: &[u8]) -> Result<Self, Error> {
        let r = Be(d);
        magic(r, 0, b"WSYS")?;
        let winf = ptr(r, 0x10)?.ok_or(Error::Bad("WSYS without WINF".into()))?;
        let wbct = ptr(r, 0x14)?.ok_or(Error::Bad("WSYS without WBCT".into()))?;
        magic(r, winf, b"WINF")?;
        magic(r, wbct, b"WBCT")?;
        let count = r.u32(wbct + 8)? as usize;
        let mut groups = Vec::with_capacity(count);
        let mut ids: HashMap<u16, Vec<(usize, usize)>> = HashMap::new();
        for g in 0..count {
            let scene = ptr(r, wbct + 0xC + g * 4)?.ok_or(Error::Bad("null scene".into()))?;
            let ctrl = ptr(r, scene + 0xC)?.ok_or(Error::Bad("scene without C-DF".into()))?;
            let arc = ptr(r, winf + 8 + g * 4)?.ok_or(Error::Bad("null archive".into()))?;
            let file = r.cstr(arc)?;
            let n = r.u32(ctrl + 4)? as usize;
            let mut waves = Vec::with_capacity(n);
            let mut group_ids = Vec::with_capacity(n);
            for k in 0..n {
                let w = ptr(r, arc + 0x74 + k * 4)?.ok_or(Error::Bad("null wave".into()))?;
                let cw = ptr(r, ctrl + 8 + k * 4)?.ok_or(Error::Bad("null wave id".into()))?;
                let id = r.u32(cw)? as u16;
                waves.push(Wave {
                    format: r.u8(w + 1)?,
                    key: r.u8(w + 2)?,
                    sample_rate: r.f32(w + 4)?,
                    offset: r.u32(w + 8)?,
                    length: r.u32(w + 0xC)?,
                    looping: r.u32(w + 0x10)? != 0,
                    loop_start: r.u32(w + 0x18)?,
                    end: r.u32(w + 0x1C)?,
                    loop_hist: [r.i16(w + 0x20)?, r.i16(w + 0x22)?],
                });
                group_ids.push(id);
                ids.entry(id).or_default().push((g, k));
            }
            groups.push(WaveGroup {
                file,
                waves,
                ids: group_ids,
            });
        }
        Ok(WaveBank { groups, ids })
    }

    /// The wave an id names. The game loads one scene group at a time on top of the resident
    /// ones (`MSound::loadWave`); `prefer` picks which group wins when several define the id.
    pub fn find(&self, id: u16, prefer: Option<usize>) -> Option<(usize, usize)> {
        let hits = self.ids.get(&id)?;
        prefer
            .and_then(|p| hits.iter().find(|h| h.0 == p))
            .or(hits.first())
            .copied()
    }

    pub fn wave(&self, at: (usize, usize)) -> Option<&Wave> {
        self.groups.get(at.0)?.waves.get(at.1)
    }
}

// ---------------------------------------------------------------- IBNK

/// An instrument bank: 240 program slots. 0x00-0x7F are instruments, 0xE4-0xEF percussion sets.
#[derive(Debug, Clone)]
pub struct Ibnk {
    /// The id sequences select the bank by (`setVir2PhyTable`).
    pub virtual_id: u32,
    pub programs: Vec<Option<Program>>,
}

#[derive(Debug, Clone)]
pub enum Program {
    Inst(Box<Inst>),
    Perc(Box<Perc>),
}

/// `TInst` -> `TBasicInst`.
#[derive(Debug, Clone)]
pub struct Inst {
    pub volume: f32,
    pub pitch: f32,
    pub oscs: [Option<Osc>; 2],
    pub rands: [Option<Rand>; 2],
    pub sensors: [Option<Sense>; 2],
    pub keys: Vec<KeyRegion>,
}

/// `TKeymap`: used for keys up to `base_key`.
#[derive(Debug, Clone)]
pub struct KeyRegion {
    pub base_key: u8,
    pub vels: Vec<Vmap>,
}

/// `TVmap`: used for velocities up to `base_vel`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vmap {
    pub base_vel: u8,
    /// Wave id in the bank's wave bank (`JSULoHalf`).
    pub wave_id: u16,
    pub volume: f32,
    pub pitch: f32,
}

/// `TOsc` -> `TOscillator::Osc_`: an envelope or LFO. Tables are (mode, time, value) triplets;
/// a mode above 0xA ends the table (`getOscTableEndPtr`).
#[derive(Debug, Clone, PartialEq)]
pub struct Osc {
    /// What it modulates (`mTarget`): 0 volume, 1 pitch, 2 pan, 3 fx, 4 dolby.
    pub target: u8,
    pub rate: f32,
    pub attack: Option<Vec<[i16; 3]>>,
    pub release: Option<Vec<[i16; 3]>>,
    pub width: f32,
    pub vertex: f32,
}

/// `TRand` -> `TInstRand`: a random offset applied to one parameter at note-on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rand {
    pub target: u8,
    pub base: f32,
    pub width: f32,
}

/// `TSense` -> `TInstSense`: a parameter scaled by key or velocity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sense {
    pub target: u8,
    pub source: u8,
    pub center: u8,
    pub low: f32,
    pub high: f32,
}

/// `TPerc` -> `TDrumSet`: one entry per key. 'PER2' sets carry pan and release per key.
#[derive(Debug, Clone)]
pub struct Perc {
    pub keys: Vec<Option<PercKey>>,
    /// Pan per key, 0..1 (`mPan[j] / 127`); 0.5 when the set has none.
    pub pan: Vec<f32>,
    /// Release time per key; 1000 when the set has none.
    pub release: Vec<u16>,
}

/// `TPmap` -> `TDrumSet::TPerc`.
#[derive(Debug, Clone)]
pub struct PercKey {
    pub volume: f32,
    pub pitch: f32,
    pub rands: [Option<Rand>; 2],
    pub vels: Vec<Vmap>,
}

impl Ibnk {
    pub fn parse(d: &[u8]) -> Result<Self, Error> {
        let r = Be(d);
        magic(r, 0, b"IBNK")?;
        let virtual_id = r.u32(8)?;
        let vmap = |at: usize| -> Result<Vmap, Error> {
            Ok(Vmap {
                base_vel: r.u8(at)?,
                wave_id: r.u32(at + 4)? as u16,
                volume: r.f32(at + 8)?,
                pitch: r.f32(at + 0xC)?,
            })
        };
        let vels = |count_at: usize, list_at: usize| -> Result<Vec<Vmap>, Error> {
            let n = r.u32(count_at)? as usize;
            if n > 128 {
                return Err(Error::Bad(format!("{n} velocity regions at {count_at:#x}")));
            }
            (0..n)
                .filter_map(|k| ptr(r, list_at + k * 4).transpose())
                .map(|p| vmap(p?))
                .collect()
        };
        let rand_at = |at: Option<usize>| -> Result<Option<Rand>, Error> {
            let Some(p) = at else {
                return Ok(None);
            };
            Ok(Some(Rand {
                target: r.u8(p)?,
                base: r.f32(p + 4)?,
                width: r.f32(p + 8)?,
            }))
        };
        let table = |at: Option<usize>| -> Result<Option<Vec<[i16; 3]>>, Error> {
            let Some(mut at) = at else {
                return Ok(None);
            };
            let mut t = Vec::new();
            loop {
                let e = [r.i16(at)?, r.i16(at + 2)?, r.i16(at + 4)?];
                t.push(e);
                at += 6;
                if e[0] > 0xA || t.len() > 256 {
                    break;
                }
            }
            Ok(Some(t))
        };
        let mut programs = vec![None; 0xF0];
        for (i, slot) in programs.iter_mut().enumerate().take(0x80) {
            let Some(p) = ptr(r, 0x24 + i * 4)? else {
                continue;
            };
            let osc = |k: usize| -> Result<Option<Osc>, Error> {
                let Some(o) = ptr(r, p + 0x10 + k * 4)? else {
                    return Ok(None);
                };
                Ok(Some(Osc {
                    target: r.u8(o)?,
                    rate: r.f32(o + 4)?,
                    attack: table(ptr(r, o + 8)?)?,
                    release: table(ptr(r, o + 0xC)?)?,
                    width: r.f32(o + 0x10)?,
                    vertex: r.f32(o + 0x14)?,
                }))
            };
            let sense = |k: usize| -> Result<Option<Sense>, Error> {
                let Some(s) = ptr(r, p + 0x20 + k * 4)? else {
                    return Ok(None);
                };
                Ok(Some(Sense {
                    target: r.u8(s)?,
                    source: r.u8(s + 1)?,
                    center: r.u8(s + 2)?,
                    low: r.f32(s + 4)?,
                    high: r.f32(s + 8)?,
                }))
            };
            let nkeys = r.u32(p + 0x28)? as usize;
            if nkeys > 128 {
                return Err(Error::Bad(format!("{nkeys} key regions at {p:#x}")));
            }
            let keys = (0..nkeys)
                .filter_map(|k| ptr(r, p + 0x2C + k * 4).transpose())
                .map(|k| {
                    let k = k?;
                    Ok(KeyRegion {
                        base_key: r.u8(k)?,
                        vels: vels(k + 4, k + 8)?,
                    })
                })
                .collect::<Result<_, Error>>()?;
            *slot = Some(Program::Inst(Box::new(Inst {
                volume: r.f32(p + 8)?,
                pitch: r.f32(p + 0xC)?,
                oscs: [osc(0)?, osc(1)?],
                rands: [rand_at(ptr(r, p + 0x18)?)?, rand_at(ptr(r, p + 0x1C)?)?],
                sensors: [sense(0)?, sense(1)?],
                keys,
            })));
        }
        for i in 0..12 {
            let Some(p) = ptr(r, 0x3B4 + i * 4)? else {
                continue;
            };
            let per2 = r.bytes(p, 4)? == b"PER2";
            let keys = (0..128)
                .map(|k| {
                    let Some(pk) = ptr(r, p + 0x88 + k * 4)? else {
                        return Ok(None);
                    };
                    Ok(Some(PercKey {
                        volume: r.f32(pk)?,
                        pitch: r.f32(pk + 4)?,
                        rands: [rand_at(ptr(r, pk + 8)?)?, rand_at(ptr(r, pk + 0xC)?)?],
                        vels: vels(pk + 0x10, pk + 0x14)?,
                    }))
                })
                .collect::<Result<_, Error>>()?;
            // `TDrumSet::TPerc::TPerc` defaults.
            let (mut pan, mut release) = (vec![0.5; 128], vec![1000; 128]);
            if per2 {
                for k in 0..128 {
                    pan[k] = r.u8(p + 0x288 + k)? as i8 as f32 / 127.0;
                    release[k] = r.u16(p + 0x308 + k * 2)?;
                }
            }
            programs[0xE4 + i] = Some(Program::Perc(Box::new(Perc { keys, pan, release })));
        }
        Ok(Ibnk {
            virtual_id,
            programs,
        })
    }

    pub fn inst(&self, program: u8) -> Option<&Inst> {
        match self.programs.get(program as usize)? {
            Some(Program::Inst(i)) => Some(i),
            _ => None,
        }
    }

    pub fn perc(&self, program: u8) -> Option<&Perc> {
        match self.programs.get(program as usize)? {
            Some(Program::Perc(p)) => Some(p),
            _ => None,
        }
    }
}

impl Inst {
    /// `TBasicInst::getParam`: the first key region whose top key covers `key`.
    pub fn key_region(&self, key: u8) -> Option<&KeyRegion> {
        self.keys.iter().find(|k| key <= k.base_key)
    }
}

/// The first velocity region whose top velocity covers `vel`.
pub fn vmap_for(vels: &[Vmap], vel: u8) -> Option<&Vmap> {
    vels.iter().find(|v| vel <= v.base_vel)
}
