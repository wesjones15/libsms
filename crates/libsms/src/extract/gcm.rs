//! GameCube disc images: plain ISO/GCM and CISO.
//!
//! The disc header holds the 6-character game ID at 0x00, the GameCube magic `C2339F3D` at 0x1C,
//! and the file system table (FST) offset and size at 0x424/0x428. The FST is an array of 12-byte
//! entries followed by a string table: `flags(1) name_offset(3) offset(4) size(4)`, where a
//! directory entry (flags = 1) stores its parent in `offset` and the index one past its last
//! descendant in `size`, and a file entry stores its byte offset and length.
//!
//! CISO is a sparse container: 8-byte header (`"CISO"`, little-endian block size), a presence map,
//! then only the present blocks at 0x8000.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use super::ExtractError;

/// Random access to the disc's logical bytes.
pub trait ReadAt {
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), ExtractError>;
}

pub struct PlainImage {
    file: File,
    len: u64,
}

impl ReadAt for PlainImage {
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), ExtractError> {
        if offset
            .checked_add(buf.len() as u64)
            .is_none_or(|e| e > self.len)
        {
            return Err(ExtractError::Format("read past the end of the disc image"));
        }
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.read_exact(buf)?;
        Ok(())
    }
}

pub struct CisoImage {
    file: File,
    block_size: u64,
    /// For each logical block, the index among stored blocks, or `None` when absent (zeros).
    slot: Vec<Option<u64>>,
}

impl ReadAt for CisoImage {
    fn read_at(&mut self, mut offset: u64, mut buf: &mut [u8]) -> Result<(), ExtractError> {
        while !buf.is_empty() {
            let block = (offset / self.block_size) as usize;
            let within = offset % self.block_size;
            let n = ((self.block_size - within) as usize).min(buf.len());
            let (head, rest) = buf.split_at_mut(n);
            match self.slot.get(block) {
                Some(Some(s)) => {
                    self.file
                        .seek(SeekFrom::Start(0x8000 + s * self.block_size + within))?;
                    self.file.read_exact(head)?;
                }
                Some(None) => head.fill(0),
                None => return Err(ExtractError::Format("read past the end of the disc image")),
            }
            offset += n as u64;
            buf = rest;
        }
        Ok(())
    }
}

pub(crate) fn open_image(path: &Path) -> Result<Box<dyn ReadAt>, ExtractError> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let mut head = [0u8; 8];
    file.read_exact(&mut head)?;
    if &head[..4] == b"CISO" {
        let block_size = u32::from_le_bytes(head[4..8].try_into().unwrap()) as u64;
        if block_size == 0 || block_size > 64 * 1024 * 1024 {
            return Err(ExtractError::Format("CISO block size is implausible"));
        }
        let mut map = vec![0u8; 0x8000 - 8];
        file.read_exact(&mut map)?;
        let mut next = 0u64;
        let slot = map
            .iter()
            .map(|&m| {
                (m != 0).then(|| {
                    next += 1;
                    next - 1
                })
            })
            .collect();
        return Ok(Box::new(CisoImage {
            file,
            block_size,
            slot,
        }));
    }
    if matches!(&head[..4], b"RVZ\x01" | b"WIA\x01") || head.starts_with(b"WBFS") {
        return Err(ExtractError::Unsupported(
            "compressed Dolphin formats (RVZ/WIA/WBFS) are not supported; convert to ISO or use tools/extract_assets.py",
        ));
    }
    Ok(Box::new(PlainImage { file, len }))
}

pub struct Disc {
    reader: Box<dyn ReadAt>,
    pub game_id: String,
    /// File path (no leading slash, e.g. `data/mario.szs`) to `(offset, size)`.
    files: HashMap<String, (u64, u64)>,
}

impl Disc {
    pub fn open(path: &Path) -> Result<Disc, ExtractError> {
        Disc::from_reader(open_image(path)?)
    }

    pub fn from_reader(mut reader: Box<dyn ReadAt>) -> Result<Disc, ExtractError> {
        let mut head = [0u8; 0x440];
        reader.read_at(0, &mut head)?;
        if head[0x1C..0x20] != [0xC2, 0x33, 0x9F, 0x3D] {
            return Err(ExtractError::Format(
                "not a GameCube disc image (bad magic)",
            ));
        }
        let game_id = String::from_utf8_lossy(&head[0..6]).into_owned();
        let fst_off = u32::from_be_bytes(head[0x424..0x428].try_into().unwrap()) as u64;
        let fst_size = u32::from_be_bytes(head[0x428..0x42C].try_into().unwrap()) as usize;
        if !(12..=64 * 1024 * 1024).contains(&fst_size) {
            return Err(ExtractError::Format(
                "file system table size is implausible",
            ));
        }
        let mut fst = vec![0u8; fst_size];
        reader.read_at(fst_off, &mut fst)?;

        let be = |o: usize| u32::from_be_bytes(fst[o..o + 4].try_into().unwrap());
        let count = be(8) as usize;
        if count == 0 || count * 12 > fst_size {
            return Err(ExtractError::Format("file system table is corrupt"));
        }
        let names = count * 12;
        let name_of = |entry: usize| -> Result<String, ExtractError> {
            let off = (be(entry * 12) & 0x00FF_FFFF) as usize + names;
            let s = fst
                .get(off..)
                .ok_or(ExtractError::Format("FST name is out of range"))?;
            let end = s
                .iter()
                .position(|&b| b == 0)
                .ok_or(ExtractError::Format("unterminated FST name"))?;
            Ok(String::from_utf8_lossy(&s[..end]).into_owned())
        };

        let mut files = HashMap::new();
        // (index one past the directory's last entry, its path prefix)
        let mut dirs: Vec<(usize, String)> = vec![(count, String::new())];
        for i in 1..count {
            while dirs.len() > 1 && i >= dirs.last().unwrap().0 {
                dirs.pop();
            }
            let is_dir = fst[i * 12] != 0;
            let name = name_of(i)?;
            let prefix = &dirs.last().unwrap().1;
            if is_dir {
                let end = be(i * 12 + 8) as usize;
                dirs.push((end.min(count), format!("{prefix}{name}/")));
            } else {
                files.insert(
                    format!("{prefix}{name}"),
                    (be(i * 12 + 4) as u64, be(i * 12 + 8) as u64),
                );
            }
        }
        Ok(Disc {
            reader,
            game_id,
            files,
        })
    }

    pub fn contains(&self, path: &str) -> bool {
        self.files.contains_key(path)
    }

    /// Every file path under `prefix` (which ends in `/`), sorted.
    pub fn paths_under(&self, prefix: &str) -> Vec<String> {
        let mut v: Vec<String> = self
            .files
            .keys()
            .filter(|p| p.starts_with(prefix))
            .cloned()
            .collect();
        v.sort();
        v
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Reads a whole file by its path on the disc, e.g. `data/mario.szs`.
    pub fn read_file(&mut self, path: &str) -> Result<Vec<u8>, ExtractError> {
        let &(off, size) = self
            .files
            .get(path)
            .ok_or_else(|| ExtractError::Missing(path.to_owned()))?;
        if size > 1 << 30 {
            return Err(ExtractError::Format("file size is implausible"));
        }
        let mut buf = vec![0u8; size as usize];
        self.reader.read_at(off, &mut buf)?;
        Ok(buf)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Reads from memory, for tests.
    pub struct Mem(pub Vec<u8>);
    impl ReadAt for Mem {
        fn read_at(&mut self, o: u64, buf: &mut [u8]) -> Result<(), ExtractError> {
            let o = o as usize;
            buf.copy_from_slice(
                self.0
                    .get(o..o + buf.len())
                    .ok_or(ExtractError::Format("past the end"))?,
            );
            Ok(())
        }
    }

    /// Builds a minimal disc: `data/mario.szs` holding `payload`, plus `sys`-level file `opening.bnr`.
    pub fn disc_with(id: &[u8; 6], files: &[(&str, &[u8])]) -> Vec<u8> {
        // Layout: header 0x440, FST at 0x1000, file data after.
        // Directories are derived from the first path component (one level only).
        let mut dirs: Vec<&str> = Vec::new();
        for (p, _) in files {
            if let Some((d, _)) = p.split_once('/') {
                if !dirs.contains(&d) {
                    dirs.push(d);
                }
            }
        }
        let mut entries: Vec<(bool, String, u32, u32)> = Vec::new(); // is_dir, name, a, b
        let mut data_at = 0x2000u32;
        let mut blobs: Vec<(u32, &[u8])> = Vec::new();
        // root-level files first, then each directory's files
        for (p, d) in files.iter().filter(|(p, _)| !p.contains('/')) {
            entries.push((false, (*p).to_owned(), data_at, d.len() as u32));
            blobs.push((data_at, d));
            data_at += (d.len() as u32 + 31) & !31;
        }
        for dir in &dirs {
            let in_dir: Vec<_> = files
                .iter()
                .filter(|(p, _)| p.split_once('/').map(|x| x.0) == Some(*dir))
                .collect();
            let index = entries.len() + 1;
            entries.push((
                true,
                (*dir).to_owned(),
                0,
                (index + 1 + in_dir.len()) as u32,
            ));
            for (p, d) in in_dir {
                entries.push((
                    false,
                    p.split_once('/').unwrap().1.to_owned(),
                    data_at,
                    d.len() as u32,
                ));
                blobs.push((data_at, d));
                data_at += (d.len() as u32 + 31) & !31;
            }
        }
        let mut names = Vec::new();
        let mut fst = vec![0u8; 12 * (entries.len() + 1)];
        fst[0] = 1;
        fst[8..12].copy_from_slice(&((entries.len() + 1) as u32).to_be_bytes());
        for (i, (is_dir, name, a, b)) in entries.iter().enumerate() {
            let at = (i + 1) * 12;
            fst[at] = *is_dir as u8;
            fst[at + 1..at + 4].copy_from_slice(&(names.len() as u32).to_be_bytes()[1..]);
            names.extend_from_slice(name.as_bytes());
            names.push(0);
            fst[at + 4..at + 8].copy_from_slice(&a.to_be_bytes());
            fst[at + 8..at + 12].copy_from_slice(&b.to_be_bytes());
        }
        fst.extend_from_slice(&names);

        let mut img = vec![0u8; data_at.max(0x2000) as usize];
        img[..6].copy_from_slice(id);
        img[0x1C..0x20].copy_from_slice(&[0xC2, 0x33, 0x9F, 0x3D]);
        img[0x424..0x428].copy_from_slice(&0x1000u32.to_be_bytes());
        img[0x428..0x42C].copy_from_slice(&(fst.len() as u32).to_be_bytes());
        img[0x1000..0x1000 + fst.len()].copy_from_slice(&fst);
        for (at, d) in blobs {
            img[at as usize..at as usize + d.len()].copy_from_slice(d);
        }
        img
    }

    #[test]
    fn lists_and_reads_files_in_directories() {
        let img = disc_with(
            b"GMSP01",
            &[
                ("opening.bnr", b"banner"),
                ("data/mario.szs", b"szs-bytes"),
                ("data/params.szs", b"p"),
            ],
        );
        let mut d = Disc::from_reader(Box::new(Mem(img))).unwrap();
        assert_eq!(d.game_id, "GMSP01");
        assert_eq!(d.len(), 3);
        assert_eq!(d.read_file("data/mario.szs").unwrap(), b"szs-bytes");
        assert_eq!(d.read_file("data/params.szs").unwrap(), b"p");
        assert_eq!(d.read_file("opening.bnr").unwrap(), b"banner");
        assert!(matches!(
            d.read_file("data/nope"),
            Err(ExtractError::Missing(_))
        ));
    }

    #[test]
    fn rejects_non_discs() {
        assert!(Disc::from_reader(Box::new(Mem(vec![0; 0x1000]))).is_err());
    }
}
