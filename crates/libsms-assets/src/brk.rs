//! J3DAnmTevRegKey / TRK1, per J3DAnmLoader::readAnmTevReg.
use crate::{Bmd, Error, reader::Be};
use libsms::skeleton::Track;

pub struct Colors {
    pub frames: f32,
    pub mode: u8,
    tracks: Vec<Color>,
}
struct Color {
    material: String,
    konst: bool,
    register: usize,
    channels: Vec<Track>,
}

impl Colors {
    pub fn parse(data: &[u8]) -> Result<Self, Error> {
        let r = Be(data);
        let b = 32;
        if r.bytes(0, 8)? != b"J3D1brk1" || r.bytes(b, 4)? != b"TRK1" {
            return Err(Error::Bad("not a BRK/TRK1".into()));
        }
        let offset = |o| r.u32(b + o).map(|v| b + v as usize);
        let mut tracks = Vec::new();
        for kind in 0..2 {
            let n = r.u16(b + 12 + kind * 2)? as usize;
            if n == 0 {
                continue;
            }
            let names = r.name_table(offset(0x30 + kind * 4)?)?;
            let table = offset(0x20 + kind * 4)?;
            for i in 0..n {
                let e = table + i * 28;
                let mut channels = Vec::new();
                for c in 0..4 {
                    let at = e + c * 6;
                    let values = offset(0x38 + kind * 16 + c * 4)?;
                    let value = |j| r.i16(values + j * 2).ok().map(|v| v as f32);
                    channels.push(
                        Track::read(
                            r.u16(at)? as usize,
                            r.u16(at + 2)? as usize,
                            r.u16(at + 4)?,
                            value,
                            value,
                        )
                        .ok_or(Error::Truncated(at))?,
                    );
                }
                tracks.push(Color {
                    material: names.get(i).cloned().unwrap_or_default(),
                    konst: kind == 1,
                    register: r.u8(e + 24)? as usize,
                    channels,
                });
            }
        }
        Ok(Self {
            frames: r.i16(b + 10)? as f32,
            mode: r.u8(b + 8)?,
            tracks,
        })
    }
    pub fn apply(&self, bmd: &mut Bmd, frame: f32) {
        let f = if self.mode == 2 && self.frames > 0. {
            frame.rem_euclid(self.frames)
        } else {
            frame.clamp(0., self.frames.max(0.))
        };
        for t in &self.tracks {
            let Some(m) = bmd.materials.iter_mut().find(|m| m.name == t.material) else {
                continue;
            };
            let c: [i16; 4] = std::array::from_fn(|i| t.channels[i].value(f).round() as i16);
            if t.konst {
                if let Some(k) = m.konst.get_mut(t.register) {
                    *k = c.map(|v| v.clamp(0, 255) as u8);
                }
            } else if let Some(r) = m.tev_colors.get_mut(t.register) {
                *r = c;
            }
        }
    }
}
