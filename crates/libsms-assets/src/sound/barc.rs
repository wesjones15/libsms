//! The sequence archive: `AudioRes/Seqs/sequence.arc` (raw BMS sequences back to back), indexed
//! by the `BARC` header that `msound.aaf` carries (`JASystem::Vload::initHeaderM`, JASVload.cpp).
//!
//! The header is 0x20 bytes ("BARC----", count at 0xC, the archive's file name at 0x10), then
//! one 0x20-byte entry per sequence: name, a redirect at 0xE (0xFFFF = none; `getRealHandle`
//! follows it), offset at 0x18 and size at 0x1C. A sequence sound id's index (`id & 0x3FF`) is
//! its entry (`JAIBasic::loadArcSeqData`): 0 is `se.scom`, the sequence that plays every SE.

use crate::Error;
use crate::reader::Be;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeqEntry {
    pub name: String,
    pub offset: u32,
    pub size: u32,
}

#[derive(Debug, Clone)]
pub struct SeqArchive {
    /// Index by sequence number, redirects resolved; `None` for broken redirects.
    pub entries: Vec<Option<SeqEntry>>,
    pub data: Vec<u8>,
}

impl SeqArchive {
    pub fn parse(header: &[u8], arc: Vec<u8>) -> Result<Self, Error> {
        let r = Be(header);
        if r.bytes(0, 8)? != b"BARC----" {
            return Err(Error::Bad("sequence header is not BARC".into()));
        }
        let count = r.u32(0xC)? as usize;
        let raw = |n: usize| -> Result<(String, u16, u32, u32), Error> {
            let e = 0x20 * (n + 1);
            let name = r.bytes(e, 14)?;
            let end = name.iter().position(|&b| b == 0).unwrap_or(14);
            Ok((
                String::from_utf8_lossy(&name[..end]).into_owned(),
                r.u16(e + 0xE)?,
                r.u32(e + 0x18)?,
                r.u32(e + 0x1C)?,
            ))
        };
        let mut entries = Vec::with_capacity(count);
        for n in 0..count {
            let mut at = n;
            let mut found = None;
            // Follow redirects; cap the chain in case of a loop.
            for _ in 0..count {
                let (name, redirect, offset, size) = raw(at)?;
                if redirect == 0xFFFF {
                    let ok = (offset as usize)
                        .checked_add(size as usize)
                        .is_some_and(|e| e <= arc.len());
                    found = ok.then_some(SeqEntry { name, offset, size });
                    break;
                }
                if redirect as usize >= count {
                    break;
                }
                at = redirect as usize;
            }
            entries.push(found);
        }
        Ok(SeqArchive { entries, data: arc })
    }

    /// Pikmin 2's `Seqs/Seq.arc` is a plain RARC of `.bms` files (no `BARC` header in the init data). The entries are
    /// the files, in the archive's order, with `first` moved to index 0: the sequence that plays every sound effect
    /// (`PSGame.cpp`: `new PSSystem::SeSeq("se.bms", ..)`) sits where Sunshine keeps `se.scom`.
    pub fn from_rarc(arc: &libsms::extract::rarc::Archive, first: &str) -> Result<Self, Error> {
        let mut names: Vec<String> = arc.paths().map(str::to_owned).collect();
        let at = names.iter().position(|n| n == first).ok_or_else(|| Error::Bad(format!("no {first} in the archive")))?;
        let lead = names.remove(at);
        names.insert(0, lead);
        let mut data = Vec::new();
        let mut entries = Vec::with_capacity(names.len());
        for name in names {
            let bytes = arc.file(&name).ok_or_else(|| Error::Bad(format!("{name} is not a file")))?;
            entries.push(Some(SeqEntry { name, offset: data.len() as u32, size: bytes.len() as u32 }));
            data.extend_from_slice(bytes);
        }
        Ok(SeqArchive { entries, data })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The sequence's bytes.
    pub fn seq(&self, n: usize) -> Option<&[u8]> {
        let e = self.entries.get(n)?.as_ref()?;
        self.data
            .get(e.offset as usize..(e.offset + e.size) as usize)
    }

    pub fn name(&self, n: usize) -> Option<&str> {
        Some(self.entries.get(n)?.as_ref()?.name.as_str())
    }
}
