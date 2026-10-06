//! Texture pattern animations (`J3D1btp1`, `TPT1` chunk): per material, a texture index for
//! each frame. Mario's eyes blink and change with these (`marioAnimeTexPatternFilenames`,
//! MarioDraw.cpp).
//!
//! Layout from `J3DAnmTexPatternFullData` (J3DAnmLoader.hpp); sampling is
//! `J3DAnmTexPattern::getTexNo` (J3DAnimation.cpp): frames before the start use the first
//! entry, frames past a track's end hold its last.

use crate::Error;
use crate::reader::Be;

#[derive(Debug, Clone)]
pub struct Track {
    pub material: String,
    /// The material's texture map slot this track replaces.
    pub tex_map: u8,
    /// Texture index (into the model's `TEX1`) per frame.
    pub frames: Vec<u16>,
}

#[derive(Debug, Clone)]
pub struct TexPattern {
    /// `J3DFrameCtrl` attribute: 0 once, 2 loop, ...
    pub loop_mode: u8,
    pub length: i16,
    pub tracks: Vec<Track>,
}

impl TexPattern {
    pub fn parse(d: &[u8]) -> Result<TexPattern, Error> {
        let r = Be(d);
        if r.bytes(0, 8)? != b"J3D1btp1" {
            return Err(Error::Bad("not a btp".into()));
        }
        let b = 0x20;
        if r.bytes(b, 4)? != b"TPT1" {
            return Err(Error::Bad("no TPT1 chunk".into()));
        }
        let loop_mode = r.u8(b + 8)?;
        let length = r.i16(b + 0xA)?;
        let n = r.u16(b + 0xC)? as usize;
        let table = b + r.u32(b + 0x10)? as usize;
        let values = b + r.u32(b + 0x14)? as usize;
        let names = r.name_table(b + r.u32(b + 0x1C)? as usize)?;
        let mut tracks = Vec::with_capacity(n);
        for i in 0..n {
            let e = table + i * 8;
            let max = r.u16(e)? as usize;
            let first = r.u16(e + 2)? as usize;
            tracks.push(Track {
                material: names.get(i).cloned().unwrap_or_default(),
                tex_map: r.u8(e + 4)?,
                frames: (0..max)
                    .map(|f| r.u16(values + (first + f) * 2))
                    .collect::<Result<_, _>>()?,
            });
        }
        Ok(TexPattern {
            loop_mode,
            length,
            tracks,
        })
    }

    /// Texture index for `track` at `frame`.
    pub fn tex_no(&self, track: usize, frame: f32) -> Option<u16> {
        let t = self.tracks.get(track)?;
        let last = t.frames.len().checked_sub(1)?;
        let i = if frame < 0.0 {
            0
        } else {
            (frame as usize).min(last)
        };
        t.frames.get(i).copied()
    }
}
