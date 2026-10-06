//! Animation sound tables (`.bas`, `JAIAnimeSoundData`, JAIAnimation.hpp): the sounds an
//! animation triggers between given frames, read by `JAIAnimeSound::setAnimSoundActor`.
//!
//! Layout (big endian): u16 entry count, 6 bytes of padding, then 0x20-byte entries.

use crate::Error;

/// One `JAIAnimeFrameSoundData` entry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimSound {
    /// Sound id.
    pub id: u32,
    /// Frame the sound starts on.
    pub start_frame: f32,
    /// Frame a looping or sustained sound ends on.
    pub end_frame: f32,
    /// Pitch multiplier.
    pub pitch: f32,
    /// Behavior flags (direction, sustain, speed-modified pitch and volume).
    pub flags: u32,
    /// Volume, 0..127.
    pub volume: u8,
    /// How strongly the playback speed scales the pitch.
    pub pitch_scale: i8,
    /// Times a looping animation retriggers it (0 = every loop).
    pub loop_count: u8,
    /// Pan, 0..127.
    pub pan: u8,
    /// How strongly the playback speed scales the volume.
    pub volume_scale: i8,
}

/// A parsed `.bas` file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AnimSoundTable {
    pub sounds: Vec<AnimSound>,
}

const HEADER: usize = 8;
const ENTRY: usize = 0x20;

impl AnimSoundTable {
    pub fn parse(d: &[u8]) -> Result<Self, Error> {
        let u32_at = |o: usize| u32::from_be_bytes(d[o..o + 4].try_into().unwrap());
        if d.len() < HEADER {
            return Err(Error::Truncated(0));
        }
        let count = u16::from_be_bytes([d[0], d[1]]) as usize;
        let end = HEADER + count * ENTRY;
        if d.len() < end {
            return Err(Error::Truncated(end));
        }
        let sounds = (0..count)
            .map(|i| {
                let o = HEADER + i * ENTRY;
                AnimSound {
                    id: u32_at(o),
                    start_frame: f32::from_bits(u32_at(o + 4)),
                    end_frame: f32::from_bits(u32_at(o + 8)),
                    pitch: f32::from_bits(u32_at(o + 0xC)),
                    flags: u32_at(o + 0x10),
                    volume: d[o + 0x14],
                    pitch_scale: d[o + 0x15] as i8,
                    loop_count: d[o + 0x16],
                    pan: d[o + 0x17],
                    volume_scale: d[o + 0x18] as i8,
                }
            })
            .collect();
        Ok(AnimSoundTable { sounds })
    }
}
