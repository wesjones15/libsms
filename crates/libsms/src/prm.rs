//! Reader for Sunshine's `.prm` parameter files (JSystem `TParams::load`).
//!
//! Layout, big-endian: `u32 count`, then per entry `u16 name_hash, u16 name_len, name bytes,
//! u32 value_size, value bytes`. The value type is not stored (size 1/2/4 = u8/s16/s32-or-f32),
//! so callers ask for the type they expect.

use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct Prm {
    entries: HashMap<String, Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrmError(pub &'static str);

impl std::fmt::Display for PrmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "bad .prm: {}", self.0)
    }
}

impl std::error::Error for PrmError {}

impl Prm {
    pub fn parse(data: &[u8]) -> Result<Prm, PrmError> {
        let mut r = Reader { data, at: 0 };
        let count = r.u32()?;
        let mut entries = HashMap::new();
        for _ in 0..count {
            let _hash = r.u16()?;
            let len = r.u16()? as usize;
            let name =
                std::str::from_utf8(r.take(len)?).map_err(|_| PrmError("name is not utf-8"))?;
            let size = r.u32()? as usize;
            let value = r.take(size)?.to_vec();
            entries.insert(name.to_owned(), value);
        }
        Ok(Prm { entries })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every key and its raw big-endian value, in no particular order.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v.as_slice()))
    }

    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    pub fn f32(&self, key: &str) -> Option<f32> {
        let b = self.entries.get(key)?;
        Some(f32::from_be_bytes(b.as_slice().try_into().ok()?))
    }

    pub fn i32(&self, key: &str) -> Option<i32> {
        let b = self.entries.get(key)?;
        Some(i32::from_be_bytes(b.as_slice().try_into().ok()?))
    }

    pub fn i16(&self, key: &str) -> Option<i16> {
        let b = self.entries.get(key)?;
        Some(i16::from_be_bytes(b.as_slice().try_into().ok()?))
    }

    pub fn u8(&self, key: &str) -> Option<u8> {
        let b = self.entries.get(key)?;
        (b.len() == 1).then(|| b[0])
    }

    pub fn vec3(&self, key: &str) -> Option<glam::Vec3> {
        let b = self.entries.get(key)?;
        Some(glam::Vec3::new(
            f32::from_be_bytes(b.get(0..4)?.try_into().ok()?),
            f32::from_be_bytes(b.get(4..8)?.try_into().ok()?),
            f32::from_be_bytes(b.get(8..12)?.try_into().ok()?),
        ))
    }
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], PrmError> {
        let end = self.at.checked_add(n).ok_or(PrmError("overflow"))?;
        let s = self.data.get(self.at..end).ok_or(PrmError("truncated"))?;
        self.at = end;
        Ok(s)
    }

    fn u16(&mut self) -> Result<u16, PrmError> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32, PrmError> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(out: &mut Vec<u8>, name: &str, value: &[u8]) {
        out.extend_from_slice(&0u16.to_be_bytes());
        out.extend_from_slice(&(name.len() as u16).to_be_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&(value.len() as u32).to_be_bytes());
        out.extend_from_slice(value);
    }

    #[test]
    fn parses_typed_values() {
        let mut d = 3u32.to_be_bytes().to_vec();
        entry(&mut d, "mSpeed", &32.0f32.to_be_bytes());
        entry(&mut d, "mRot", &(-60i16).to_be_bytes());
        entry(&mut d, "mFlag", &[1]);
        let p = Prm::parse(&d).unwrap();
        assert_eq!(p.len(), 3);
        assert_eq!(p.f32("mSpeed"), Some(32.0));
        assert_eq!(p.i16("mRot"), Some(-60));
        assert_eq!(p.u8("mFlag"), Some(1));
        // asking for the wrong width yields None rather than garbage
        assert_eq!(p.i16("mSpeed"), None);
        assert_eq!(p.f32("missing"), None);
    }

    #[test]
    fn truncated_input_is_an_error() {
        let mut d = 1u32.to_be_bytes().to_vec();
        entry(&mut d, "mSpeed", &1.0f32.to_be_bytes());
        d.pop();
        assert!(Prm::parse(&d).is_err());
        assert!(Prm::parse(&[]).is_err());
    }
}
