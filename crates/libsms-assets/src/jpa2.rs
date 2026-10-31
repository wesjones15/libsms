//! Pikmin 2's particle containers (`game.jpc`, magic `JPAC2-10`): `JPAResourceLoader::load_jpc`
//! (JPAResourceLoader.cpp). Resource blocks are retained and decoded by `Jpc::particle`
//! into the common JParticle data, including dynamics, fields, child particles, animation
//! keys, TEV selectors and independent texture samplers. Sunshine's `.jpa` layout differs.
//!
//! ```text
//! 0x00 "JPAC2-10"   0x08 u16 resources   0x0A u16 textures   0x0C u32 offset of the textures
//! 0x10 resources: u16 id, u16 blocks, u8 fields, u8 keys, u8 TDB1 entries, pad, then `blocks` blocks of
//!      (4-byte tag, u32 size, ...): FLD1 BEM1 BSP1 ESP1 SSP1 ETX1 KFA1 TDB1
//! ```

use std::collections::HashMap;

use crate::Error;
use crate::reader::Be;
use crate::texture::{Texture, read_timg};

/// `BSP1` (`JPABaseShapeData`, JPAShape.h): how a resource's particles look.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BaseShape {
    pub flags: u32,
    /// `mBaseSizeX/Y`: half-extent of a particle at scale 1, in world units.
    pub size: [f32; 2],
    /// `mBlendModeCfg`: bits 0-1 mode, 2-5 source factor, 6-9 destination factor.
    pub blend: u16,
    /// Index into the resource's `TDB1` list.
    pub texture_index: u8,
    /// `mClrPrm` / `mClrEnv`: RGBA at the start of a particle's life.
    pub prm: [u8; 4],
    pub env: [u8; 4],
}

#[derive(Debug, Clone, Default)]
pub struct Resource {
    pub id: u16,
    pub shape: Option<BaseShape>,
    /// `TDB1`: indices into the container's textures.
    pub textures: Vec<u16>,
    /// Source blocks retained for decoding into the shared JParticle runtime.
    pub blocks: Vec<Vec<u8>>,
}

pub struct Jpc {
    pub resources: HashMap<u16, Resource>,
    pub textures: Vec<Texture>,
}

impl Jpc {
    pub fn parse(d: &[u8]) -> Result<Self, Error> {
        let r = Be(d);
        if r.bytes(0, 8)? != b"JPAC2-10" {
            return Err(Error::Bad("not a JPAC2-10 container".into()));
        }
        let count = r.u16(8)? as usize;
        let texture_count = r.u16(0xA)? as usize;
        let mut o = 0x10usize;
        let mut resources = HashMap::new();
        for _ in 0..count {
            let id = r.u16(o)?;
            let blocks = r.u16(o + 2)? as usize;
            o += 8;
            let mut res = Resource { id, ..Default::default() };
            for _ in 0..blocks {
                let tag = r.bytes(o, 4)?;
                let size = r.u32(o + 4)? as usize;
                if size < 8 { return Err(Error::Bad("empty JPA2 block".into())); }
                res.blocks.push(r.bytes(o, size)?.to_vec());
                match tag {
                    b"BSP1" => {
                        res.shape = Some(BaseShape {
                            flags: r.u32(o + 8)?,
                            size: [r.f32(o + 0x10)?, r.f32(o + 0x14)?],
                            blend: r.u16(o + 0x18)?,
                            texture_index: r.u8(o + 0x20)?,
                            prm: [r.u8(o + 0x26)?, r.u8(o + 0x27)?, r.u8(o + 0x28)?, r.u8(o + 0x29)?],
                            env: [r.u8(o + 0x2A)?, r.u8(o + 0x2B)?, r.u8(o + 0x2C)?, r.u8(o + 0x2D)?],
                        });
                    }
                    b"TDB1" => {
                        let n = size.saturating_sub(8) / 2;
                        res.textures = (0..n).map(|i| r.u16(o + 8 + i * 2)).collect::<Result<_, _>>()?;
                    }
                    _ => {}
                }
                if size < 8 {
                    return Err(Error::Bad("empty JPA2 block".into()));
                }
                o += size;
            }
            resources.insert(id, res);
        }
        let mut textures = Vec::with_capacity(texture_count);
        let mut at = r.u32(0xC)? as usize;
        for _ in 0..texture_count {
            let size = r.u32(at + 4)? as usize;
            let name = r.cstr(at + 0xC)?;
            textures.push(read_timg(d, at + 0x20, name)?);
            at += size;
        }
        Ok(Jpc { resources, textures })
    }

    /// Resource `id`'s first texture and its base shape.
    pub fn look(&self, id: u16) -> Option<(&BaseShape, &Texture)> {
        let res = self.resources.get(&id)?;
        let shape = res.shape.as_ref()?;
        let slot = *res.textures.get(shape.texture_index as usize)? as usize;
        Some((shape, self.textures.get(slot)?))
    }

    /// Decode JPAC2-10 dynamics into the same engine-neutral data used by Sunshine.
    pub fn particle(&self, id: u16) -> Result<crate::jpa::Jpa, Error> {
        let res = self.resources.get(&id).ok_or_else(|| Error::Bad(format!("no JPA2 emitter {id:#x}")))?;
        crate::jpa2_runtime::decode(res, &self.textures)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs the user's extracted Pikmin 2 effects (`libsms/assets/p2/effects/game.jpc`).
    #[test]
    fn the_navi_lights_read() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/p2/effects/game.jpc");
        let Ok(bytes) = std::fs::read(&path) else { eprintln!("SKIP: no Pikmin 2 effects"); return };
        let jpc = Jpc::parse(&bytes).expect("parse");
        assert!(jpc.resources.len() > 100, "{} resources", jpc.resources.len());
        // PID_OrimaLight_Loozy_1 / _2 and the Olimar pair (ParticleID.h).
        for id in [0xDE, 0xE1, 0x123, 0x126] {
            let (shape, tex) = jpc.look(id).unwrap_or_else(|| panic!("no look for {id:#x}"));
            eprintln!("{id:#x}: size {:?} prm {:?} env {:?} blend {:#x} texture {} {}x{}", shape.size, shape.prm, shape.env,
                shape.blend, tex.name, tex.image.width, tex.image.height);
            assert!(tex.image.width > 0 && shape.size[0] > 0.0);
        }
    }
}
