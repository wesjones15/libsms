//! Streamed audio (`AudioRes/Streams/*.afc`, the title screen music): a 0x20-byte
//! `StreamHeader` then the samples (`JAInter::StreamLib`, JAIGFrameStream.cpp).
//!
//! Format 4 (`__DecodeADPCM`) stores each 16 stereo samples as a 9-byte left frame then a 9-byte
//! right frame. Unlike the DSP, the CPU decoder doesn't saturate: each sample is truncated to
//! 16 bits. Format 2 (`__DecodePCM`) is interleaved big-endian PCM16. When the loop flag is set
//! the stream restarts at the loop start sample (`LoopInit`) after reaching the end.

use crate::Error;
use crate::reader::Be;
use libcommon::audio::afc::{History, Overflow, decode_frame4};

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

impl Stream {
    pub fn decode(d: &[u8]) -> Result<Self, Error> {
        let header = StreamHeader::parse(d)?;
        let end = (0x20 + header.data_size as usize).min(d.len());
        let data = d.get(0x20..end).ok_or(Error::Truncated(0x20))?;
        let n = header.samples as usize;
        let mut stereo = Vec::with_capacity(n * 2);
        match header.format {
            4 => {
                let (mut hl, mut hr) = (History::default(), History::default());
                let (mut l, mut r) = ([0i16; 16], [0i16; 16]);
                for f in data.as_chunks::<18>().0 {
                    decode_frame4(&f[..9], &mut hl, &mut l, Overflow::Wrap);
                    decode_frame4(&f[9..], &mut hr, &mut r, Overflow::Wrap);
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
        let mut h = History::default();
        let mut out = [0i16; 16];
        decode_frame4(&[0xC1, 0x77, 0, 0, 0, 0, 0, 0, 0], &mut h, &mut out, Overflow::Wrap);
        assert_eq!(out[0], 0x7000);
        assert_eq!(out[1], (0xE000u32 as u16) as i16);
    }
}

#[cfg(test)]
mod baseline {
    use super::super::adpcm::baseline::{fnv, random};
    use super::*;

    /// Whole-stream output over random data (the CPU decoder wraps), pinned before the
    /// frame decoder moved to libcommon.
    #[test]
    fn decoded_stream_matches_baseline() {
        let data = random(18 * 40);
        let n = 16 * 40u32;
        let mut d = vec![0u8; 0x20];
        d[0..4].copy_from_slice(&(data.len() as u32).to_be_bytes());
        d[4..8].copy_from_slice(&n.to_be_bytes());
        d[10..12].copy_from_slice(&4u16.to_be_bytes());
        d.extend_from_slice(&data);
        let s = Stream::decode(&d).unwrap();
        assert_eq!(s.stereo.len(), n as usize * 2);
        println!("{:#x}", fnv(&s.stereo));
        assert_eq!(fnv(&s.stereo), BASELINE);
    }

    // Recorded from the decoder before it moved to libcommon::audio::afc.
    const BASELINE: u64 = 0xb248114cd89d3252;
}

impl Stream {
    /// Pikmin 2's `.ast` streams (`AudioRes/Stream/*.ast`): a 0x40-byte `STRM` header (format, bits, channels, loop
    /// flag, rate, samples, loop start/end, first block size) then `BLCK` blocks of `size` bytes per channel after a
    /// 0x20-byte block header. Only 4-bit ADPCM (format 0) is read, mixed to stereo (mono is doubled, channels past two
    /// are dropped). The decoder history carries across blocks.
    pub fn decode_ast(d: &[u8]) -> Result<Self, Error> {
        let r = Be(d);
        if d.get(0..4) != Some(b"STRM") {
            return Err(Error::Bad("not an AST stream".into()));
        }
        let (format, channels) = (r.u16(0x8)?, r.u16(0xC)? as usize);
        if format != 0 || channels == 0 || channels > 8 {
            return Err(Error::Bad(format!("AST format {format} with {channels} channels is not supported")));
        }
        let samples = r.u32(0x14)?;
        let looping = r.u16(0xE)? != 0;
        let loop_start = r.u32(0x18)?;
        let rate = r.u32(0x10)?;
        let mut hist = vec![History::default(); channels];
        let mut chan: Vec<Vec<i16>> = vec![Vec::new(); channels];
        let mut at = 0x40;
        while at + 0x20 <= d.len() && d.get(at..at + 4) == Some(b"BLCK") {
            let size = r.u32(at + 4)? as usize;
            let mut p = at + 0x20;
            for c in 0..channels {
                let block = d.get(p..p + size).ok_or(Error::Truncated(p))?;
                let mut out = [0i16; 16];
                for f in block.chunks_exact(9) {
                    decode_frame4(f, &mut hist[c], &mut out, Overflow::Clamp);
                    chan[c].extend_from_slice(&out);
                }
                p += size;
            }
            at = p;
        }
        let n = samples as usize;
        let mut stereo = Vec::with_capacity(n * 2);
        for i in 0..n {
            let l = chan[0].get(i).copied().unwrap_or(0);
            let rr = if channels > 1 { chan[1].get(i).copied().unwrap_or(0) } else { l };
            stereo.extend_from_slice(&[l, rr]);
        }
        let header = StreamHeader {
            data_size: d.len().saturating_sub(0x40) as u32,
            samples,
            sample_rate: rate.min(u16::MAX as u32) as u16,
            format: 4,
            frame_rate: 0,
            looping,
            loop_start,
        };
        Ok(Stream { header, stereo })
    }
}

#[cfg(test)]
mod ast_tests {
    use super::*;

    #[test]
    fn ast_decodes_blocks_of_two_channels() {
        let mut d = b"STRM".to_vec();
        d.extend([0; 0x3C]);
        d[0x8..0xA].copy_from_slice(&0u16.to_be_bytes());
        d[0xC..0xE].copy_from_slice(&2u16.to_be_bytes());
        d[0xE..0x10].copy_from_slice(&0xffffu16.to_be_bytes());
        d[0x10..0x14].copy_from_slice(&32000u32.to_be_bytes());
        d[0x14..0x18].copy_from_slice(&16u32.to_be_bytes());
        d.extend(b"BLCK");
        d.extend(9u32.to_be_bytes());
        d.extend([0; 0x18]);
        // Left: coef 0, shift 1, every nibble 1 -> 2; right: all zero.
        d.extend([0x10, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11]);
        d.extend([0; 9]);
        let s = Stream::decode_ast(&d).unwrap();
        assert_eq!(s.stereo.len(), 32);
        assert_eq!(&s.stereo[..4], &[2, 0, 2, 0]);
        assert_eq!(s.header.sample_rate, 32000);
        assert!(s.header.looping);
    }
}
