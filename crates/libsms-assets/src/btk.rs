//! J3DAnmTextureSRTKey (TTK1), per J3DAnmLoader / J3DAnimation on the disc.
use crate::{Bmd, Error, reader::{Be, NameTable}};
use glam::{Vec2, Vec3};
use libsms::skeleton::Track;

#[derive(Debug, Clone)]
pub struct TexSrt {
    pub frames: f32,
    pub mode: u8,
    pub tracks: Vec<Transform>,
}
#[derive(Debug, Clone)]
pub struct Transform {
    pub material: String,
    pub matrix: usize,
    pub center: Vec3,
    channels: Vec<Track>,
}
impl TexSrt {
    pub fn parse(data: &[u8]) -> Result<Self, Error> {
        let r = Be(data);
        let b = 0x20;
        if r.bytes(0, 8)? != b"J3D1btk1" || r.bytes(b, 4)? != b"TTK1" {
            return Err(Error::Bad("not a BTK/TTK1".into()));
        }
        let offset = |o| r.u32(b + o).map(|v| b + v as usize);
        let count = r.u16(b + 12)? as usize / 3;
        let table = offset(0x14)?;
        let remap = offset(0x18)?;
        let names = r.name_table(offset(0x1c)?)?;
        let matrices = offset(0x20)?;
        let centers = offset(0x24)?;
        let values = [offset(0x28)?, offset(0x2c)?, offset(0x30)?];
        let angle = std::f32::consts::PI / 32768. * 2f32.powi(r.u8(b + 9)? as i8 as i32);
        let mut tracks = Vec::new();
        for i in 0..count {
            let entry = table + r.u16(remap + i * 2)? as usize * 54;
            let mut channels = Vec::new();
            for axis in 0..3 {
                for kind in 0..3 {
                    let at = entry + (axis * 3 + kind) * 6;
                    let value = |j| {
                        if kind == 1 {
                            r.i16(values[kind] + j * 2).ok().map(|v| v as f32 * angle)
                        } else {
                            r.f32(values[kind] + j * 4).ok()
                        }
                    };
                    let time = |j| {
                        if kind == 1 {
                            r.i16(values[kind] + j * 2).ok().map(|v| v as f32)
                        } else {
                            r.f32(values[kind] + j * 4).ok()
                        }
                    };
                    channels.push(
                        Track::read(
                            r.u16(at)? as usize,
                            r.u16(at + 2)? as usize,
                            r.u16(at + 4)?,
                            value,
                            time,
                        )
                        .ok_or(Error::Truncated(at))?,
                    );
                }
            }
            tracks.push(Transform {
                material: names.get(i).cloned().unwrap_or_default(),
                matrix: r.u8(matrices + i)? as usize,
                center: Vec3::new(
                    r.f32(centers + i * 12)?,
                    r.f32(centers + i * 12 + 4)?,
                    r.f32(centers + i * 12 + 8)?,
                ),
                channels,
            });
        }
        Ok(Self {
            frames: r.i16(b + 10)? as f32,
            mode: r.u8(b + 8)?,
            tracks,
        })
    }
    pub fn apply(&self, bmd: &mut Bmd, frame: f32) {
        let frame = if self.mode == 2 && self.frames > 0. {
            frame.rem_euclid(self.frames)
        } else {
            frame.clamp(0., self.frames.max(0.))
        };
        for track in &self.tracks {
            let Some(material) = bmd.materials.iter_mut().find(|m| m.name == track.material) else {
                continue;
            };
            let Some(Some(matrix)) = material.tex_mtx.get_mut(track.matrix) else {
                continue;
            };
            let value = |i: usize| track.channels[i].value(frame);
            matrix.center = track.center;
            matrix.scale = Vec2::new(value(0), value(3));
            matrix.rotation = value(7);
            matrix.translation = Vec2::new(value(2), value(5));
        }
    }
}
