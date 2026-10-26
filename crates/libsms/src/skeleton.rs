//! Skeleton and clip sampling for Mario's rig, read from the user's own disc files.
//!
//! This is the data a host needs to play the game's real motion on *its* skeleton: the joint
//! hierarchy and bind pose from the model's `JNT1`/`INF1` chunks (`mario.szs:bmd/ma_mdl1.bmd`) and
//! the per-joint keyframe tracks from `ANK1` in each clip (`mario.szs:bck/ma_<clip>.bck`).
//! Meshes, materials and textures are not read.
//!
//! Joint convention (J3D): local transform `T * Rz * Ry * Rx * S`, bones run along local +X, units
//! are the decomp's (Mario's pelvis `center` joint sits 58 above the root).

use libcommon::anim::hermite;
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
        Mat4::from_scale_rotation_translation(self.scale, self.quat(), self.trans)
    }

    pub fn quat(&self) -> Quat {
        Quat::from_euler(EulerRot::ZYX, self.rot.z, self.rot.y, self.rot.x)
    }
}

/// How a model chains its joints (`INF1` flags & 0xF, `J3DModelLoader::readInformation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MtxCalc {
    /// `J3DMtxCalcBasic`: each joint's scale is part of the matrix its children build on.
    #[default]
    Basic,
    /// `J3DMtxCalcSoftimage`: the chain carries rotation and translation only; scales multiply
    /// down the hierarchy, scale the children's offsets, and are applied to each joint's own
    /// matrix at the end. A child can undo its parent's stretch by scaling the other way, which
    /// is how Yoshi's riding clips stretch his neck without his head (Mario, Yoshi and FLUDD
    /// are Softimage models).
    Softimage,
    /// `J3DMtxCalcMaya`: like Basic, but a joint with `mScaleCompensate` divides out its
    /// parent's scale.
    Maya,
}

impl MtxCalc {
    pub fn from_flags(flags: u32) -> MtxCalc {
        match flags & 0xF {
            1 => MtxCalc::Softimage,
            2 => MtxCalc::Maya,
            _ => MtxCalc::Basic,
        }
    }
}

/// Model-space joint matrices (`J3DMtxCalcBasic::recursiveCalc` with the model's
/// `calcTransform`). `parents` must list parents before children. `compensate` is each joint's
/// `mScaleCompensate` (Maya only; may be empty). `callback` is a joint callback's matrix
/// (`J3DNode::getCallBack()(node, 0)`): it runs after the joint's own matrix is stored and
/// concatenates onto the running one, so it turns the joint's children, not the joint.
pub fn calc_joints(
    calc: MtxCalc,
    parents: &[Option<usize>],
    locals: &[Srt],
    compensate: &[bool],
    callback: impl Fn(usize) -> Option<Mat4>,
) -> Vec<Mat4> {
    let n = locals.len().min(parents.len());
    let mut out = Vec::with_capacity(n);
    // Per joint: the running matrix its children start from (`J3DSys::mCurrentMtx`), the
    // product of scales down to it (`mCurrentS`) and its own scale (`mParentS`).
    let mut current: Vec<Mat4> = Vec::with_capacity(n);
    let mut acc: Vec<Vec3> = Vec::with_capacity(n);
    for j in 0..n {
        let l = &locals[j];
        let p = parents[j].filter(|&p| p < j);
        let (pm, ps, pscale) = match p {
            Some(p) => (current[p], acc[p], locals[p].scale),
            None => (Mat4::IDENTITY, Vec3::ONE, Vec3::ONE),
        };
        let one = |s: Vec3| (s - Vec3::ONE).abs().max_element() < 1.0e-6;
        let s = ps * l.scale;
        let (run, own) = match calc {
            MtxCalc::Basic => {
                let m = pm * l.matrix();
                (m, m)
            }
            MtxCalc::Softimage => {
                let m = pm * Mat4::from_rotation_translation(l.quat(), l.trans * ps);
                let own = if one(s) { m } else { m * Mat4::from_scale(s) };
                (m, own)
            }
            MtxCalc::Maya => {
                let mut local = l.matrix();
                if compensate.get(j).copied().unwrap_or(false) && p.is_some() {
                    let inv = Mat4::from_scale(Vec3::ONE / pscale);
                    let t = local.w_axis;
                    local = inv * local;
                    local.w_axis = t;
                }
                let m = pm * local;
                (m, m)
            }
        };
        out.push(own);
        current.push(match callback(j) {
            Some(c) => run * c,
            None => run,
        });
        acc.push(s);
    }
    out
}

#[derive(Debug, Clone)]
pub struct Skeleton {
    pub names: Vec<String>,
    /// Parent joint index, `None` for the root.
    pub parents: Vec<Option<usize>>,
    pub bind: Vec<Srt>,
    pub calc: MtxCalc,
    /// `mScaleCompensate` per joint.
    pub compensate: Vec<bool>,
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
        let mut compensate = Vec::with_capacity(count);
        for j in 0..count {
            let e = entries + j * 0x40;
            compensate.push(*d.get(e + 2)? == 1);
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
            calc: MtxCalc::from_flags(be_u16(d, inf1 + 8)? as u32),
            compensate,
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
        calc_joints(self.calc, &self.parents, locals, &self.compensate, |_| None)
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

    /// Is `j` the joint `root` or below it?
    pub fn under(&self, j: usize, root: usize) -> bool {
        let mut at = Some(j);
        while let Some(i) = at {
            if i == root {
                return true;
            }
            at = self.parents[i].filter(|&p| p < i);
        }
        false
    }

    /// Body clip with a second clip over `root` and its subtree: Mario's pumping arm, which
    /// `M3UModelMario` hangs on `jnt_chest` (`setInfo[1]`, `TMario::initModel`).
    pub fn pose_layered(
        &self,
        base: Option<(&Clip, f32)>,
        upper: Option<(&Clip, f32, usize)>,
    ) -> Vec<Mat4> {
        self.pose_layered_with(base, upper, |_| None)
    }

    /// [`Self::pose_layered`] with a joint callback's matrix per joint (`J3DNode::getCallBack`, see
    /// [`calc_joints`]): Mario's head, tilted by `MarioHeadCtrl`, turns what hangs on it.
    pub fn pose_layered_with(
        &self,
        base: Option<(&Clip, f32)>,
        upper: Option<(&Clip, f32, usize)>,
        callback: impl Fn(usize) -> Option<Mat4>,
    ) -> Vec<Mat4> {
        let locals: Vec<Srt> = (0..self.len())
            .map(|j| {
                let up = upper
                    .filter(|&(_, _, root)| self.under(j, root))
                    .and_then(|(c, f, _)| c.sample(j, f));
                up.or_else(|| base.and_then(|(c, f)| c.sample(j, f)))
                    .unwrap_or(self.bind[j])
            })
            .collect();
        calc_joints(self.calc, &self.parents, &locals, &self.compensate, callback)
    }
}

/// One animated channel: either a constant or a list of Hermite keys.
#[derive(Debug, Clone)]
pub struct Track {
    /// `(time, value, tangent_in, tangent_out)`; one entry means constant.
    keys: Vec<(f32, f32, f32, f32)>,
}

impl Track {
    /// Shared J3D key-table reader for joint BCK and texture BTK channels.
    pub fn read(count: usize, index: usize, tangent: u16,
        value: impl Fn(usize) -> Option<f32>, time: impl Fn(usize) -> Option<f32>) -> Option<Self> {
        if count <= 1 { return Some(Self { keys: vec![(0., value(index)?, 0., 0.)] }); }
        let stride = if tangent == 0 { 3 } else { 4 };
        let mut keys = Vec::with_capacity(count);
        for k in 0..count {
            let i = index + k * stride;
            let tin = value(i + 2)?;
            keys.push((time(i)?, value(i + 1)?, tin, if tangent == 0 { tin } else { value(i + 3)? }));
        }
        Some(Self { keys })
    }

    pub fn value(&self, t: f32) -> f32 {
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
                // Cubic Hermite with the outgoing tangent of `a` and incoming tangent of `b`.
                hermite(t, a.0, a.1, a.3, b.0, b.1, b.2)
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
            Track::read(count, index, tangent, value, |i| {
                // Key times are stored as floats in the same table as the values; read them
                // through the f32 table for scale/translation tracks and through the s16 table
                // for rotation tracks, where time is stored as a raw s16.
                if kind == 1 {
                    Some(be_i16(d, rot_at + 2 * i)? as f32)
                } else {
                    be_f32(d, scale_or_trans(kind, scale_at, trans_at) + 4 * i)
                }
            })
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

#[cfg(test)]
mod hermite_baseline {
    use super::*;

    /// Sampled bits over random keys (both tangent layouts) and times outside the range,
    /// pinned before the curve moved to libcommon.
    #[test]
    fn track_values_match_baseline() {
        let mut s = 0x1357_9BDFu32;
        let mut next = || {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            (s >> 8) as f32 / (1u32 << 24) as f32
        };
        let times = [0.0f32, 4.0, 9.0, 15.0, 22.0, 30.0];
        let mut hash = 0xcbf29ce484222325u64;
        for tangent in [0u16, 1] {
            let stride = if tangent == 0 { 3 } else { 4 };
            let mut flat = Vec::new();
            for t in times {
                flat.push(t);
                flat.push(next() * 8.0 - 4.0);
                flat.push(next() * 2.0 - 1.0);
                if stride == 4 {
                    flat.push(next() * 2.0 - 1.0);
                }
            }
            let at = |i: usize| flat.get(i).copied();
            let track = Track::read(times.len(), 0, tangent, at, at).unwrap();
            for step in 0..100 {
                let v = track.value(-2.0 + step as f32 * 0.37);
                hash = (hash ^ v.to_bits() as u64).wrapping_mul(0x100000001b3);
            }
        }
        println!("{hash:#x}");
        assert_eq!(hash, BASELINE);
    }

    // Recorded from the curve before it moved to libcommon::anim.
    const BASELINE: u64 = 0x6797bedbaa821b37;
}
