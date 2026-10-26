//! Big-endian reads at absolute offsets, with bounds checks (`libcommon::binary`).

use crate::Error;
use libcommon::binary::Truncated;

pub use libcommon::binary::Be;

impl From<Truncated> for Error {
    fn from(e: Truncated) -> Self {
        Error::Truncated(e.offset)
    }
}

/// J3D's name table, read through [`Be`].
pub trait NameTable {
    /// A `ResNTAB` name table (JUTNameTab): count, pad, then (hash, offset) pairs whose
    /// offsets are relative to the table.
    fn name_table(&self, o: usize) -> Result<Vec<String>, Error>;
}

impl NameTable for Be<'_> {
    fn name_table(&self, o: usize) -> Result<Vec<String>, Error> {
        let n = self.u16(o)? as usize;
        (0..n)
            .map(|i| Ok(self.cstr(o + self.u16(o + 6 + i * 4)? as usize)?))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn big_endian_reads_and_truncation_offsets() {
        let d = [0x12, 0x34, 0x56, 0x78, 0xFF, 0xFE, 0x3F, 0x80, 0x00, 0x00];
        let r = Be(&d);
        assert_eq!(r.u8(0).unwrap(), 0x12);
        assert_eq!(r.u16(0).unwrap(), 0x1234);
        assert_eq!(r.u32(0).unwrap(), 0x1234_5678);
        assert_eq!(r.i16(4).unwrap(), -2);
        assert_eq!(r.f32(6).unwrap(), 1.0);
        assert_eq!(r.bytes(2, 3).unwrap(), &[0x56, 0x78, 0xFF]);
        let e = |r: Result<(), Truncated>| r.map_err(Error::from);
        for result in [e(r.u8(10).map(|_| ())), e(r.u16(9).map(|_| ())), e(r.u32(7).map(|_| ()))] {
            assert!(matches!(result, Err(Error::Truncated(_))));
        }
        assert!(matches!(e(r.u32(7).map(|_| ())), Err(Error::Truncated(7))));
        assert!(matches!(e(r.bytes(usize::MAX, 2).map(|_| ())), Err(Error::Truncated(usize::MAX))));
    }

    #[test]
    fn strings_and_name_tables() {
        let e = |r: Result<(), Truncated>| r.map_err(Error::from);
        let mut d = vec![0, 2, 0, 0, 0xAA, 0xAA, 0, 12, 0xBB, 0xBB, 0, 15];
        d.extend_from_slice(b"ab\0cd\0");
        let r = Be(&d);
        assert_eq!(r.cstr(12).unwrap(), "ab");
        assert_eq!(r.cstr(d.len()).unwrap(), "");
        assert!(matches!(e(r.cstr(d.len() + 1).map(|_| ())), Err(Error::Truncated(_))));
        assert_eq!(r.name_table(0).unwrap(), ["ab", "cd"]);
        assert_eq!(Be(&[b'x', 0xFF]).cstr(0).unwrap(), "x\u{FFFD}");
    }
}
