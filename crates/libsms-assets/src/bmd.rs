//! J3D models (`J3D2bmd3`), read the way `J3DModelLoader` (J3DModelLoader.cpp) builds
//! `J3DModelData`:
//!
//! - `INF1` (`readInformation`): the scene graph. Joint nodes nest; a material node holds the
//!   shape drawn with it.
//! - `VTX1` (`readVertex`): attribute formats and the vertex arrays.
//! - `EVP1` (`readEnvelop`): weighted matrices (joint, weight lists) and each joint's inverse
//!   bind matrix.
//! - `DRW1` (`readDraw`): draw matrices, each a joint or an envelope.
//! - `JNT1` (`readJoint`): joint names and bind transforms.
//! - `SHP1` (`readShape`, J3DShapeFactory.cpp): per shape a vertex descriptor and matrix groups,
//!   each with its draw-matrix table and a GX display list.
//! - `MAT3` (`J3DModelLoader_v26::readMaterial`, J3DMaterialFactory.cpp): materials built from
//!   indexed component tables.
//! - `TEX1` (`readTexture`): `ResTIMG` images.
//!
//! Display lists are decoded to triangle lists here so a host can upload them directly.

use glam::{Mat4, Vec2, Vec3, Vec4};

use crate::Error;
use crate::reader::{Be, NameTable};
use crate::texture::{Texture, read_timg};

/// GX vertex attributes (`GXAttr`).
mod attr {
    pub const PNMTXIDX: u32 = 0;
    pub const TEX7MTXIDX: u32 = 8;
    pub const POS: u32 = 9;
    pub const NRM: u32 = 10;
    pub const CLR0: u32 = 11;
    pub const CLR1: u32 = 12;
    pub const TEX0: u32 = 13;
    pub const TEX7: u32 = 20;
    pub const NULL: u32 = 0xFF;
}

/// One attribute's array format (`GXVtxAttrFmtList`).
#[derive(Debug, Clone, Copy, Default)]
struct Format {
    count: u32,
    kind: u32,
    frac: u8,
}

/// An attribute array: its format and the bytes from its offset to the next array.
#[derive(Debug, Clone, Default)]
struct Array {
    fmt: Format,
    data: Vec<u8>,
}

impl Array {
    fn comp_size(&self) -> usize {
        match self.fmt.kind {
            0 | 1 => 1,
            2 | 3 => 2,
            _ => 4,
        }
    }

    fn comp(&self, o: usize) -> Option<f32> {
        let d = &self.data;
        let scale = 1.0 / (1u32 << self.fmt.frac) as f32;
        Some(match self.fmt.kind {
            0 => *d.get(o)? as f32 * scale,
            1 => *d.get(o)? as i8 as f32 * scale,
            2 => u16::from_be_bytes(d.get(o..o + 2)?.try_into().ok()?) as f32 * scale,
            3 => i16::from_be_bytes(d.get(o..o + 2)?.try_into().ok()?) as f32 * scale,
            _ => f32::from_be_bytes(d.get(o..o + 4)?.try_into().ok()?),
        })
    }

    fn vec(&self, index: usize, n: usize) -> Option<Vec<f32>> {
        let cs = self.comp_size();
        (0..n).map(|c| self.comp((index * n + c) * cs)).collect()
    }

    /// `GXCompType` for colours.
    fn color(&self, index: usize) -> Option<[u8; 4]> {
        let d = &self.data;
        let size = match self.fmt.kind {
            0 | 3 => 2,
            1 | 4 => 3,
            _ => 4,
        };
        let b = d.get(index * size..index * size + size)?;
        Some(match self.fmt.kind {
            0 => {
                let c = u16::from_be_bytes([b[0], b[1]]);
                let (r, g, bl) = ((c >> 11) & 31, (c >> 5) & 63, c & 31);
                [
                    (r * 255 / 31) as u8,
                    (g * 255 / 63) as u8,
                    (bl * 255 / 31) as u8,
                    255,
                ]
            }
            1 => [b[0], b[1], b[2], 255],
            2 => [b[0], b[1], b[2], 255],
            3 => {
                let c = u16::from_be_bytes([b[0], b[1]]);
                let f = |s: u16| ((c >> s) & 15) as u8 * 0x11;
                [f(12), f(8), f(4), f(0)]
            }
            4 => {
                let c = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
                let f = |s: u32| (((c >> s) & 63) * 255 / 63) as u8;
                [f(18), f(12), f(6), f(0)]
            }
            _ => [b[0], b[1], b[2], b[3]],
        })
    }
}

/// Where a vertex's matrix comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skin {
    /// A joint's world matrix: the vertex is stored in that joint's space.
    Joint(u16),
    /// A weighted envelope: the vertex is stored in model space (bind pose).
    Envelope(u16),
}

/// A draw matrix (`DRW1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawMtx {
    Joint(u16),
    Envelope(u16),
}

#[derive(Debug, Clone)]
pub struct Joint {
    pub name: String,
    pub parent: Option<usize>,
    /// `J3DJoint::mKind`: 0 normal, 1 billboard, 2 Y billboard.
    pub kind: u16,
    /// Children ignore this joint's scale (`J3DJoint::mScaleCompensate`).
    pub scale_compensate: bool,
    pub scale: Vec3,
    /// Radians, applied Z * Y * X.
    pub rotation: Vec3,
    pub translation: Vec3,
}

/// A `GXTexGenType` / `GXTexGenSrc` / matrix triple (`J3DTexCoordInfo`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TexGen {
    /// 0 MTX3x4, 1 MTX2x4, 2.. bump, 10 SRTG.
    pub kind: u8,
    /// 0 POS, 1 NRM, 2 BINRM, 3 TANGENT, 4..=11 TEX0..7, 19.. colours.
    pub src: u8,
    /// `GX_TEXMTX0 + 3n` (30..=57), or 60 for identity.
    pub matrix: u8,
}

/// `J3DTexMtxInfo`: a texture matrix's mode and SRT.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TexMtx {
    /// `GXTexMtxType`: 0 3x4, 1 2x4.
    pub projection: u8,
    /// `J3DTexMtxMode` in the low 7 bits, Maya-style SRT in bit 7.
    pub info: u8,
    pub center: Vec3,
    pub scale: Vec2,
    /// Radians.
    pub rotation: f32,
    pub translation: Vec2,
    pub effect: Mat4,
}

impl TexMtx {
    pub fn mode(&self) -> u8 {
        self.info & 0x7F
    }

    /// The SRT part as two rows (`J3DGetTextureMtxOld`, J3DTransform.cpp), or the Maya variant.
    pub fn srt_rows(&self) -> [[f32; 3]; 2] {
        let (s, c) = self.rotation.sin_cos();
        let (sx, sy) = (self.scale.x, self.scale.y);
        if self.info & 0x80 != 0 {
            // `J3DGetTextureMtxMayaOld`.
            let (tx, ty) = (self.translation.x, self.translation.y);
            [
                [sx * c, sy * s, (tx - 0.5) * c - s * (ty - 0.5 + sy) + 0.5],
                [-sx * s, sy * c, -(tx - 0.5) * s - c * (ty - 0.5 + sy) + 0.5],
            ]
        } else {
            let (cx, cy) = (self.center.x, self.center.y);
            [
                [
                    sx * c,
                    -sx * s,
                    -sx * c * cx + sx * s * cy + cx + self.translation.x,
                ],
                [
                    sy * s,
                    sy * c,
                    -sy * s * cx - sy * c * cy + cy + self.translation.y,
                ],
            ]
        }
    }
}

/// One colour/alpha combiner (`GXSetTevColorIn` / `GXSetTevColorOp`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TevCombiner {
    pub a: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    /// 0 add, 1 sub, 8.. compare ops.
    pub op: u8,
    pub bias: u8,
    pub scale: u8,
    pub clamp: bool,
    /// 0 PREV, 1..=3 REG0..2.
    pub out: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TevStage {
    pub tex_coord: u8,
    /// 0xFF: no texture.
    pub tex_map: u8,
    /// `GXChannelID`: 4 COLOR0A0, 5 COLOR1A1, 0xFF none.
    pub channel: u8,
    pub kcolor_sel: u8,
    pub kalpha_sel: u8,
    pub color: TevCombiner,
    pub alpha: TevCombiner,
    /// Swap table indices for the rasterized colour and the texture.
    pub ras_swap: u8,
    pub tex_swap: u8,
}

/// `J3DColorChanInfo`: one of COLOR0, ALPHA0, COLOR1, ALPHA1.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ColorChan {
    pub lighting: bool,
    /// Material colour from the register (false) or the vertex (true).
    pub mat_from_vertex: bool,
    pub light_mask: u8,
    pub diffuse_fn: u8,
    pub attn_fn: u8,
    pub amb_from_vertex: bool,
}

/// `J3DAlphaCompInfo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlphaCompare {
    pub comp0: u8,
    pub ref0: u8,
    /// 0 AND, 1 OR, 2 XOR, 3 XNOR.
    pub op: u8,
    pub comp1: u8,
    pub ref1: u8,
}

impl AlphaCompare {
    pub const ALWAYS: AlphaCompare = AlphaCompare {
        comp0: 7,
        ref0: 0,
        op: 0,
        comp1: 7,
        ref1: 0,
    };

    pub fn always_passes(&self) -> bool {
        let pass = |c: u8| c == 7;
        match self.op {
            0 => pass(self.comp0) && pass(self.comp1),
            1 => pass(self.comp0) || pass(self.comp1),
            _ => false,
        }
    }
}

/// `J3DBlendInfo`: 0 none, 1 blend, 2 logic, 3 subtract; factors are `GXBlendFactor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlendMode {
    pub mode: u8,
    pub src: u8,
    pub dst: u8,
}

/// How a material is composited, from its PE settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blend {
    Opaque,
    /// Opaque with an alpha test (`J3DPEBlockTexEdge`).
    AlphaTest,
    /// src * a + dst * (1 - a).
    AlphaBlend,
    /// src * a + dst, or src + dst.
    Additive,
}

/// `GXCullMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cull {
    None,
    Front,
    Back,
    All,
}

#[derive(Debug, Clone)]
pub struct Indirect {
    /// J3DIndInitData, J3DMaterialFactory.hpp: coordinate/map and coordinate scales.
    pub orders: Vec<[u8; 2]>,
    pub scales: Vec<[u8; 2]>,
    pub matrices: [[[f32; 3]; 2]; 3],
    /// GX stage, format, bias, matrix, wrap S/T, add previous, LOD, alpha selection.
    pub stages: Vec<[u8; 9]>,
}

#[derive(Debug, Clone)]
pub struct Material {
    pub name: String,
    /// `J3DMaterial::mMaterialMode`: 1 opaque, 2 texture edge, 4 translucent.
    pub mode: u8,
    pub cull: Cull,
    pub chan_num: u8,
    pub mat_colors: [[u8; 4]; 2],
    pub amb_colors: [[u8; 4]; 2],
    pub chans: [ColorChan; 4],
    pub tex_gens: Vec<TexGen>,
    /// Per `GX_TEXMTXn` slot.
    pub tex_mtx: [Option<TexMtx>; 8],
    pub indirect: Option<Indirect>,
    /// Per texture map: an index into [`Bmd::textures`].
    pub textures: [Option<u16>; 8],
    pub stages: Vec<TevStage>,
    /// REG0..REG2 (`J3DTevBlock::load` sets `mTevColor[i]` to `GX_TEVREG0 + i`).
    pub tev_colors: [[i16; 4]; 3],
    pub konst: [[u8; 4]; 4],
    /// Four (R, G, B, A) channel selections.
    pub swap_tables: [[u8; 4]; 4],
    pub alpha_compare: AlphaCompare,
    pub blend_mode: BlendMode,
    pub z_test: bool,
    pub z_write: bool,
}

impl Material {
    pub fn blend(&self) -> Blend {
        let b = self.blend_mode;
        if b.mode == 1 {
            return match (b.src, b.dst) {
                (_, 1) => Blend::Additive,
                (1, 0) => self.opaque_or_test(),
                _ => Blend::AlphaBlend,
            };
        }
        self.opaque_or_test()
    }

    fn opaque_or_test(&self) -> Blend {
        if self.alpha_compare.always_passes() {
            Blend::Opaque
        } else {
            Blend::AlphaTest
        }
    }
}

/// A vertex of a decoded triangle list.
#[derive(Debug, Clone, Copy)]
pub struct BatchVertex {
    pub position: Vec3,
    pub normal: Vec3,
    pub colors: [[u8; 4]; 2],
    pub uv: [Vec2; 3],
    pub skin: Skin,
}

/// One shape drawn with one material, as a triangle list.
#[derive(Debug, Clone)]
pub struct Batch {
    pub shape: usize,
    pub material: usize,
    pub vertices: Vec<BatchVertex>,
    pub indices: Vec<u32>,
}

/// A shape's matrix group: draw matrices for its `PNMTXIDX` slots and a display list.
#[derive(Debug, Clone)]
struct Packet {
    use_mtx: u16,
    table: Vec<u16>,
    dl: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Shape {
    /// 0 single matrix, 1 billboard, 2 Y billboard, 3 multi-matrix.
    pub mtx_type: u8,
    desc: Vec<(u32, u32)>,
    packets: Vec<Packet>,
}

#[derive(Debug, Clone)]
pub struct Bmd {
    /// How the joints chain (`INF1` flags).
    pub calc: libsms::MtxCalc,
    pub joints: Vec<Joint>,
    pub draw_mtx: Vec<DrawMtx>,
    pub envelopes: Vec<Vec<(u16, f32)>>,
    /// Indexed by joint; joints no envelope uses may be missing.
    pub inverse_bind: Vec<Mat4>,
    pub shapes: Vec<Shape>,
    pub materials: Vec<Material>,
    pub textures: Vec<Texture>,
    /// (material, shape, joint) in scene-graph order.
    pub draws: Vec<(usize, usize, usize)>,
    arrays: Vec<Array>,
}

fn chunk<'a>(d: &'a [u8], tag: &[u8; 4]) -> Result<(usize, Be<'a>), Error> {
    let r = Be(d);
    let n = r.u32(12)? as usize;
    let mut o = 32;
    for _ in 0..n {
        let size = r.u32(o + 4)? as usize;
        if r.bytes(o, 4)? == tag {
            return Ok((o, r));
        }
        o += size.max(8);
    }
    Err(Error::Bad(format!(
        "no {} chunk",
        String::from_utf8_lossy(tag)
    )))
}

fn euler(r: Be, o: usize) -> Result<Vec3, Error> {
    let k = std::f32::consts::PI / 32768.0;
    Ok(Vec3::new(
        r.i16(o)? as f32 * k,
        r.i16(o + 2)? as f32 * k,
        r.i16(o + 4)? as f32 * k,
    ))
}

fn vec3(r: Be, o: usize) -> Result<Vec3, Error> {
    Ok(Vec3::new(r.f32(o)?, r.f32(o + 4)?, r.f32(o + 8)?))
}

fn rgba(r: Be, o: usize) -> Result<[u8; 4], Error> {
    Ok(r.bytes(o, 4)?.try_into().unwrap())
}

impl Bmd {
    pub fn parse(d: &[u8]) -> Result<Bmd, Error> {
        if d.get(0..8) != Some(b"J3D2bmd3") {
            return Err(Error::Bad("not a J3D2bmd3 model".into()));
        }
        let joints = read_joints(d)?;
        let (draw_mtx, envelopes, inverse_bind) = read_skin(d, joints.len())?;
        let arrays = read_vertex(d)?;
        let shapes = read_shapes(d)?;
        let textures = read_textures(d)?;
        let materials = read_materials(d)?;
        let draws = read_hierarchy(d)?;
        let (b, r) = chunk(d, b"INF1")?;
        let calc = libsms::MtxCalc::from_flags(r.u16(b + 8)? as u32);
        Ok(Bmd {
            calc,
            joints,
            draw_mtx,
            envelopes,
            inverse_bind,
            shapes,
            materials,
            textures,
            draws,
            arrays,
        })
    }

    pub fn joint(&self, name: &str) -> Option<usize> {
        self.joints.iter().position(|j| j.name == name)
    }

    pub fn material(&self, name: &str) -> Option<usize> {
        self.materials.iter().position(|m| m.name == name)
    }

    /// Bind-pose world matrices (no animation).
    pub fn bind_world(&self) -> Vec<Mat4> {
        self.pose(|_| None, |_| None)
    }

    /// Model-space joint matrices with the model's chaining (`calc`): `local` gives a joint's
    /// animated transform (`None` keeps the bind transform), `callback` a joint callback's
    /// matrix, which turns the joint's children ([`libsms::calc_joints`]).
    pub fn pose(
        &self,
        local: impl Fn(usize) -> Option<libsms::Srt>,
        callback: impl Fn(usize) -> Option<Mat4>,
    ) -> Vec<Mat4> {
        let parents: Vec<Option<usize>> = self.joints.iter().map(|j| j.parent).collect();
        let compensate: Vec<bool> = self.joints.iter().map(|j| j.scale_compensate).collect();
        let locals: Vec<libsms::Srt> = self
            .joints
            .iter()
            .enumerate()
            .map(|(i, j)| {
                local(i).unwrap_or(libsms::Srt {
                    scale: j.scale,
                    rot: j.rotation,
                    trans: j.translation,
                })
            })
            .collect();
        libsms::calc_joints(self.calc, &parents, &locals, &compensate, callback)
    }

    /// Every (material, shape) pair as a triangle list, in scene-graph order.
    pub fn batches(&self) -> Result<Vec<Batch>, Error> {
        self.draws
            .iter()
            .map(|&(material, shape, _)| self.decode_shape(shape, material))
            .collect()
    }

    fn decode_shape(&self, shape: usize, material: usize) -> Result<Batch, Error> {
        let s = self
            .shapes
            .get(shape)
            .ok_or_else(|| Error::Bad(format!("shape {shape}")))?;
        let mut batch = Batch {
            shape,
            material,
            vertices: Vec::new(),
            indices: Vec::new(),
        };
        let skin_of = |drw: u16| match self.draw_mtx.get(drw as usize) {
            Some(DrawMtx::Envelope(e)) => Skin::Envelope(*e),
            Some(DrawMtx::Joint(j)) => Skin::Joint(*j),
            None => Skin::Joint(0),
        };
        let array = |a: u32| self.arrays.get(a as usize);
        // `J3DShapeMtxMulti`: slots persist across groups; 0xFFFF keeps the previous one.
        let mut slots = [0u16; 10];
        for p in &s.packets {
            for (i, &m) in p.table.iter().enumerate().take(10) {
                if m != 0xFFFF {
                    slots[i] = m;
                }
            }
            let single = s.mtx_type != 3 || p.table.is_empty();
            let r = Be(&p.dl);
            let mut o = 0;
            while o < p.dl.len() {
                let cmd = r.u8(o)?;
                o += 1;
                match cmd {
                    0x00 => continue,
                    0x80..=0xBF => {
                        let prim = cmd & 0xF8;
                        let count = r.u16(o)? as usize;
                        o += 2;
                        let first = batch.vertices.len() as u32;
                        for _ in 0..count {
                            let mut v = BatchVertex {
                                position: Vec3::ZERO,
                                normal: Vec3::Y,
                                colors: [[255; 4]; 2],
                                uv: [Vec2::ZERO; 3],
                                skin: skin_of(p.use_mtx),
                            };
                            for &(a, kind) in &s.desc {
                                let idx = match kind {
                                    1 if a <= attr::TEX7MTXIDX => {
                                        let x = r.u8(o)? as usize;
                                        o += 1;
                                        x
                                    }
                                    2 => {
                                        let x = r.u8(o)? as usize;
                                        o += 1;
                                        x
                                    }
                                    3 => {
                                        let x = r.u16(o)? as usize;
                                        o += 2;
                                        x
                                    }
                                    _ => {
                                        return Err(Error::Bad(format!(
                                            "direct attribute {a} in shape {shape}"
                                        )));
                                    }
                                };
                                match a {
                                    attr::PNMTXIDX => {
                                        if !single {
                                            let slot = slots[(idx / 3).min(9)];
                                            v.skin = skin_of(slot);
                                        }
                                    }
                                    attr::POS => {
                                        let arr = array(a).ok_or(Error::Truncated(0))?;
                                        let n = if arr.fmt.count == 0 { 2 } else { 3 };
                                        let c = arr
                                            .vec(idx, n)
                                            .ok_or_else(|| Error::Bad("position index".into()))?;
                                        v.position =
                                            Vec3::new(c[0], c[1], *c.get(2).unwrap_or(&0.0));
                                    }
                                    attr::NRM => {
                                        if let Some(c) = array(a).and_then(|arr| arr.vec(idx, 3)) {
                                            v.normal = Vec3::new(c[0], c[1], c[2]);
                                        }
                                    }
                                    attr::CLR0 | attr::CLR1 => {
                                        if let Some(c) = array(a).and_then(|arr| arr.color(idx)) {
                                            v.colors[(a - attr::CLR0) as usize] = c;
                                        }
                                    }
                                    attr::TEX0..=attr::TEX7 => {
                                        let t = (a - attr::TEX0) as usize;
                                        if t < 3
                                            && let Some(arr) = array(a)
                                        {
                                            let n = if arr.fmt.count == 0 { 1 } else { 2 };
                                            if let Some(c) = arr.vec(idx, n) {
                                                v.uv[t] =
                                                    Vec2::new(c[0], *c.get(1).unwrap_or(&0.0));
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            batch.vertices.push(v);
                        }
                        let n = count as u32;
                        let idx = &mut batch.indices;
                        match prim {
                            0x90 => idx.extend((0..n / 3 * 3).map(|i| first + i)),
                            0x98 => {
                                for i in 0..n.saturating_sub(2) {
                                    let (a, b) = if i % 2 == 0 { (i, i + 1) } else { (i + 1, i) };
                                    idx.extend([first + a, first + b, first + i + 2]);
                                }
                            }
                            0xA0 => {
                                for i in 1..n.saturating_sub(1) {
                                    idx.extend([first, first + i, first + i + 1]);
                                }
                            }
                            0x80 => {
                                for q in 0..n / 4 {
                                    let b = first + q * 4;
                                    idx.extend([b, b + 1, b + 2, b, b + 2, b + 3]);
                                }
                            }
                            _ => {} // lines and points
                        }
                    }
                    // Anything else ends the useful part (padding).
                    _ => break,
                }
            }
        }
        Ok(batch)
    }
}

/// `JNT1` names and bind transforms; parents from `INF1`.
fn read_joints(d: &[u8]) -> Result<Vec<Joint>, Error> {
    let (b, r) = chunk(d, b"JNT1")?;
    let count = r.u16(b + 8)? as usize;
    let entries = b + r.u32(b + 0xC)? as usize;
    let remap = b + r.u32(b + 0x10)? as usize;
    let names = r.name_table(b + r.u32(b + 0x14)? as usize)?;
    let mut joints = Vec::with_capacity(count);
    for i in 0..count {
        let e = entries + r.u16(remap + i * 2)? as usize * 0x40;
        joints.push(Joint {
            name: names.get(i).cloned().unwrap_or_default(),
            parent: None,
            kind: r.u16(e)?,
            scale_compensate: r.u8(e + 2)? == 1,
            scale: vec3(r, e + 4)?,
            rotation: euler(r, e + 0x10)?,
            translation: vec3(r, e + 0x18)?,
        });
    }
    // Scene graph: 0x10 joint, 0x11 material, 0x12 shape, 0x01 down, 0x02 up.
    let (b, r) = chunk(d, b"INF1")?;
    let mut h = b + r.u32(b + 0x14)? as usize;
    let mut stack: Vec<Option<usize>> = Vec::new();
    let mut last: Option<usize> = None;
    let mut current: Option<usize> = None;
    loop {
        let (t, i) = (r.u16(h)?, r.u16(h + 2)? as usize);
        h += 4;
        match t {
            0 => break,
            1 => {
                stack.push(current);
                current = last;
            }
            2 => current = stack.pop().flatten(),
            0x10 => {
                if let Some(j) = joints.get_mut(i) {
                    j.parent = current;
                }
                last = Some(i);
            }
            _ => {}
        }
    }
    Ok(joints)
}

/// (material, shape, joint) triples from `INF1`.
fn read_hierarchy(d: &[u8]) -> Result<Vec<(usize, usize, usize)>, Error> {
    let (b, r) = chunk(d, b"INF1")?;
    let mut h = b + r.u32(b + 0x14)? as usize;
    // Each level remembers the last joint and material seen in it.
    let mut stack: Vec<(usize, Option<usize>)> = Vec::new();
    let (mut joint, mut material) = (0usize, None::<usize>);
    let mut out = Vec::new();
    loop {
        let (t, i) = (r.u16(h)?, r.u16(h + 2)? as usize);
        h += 4;
        match t {
            0 => break,
            1 => stack.push((joint, material)),
            2 => {
                if let Some((j, m)) = stack.pop() {
                    (joint, material) = (j, m);
                }
            }
            0x10 => joint = i,
            0x11 => material = Some(i),
            0x12 => {
                if let Some(m) = material {
                    out.push((m, i, joint));
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

type Skinning = (Vec<DrawMtx>, Vec<Vec<(u16, f32)>>, Vec<Mat4>);

/// `DRW1` and `EVP1`.
fn read_skin(d: &[u8], joint_count: usize) -> Result<Skinning, Error> {
    let (b, r) = chunk(d, b"DRW1")?;
    let n = r.u16(b + 8)? as usize;
    let flags = b + r.u32(b + 0xC)? as usize;
    let index = b + r.u32(b + 0x10)? as usize;
    let draw_mtx = (0..n)
        .map(|i| {
            let ix = r.u16(index + i * 2)?;
            Ok(if r.u8(flags + i)? == 0 {
                DrawMtx::Joint(ix)
            } else {
                DrawMtx::Envelope(ix)
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;

    let (b, r) = chunk(d, b"EVP1")?;
    let n = r.u16(b + 8)? as usize;
    let mut envelopes = Vec::with_capacity(n);
    let mut inverse_bind = Vec::new();
    if n > 0 {
        let counts = b + r.u32(b + 0xC)? as usize;
        let indices = b + r.u32(b + 0x10)? as usize;
        let weights = b + r.u32(b + 0x14)? as usize;
        let inv = b + r.u32(b + 0x18)? as usize;
        let mut k = 0;
        let mut max_joint = 0;
        for e in 0..n {
            let c = r.u8(counts + e)? as usize;
            let mut list = Vec::with_capacity(c);
            for _ in 0..c {
                let j = r.u16(indices + k * 2)?;
                max_joint = max_joint.max(j as usize + 1);
                list.push((j, r.f32(weights + k * 4)?));
                k += 1;
            }
            envelopes.push(list);
        }
        // One 3x4 row-major matrix per joint, up to the highest joint an envelope uses.
        for j in 0..max_joint.max(joint_count.min(max_joint)) {
            let o = inv + j * 48;
            let row = |i: usize| -> Result<Vec4, Error> {
                Ok(Vec4::new(
                    r.f32(o + i * 16)?,
                    r.f32(o + i * 16 + 4)?,
                    r.f32(o + i * 16 + 8)?,
                    r.f32(o + i * 16 + 12)?,
                ))
            };
            let (r0, r1, r2) = (row(0)?, row(1)?, row(2)?);
            inverse_bind.push(Mat4::from_cols(
                Vec4::new(r0.x, r1.x, r2.x, 0.0),
                Vec4::new(r0.y, r1.y, r2.y, 0.0),
                Vec4::new(r0.z, r1.z, r2.z, 0.0),
                Vec4::new(r0.w, r1.w, r2.w, 1.0),
            ));
        }
    }
    Ok((draw_mtx, envelopes, inverse_bind))
}

/// `VTX1`: arrays indexed by `GXAttr`.
fn read_vertex(d: &[u8]) -> Result<Vec<Array>, Error> {
    let (b, r) = chunk(d, b"VTX1")?;
    let size = r.u32(b + 4)? as usize;
    let fmt_list = b + r.u32(b + 8)? as usize;
    // Array offsets: POS, NRM, NBT, CLR0, CLR1, TEX0..7.
    let slots: [(u32, usize); 13] = [
        (attr::POS, 0xC),
        (attr::NRM, 0x10),
        (0x19, 0x14),
        (attr::CLR0, 0x18),
        (attr::CLR1, 0x1C),
        (attr::TEX0, 0x20),
        (attr::TEX0 + 1, 0x24),
        (attr::TEX0 + 2, 0x28),
        (attr::TEX0 + 3, 0x2C),
        (attr::TEX0 + 4, 0x30),
        (attr::TEX0 + 5, 0x34),
        (attr::TEX0 + 6, 0x38),
        (attr::TEX0 + 7, 0x3C),
    ];
    let offsets: Vec<usize> = slots
        .iter()
        .map(|&(_, o)| r.u32(b + o).map(|v| v as usize))
        .collect::<Result<_, _>>()?;
    let mut arrays = vec![Array::default(); 0x1A];
    let mut f = fmt_list;
    loop {
        let a = r.u32(f)?;
        if a == attr::NULL {
            break;
        }
        let fmt = Format {
            count: r.u32(f + 4)?,
            kind: r.u32(f + 8)?,
            frac: r.u8(f + 12)?,
        };
        f += 16;
        let Some(slot) = slots.iter().position(|s| s.0 == a) else {
            continue;
        };
        let start = offsets[slot];
        if start == 0 {
            continue;
        }
        let end = offsets
            .iter()
            .copied()
            .filter(|&o| o > start)
            .min()
            .unwrap_or(size);
        if let Some(arr) = arrays.get_mut(a as usize) {
            *arr = Array {
                fmt,
                data: r.bytes(b + start, end.min(size) - start)?.to_vec(),
            };
        }
    }
    Ok(arrays)
}

/// `SHP1` (`J3DShapeFactory`).
fn read_shapes(d: &[u8]) -> Result<Vec<Shape>, Error> {
    let (b, r) = chunk(d, b"SHP1")?;
    let n = r.u16(b + 8)? as usize;
    let init = b + r.u32(b + 0xC)? as usize;
    let index = b + r.u32(b + 0x10)? as usize;
    let desc_base = b + r.u32(b + 0x18)? as usize;
    let mtx_table = b + r.u32(b + 0x1C)? as usize;
    let dl_base = b + r.u32(b + 0x20)? as usize;
    let mtx_init = b + r.u32(b + 0x24)? as usize;
    let draw_init = b + r.u32(b + 0x28)? as usize;
    let mut shapes = Vec::with_capacity(n);
    for s in 0..n {
        let e = init + r.u16(index + s * 2)? as usize * 0x28;
        let mtx_type = r.u8(e)?;
        let groups = r.u16(e + 2)? as usize;
        let mut desc = Vec::new();
        let mut o = desc_base + r.u16(e + 4)? as usize;
        loop {
            let a = r.u32(o)?;
            if a == attr::NULL {
                break;
            }
            desc.push((a, r.u32(o + 4)?));
            o += 8;
        }
        let mtx0 = r.u16(e + 6)? as usize;
        let draw0 = r.u16(e + 8)? as usize;
        let mut packets = Vec::with_capacity(groups);
        for g in 0..groups {
            let m = mtx_init + (mtx0 + g) * 8;
            let use_mtx = r.u16(m)?;
            let count = r.u16(m + 2)? as usize;
            let first = r.u32(m + 4)? as usize;
            let table = (0..count)
                .map(|i| r.u16(mtx_table + (first + i) * 2))
                .collect::<Result<Vec<_>, _>>()?;
            let dw = draw_init + (draw0 + g) * 8;
            let size = r.u32(dw)? as usize;
            let off = r.u32(dw + 4)? as usize;
            packets.push(Packet {
                use_mtx,
                table,
                dl: r.bytes(dl_base + off, size)?.to_vec(),
            });
        }
        shapes.push(Shape {
            mtx_type,
            desc,
            packets,
        });
    }
    Ok(shapes)
}

/// `TEX1`.
fn read_textures(d: &[u8]) -> Result<Vec<Texture>, Error> {
    let (b, r) = chunk(d, b"TEX1")?;
    let n = r.u16(b + 8)? as usize;
    let headers = b + r.u32(b + 0xC)? as usize;
    let names = r.name_table(b + r.u32(b + 0x10)? as usize)?;
    (0..n)
        .map(|i| {
            read_timg(
                d,
                headers + i * 0x20,
                names.get(i).cloned().unwrap_or_default(),
            )
        })
        .collect()
}

/// `MAT3` (`J3DMaterialFactory`): each material's init data indexes shared component tables.
fn read_materials(d: &[u8]) -> Result<Vec<Material>, Error> {
    let (b, r) = chunk(d, b"MAT3")?;
    let n = r.u16(b + 8)? as usize;
    let table = |slot: usize| -> Result<usize, Error> { Ok(b + r.u32(b + slot)? as usize) };
    let init = table(0xC)?;
    let ids = table(0x10)?;
    let names = r.name_table(table(0x14)?)?;
    let (cull_t, mat_color_t, chan_num_t, chan_t, amb_t) = (
        table(0x1C)?,
        table(0x20)?,
        table(0x24)?,
        table(0x28)?,
        table(0x2C)?,
    );
    let (gen_num_t, gen_t, tex_mtx_t, tex_no_t, order_t) = (
        table(0x34)?,
        table(0x38)?,
        table(0x40)?,
        table(0x48)?,
        table(0x4C)?,
    );
    let (tev_color_t, kcolor_t, stage_num_t, stage_t, swap_t, swap_table_t) = (
        table(0x50)?,
        table(0x54)?,
        table(0x58)?,
        table(0x5C)?,
        table(0x60)?,
        table(0x64)?,
    );
    let (alpha_t, blend_t, zmode_t) = (table(0x6C)?, table(0x70)?, table(0x74)?);
    // A table pointer of 0 resolves to the chunk start; treat those entries as absent.
    let present = |t: usize| t != b;

    let mut out = Vec::with_capacity(n);
    for m in 0..n {
        let e = init + r.u16(ids + m * 2)? as usize * 0x14C;
        let u8_at = |o: usize| r.u8(e + o);
        let u16_at = |o: usize| r.u16(e + o);

        let cull = match u8_at(1)? {
            0xFF => Cull::Back,
            i => match r.u32(cull_t + i as usize * 4)? {
                0 => Cull::None,
                1 => Cull::Front,
                2 => Cull::Back,
                _ => Cull::All,
            },
        };
        let chan_num = match u8_at(2)? {
            0xFF => 1,
            i => r.u8(chan_num_t + i as usize)?,
        };
        let color = |t: usize, i: u16, default: [u8; 4]| -> Result<[u8; 4], Error> {
            if i == 0xFFFF || !present(t) {
                Ok(default)
            } else {
                rgba(r, t + i as usize * 4)
            }
        };
        let mat_colors = [
            color(mat_color_t, u16_at(8)?, [255; 4])?,
            color(mat_color_t, u16_at(0xA)?, [255; 4])?,
        ];
        let amb_colors = [
            color(amb_t, u16_at(0x14)?, [50, 50, 50, 50])?,
            color(amb_t, u16_at(0x16)?, [50, 50, 50, 50])?,
        ];
        let mut chans = [ColorChan::default(); 4];
        for (k, ch) in chans.iter_mut().enumerate() {
            let i = u16_at(0xC + k * 2)?;
            if i == 0xFFFF || !present(chan_t) {
                continue;
            }
            let o = chan_t + i as usize * 8;
            *ch = ColorChan {
                lighting: r.u8(o)? != 0,
                mat_from_vertex: r.u8(o + 1)? == 1,
                light_mask: r.u8(o + 2)?,
                diffuse_fn: r.u8(o + 3)?,
                attn_fn: r.u8(o + 4)?,
                amb_from_vertex: r.u8(o + 5)? == 1,
            };
        }

        let gen_num = match u8_at(3)? {
            0xFF => 0,
            i => r.u8(gen_num_t + i as usize)? as usize,
        };
        let mut tex_gens = Vec::with_capacity(gen_num);
        for k in 0..gen_num.min(8) {
            let i = u16_at(0x28 + k * 2)?;
            if i == 0xFFFF {
                tex_gens.push(TexGen {
                    kind: 1,
                    src: 4,
                    matrix: 60,
                });
                continue;
            }
            let o = gen_t + i as usize * 4;
            tex_gens.push(TexGen {
                kind: r.u8(o)?,
                src: r.u8(o + 1)?,
                matrix: r.u8(o + 2)?,
            });
        }
        let mut tex_mtx = [None; 8];
        for (k, slot) in tex_mtx.iter_mut().enumerate() {
            let i = u16_at(0x48 + k * 2)?;
            if i == 0xFFFF || !present(tex_mtx_t) {
                continue;
            }
            let o = tex_mtx_t + i as usize * 0x64;
            let mut effect = [0f32; 16];
            for (q, v) in effect.iter_mut().enumerate() {
                *v = r.f32(o + 0x24 + q * 4)?;
            }
            *slot = Some(TexMtx {
                projection: r.u8(o)?,
                info: r.u8(o + 1)?,
                center: vec3(r, o + 4)?,
                scale: Vec2::new(r.f32(o + 0x10)?, r.f32(o + 0x14)?),
                rotation: r.i16(o + 0x18)? as f32 * std::f32::consts::PI / 32768.0,
                translation: Vec2::new(r.f32(o + 0x1C)?, r.f32(o + 0x20)?),
                // Stored row-major.
                effect: Mat4::from_cols_array(&effect).transpose(),
            });
        }
        let mut textures = [None; 8];
        for (k, t) in textures.iter_mut().enumerate() {
            let i = u16_at(0x84 + k * 2)?;
            if i != 0xFFFF {
                *t = Some(r.u16(tex_no_t + i as usize * 2)?);
            }
        }
        let mut konst = [[255u8; 4]; 4];
        for (k, c) in konst.iter_mut().enumerate() {
            *c = color(kcolor_t, u16_at(0x94 + k * 2)?, [255; 4])?;
        }
        let mut tev_colors = [[255i16; 4]; 3];
        for (k, c) in tev_colors.iter_mut().enumerate() {
            let i = u16_at(0xDC + k * 2)?;
            if i == 0xFFFF {
                continue;
            }
            let o = tev_color_t + i as usize * 8;
            *c = [r.i16(o)?, r.i16(o + 2)?, r.i16(o + 4)?, r.i16(o + 6)?];
        }
        let mut swap_tables = [[0, 1, 2, 3]; 4];
        for (k, t) in swap_tables.iter_mut().enumerate() {
            let i = u16_at(0x124 + k * 2)?;
            if i != 0xFFFF && present(swap_table_t) {
                let o = swap_table_t + i as usize * 4;
                *t = [r.u8(o)?, r.u8(o + 1)?, r.u8(o + 2)?, r.u8(o + 3)?];
            }
        }
        let stage_num = match u8_at(4)? {
            0xFF => 1,
            i => r.u8(stage_num_t + i as usize)? as usize,
        };
        let mut stages = Vec::with_capacity(stage_num);
        for k in 0..stage_num.min(16) {
            let oi = u16_at(0xBC + k * 2)?;
            let (tex_coord, tex_map, channel) = if oi == 0xFFFF {
                (0xFF, 0xFF, 0xFF)
            } else {
                let o = order_t + oi as usize * 4;
                (r.u8(o)?, r.u8(o + 1)?, r.u8(o + 2)?)
            };
            let si = u16_at(0xE4 + k * 2)?;
            let o = stage_t + si as usize * 20;
            let comb = |base: usize| -> Result<TevCombiner, Error> {
                Ok(TevCombiner {
                    a: r.u8(o + base)?,
                    b: r.u8(o + base + 1)?,
                    c: r.u8(o + base + 2)?,
                    d: r.u8(o + base + 3)?,
                    op: r.u8(o + base + 4)?,
                    bias: r.u8(o + base + 5)?,
                    scale: r.u8(o + base + 6)?,
                    clamp: r.u8(o + base + 7)? != 0,
                    out: r.u8(o + base + 8)?,
                })
            };
            let wi = u16_at(0x104 + k * 2)?;
            let (ras_swap, tex_swap) = if wi == 0xFFFF || !present(swap_t) {
                (0, 0)
            } else {
                let o = swap_t + wi as usize * 4;
                (r.u8(o)?, r.u8(o + 1)?)
            };
            stages.push(TevStage {
                tex_coord,
                tex_map,
                channel,
                kcolor_sel: u8_at(0x9C + k)?,
                kalpha_sel: u8_at(0xAC + k)?,
                color: comb(1)?,
                alpha: comb(10)?,
                ras_swap,
                tex_swap,
            });
        }
        let alpha_compare = match u16_at(0x146)? {
            0xFFFF => AlphaCompare::ALWAYS,
            i => {
                let o = alpha_t + i as usize * 8;
                AlphaCompare {
                    comp0: r.u8(o)?,
                    ref0: r.u8(o + 1)?,
                    op: r.u8(o + 2)?,
                    comp1: r.u8(o + 3)?,
                    ref1: r.u8(o + 4)?,
                }
            }
        };
        let blend_mode = match u16_at(0x148)? {
            0xFFFF => BlendMode {
                mode: 0,
                src: 1,
                dst: 0,
            },
            i => {
                let o = blend_t + i as usize * 4;
                BlendMode {
                    mode: r.u8(o)?,
                    src: r.u8(o + 1)?,
                    dst: r.u8(o + 2)?,
                }
            }
        };
        let (z_test, z_write) = match u8_at(6)? {
            0xFF => (true, true),
            i => {
                let o = zmode_t + i as usize * 4;
                (r.u8(o)? != 0, r.u8(o + 2)? != 0)
            }
        };
        let ind = table(0x18)?;
        let ind_at = ind + m * 0x138;
        let indirect = if present(ind) && ind > table(0x14)? + 4 && r.u8(ind_at)? != 0 {
            let count = r.u8(ind_at + 1)? as usize;
            let mut orders = Vec::new();
            let mut scales = Vec::new();
            for i in 0..count.min(4) {
                orders.push([r.u8(ind_at + 4 + i * 4)?, r.u8(ind_at + 5 + i * 4)?]);
                scales.push([r.u8(ind_at + 0x68 + i * 4)?, r.u8(ind_at + 0x69 + i * 4)?]);
            }
            let mut matrices = [[[0.; 3]; 2]; 3];
            for (i, matrix) in matrices.iter_mut().enumerate() {
                let at = ind_at + 0x14 + i * 28;
                let scale = 2f32.powi(r.u8(at + 24)? as i8 as i32);
                for (j, v) in matrix.iter_mut().flatten().enumerate() {
                    *v = r.f32(at + j * 4)? * scale;
                }
            }
            let mut ind_stages = Vec::new();
            for i in 0..stages.len() {
                ind_stages.push(r.bytes(ind_at + 0x78 + i * 12, 9)?.try_into().unwrap());
            }
            Some(Indirect {
                orders,
                scales,
                matrices,
                stages: ind_stages,
            })
        } else {
            None
        };
        out.push(Material {
            name: names.get(m).cloned().unwrap_or_default(),
            mode: u8_at(0)?,
            cull,
            chan_num,
            mat_colors,
            amb_colors,
            chans,
            tex_gens,
            tex_mtx,
            indirect,
            textures,
            stages,
            tev_colors,
            konst,
            swap_tables,
            alpha_compare,
            blend_mode,
            z_test,
            z_write,
        });
    }
    Ok(out)
}
