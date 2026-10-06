//! `AudioRes/mSound.asn`: the names of the sound ids (`MSD_SE_MA_JUMP`, `MSD_BGM_DOLPIC`, ...).
//! `JAIAsnData` (JAIAsnData.cpp) only declares it; the layout is read from the file: a 0x10-byte
//! header, then 0x20-byte records of a NUL-padded name (0x1C bytes) and two u16s. The first 18
//! records describe the categories (count, first index); the rest are sounds, whose two halves
//! form the id (the high half's low bits are flags; only the type bits 0xC000 belong to it).

use std::collections::HashMap;

use crate::Error;
use crate::reader::Be;

#[derive(Debug, Clone, Default)]
pub struct SoundNames {
    by_name: HashMap<String, u32>,
    by_id: HashMap<u32, String>,
}

impl SoundNames {
    pub fn parse(d: &[u8]) -> Result<Self, Error> {
        let r = Be(d);
        let mut names = SoundNames::default();
        let records = d.len().saturating_sub(0x10) / 0x20;
        for i in 18..records {
            let o = 0x10 + i * 0x20;
            let raw = r.bytes(o, 0x1C)?;
            let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
            let name = String::from_utf8_lossy(&raw[..end]);
            if name.is_empty() || name.starts_with('(') {
                continue;
            }
            let id = ((r.u16(o + 0x1C)? as u32 & 0xC000) << 16) | r.u16(o + 0x1E)? as u32;
            names.by_name.insert(name.to_string(), id);
            names.by_id.entry(id).or_insert_with(|| name.to_string());
        }
        Ok(names)
    }

    /// The id of `MSD_...`.
    pub fn id(&self, name: &str) -> Option<u32> {
        self.by_name.get(name).copied()
    }

    pub fn name(&self, id: u32) -> Option<&str> {
        self.by_id.get(&id).map(String::as_str)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, u32)> {
        self.by_name.iter().map(|(n, &i)| (n.as_str(), i))
    }

    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }
}
