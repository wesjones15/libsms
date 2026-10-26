//! Materials for hosts whose shaders can't run TEV (Risk of Rain 2's HGStandard takes one
//! texture times one colour).
//!
//! [`bake`] runs a material's TEV stages on the CPU once per texel over the mesh's first set
//! of texture coordinates, folding every texture map, colour register, konst colour and the
//! unlit material colour into one RGBA image; lighting is left to the host. GX semantics are
//! the viewer's TEV interpreter's (`GXSetTevColorIn/Op` incl. compare ops, konst selections,
//! swap tables; `J3DTevBlock::load`, J3DMaterial.cpp). Reads it can't place over those
//! coordinates count as the map's average colour: other coordinate sets, colour-channel
//! (`GX_TG_SRTG`) coordinates, and environment maps in a material that also has a mesh-mapped
//! texture. A material whose only maps are environment maps (FLUDD's shine) bakes over the
//! environment map's own space ([`Baked::env`]) and the host gives that batch sphere-mapped
//! coordinates each frame. The alpha test is folded into the alpha (0 or 1), so a host's
//! cutout at one half reproduces it. [`flat_color`] is the same with no texture, for
//! untextured materials.

use libsms_assets::bmd::{Material, TevCombiner};
use libsms_assets::texture::Wrap;
use libsms_assets::{Bmd, Image};

pub use libcommon::tev::C4;
use libcommon::tev::{Combine, Regs, alpha_in, color_in, combine_alpha, combine_color, konst};
use libcommon::texture::sample_rgba8_bilinear;

/// Per-draw changes to a material's inputs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Inputs {
    /// Konst colour 0's alpha: Mario's dirt level (`TMario::addDirty`), 0 clean.
    pub k0_alpha: Option<u8>,
    /// Texture pattern swaps (`.btp`): map `i` shows texture `tex[i]` instead.
    pub tex: [Option<u16>; 8],
}

/// One texture image with its wrap modes (0 repeat, 1 clamp, 2 mirror).
#[derive(Debug, Clone)]
pub struct Tex {
    pub image: Image,
    pub wrap: [u8; 2],
    pub average: C4,
}

impl Tex {
    pub fn new(image: Image, wrap_s: Wrap, wrap_t: Wrap) -> Tex {
        let n = (image.rgba.len() / 4).max(1) as f32;
        let mut s = [0.0f32; 4];
        for px in image.rgba.as_chunks::<4>().0 {
            for (acc, &c) in s.iter_mut().zip(px) {
                *acc += c as f32;
            }
        }
        Tex {
            average: s.map(|v| v / n / 255.0),
            wrap: [wrap(wrap_s), wrap(wrap_t)],
            image,
        }
    }
}

pub fn wrap(w: Wrap) -> u8 {
    w.to_host()
}

/// A baked material: RGBA8, rows top first, sampled with the mesh's texture coordinates.
#[derive(Debug, Clone)]
pub struct Baked {
    pub width: u32,
    pub height: u32,
    /// 0 repeat, 1 clamp, 2 mirror.
    pub wrap: [u8; 2],
    pub rgba: Vec<u8>,
    /// Baked over an environment map: sample it at the view-space normal through this
    /// matrix ([`env_uv`]), not the mesh's coordinates.
    pub env: Option<Option<[[f32; 3]; 2]>>,
}

impl Baked {
    /// The lowest alpha anywhere, 0..1.
    pub fn min_alpha(&self) -> f32 {
        self.rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| p[3])
            .min()
            .unwrap_or(255) as f32
            / 255.0
    }
}

/// An environment-mapped texture coordinate: view-space normal `n` (x right, y up) through
/// `mtx`, as GX's normal texgen with J3D's `fixupMtx1` folded in when there is a matrix (the
/// viewer's shader does the same).
pub fn env_uv(nx: f32, ny: f32, mtx: Option<[[f32; 3]; 2]>) -> [f32; 2] {
    match mtx {
        Some(m) => {
            let (x, y) = (0.5 * nx + 0.5, 0.5 - 0.5 * ny);
            [
                m[0][0] * x + m[0][1] * y + m[0][2],
                m[1][0] * x + m[1][1] * y + m[1][2],
            ]
        }
        None => [nx, ny],
    }
}

fn swizzle(c: C4, table: [u8; 4]) -> C4 {
    table.map(|i| c[(i & 3) as usize])
}


fn b8(v: f32) -> u32 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u32
}

/// `GXSetAlphaCompare` on the 8-bit alpha.
fn alpha_passes(m: &Material, a: f32) -> bool {
    let ac = &m.alpha_compare;
    let a = b8(a);
    let cmp = |comp: u8, r: u8| {
        let r = r as u32;
        match comp {
            0 => false,
            1 => a < r,
            2 => a == r,
            3 => a <= r,
            4 => a > r,
            5 => a != r,
            6 => a >= r,
            _ => true,
        }
    };
    let (p0, p1) = (cmp(ac.comp0, ac.ref0), cmp(ac.comp1, ac.ref1));
    match ac.op {
        0 => p0 && p1,
        1 => p0 || p1,
        2 => p0 != p1,
        _ => p0 == p1,
    }
}

/// Where a stage's texture read comes from.
#[derive(Debug, Clone, Copy)]
enum Read {
    /// No texture: white.
    None,
    /// Texture `tex` over the mesh's first coordinates, through a texture matrix.
    Uv {
        tex: usize,
        mtx: Option<[[f32; 3]; 2]>,
    },
    /// An environment map (`GX_TG_NRM`) through a texture matrix.
    Env {
        tex: usize,
        mtx: Option<[[f32; 3]; 2]>,
    },
    /// A colour channel's red and green as coordinates (`GX_TG_SRTG`): Mario's toon ramps,
    /// read where the channel's baked colour points.
    Srtg { tex: usize, channel: usize },
    /// Some other generated coordinate, as the map's average colour.
    Average(C4),
}

/// A model's inputs to baking: its materials and texture images (after any replacements, like
/// the caps' and hands' texture 0 being the body's), and each material's channel colours as
/// baked: fully lit by its diffuse lights, with no specular highlight (the host lights the
/// result).
pub struct Source<'a> {
    pub bmd: &'a Bmd,
    pub textures: &'a [Option<Tex>],
    /// Per material: channel 0 and 1 colours (RGBA 0..1).
    pub channels: &'a [[C4; 2]],
}

impl Source<'_> {
    /// GX 8-bit indirect displacement in texels. J3DIndTevStage::load; the M gate
    /// uses a biased ABG sample and matrix 0, with no wrapping or previous offset.
    fn indirect_uv(&self, mi: usize, stage: usize, u: f32, v: f32, size: [u32; 2]) -> [f32; 2] {
        let m = &self.bmd.materials[mi];
        let Some(ind) = &m.indirect else {
            return [u, v];
        };
        let Some(s) = ind.stages.get(stage) else {
            return [u, v];
        };
        if !(1..=3).contains(&s[3]) || s[1] != 0 || s[4..9] != [0; 5] {
            return [u, v];
        }
        let Some(order) = ind.orders.get(s[0] as usize) else {
            return [u, v];
        };
        let Some(texgen) = m.tex_gens.get(order[0] as usize) else {
            return [u, v];
        };
        let Some(tex) = m
            .textures
            .get(order[1] as usize)
            .copied()
            .flatten()
            .and_then(|id| self.tex(id as usize))
        else {
            return [u, v];
        };
        let uv = m
            .tex_mtx
            .get(texgen.matrix.wrapping_sub(30) as usize / 3)
            .copied()
            .flatten()
            .map_or([u, v], |m| {
                let r = m.srt_rows();
                [
                    r[0][0] * u + r[0][1] * v + r[0][2],
                    r[1][0] * u + r[1][1] * v + r[1][2],
                ]
            });
        let scale = ind.scales[s[0] as usize];
        let c = sample(
            &tex.image,
            tex.wrap,
            uv[0] * 2f32.powi(-(scale[0] as i32)),
            uv[1] * 2f32.powi(-(scale[1] as i32)),
        );
        let abg = [c[3], c[2], c[1]];
        let values: [f32; 3] = std::array::from_fn(|i| {
            (abg[i] * 255.).round() - if s[2] & (1 << i) != 0 { 128. } else { 0. }
        });
        let matrix = ind.matrices[s[3] as usize - 1];
        let offset: [f32; 2] = std::array::from_fn(|i| {
            (0..3).map(|j| matrix[i][j] * values[j]).sum::<f32>() / size[i].max(1) as f32
        });
        [u + offset[0], v + offset[1]]
    }
    fn tex(&self, i: usize) -> Option<&Tex> {
        self.textures.get(i).and_then(|t| t.as_ref())
    }

    /// A texture read at a colour channel's red and green (`GX_TG_SRTG`).
    fn srtg(&self, mi: usize, tex: usize, channel: usize) -> C4 {
        let ch = self
            .channels
            .get(mi)
            .map_or([1.0; 4], |c| c[channel.min(1)]);
        self.tex(tex)
            .map_or([1.0; 4], |t| sample(&t.image, t.wrap, ch[0], ch[1]))
    }

    /// Each stage's read through the model's current texture matrices, including BTK updates.
    fn reads(&self, mi: usize, inputs: &Inputs) -> Vec<Read> {
        let m = &self.bmd.materials[mi];
        m.stages
            .iter()
            .map(|s| {
                if s.tex_map == 0xFF {
                    return Read::None;
                }
                let map = (s.tex_map & 7) as usize;
                let Some(tex) = inputs.tex[map].or(m.textures[map]).map(|t| t as usize) else {
                    return Read::None;
                };
                let Some(t) = self.tex(tex) else {
                    return Read::None;
                };
                let Some(g) = m.tex_gens.get(s.tex_coord as usize) else {
                    return Read::None;
                };
                let mtx = (g.kind <= 1 && g.matrix != 60)
                    .then(|| m.tex_mtx.get((g.matrix.wrapping_sub(30) / 3) as usize))
                    .flatten()
                    .and_then(|m| m.as_ref())
                    .map(|m| m.srt_rows());
                match (g.kind, g.src) {
                    (0 | 1, 4) => Read::Uv { tex, mtx },
                    (0 | 1, 1) => Read::Env { tex, mtx },
                    (10, 20) => Read::Srtg { tex, channel: 1 },
                    (10, _) => Read::Srtg { tex, channel: 0 },
                    _ => Read::Average(t.average),
                }
            })
            .collect()
    }

    /// Runs material `mi` with `tex(stage)` for each stage's texture read; the alpha test is
    /// folded into the alpha.
    fn eval(
        &self,
        mi: usize,
        inputs: &Inputs,
        preserve_alpha: bool,
        tex: &dyn Fn(usize) -> C4,
    ) -> C4 {
        let m = &self.bmd.materials[mi];
        let ch = self.channels.get(mi).copied().unwrap_or([[1.0; 4]; 2]);
        let mut k = m.konst.map(|c| c.map(|v| v as f32 / 255.0));
        if let Some(a) = inputs.k0_alpha {
            k[0][3] = a as f32 / 255.0;
        }
        let mut g = Regs {
            prev: [0.0; 4],
            r: m.tev_colors.map(|c| c.map(|v| v as f32 / 255.0)),
        };
        let combine = |c: &TevCombiner| Combine {
            op: c.op,
            bias: c.bias,
            scale: c.scale,
            clamp: c.clamp,
        };
        for (i, s) in m.stages.iter().enumerate() {
            let ras = match s.channel {
                0 => [ch[0][0], ch[0][1], ch[0][2], 0.0],
                1 => [ch[1][0], ch[1][1], ch[1][2], 0.0],
                4 => ch[0],
                5 => ch[1],
                _ => [0.0; 4],
            };
            let ras = swizzle(ras, m.swap_tables[(s.ras_swap & 3) as usize]);
            let tx = swizzle(tex(i), m.swap_tables[(s.tex_swap & 3) as usize]);
            let kc = konst(s.kcolor_sel, &k);
            let ka = konst(s.kalpha_sel, &k);
            let c = &s.color;
            let rgb = combine_color(
                &combine(c),
                color_in(c.a, &g, tx, ras, kc),
                color_in(c.b, &g, tx, ras, kc),
                color_in(c.c, &g, tx, ras, kc),
                color_in(c.d, &g, tx, ras, kc),
            );
            let al = &s.alpha;
            let a = combine_alpha(
                &combine(al),
                alpha_in(al.a, &g, tx, ras, ka),
                alpha_in(al.b, &g, tx, ras, ka),
                alpha_in(al.c, &g, tx, ras, ka),
                alpha_in(al.d, &g, tx, ras, ka),
            );
            g.slot(c.out)[..3].copy_from_slice(&rgb);
            g.slot(al.out)[3] = a;
        }
        let mut out = g.prev.map(|v| v.clamp(0.0, 1.0));
        if !m.alpha_compare.always_passes() {
            out[3] = if !alpha_passes(m, out[3]) {
                0.0
            } else if preserve_alpha {
                out[3]
            } else {
                1.0
            };
        }
        out
    }

    /// The colour of material `mi` with every texture over the mesh's coordinates read as
    /// white (what a baked texture is multiplied by is white; this is for untextured
    /// materials), RGBA 0..1.
    pub fn flat_color(&self, mi: usize, inputs: &Inputs) -> C4 {
        let rd = self.reads(mi, inputs);
        self.eval(mi, inputs, false, &|i| match rd.get(i) {
            Some(Read::Average(c)) => *c,
            Some(Read::Srtg { tex, channel }) => self.srtg(mi, *tex, *channel),
            Some(Read::Env { tex, .. }) => self.tex(*tex).map_or([1.0; 4], |t| t.average),
            _ => [1.0; 4],
        })
    }

    /// Whether material `mi` reads a texture over the mesh's or an environment map's
    /// coordinates (so has a [`Source::bake`]).
    pub fn textured(&self, mi: usize, inputs: &Inputs) -> bool {
        self.reads(mi, inputs)
            .iter()
            .any(|r| matches!(r, Read::Uv { .. } | Read::Env { .. }))
    }

    /// Material `mi` with `inputs`, baked over the mesh's first texture coordinates at the
    /// largest of its textures' sizes (at most `max_size` a side), or failing that over its
    /// environment maps' own space ([`Baked::env`]). `None` for an untextured material.
    pub fn bake(&self, mi: usize, inputs: &Inputs, max_size: u32) -> Option<Baked> {
        self.bake_with_alpha(mi, inputs, max_size, false)
    }

    /// Keep fractional TEV alpha after testing. Materials such as the M gate
    /// multiply source colour by that alpha with a zero destination factor.
    pub fn bake_preserving_alpha(
        &self,
        mi: usize,
        inputs: &Inputs,
        max_size: u32,
    ) -> Option<Baked> {
        self.bake_with_alpha(mi, inputs, max_size, true)
    }

    fn bake_with_alpha(
        &self,
        mi: usize,
        inputs: &Inputs,
        max_size: u32,
        preserve_alpha: bool,
    ) -> Option<Baked> {
        let rd = self.reads(mi, inputs);
        let uv_mapped = rd.iter().any(|r| matches!(r, Read::Uv { .. }));
        let mut size = (0u32, 0u32);
        let mut out_wrap = None;
        let mut env = None;
        for r in &rd {
            let (tex, env_mtx) = match *r {
                Read::Uv { tex, .. } => (tex, None),
                Read::Env { tex, mtx } if !uv_mapped => (tex, Some(mtx)),
                _ => continue,
            };
            if let Some(t) = self.tex(tex) {
                size = (size.0.max(t.image.width), size.1.max(t.image.height));
                out_wrap.get_or_insert(t.wrap);
                if env.is_none() {
                    env = env_mtx;
                }
            }
        }
        let wrap = out_wrap?;
        let (w, h) = (size.0.clamp(1, max_size), size.1.clamp(1, max_size));
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let (u, v) = ((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
                let c = self.eval(mi, inputs, preserve_alpha, &|i| match rd.get(i) {
                    Some(Read::Uv { tex, mtx }) => match self.tex(*tex) {
                        Some(t) => {
                            let (su, sv) = match mtx {
                                Some(m) => (
                                    m[0][0] * u + m[0][1] * v + m[0][2],
                                    m[1][0] * u + m[1][1] * v + m[1][2],
                                ),
                                None => (u, v),
                            };
                            let [iu, iv] =
                                self.indirect_uv(mi, i, u, v, [t.image.width, t.image.height]);
                            sample(&t.image, t.wrap, su + iu - u, sv + iv - v)
                        }
                        None => [1.0; 4],
                    },
                    // Over the mesh's coordinates an environment map is its average; baked
                    // over its own space, it's read where the host's coordinates will point.
                    Some(Read::Env { tex, .. }) if uv_mapped => {
                        self.tex(*tex).map_or([1.0; 4], |t| t.average)
                    }
                    Some(Read::Env { tex, .. }) => match self.tex(*tex) {
                        Some(t) => sample(&t.image, t.wrap, u, v),
                        None => [1.0; 4],
                    },
                    Some(Read::Average(c)) => *c,
                    Some(Read::Srtg { tex, channel }) => self.srtg(mi, *tex, *channel),
                    _ => [1.0; 4],
                });
                rgba.extend(c.map(|v| (v * 255.0).round() as u8));
            }
        }
        Some(Baked {
            width: w,
            height: h,
            wrap,
            rgba,
            env,
        })
    }
}

/// Bilinear filtering at `uv`, wrapping as the texture says (host codes: 0 repeat, 1 clamp,
/// 2 mirror). White for an empty image.
fn sample(img: &Image, wrap: [u8; 2], u: f32, v: f32) -> C4 {
    sample_rgba8_bilinear(img.width, img.height, &img.rgba, wrap.map(Wrap::from_host), u, v)
        .unwrap_or([1.0; 4])
}
