//! Streamed audio (`AudioRes/Streams/*.afc`, the title screen music): a 0x20-byte
//! `StreamHeader` then the samples (`JAInter::StreamLib`, JAIGFrameStream.cpp).
//!
//! Format 4 (`__DecodeADPCM`) stores each 16 stereo samples as a 9-byte left frame then a 9-byte
//! right frame. Unlike the DSP, the CPU decoder doesn't saturate: each sample is truncated to
//! 16 bits. Format 2 (`__DecodePCM`) is interleaved big-endian PCM16. When the loop flag is set
//! the stream restarts at the loop start sample (`LoopInit`) after reaching the end.

use crate::Error;
use crate::reader::Be;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamHeader {
    /// Bytes after the header.
    pub data_size: u32,
    pub samples: u32,
    pub sample_rate: u16,
    pub format: u16,
    /// Frames per second for sync callbacks (`sync(playedSamples * unkE / unk8)`).
    pub frame_rate: u16,
    pub looping: bool,
    pub loop_start: u32,
}

/// A decoded stream: interleaved stereo.
#[derive(Debug, Clone)]
pub struct Stream {
    pub header: StreamHeader,
    pub stereo: Vec<i16>,
}

impl StreamHeader {
    pub fn parse(d: &[u8]) -> Result<Self, Error> {
        let r = Be(d);
        Ok(StreamHeader {
            data_size: r.u32(0)?,
            samples: r.u32(4)?,
            sample_rate: r.u16(8)?,
            format: r.u16(0xA)?,
            frame_rate: r.u16(0xE)?,
            looping: r.u32(0x10)? != 0,
            loop_start: r.u32(0x14)?,
        })
    }
}

/// `filter_table` (JAIGFrameStream.cpp), in 1/2048ths.
const FILTER: [[i32; 2]; 16] = [
    [0, 0],
    [0x800, 0],
    [0, 0x800],
    [0x400, 0x400],
    [0x1000, -0x800],
    [0xE00, -0x600],
    [0xC00, -0x400],
    [0x1200, -0xA00],
    [0x1068, -0x8C8],
    [0x12C0, -0x8FC],
    [0x1400, -0xC00],
    [0x800, -0x800],
    [0x400, -0x400],
    [-0x400, 0x400],
    [-0x400, 0],
    [-0x800, 0],
];

/// One 9-byte frame of one channel; `h` is (last, before last).
fn decode_frame(f: &[u8], h: &mut (i16, i16), out: &mut [i16; 16]) {
    let [c1, c2] = FILTER[(f[0] & 0xF) as usize];
    let shift = (f[0] >> 4) as u32;
    for i in 0..16 {
        let b = f[1 + i / 2];
        let nib = if i % 2 == 0 { b >> 4 } else { b & 0xF };
        let code = ((nib as i8) << 4 >> 4) as i32;
        let s = (code << shift) + ((c1 * h.0 as i32 + c2 * h.1 as i32) >> 11);
        let s = s as i16;
        *h = (s, h.0);
        out[i] = s;
    }
}

impl Stream {
    pub fn decode(d: &[u8]) -> Result<Self, Error> {
        let header = StreamHeader::parse(d)?;
        let end = (0x20 + header.data_size as usize).min(d.len());
        let data = d.get(0x20..end).ok_or(Error::Truncated(0x20))?;
        let n = header.samples as usize;
        let mut stereo = Vec::with_capacity(n * 2);
        match header.format {
            4 => {
                let (mut hl, mut hr) = ((0, 0), (0, 0));
                let (mut l, mut r) = ([0i16; 16], [0i16; 16]);
                for f in data.as_chunks::<18>().0 {
                    decode_frame(&f[..9], &mut hl, &mut l);
                    decode_frame(&f[9..], &mut hr, &mut r);
                    for i in 0..16 {
                        stereo.extend_from_slice(&[l[i], r[i]]);
                    }
                }
            }
            2 => stereo.extend(
                data.as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| i16::from_be_bytes([c[0], c[1]])),
            ),
            f => return Err(Error::Bad(format!("stream format {f} not supported"))),
        }
        // The last frame can be cut short; the header's count wins.
        stereo.resize(n * 2, 0);
        Ok(Stream { header, stereo })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoder_wraps_instead_of_clamping() {
        // Predictor 1 (x2048/2048), scale 12: nibble 7 = 0x7000, then 0x7000 + 0x7000 wraps.
        let mut h = (0, 0);
        let mut out = [0i16; 16];
        decode_frame(&[0xC1, 0x77, 0, 0, 0, 0, 0, 0, 0], &mut h, &mut out);
        assert_eq!(out[0], 0x7000);
        assert_eq!(out[1], (0xE000u32 as u16) as i16);
    }
}
