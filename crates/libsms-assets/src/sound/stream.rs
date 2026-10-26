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
