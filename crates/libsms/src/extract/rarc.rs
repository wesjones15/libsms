//! RARC archive reader (the container inside Sunshine's `.szs` files).
//!
//! Layout: a 0x20-byte header (`"RARC"`, total size, offset of the info block (0x20), offset of the
//! file data relative to the info block, data size, ...), then an info block of counts and
//! offsets (directory nodes, file entries, string table, all relative to the info block), then the
//! tables, then the data. Directory nodes are 16 bytes, file entries 20 bytes.

use std::ops::Range;

use super::ExtractError;

#[derive(Debug)]
pub struct Archive {
    data: Vec<u8>,
    /// `(path, byte range in data)` for every file, with `/` separators and no leading slash.
    files: Vec<(String, Range<usize>)>,
}

fn u16_at(d: &[u8], o: usize) -> Result<usize, ExtractError> {
    Ok(u16::from_be_bytes(
        d.get(o..o + 2)
            .ok_or(ExtractError::Format("RARC table is out of range"))?
            .try_into()
            .unwrap(),
    ) as usize)
}

fn u32_at(d: &[u8], o: usize) -> Result<usize, ExtractError> {
    Ok(u32::from_be_bytes(
        d.get(o..o + 4)
            .ok_or(ExtractError::Format("RARC table is out of range"))?
            .try_into()
            .unwrap(),
    ) as usize)
}

fn cstr_at(d: &[u8], o: usize) -> Result<String, ExtractError> {
    let s = d
        .get(o..)
        .ok_or(ExtractError::Format("RARC string is out of range"))?;
    let end = s
        .iter()
        .position(|&b| b == 0)
        .ok_or(ExtractError::Format("unterminated RARC string"))?;
    Ok(String::from_utf8_lossy(&s[..end]).into_owned())
}

impl Archive {
    pub fn parse(data: Vec<u8>) -> Result<Archive, ExtractError> {
        if data.get(0..4) != Some(b"RARC") {
            return Err(ExtractError::Format("not a RARC archive"));
        }
        let info = u32_at(&data, 8)?;
        let data_start = info + u32_at(&data, 12)?;
        let node_count = u32_at(&data, info)?;
        let node_off = info + u32_at(&data, info + 4)?;
        let entry_count = u32_at(&data, info + 8)?;
        let entry_off = info + u32_at(&data, info + 12)?;
        let str_off = info + u32_at(&data, info + 20)?;
        if node_count == 0 || node_count > 65_536 || entry_count > 262_144 {
            return Err(ExtractError::Format("RARC counts are implausible"));
        }

        let mut files = Vec::new();
        // (node index, path prefix); depth-limited to stop a malicious cycle.
        let mut stack = vec![(0usize, String::new(), 0usize)];
        while let Some((node, prefix, depth)) = stack.pop() {
            if depth > 32 || node >= node_count {
                return Err(ExtractError::Format("RARC directory structure is invalid"));
            }
            let n = node_off + node * 16;
            let count = u16_at(&data, n + 10)?;
            let first = u32_at(&data, n + 12)?;
            for e in first..first + count {
                if e >= entry_count {
                    return Err(ExtractError::Format(
                        "RARC file entry index is out of range",
                    ));
                }
                let at = entry_off + e * 20;
                let flags = *data
                    .get(at + 4)
                    .ok_or(ExtractError::Format("RARC entry is out of range"))?;
                let name = cstr_at(&data, str_off + u16_at(&data, at + 6)?)?;
                let a = u32_at(&data, at + 8)?;
                let b = u32_at(&data, at + 12)?;
                if flags & 0x02 != 0 {
                    if name != "." && name != ".." {
                        stack.push((a, format!("{prefix}{name}/"), depth + 1));
                    }
                } else {
                    let start = data_start + a;
                    let end = start.checked_add(b).filter(|&e| e <= data.len());
                    let Some(end) = end else {
                        return Err(ExtractError::Format("RARC file data is out of range"));
                    };
                    files.push((format!("{prefix}{name}"), start..end));
                }
            }
        }
        files.sort_by(|x, y| x.0.cmp(&y.0));
        Ok(Archive { data, files })
    }

    /// Every file path in the archive.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.iter().map(|f| f.0.as_str())
    }

    /// Bytes of one file by its path inside the archive.
    pub fn file(&self, path: &str) -> Option<&[u8]> {
        self.files
            .iter()
            .find(|f| f.0 == path)
            .map(|f| &self.data[f.1.clone()])
    }

    /// `(relative path, bytes)` of everything under `prefix` (which ends in `/`).
    pub fn under<'a>(&'a self, prefix: &'a str) -> impl Iterator<Item = (&'a str, &'a [u8])> {
        self.files.iter().filter_map(move |f| {
            f.0.strip_prefix(prefix)
                .map(|rel| (rel, &self.data[f.1.clone()]))
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builds a tiny archive: root contains `a.txt` and a directory `sub` holding `b.bin`.
    pub fn sample() -> Vec<u8> {
        let strings = b".\0..\0a.txt\0sub\0b.bin\0ROOT\0";
        let off = |s: &str| {
            let hay = std::str::from_utf8(strings).unwrap();
            hay.find(&format!("{s}\0")).unwrap() as u16
        };
        let (a_data, b_data) = (b"hello".to_vec(), vec![1u8, 2, 3]);
        let info = 0x20usize;
        let nodes_at = 0x20usize; // relative to info
        let entries_at = nodes_at + 2 * 16;
        let strs_at = entries_at + 7 * 20;
        let data_rel = (strs_at + strings.len() + 3) & !3;

        let mut out = vec![0u8; info + data_rel];
        out[..4].copy_from_slice(b"RARC");
        out[8..12].copy_from_slice(&(info as u32).to_be_bytes());
        out[12..16].copy_from_slice(&(data_rel as u32).to_be_bytes());
        let put32 = |o: &mut Vec<u8>, at: usize, v: usize| {
            o[at..at + 4].copy_from_slice(&(v as u32).to_be_bytes())
        };
        put32(&mut out, info, 2);
        put32(&mut out, info + 4, nodes_at);
        put32(&mut out, info + 8, 7);
        put32(&mut out, info + 12, entries_at);
        put32(&mut out, info + 16, strings.len());
        put32(&mut out, info + 20, strs_at);

        // node 0 (root): entries 0..4 ; node 1 (sub): entries 4..7
        let node = |o: &mut Vec<u8>, i: usize, first: usize, count: usize| {
            let at = info + nodes_at + i * 16;
            o[at..at + 4].copy_from_slice(b"ROOT");
            o[at + 10..at + 12].copy_from_slice(&(count as u16).to_be_bytes());
            o[at + 12..at + 16].copy_from_slice(&(first as u32).to_be_bytes());
        };
        node(&mut out, 0, 0, 4);
        node(&mut out, 1, 4, 3);
        // entry: flags, name, a, b
        let entry = |o: &mut Vec<u8>, i: usize, flags: u8, name: &str, a: usize, b: usize| {
            let at = info + entries_at + i * 20;
            o[at + 4] = flags;
            o[at + 6..at + 8].copy_from_slice(&off(name).to_be_bytes());
            o[at + 8..at + 12].copy_from_slice(&(a as u32).to_be_bytes());
            o[at + 12..at + 16].copy_from_slice(&(b as u32).to_be_bytes());
        };
        entry(&mut out, 0, 0x01, "a.txt", 0, a_data.len());
        entry(&mut out, 1, 0x02, "sub", 1, 0);
        entry(&mut out, 2, 0x02, ".", 0, 0);
        entry(&mut out, 3, 0x02, "..", 0xFFFF_FFFF, 0);
        entry(&mut out, 4, 0x01, "b.bin", 8, b_data.len());
        entry(&mut out, 5, 0x02, ".", 1, 0);
        entry(&mut out, 6, 0x02, "..", 0, 0);
        let s = info + strs_at;
        out[s..s + strings.len()].copy_from_slice(strings);
        let mut data = vec![0u8; 16];
        data[..5].copy_from_slice(&a_data);
        data[8..11].copy_from_slice(&b_data);
        out.extend_from_slice(&data);
        out
    }

    #[test]
    fn walks_directories_and_reads_files() {
        let a = Archive::parse(sample()).unwrap();
        let paths: Vec<_> = a.paths().collect();
        assert_eq!(paths, ["a.txt", "sub/b.bin"]);
        assert_eq!(a.file("a.txt").unwrap(), b"hello");
        assert_eq!(a.file("sub/b.bin").unwrap(), [1, 2, 3]);
        let under: Vec<_> = a
            .under("sub/")
            .map(|(p, d)| (p.to_owned(), d.len()))
            .collect();
        assert_eq!(under, [("b.bin".to_owned(), 3)]);
    }

    #[test]
    fn corrupt_archives_are_errors() {
        assert!(Archive::parse(b"RARC".to_vec()).is_err());
        assert!(Archive::parse(b"NOPE0000000000000000".to_vec()).is_err());
        let mut bad = sample();
        let n = bad.len();
        bad.truncate(n - 8); // file data now runs past the end
        assert!(Archive::parse(bad).is_err());
    }
}
