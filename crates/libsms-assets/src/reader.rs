//! Big-endian reads at absolute offsets, with bounds checks.

use crate::Error;

#[derive(Clone, Copy)]
pub struct Be<'a>(pub &'a [u8]);

impl<'a> Be<'a> {
    pub fn bytes(&self, o: usize, n: usize) -> Result<&'a [u8], Error> {
        self.0.get(o..o + n).ok_or(Error::Truncated(o))
    }
    pub fn u8(&self, o: usize) -> Result<u8, Error> {
        self.0.get(o).copied().ok_or(Error::Truncated(o))
    }
    pub fn u16(&self, o: usize) -> Result<u16, Error> {
        Ok(u16::from_be_bytes(self.bytes(o, 2)?.try_into().unwrap()))
    }
    pub fn i16(&self, o: usize) -> Result<i16, Error> {
        Ok(self.u16(o)? as i16)
    }
    pub fn u32(&self, o: usize) -> Result<u32, Error> {
        Ok(u32::from_be_bytes(self.bytes(o, 4)?.try_into().unwrap()))
    }
    pub fn f32(&self, o: usize) -> Result<f32, Error> {
        Ok(f32::from_bits(self.u32(o)?))
    }
    /// A NUL-terminated string.
    pub fn cstr(&self, o: usize) -> Result<String, Error> {
        let s = self.0.get(o..).ok_or(Error::Truncated(o))?;
        let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
        Ok(String::from_utf8_lossy(&s[..end]).into_owned())
    }
    /// A `ResNTAB` name table (JUTNameTab): count, pad, then (hash, offset) pairs whose
    /// offsets are relative to the table.
    pub fn name_table(&self, o: usize) -> Result<Vec<String>, Error> {
        let n = self.u16(o)? as usize;
        (0..n)
            .map(|i| self.cstr(o + self.u16(o + 6 + i * 4)? as usize))
            .collect()
    }
}
