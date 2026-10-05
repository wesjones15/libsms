//! Skeleton and clip sampling for Mario's rig, read from the user's own disc files.
//!
//! This is the data a host needs to play the game's real motion on *its* skeleton: the joint
//! hierarchy and bind pose from the model's `JNT1`/`INF1` chunks (`mario.szs:bmd/ma_mdl1.bmd`) and
//! the per-joint keyframe tracks from `ANK1` in each clip (`mario.szs:bck/ma_<clip>.bck`).
//! Meshes, materials and textures are not read.
//!
//! Joint convention (J3D): local transform `T * Rz * Ry * Rx * S`, bones run along local +X, units
//! are the decomp's (Mario's pelvis `center` joint sits 58 above the root).

use std::collections::HashMap;
use std::f32::consts::PI;

use glam::{EulerRot, Mat4, Quat, Vec3};

use crate::anim::LoopMode;

fn be_u16(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(o..o + 2)?.try_into().ok()?))
}
fn be_i16(d: &[u8], o: usize) -> Option<i16> {
    Some(i16::from_be_bytes(d.get(o..o + 2)?.try_into().ok()?))
}
fn be_u32(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(o..o + 4)?.try_into().ok()?))
}
fn be_f32(d: &[u8], o: usize) -> Option<f32> {
    Some(f32::from_be_bytes(d.get(o..o + 4)?.try_into().ok()?))
}

/// Translation, rotation (Euler radians, applied Z*Y*X) and scale of one joint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Srt {
    pub scale: Vec3,
    pub rot: Vec3,
    pub trans: Vec3,
}

impl Srt {
    pub fn matrix(&self) -> Mat4 {
        let q = Quat::from_euler(EulerRot::ZYX, self.rot.z, self.rot.y, self.rot.x);
        Mat4::from_scale_rotation_translation(self.scale, q, self.trans)
    }
}

#[derive(Debug, Clone)]
pub struct Skeleton {
    pub names: Vec<String>,
    /// Parent joint index, `None` for the root.
    pub parents: Vec<Option<usize>>,
    pub bind: Vec<Srt>,
}

impl Skeleton {
    /// Reads joints out of a `J3D2bmd3` model: names and bind pose from `JNT1`, hierarchy from
    /// the `INF1` scene graph.
    pub fn from_bmd(d: &[u8]) -> Option<Skeleton> {
        if d.get(0..8)? != b"J3D2bmd3" {
            return None;
        }
        let chunk_count = be_u32(d, 12)? as usize;
        let (mut inf1, mut jnt1) = (None, None);
        let mut o = 32usize;
        for _ in 0..chunk_count {
            let tag = d.get(o..o + 4)?;
            let size = be_u32(d, o + 4)? as usize;
            match tag {
                b"INF1" => inf1 = Some(o),
                b"JNT1" => jnt1 = Some(o),
                _ => {}
            }
            o += size.max(8);
        }
        let (inf1, jnt1) = (inf1?, jnt1?);

        let count = be_u16(d, jnt1 + 8)? as usize;
        let entries = jnt1 + be_u32(d, jnt1 + 12)? as usize;
        let names_at = jnt1 + be_u32(d, jnt1 + 20)? as usize;
        let n_names = be_u16(d, names_at)? as usize;
        let mut names = Vec::with_capacity(count);
        for i in 0..n_names.min(count) {
            let off = be_u16(d, names_at + 6 + i * 4)? as usize;
            let s = d.get(names_at + off..)?;
            let end = s.iter().position(|&b| b == 0)?;
            names.push(String::from_utf8_lossy(&s[..end]).into_owned());
        }
        names.resize(count, String::new());

        let rot_scale = PI / 32768.0;
        let mut bind = Vec::with_capacity(count);
        for j in 0..count {
            let e = entries + j * 0x40;
            bind.push(Srt {
                scale: Vec3::new(be_f32(d, e + 4)?, be_f32(d, e + 8)?, be_f32(d, e + 12)?),
                rot: Vec3::new(
                    be_i16(d, e + 16)? as f32 * rot_scale,
                    be_i16(d, e + 18)? as f32 * rot_scale,
                    be_i16(d, e + 20)? as f32 * rot_scale,
                ),
                trans: Vec3::new(be_f32(d, e + 24)?, be_f32(d, e + 28)?, be_f32(d, e + 32)?),
            });
        }

        // Scene graph: 0x10 joint, 0x11 material, 0x12 shape, 0x01 open, 0x02 close.
        let mut h = inf1 + be_u32(d, inf1 + 20)? as usize;
        let mut parents = vec![None; count];
        let mut stack: Vec<Option<usize>> = Vec::new();
        let mut ctx: Option<usize> = None;
        loop {
            let (t, i) = (be_u16(d, h)?, be_u16(d, h + 2)? as usize);
            h += 4;
            match t {
                0 => break,
                1 => stack.push(ctx),
                2 => {
                    stack.pop();
                }
                0x10 => {
                    if i < count {
                        parents[i] = stack.last().copied().flatten();
                    }
                    ctx = Some(i);
                }
                _ => ctx = stack.last().copied().flatten(),
            }
        }
        Some(Skeleton {
            names,
            parents,
            bind,
        })
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    pub fn find(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n == name)
    }

    /// Model-space matrix of every joint, from per-joint local transforms.
    pub fn world(&self, locals: &[Srt]) -> Vec<Mat4> {
        let mut out: Vec<Mat4> = Vec::with_capacity(self.len());
        for (j, srt) in locals.iter().enumerate().take(self.len()) {
            // Parents always precede children in J3D joint order.
            let m = match self.parents[j] {
                Some(p) if p < j => out[p] * srt.matrix(),
                _ => srt.matrix(),
            };
            out.push(m);
        }
        out
    }

    /// Bind pose in model space.
    pub fn bind_world(&self) -> Vec<Mat4> {
        self.world(&self.bind)
    }

    /// Pose at `frame` of `clip` (joints the clip doesn't animate keep their bind transform).
    pub fn pose(&self, clip: &Clip, frame: f32) -> Vec<Mat4> {
        let locals: Vec<Srt> = (0..self.len())
            .map(|j| clip.sample(j, frame).unwrap_or(self.bind[j]))
            .collect();
        self.world(&locals)
    }
}

/// One animated channel: either a constant or a list of Hermite keys.
#[derive(Debug, Clone)]
struct Track {
    /// `(time, value, tangent_in, tangent_out)`; one entry means constant.
    keys: Vec<(f32, f32, f32, f32)>,
}

impl Track {
    fn value(&self, t: f32) -> f32 {
        let k = &self.keys;
        match k.len() {
            0 => 0.0,
            1 => k[0].1,
            n => {
                if t <= k[0].0 {
                    return k[0].1;
                }
                if t >= k[n - 1].0 {
                    return k[n - 1].1;
                }
                let i = k
                    .windows(2)
                    .position(|w| t >= w[0].0 && t < w[1].0)
                    .unwrap_or(n - 2);
                let (a, b) = (k[i], k[i + 1]);
                let dt = b.0 - a.0;
                if dt <= 0.0 {
                    return a.1;
                }
                let s = (t - a.0) / dt;
                let (s2, s3) = (s * s, s * s * s);
                // Cubic Hermite with the outgoing tangent of `a` and incoming tangent of `b`.
                (2.0 * s3 - 3.0 * s2 + 1.0) * a.1
                    + (s3 - 2.0 * s2 + s) * dt * a.3
                    + (-2.0 * s3 + 3.0 * s2) * b.1
                    + (s3 - s2) * dt * b.2
            }
        }
    }
}

/// A `ANK1` joint animation (`.bck`).
#[derive(Debug, Clone)]
pub struct Clip {
    pub frames: f32,
    pub mode: LoopMode,
    /// Per joint: scale xyz, rotation xyz, translation xyz tracks.
    joints: Vec<[Track; 9]>,
}

impl Clip {
    pub fn parse(d: &[u8]) -> Option<Clip> {
        if d.get(0..8)? != b"J3D1bck1" {
            return None;
        }
        let a = 0x20usize;
        if d.get(a..a + 4)? != b"ANK1" {
            return None;
        }
        let mode = match *d.get(a + 8)? {
            0 => LoopMode::Once,
            1 => LoopMode::OnceReset,
            2 => LoopMode::Loop,
            3 => LoopMode::MirrorOnce,
            _ => LoopMode::MirrorLoop,
        };
        let angle_mult = *d.get(a + 9)? as i8;
        let frames = be_u16(d, a + 10)? as f32;
        let joint_count = be_u16(d, a + 12)? as usize;
        let joints_at = a + be_u32(d, a + 20)? as usize;
        let scale_at = a + be_u32(d, a + 24)? as usize;
        let rot_at = a + be_u32(d, a + 28)? as usize;
        let trans_at = a + be_u32(d, a + 32)? as usize;
        let rot_scale = (PI / 32768.0) * 2f32.powi(angle_mult as i32);

        // Channel kinds within an axis triple: 0 scale, 1 rotation, 2 translation.
        let read = |kind: usize, count: usize, index: usize, tangent: u16| -> Option<Track> {
            let value = |i: usize| -> Option<f32> {
                match kind {
                    1 => Some(be_i16(d, rot_at + 2 * i)? as f32 * rot_scale),
                    0 => be_f32(d, scale_at + 4 * i),
                    _ => be_f32(d, trans_at + 4 * i),
                }
            };
            if count <= 1 {
                return Some(Track {
                    keys: vec![(0.0, value(index)?, 0.0, 0.0)],
                });
            }
            let stride = if tangent == 0 { 3 } else { 4 };
            let mut keys = Vec::with_capacity(count);
            for k in 0..count {
                let i = index + k * stride;
                // Key times are stored as floats in the same table as the values; read them
                // through the f32 table for scale/translation tracks and through the s16 table
                // for rotation tracks, where time is stored as a raw s16.
                let t = if kind == 1 {
                    be_i16(d, rot_at + 2 * i)? as f32
                } else {
                    be_f32(d, scale_or_trans(kind, scale_at, trans_at) + 4 * i)?
                };
                let v = value(i + 1)?;
                let tin = value(i + 2)?;
                let tout = if tangent == 0 { tin } else { value(i + 3)? };
                keys.push((t, v, tin, tout));
            }
            Some(Track { keys })
        };

        let mut joints = Vec::with_capacity(joint_count);
        for j in 0..joint_count {
            let base = joints_at + j * 54;
            let mut tracks: Vec<Track> = Vec::with_capacity(9);
            // Entries are stored per axis (x, y, z), each as scale, rotation, translation.
            let mut by_axis: [[Option<Track>; 3]; 3] = Default::default();
            for axis in 0..3 {
                for kind in 0..3 {
                    let e = base + (axis * 3 + kind) * 6;
                    let (count, index, tangent) = (
                        be_u16(d, e)? as usize,
                        be_u16(d, e + 2)? as usize,
                        be_u16(d, e + 4)?,
                    );
                    by_axis[kind][axis] = Some(read(kind, count, index, tangent)?);
                }
            }
            for kind in by_axis {
                for t in kind {
                    tracks.push(t?);
                }
            }
            joints.push(tracks.try_into().ok()?);
        }
        Some(Clip {
            frames,
            mode,
            joints,
        })
    }

    pub fn joint_count(&self) -> usize {
        self.joints.len()
    }

    /// Local transform of `joint` at `frame` (looped into range for looping clips).
    pub fn sample(&self, joint: usize, frame: f32) -> Option<Srt> {
        let t = self.joints.get(joint)?;
        let f = match self.mode {
            LoopMode::Loop | LoopMode::MirrorLoop if self.frames > 0.0 => {
                frame.rem_euclid(self.frames)
            }
            _ => frame.clamp(0.0, self.frames),
        };
        Some(Srt {
            scale: Vec3::new(t[0].value(f), t[1].value(f), t[2].value(f)),
            rot: Vec3::new(t[3].value(f), t[4].value(f), t[5].value(f)),
            trans: Vec3::new(t[6].value(f), t[7].value(f), t[8].value(f)),
        })
    }
}

fn scale_or_trans(kind: usize, scale_at: usize, trans_at: usize) -> usize {
    if kind == 0 { scale_at } else { trans_at }
}

/// Every `ma_*.bck` in an extracted `mario.szs:bck/` folder, parsed.
#[derive(Debug, Clone, Default)]
pub struct ClipLibrary {
    clips: HashMap<String, Clip>,
}

impl ClipLibrary {
    pub fn from_dir(dir: &std::path::Path) -> ClipLibrary {
        let mut lib = ClipLibrary::default();
        let Ok(rd) = std::fs::read_dir(dir) else {
            return lib;
        };
        for e in rd.flatten() {
            let path = e.path();
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let Some(name) = stem.strip_prefix("ma_") else {
                continue;
            };
            if path.extension().and_then(|s| s.to_str()) != Some("bck") {
                continue;
            }
            if let Some(c) = std::fs::read(&path).ok().and_then(|b| Clip::parse(&b)) {
                lib.clips.insert(name.to_owned(), c);
            }
        }
        lib
    }

    pub fn get(&self, name: &str) -> Option<&Clip> {
        self.clips.get(name)
    }

    pub fn len(&self) -> usize {
        self.clips.len()
    }

    pub fn is_empty(&self) -> bool {
        self.clips.is_empty()
    }
}
