//! Yaz0 decompression (the `.szs` wrapper around RARC archives).
//!
//! Format: `"Yaz0"`, big-endian `u32` decompressed size, 8 reserved bytes, then groups of a code
//! byte followed by up to 8 items, high bit first. A 1 bit copies one literal byte; a 0 bit is a
//! back-reference of two bytes `NR RR` (distance `RRR + 1`, length `N + 2`) or, when `N` is 0, of
//! three bytes with the length taken from the third byte plus 0x12.

use super::ExtractError;

pub fn is_yaz0(d: &[u8]) -> bool {
    d.get(0..4) == Some(b"Yaz0")
}

/// Hard cap on output, so a corrupt size field can't allocate gigabytes.
const MAX_OUT: usize = 256 * 1024 * 1024;

pub fn decompress(d: &[u8]) -> Result<Vec<u8>, ExtractError> {
    if !is_yaz0(d) || d.len() < 16 {
        return Err(ExtractError::Format("not a Yaz0 stream"));
    }
    let size = u32::from_be_bytes(d[4..8].try_into().unwrap()) as usize;
    if size > MAX_OUT {
        return Err(ExtractError::Format("Yaz0 output size is implausible"));
    }
    let mut out = Vec::with_capacity(size);
    let mut i = 16;
    while out.len() < size {
        let code = *d
            .get(i)
            .ok_or(ExtractError::Format("Yaz0 stream ends early"))?;
        i += 1;
        for bit in (0..8).rev() {
            if out.len() >= size {
                break;
            }
            if code & (1 << bit) != 0 {
                out.push(
                    *d.get(i)
                        .ok_or(ExtractError::Format("Yaz0 stream ends early"))?,
                );
                i += 1;
            } else {
                let b1 = *d
                    .get(i)
                    .ok_or(ExtractError::Format("Yaz0 stream ends early"))?
                    as usize;
                let b2 = *d
                    .get(i + 1)
                    .ok_or(ExtractError::Format("Yaz0 stream ends early"))?
                    as usize;
                i += 2;
                let dist = ((b1 & 0xF) << 8 | b2) + 1;
                let mut n = b1 >> 4;
                if n == 0 {
                    n = *d
                        .get(i)
                        .ok_or(ExtractError::Format("Yaz0 stream ends early"))?
                        as usize
                        + 0x12;
                    i += 1;
                } else {
                    n += 2;
                }
                if dist > out.len() {
                    return Err(ExtractError::Format("Yaz0 back-reference before the start"));
                }
                // Source and destination may overlap, so copy byte by byte.
                for _ in 0..n.min(size - out.len()) {
                    out.push(out[out.len() - dist]);
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(size: u32, body: &[u8]) -> Vec<u8> {
        let mut v = b"Yaz0".to_vec();
        v.extend_from_slice(&size.to_be_bytes());
        v.extend_from_slice(&[0; 8]);
        v.extend_from_slice(body);
        v
    }

    #[test]
    fn literals_only() {
        let d = stream(3, &[0b1110_0000, b'a', b'b', b'c']);
        assert_eq!(decompress(&d).unwrap(), b"abc");
    }

    #[test]
    fn short_and_overlapping_back_reference() {
        // "ab" literals, then copy 6 bytes from distance 2 (overlapping): "abababab".
        // N=4 -> length 6, distance field 1 -> dist 2.
        let d = stream(8, &[0b1100_0000, b'a', b'b', 0x40, 0x01]);
        assert_eq!(decompress(&d).unwrap(), b"abababab");
    }

    #[test]
    fn long_back_reference_uses_the_third_byte() {
        // "x" then copy 0x12 + 3 = 21 bytes from distance 1.
        let d = stream(22, &[0b1000_0000, b'x', 0x00, 0x00, 0x03]);
        assert_eq!(decompress(&d).unwrap(), vec![b'x'; 22]);
    }

    #[test]
    fn corrupt_input_is_an_error_not_a_panic() {
        assert!(decompress(b"nope").is_err());
        assert!(decompress(&stream(10, &[0b1000_0000])).is_err());
        assert!(decompress(&stream(4, &[0b0000_0000, 0x00, 0x05])).is_err()); // reference before start
        assert!(decompress(&stream(u32::MAX, &[])).is_err());
    }
}
