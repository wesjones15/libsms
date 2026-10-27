//! The game's own particle effects (JParticle, `libs/JSystem/src/JParticle/`), run on the CPU
//! from the disc's `.jpa` files ([`libsms_assets::jpa`]) and handed to hosts as textured quads
//! in the sim's world space, like the water's.
//!
//! The sim is `JPABaseEmitter::calc` and its helpers, once per 30 Hz game frame: emission
//! (`calcCreateParticle`, `createParticle`), fields (`JPAFieldManager`; gravity, air,
//! magnet, Newton attraction and vortex),
//! particle motion (`JPAParticle::calcVelocity`, `calcGlobalPosition`) and the draw-side
//! per-particle work (`JPADraw::calcParticle` with the visitors `setDrawCalcVisitors` picks:
//! scale, colour tables, alpha, spin). Shapes are built as `JPADrawExec*` draws them:
//! billboards, directional quads and (crossed) stripes.
//!
//! Colour: additive effects use `GX_BM_BLEND`, source alpha, one. TEV stage 0
//! (`JPADrawSetupTev::setupTev`) mixes the prm and env colours with the texture colour `t`
//! (`d + a (1 - c) + b c`, inputs per `JPABaseShape::unk48`), and its alpha is the texture's
//! times prm's. That is a polynomial in `t` with colour coefficients, so each effect's texture
//! is baked once into basis images (`(1 - t) tA` and `t tA`, or the quadratic Bernstein three
//! when the texture appears twice) and every particle is drawn once per basis image with the
//! coefficient (times prm alpha) as its vertex colour: summed by additive blending, that is
//! the game's colour exactly, for any host shader that multiplies a texture by a vertex colour.
//! A stripe with a second texture (`ETX1`, which reads the texture at other coordinates) can't
//! be split like that; its whole TEV is baked into a texture each frame instead.
//! Static random colour palettes with source-alpha blending use a single atlas so each
//! overlapping particle correctly hides earlier particles instead of adding their colours.
//!
//! Child particles (`SSP1`) use their own lifetime, motion, texture and inherited draw
//! parameters. Not ported: general
//! indirect texture stages (projected scene refraction is handed to the host), the
//! remaining field types. Source-alpha blending (the dust, smoke and stars) whose
//! TEV mixes env in (`d + a (1 - c) + b c` with env as an input) is drawn exactly as a black
//! coverage draw under the same additive basis images (`Paint::Basis::over`); one without env
//! is drawn as see-through texture times prm colour.

use std::collections::HashMap;
use std::f32::consts::TAU;
use std::path::Path;

use glam::{Mat3, Mat4, Vec3};
use libcommon::texture::{Wrap, sample_rgba8_nearest};
use libsms_assets::jpa::{BaseShape, ChildShape, ExtraShape, Jpa};
use libsms_assets::texture::Texture;

/// Game frames are four sim ticks (30 Hz).
pub const FRAME_TICKS: u64 = 4;

const STOP_EMIT: u32 = 0x1;
const ENABLE_DELETE: u32 = 0x8;
const FIRST_EMIT: u32 = 0x10;
const EMIT_NEXT_FRAME: u32 = 0x20;

const FIXED_DENSITY: u32 = 0x1;
const FIXED_INTERVAL: u32 = 0x2;
const INHERIT_SCALE: u32 = 0x4;
const FOLLOW_EMITTER: u32 = 0x8;

/// How a batch blends.
pub mod blend {
    /// `src * src.a + dst`. The vertex colours carry the alpha already.
    pub const ADDITIVE: u32 = 0;
    /// `src * src.a + dst * (1 - src.a)`.
    pub const ALPHA: u32 = 1;
    /// Refract the host's scene with a packed map (R=1, G=gain*Y, B=mask, A=gain*X).
    /// This replaces Sunshine's captured framebuffer, never the disc's empty placeholder.
    pub const REFRACTION: u32 = 2;
    /// Keep authored subtle offsets above RoR2's normal-Z discard threshold. Hosts divide
    /// their distortion magnitude by this and use B separately as the coverage mask.
    pub const REFRACTION_NORMAL_GAIN: f32 = 4.0;
}

/// One quad corner, sim world space.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct FxVertex {
    pub pos: Vec3,
    pub u: f32,
    pub v: f32,
    pub color: [f32; 4],
}

/// A run of quads (four corners each) over one texture.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct FxBatch {
    pub texture: u32,
    pub first_vertex: u32,
    pub vertex_count: u32,
    pub blend: u32,
}

/// An RGBA8 texture; `version` goes up when its pixels change.
#[derive(Debug, Clone)]
pub struct FxTexture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub wrap: [Wrap; 2],
    pub version: u32,
}

impl FxTexture {
    /// Host ABI sampler codes, shared with model textures: repeat/clamp/mirror.
    pub fn wrap_modes(&self) -> [u8; 2] {
        self.wrap.map(Wrap::to_host)
    }

    /// Shared nearest sampler for headless previews, including negative mirrored UVs.
    pub fn sample_nearest(&self, u: f32, v: f32) -> [u8; 4] {
        sample_rgba8_nearest(self.width, self.height, &self.rgba, self.wrap, u, v)
            .expect("valid particle texture and finite UVs")
    }
}

/// `JMath::TRandom_fast_`.
#[derive(Debug, Clone, Copy)]
struct Rng(u32);

impl Rng {
    fn get(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(0x19660d).wrapping_add(0x3c6ef35f);
        self.0
    }
    /// `get_ufloat_1`: [0, 1).
    fn f(&mut self) -> f32 {
        f32::from_bits((self.get() >> 9) | 0x3f80_0000) - 1.0
    }
    /// `getRandomRF`: [-1, 1).
    fn rf(&mut self) -> f32 {
        self.f() * 2.0 - 1.0
    }
    /// `getRandomSF`: [-0.5, 0.5).
    fn sf(&mut self) -> f32 {
        self.f() - 0.5
    }
    /// `getRandomSS`.
    fn ss(&mut self) -> f32 {
        ((self.get() & 0xffff) as i32 - 0x8000) as f32
    }
    /// `getRandomRS`.
    fn rs(&mut self) -> i16 {
        ((self.get() & 0x7fff) as i32 - 0x4000) as i16
    }
    /// A raw draw as an angle (`mRng.get()` passed as `s16`).
    fn angle(&mut self) -> i16 {
        self.get() as i16
    }
}

fn sin(a: i16) -> f32 {
    (a as f32 * TAU / 65536.0).sin()
}
fn cos(a: i16) -> f32 {
    (a as f32 * TAU / 65536.0).cos()
}

/// A GX `Mtx` 3x3 written row by row (`dst[row][col]`), as glam's column-major matrix.
fn rows(r: [[f32; 3]; 3]) -> Mat3 {
    Mat3::from_cols(
        Vec3::new(r[0][0], r[1][0], r[2][0]),
        Vec3::new(r[0][1], r[1][1], r[2][1]),
        Vec3::new(r[0][2], r[1][2], r[2][2]),
    )
}

/// `JPAGetXYZRotateMtx`.
fn rot_xyz(x: i16, y: i16, z: i16) -> Mat3 {
    let (sx, cx, sy, cy, sz, cz) = (sin(x), cos(x), sin(y), cos(y), sin(z), cos(z));
    rows([
        [cz * cy, -sz * cx + cz * sy * sx, sz * sx + cz * sy * cx],
        [sz * cy, cz * cx + sz * sy * sx, -cz * sx + sz * sy * cx],
        [-sy, cy * sx, cy * cx],
    ])
}

/// `JPAGetXYRotateMtx`.
fn rot_xy(x: i16, y: i16) -> Mat3 {
    let (sx, cx, sy, cy) = (sin(x), cos(x), sin(y), cos(y));
    rows([
        [cy, sy * sx, sy * cx],
        [0.0, cx, -sx],
        [-sy, cy * sx, cy * cx],
    ])
}

/// `JPAGetYZRotateMtx`.
fn rot_yz(y: i16, z: i16) -> Mat3 {
    let (sy, cy, sz, cz) = (sin(y), cos(y), sin(z), cos(z));
    rows([
        [cz * cy, -sz, cz * sy],
        [sz * cy, cz, sz * sy],
        [-sy, 0.0, cy],
    ])
}

/// `JPAGetYRotateMtx`.
fn rot_y(y: i16) -> Mat3 {
    let (s, c) = (sin(y), cos(y));
    rows([[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]])
}

/// `JPAVecToRotaMtx(dst, a, b)`.
fn vec_to_rota(a: Vec3, b: Vec3) -> Mat3 {
    let mut axis = a.cross(b);
    let c = a.dot(b);
    let s = axis.length();
    axis = if s > f32::EPSILON {
        axis / s
    } else {
        Vec3::ZERO
    };
    let t = 1.0 - c;
    rows([
        [
            c * (1.0 - axis.x * axis.x) + axis.x * axis.x,
            axis.x * axis.y * t + axis.z * s,
            axis.x * axis.z * t - axis.y * s,
        ],
        [
            axis.y * axis.x * t - axis.z * s,
            c * (1.0 - axis.y * axis.y) + axis.y * axis.y,
            axis.y * axis.z * t + axis.x * s,
        ],
        [
            axis.z * axis.x * t + axis.y * s,
            axis.z * axis.y * t - axis.x * s,
            c * (1.0 - axis.z * axis.z) + axis.z * axis.z,
        ],
    ])
}

/// `TVec3::setLength`: `v` scaled to `len` (zero stays zero).
fn set_length(v: Vec3, len: f32) -> Vec3 {
    let l = v.length();
    if l <= f32::EPSILON {
        Vec3::ZERO
    } else {
        v * (len / l)
    }
}

/// `JPADrawParams`.
#[derive(Debug, Clone, Copy)]
struct Look {
    /// `unk0`: the quad's remembered up, for directional shapes.
    up: Vec3,
    prm: [u8; 4],
    env: [u8; 4],
    alpha: f32,
    scale_x: f32,
    scale_y: f32,
    /// `unkC`.
    base_scale: f32,
    /// `unk24`.
    wave: f32,
    /// `unk28`.
    loop_offset: i32,
    /// `unk34`, `unk36`.
    angle: i16,
    spin: i16,
    /// `unk3A`.
    texture: u8,
}

#[derive(Debug, Clone, Copy)]
struct Particle {
    local: Vec3,
    global: Vec3,
    /// `unk14`: where the emitter was (followed every frame with `FOLLOW_EMITTER`).
    anchor: Vec3,
    base_vel: Vec3,
    field_accel: Vec3,
    field_vel: Vec3,
    vel: Vec3,
    /// `unk68`, `unk78`.
    dir: Vec3,
    moment: f32,
    air: f32,
    weight: f32,
    age: f32,
    life: f32,
    progress: f32,
    just_born: bool,
    dead: bool,
    follow: bool,
    look: Look,
    previous_local: Vec3,
    previous_global: Vec3,
    previous_look: Look,
}

/// `JPAEmitterInfo`: what one emitter's calc shares with its particles.
#[derive(Debug, Clone, Copy, Default)]
struct Info {
    /// `unk6C`: the bound matrix's rotation.
    global_rot: Mat3,
    /// `unkCC`: that times the emitter's own rotation.
    rot: Mat3,
    /// `unkFC`: and its scale.
    rot_scale: Mat3,
    /// `unk30`.
    scale: Vec3,
    /// `unk24`: the emitter in the world.
    pos: Vec3,
    axis_y: Vec3,
    /// `unk3C`, `unk12C`.
    dir: Vec3,
    dir_rot: Mat3,
    emit_count: i32,
    volume_index: i32,
    sphere_done_in_layer: i32,
    sphere_in_layer: i32,
    sphere_pitch: i16,
    sphere_yaw: i16,
    sphere_pitch_step: i16,
    sphere_yaw_step: i16,
    hemisphere_flip: bool,
}

/// One running emitter (`JPABaseEmitter`).
#[derive(Debug, Clone)]
struct Emitter {
    effect: usize,
    /// Which request keeps it alive (none once let go).
    slot: Option<u32>,
    /// The slot whose matrix it follows, kept after it's let go (`userWork` still points at
    /// Mario's matrix).
    bound_slot: Option<u32>,
    /// `unk10`, `unk18`.
    frame: f32,
    start_frame: f32,
    start: i16,
    status: u32,
    max_frame: i32,
    rate: f32,
    volume_size: f32,
    volume_sweep: f32,
    lifetime: f32,
    spawn_timer: f32,
    emit_timer: f32,
    emit_rate: f32,
    rng: Rng,
    global_rot: Mat3,
    global_pos: Vec3,
    /// `setGlobalScale`: dynamics (local positions) and particle sizes together.
    global_scale: f32,
    /// `JPADraw::mScaleOut`, driven by the emitter's KFA1 draw-scale curve.
    draw_scale: f32,
    /// The emitter's other KFA1 curves this frame, by key kind 0..=9: rate, volume size, sweep,
    /// minimum radius, lifetime, moment, and the four initial velocities (omni, axis, random, direction).
    keyed: [Option<f32>; 10],
    world_space: bool,
    fixed_height: bool,
    time: f32,
    /// `JPADraw::mPrmColor`, `mEnvColor`, `mTexIdx`.
    prm: [u8; 4],
    env: [u8; 4],
    texture: u8,
    /// Newest first, as `mParticleList` (particles are prepended).
    particles: Vec<Particle>,
    children: Vec<Particle>,
}

/// How an effect is drawn.
#[derive(Debug, Clone)]
enum Paint {
    /// Static per-particle colour palettes baked into one atlas. Ordinary alpha
    /// blending preserves painter order when the coloured particles overlap.
    Palette {
        texture: u32,
        colors: Vec<([u8; 4], [u8; 4])>,
        columns: u32,
        tile: [u32; 2],
    },
    /// Static basis textures (per main texture: the first of `count`, then with `over` the
    /// coverage mask), coefficients per particle. `over`: the game blends by source alpha
    /// and its TEV mixes env into the colour, which no single texture-times-colour draw can
    /// show. So a black draw of the coverage (what the particle hides of the scene) goes
    /// first and the basis images add the colour on top: `C a + dst (1 - a)` exactly.
    Basis {
        first: HashMap<u8, u32>,
        count: u32,
        over: bool,
    },
    /// One texture rebaked every frame (stripes with a second texture).
    Baked { texture: u32 },
    /// Projected framebuffer stripes: the disc's indirect map and wave mask, for the host's
    /// existing refraction renderer instead of baking the empty screen texture.
    Refraction { texture: u32 },
}

/// A loaded `.jpa`.
#[derive(Debug, Clone)]
struct Effect {
    name: String,
    jpa: Jpa,
    paint: Paint,
    child_draw: Option<(Jpa, Paint)>,
    /// The file's own colours, kept after a skin tint so another skin can restore them.
    untinted: Option<Jpa>,
    world_space: bool,
    fixed_height: bool,
    speed: f32,
    /// `setRate`: the emitter's rate times this (1: the file's).
    rate_scale: f32,
    /// `setLifeTime`: the particles' life in frames, instead of the file's.
    life: Option<f32>,
}

/// Every effect the host may ask for, and the emitters running.
#[derive(Debug, Clone, Default)]
pub struct Particles {
    effects: Vec<Effect>,
    by_name: HashMap<String, usize>,
    emitters: Vec<Emitter>,
    /// Requests this frame: (slot, effect).
    wanted: Vec<(u32, usize, f32)>,
    seed: Rng,
    /// The sim tick the last game frame ran at.
    last_tick: Option<u64>,
    /// The latest bound matrix per slot, for emitters that keep following after being let go.
    binds: HashMap<u32, Mat4>,
    pub textures: Vec<FxTexture>,
    pub vertices: Vec<FxVertex>,
    pub batches: Vec<FxBatch>,
}

impl Default for Rng {
    fn default() -> Self {
        Rng(0)
    }
}

/// Sample `tex` at `(u, v)` with its wrap, bilinear, as 0..1 floats.
fn sample(tex: &Texture, u: f32, v: f32) -> [f32; 4] {
    let (w, h) = (tex.image.width as i32, tex.image.height as i32);
    if w == 0 || h == 0 {
        return [1.0; 4];
    }
    let wrap = |i: i32, n: i32, mode: Wrap| -> i32 { mode.index(i64::from(i), n as u32) as i32 };
    let x = u * w as f32 - 0.5;
    let y = v * h as f32 - 0.5;
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let px = |xi: i32, yi: i32| -> [f32; 4] {
        let xi = wrap(xi, w, tex.wrap_s);
        let yi = wrap(yi, h, tex.wrap_t);
        let o = ((yi * w + xi) * 4) as usize;
        let p = &tex.image.rgba[o..o + 4];
        [p[0], p[1], p[2], p[3]].map(|c| c as f32 / 255.0)
    };
    let (a, b, c, d) = (
        px(x0 as i32, y0 as i32),
        px(x0 as i32 + 1, y0 as i32),
        px(x0 as i32, y0 as i32 + 1),
        px(x0 as i32 + 1, y0 as i32 + 1),
    );
    let mut out = [0.0; 4];
    for k in 0..4 {
        let top = a[k] + (b[k] - a[k]) * fx;
        let bot = c[k] + (d[k] - c[k]) * fx;
        out[k] = top + (bot - top) * fy;
    }
    out
}

/// The TEV colour inputs as linear forms in the texture colour `t`: `(constant, t's
/// coefficient)`, given the registers (0..1 rgb).
fn tev_forms(base: &BaseShape, prm: Vec3, env: Vec3) -> [(Vec3, Vec3); 4] {
    base.tev().map(|i| match i {
        1 => (Vec3::ONE, Vec3::ZERO),
        2 => (Vec3::ZERO, Vec3::ONE),
        3 => (prm, Vec3::ZERO),
        4 => (env, Vec3::ZERO),
        _ => (Vec3::ZERO, Vec3::ZERO),
    })
}

/// Does the texture appear in both `a`/`b` and `c` (a `t^2` term)?
fn quadratic(base: &BaseShape) -> bool {
    let [a, b, c, _] = base.tev();
    c == 2 && (a == 2 || b == 2)
}

/// TEV stage 0's colour `d + a (1 - c) + b c` as Bernstein coefficients over `t`: linear
/// `[(1 - t), t]` or quadratic `[(1 - t)^2, 2 t (1 - t), t^2]`.
fn tev_coefficients(base: &BaseShape, prm: Vec3, env: Vec3) -> ([Vec3; 3], usize) {
    let [(a0, a1), (b0, b1), (c0, c1), (d0, d1)] = tev_forms(base, prm, env);
    let k0 = d0 + a0 * (Vec3::ONE - c0) + b0 * c0;
    let k1 = d1 + a1 * (Vec3::ONE - c0) - a0 * c1 + b1 * c0 + b0 * c1;
    let k2 = -a1 * c1 + b1 * c1;
    let clamp = |v: Vec3| v.clamp(Vec3::ZERO, Vec3::ONE);
    if quadratic(base) {
        ([clamp(k0), clamp(k0 + k1 * 0.5), clamp(k0 + k1 + k2)], 3)
    } else {
        ([clamp(k0), clamp(k0 + k1), Vec3::ZERO], 2)
    }
}

/// The texture's coverage alone (its alpha; the TEV alpha's texture half), for the black
/// draw under an `over` effect.
fn coverage_image(tex: &Texture) -> FxTexture {
    let img = &tex.image;
    let mut rgba = Vec::with_capacity(img.rgba.len());
    for p in img.rgba.chunks_exact(4) {
        rgba.extend_from_slice(&[255, 255, 255, p[3]]);
    }
    FxTexture {
        width: img.width,
        height: img.height,
        rgba,
        wrap: [tex.wrap_s, tex.wrap_t],
        version: 1,
    }
}

/// The basis images of `tex`: each texel's `t` turned into the basis polynomial, times the
/// texture's alpha (the TEV alpha's texture half). Alpha is 1: the vertex colours carry the
/// rest.
fn basis_images(tex: &Texture, count: u32) -> Vec<FxTexture> {
    let img = &tex.image;
    let wrap = [tex.wrap_s, tex.wrap_t];
    (0..count)
        .map(|k| {
            let mut rgba = Vec::with_capacity(img.rgba.len());
            for p in img.rgba.chunks_exact(4) {
                let ta = p[3] as f32 / 255.0;
                for &c in &p[..3] {
                    let t = c as f32 / 255.0;
                    let b = match (count, k) {
                        (2, 0) => 1.0 - t,
                        (2, _) => t,
                        (_, 0) => (1.0 - t) * (1.0 - t),
                        (_, 1) => 2.0 * t * (1.0 - t),
                        _ => t * t,
                    };
                    rgba.push((b * ta * 255.0).round().clamp(0.0, 255.0) as u8);
                }
                rgba.push(255);
            }
            FxTexture {
                width: img.width,
                height: img.height,
                rgba,
                wrap,
                version: 1,
            }
        })
        .collect()
}

/// How a skin's palette swap picks the entries it turns green.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GreenRule {
    /// Only the strongly red entries (twice the green and blue): the smears and the statue's gate.
    Red,
    /// Every warm entry (red above both green and blue): fire, whose red runs through orange.
    Fire,
}

/// The strongly red colours (twice the green and blue, the rule the statue's gate particles
/// use) become green; whites, blues and everything else are left alone. Shared with the
/// statue's gate particles, which are tinted at load.
pub(crate) fn red_to_green(jpa: &mut Jpa) {
    turn_green(jpa, GreenRule::Red);
}

fn turn_green(jpa: &mut Jpa, rule: GreenRule) {
    let base = &mut jpa.base;
    let swap = |c: &mut [u8; 4]| {
        let warm = match rule {
            GreenRule::Red => c[0] as u16 > c[1] as u16 * 2 && c[0] as u16 > c[2] as u16 * 2,
            GreenRule::Fire => c[0] > c[1] && c[0] > c[2],
        };
        if warm {
            c.swap(0, 1);
        }
    };
    swap(&mut base.prm);
    swap(&mut base.env);
    for c in base.prm_table.iter_mut().chain(base.env_table.iter_mut()) {
        swap(c);
    }
    if let Some(child) = &mut jpa.child {
        swap(&mut child.prm);
        swap(&mut child.env);
    }
}

/// [`Particles::retint_to_colors`]: warm entries (strongly red, or for [`GreenRule::Fire`] anything
/// from red to yellow) become `warm`, blue-led ones `cool` (`[0]` for the primary colour slots,
/// `[1]` for the environment ones), each scaled by the entry's strongest channel; whites and the
/// rest stay as they are.
fn turn_to_colors(jpa: &mut Jpa, rule: GreenRule, warm: [u8; 3], cool: Option<[[u8; 3]; 2]>) {
    let paint = |c: &mut [u8; 4], slot: usize| {
        let is_warm = match rule {
            GreenRule::Red => c[0] as u16 > c[1] as u16 * 2 && c[0] as u16 > c[2] as u16 * 2,
            GreenRule::Fire => c[0] >= c[1] && c[0] > c[2],
        };
        let target = if is_warm {
            Some((warm, c[0]))
        } else if c[2] > c[0] && c[2] > c[1] {
            cool.map(|t| (t[slot], c[2]))
        } else {
            None
        };
        if let Some((t, level)) = target {
            for k in 0..3 {
                c[k] = (t[k] as u16 * level as u16 / 255) as u8;
            }
        }
    };
    let base = &mut jpa.base;
    paint(&mut base.prm, 0);
    paint(&mut base.env, 1);
    for c in base.prm_table.iter_mut() {
        paint(c, 0);
    }
    for c in base.env_table.iter_mut() {
        paint(c, 1);
    }
    if let Some(child) = &mut jpa.child {
        paint(&mut child.prm, 0);
        paint(&mut child.env, 1);
    }
}

/// How a `.jpa` is drawn: a static palette atlas, basis textures, or a stripe texture the draw
/// rebakes each frame.
fn bake_paint(textures: &mut Vec<FxTexture>, jpa: &Jpa) -> Paint {
    let base = &jpa.base;
    let stripe = base.kind == 5 || base.kind == 6;
    let second = jpa.ex_tex.as_ref().is_some_and(|e| e.second_on());
    let projected = stripe
        && base.projection
        && jpa
            .ex_tex
            .as_ref()
            .is_some_and(|e| e.ind_mode == 1 && e.second_on());
    let palette = base.kind == 2
        && base.color_mode == 3
        && base.col_anm_type == 4
        && !base.global_col_anm
        && !base.tex_anm
        && (base.prm_anm || base.env_anm)
        && base.blend_mode == 1
        && base.blend_src == 6
        && base.blend_dst == 7;
    if let Some(source) = jpa
        .textures
        .get(base.texture as usize)
        .filter(|_| palette)
        .map(|t| &t.image)
    {
        let mut colors = Vec::new();
        for i in 0..=base.col_anm_max.max(0) as usize {
            let pair = (
                base.prm_table.get(i).copied().unwrap_or(base.prm),
                base.env_table.get(i).copied().unwrap_or(base.env),
            );
            if !colors.contains(&pair) {
                colors.push(pair);
            }
        }
        let columns = (colors.len() as f32).sqrt().ceil().max(1.) as u32;
        let tile = [source.width + 2, source.height + 2];
        let width = columns * tile[0];
        let height = (colors.len() as u32).div_ceil(columns) * tile[1];
        let mut rgba = vec![0u8; (width * height * 4) as usize];
        for (i, (prm, env)) in colors.iter().enumerate() {
            for y in 0..source.height {
                for x in 0..source.width {
                    let src = ((y * source.width + x) * 4) as usize;
                    let dst = (((i as u32 / columns * tile[1] + 1 + y) * width
                        + i as u32 % columns * tile[0]
                        + 1
                        + x)
                        * 4) as usize;
                    for k in 0..3 {
                        let t = source.rgba[src + k] as f32 / 255.;
                        rgba[dst + k] =
                            (env[k] as f32 * (1. - t) + prm[k] as f32 * t).round() as u8;
                    }
                    rgba[dst + 3] = (source.rgba[src + 3] as u32 * prm[3] as u32 / 255) as u8;
                }
            }
        }
        let texture = textures.len() as u32;
        textures.push(FxTexture {
            width,
            height,
            rgba,
            wrap: [Wrap::Clamp; 2],
            version: 1,
        });
        Paint::Palette {
            texture,
            colors,
            columns,
            tile,
        }
    } else if projected {
        textures.push(refraction_image(jpa));
        Paint::Refraction {
            texture: textures.len() as u32 - 1,
        }
    } else if stripe && second {
        let size = jpa
            .ex_tex
            .as_ref()
            .and_then(|e| jpa.textures.get(e.second_texture as usize))
            .map_or(32, |t| t.image.width.max(t.image.height).clamp(8, 64));
        textures.push(FxTexture {
            width: size,
            height: size,
            rgba: vec![0; (size * size * 4) as usize],
            wrap: [Wrap::Clamp; 2],
            version: 0,
        });
        Paint::Baked {
            texture: textures.len() as u32 - 1,
        }
    } else {
        let over = base.blend_dst != 1 && (base.tev().contains(&4) || base.color_mode == 2);
        let count = if quadratic(base) { 3 } else { 2 };
        let count = if base.blend_dst == 1 || over {
            count
        } else {
            1
        };
        let mut first = HashMap::new();
        let mut wanted: Vec<u8> = base.tex_keys.clone();
        wanted.push(base.texture);
        for t in wanted {
            if first.contains_key(&t) {
                continue;
            }
            let Some(tex) = jpa.textures.get(t as usize) else {
                continue;
            };
            first.insert(t, textures.len() as u32);
            if count == 1 {
                // See-through effects: the texture itself, times prm.
                textures.push(FxTexture {
                    width: tex.image.width,
                    height: tex.image.height,
                    rgba: tex.image.rgba.clone(),
                    wrap: [tex.wrap_s, tex.wrap_t],
                    version: 1,
                });
            } else {
                textures.extend(basis_images(tex, count));
                if over {
                    textures.push(coverage_image(tex));
                }
            }
        }
        Paint::Basis { first, count, over }
    }
}

/// Reuse parent TEV/blending for SSP1's separate geometry, texture and registers.
fn bake_child_draw(textures: &mut Vec<FxTexture>, jpa: &Jpa) -> Option<(Jpa, Paint)> {
    jpa.child.as_ref().map(|child| {
        let mut draw = jpa.clone();
        draw.child = None;
        draw.base.kind = child.kind;
        draw.base.dir_type = child.dir_type;
        draw.base.rot_type = child.rot_type;
        if child.inherit_flags & 1 == 0 {
            draw.base.size_x = child.size_x;
            draw.base.size_y = child.size_y;
        }
        draw.base.tiling = [1.0; 2];
        draw.base.tex_scroll = false;
        draw.base.tex_anm = false;
        draw.base.tex_keys.clear();
        draw.base.texture = child.texture;
        draw.base.global_tex_anm = false;
        draw.base.global_col_anm = false;
        draw.base.prm_anm = true;
        draw.base.env_anm = true;
        draw.base.col_anm_type = 0;
        if child.kind == 2 && draw.base.color_mode == 3 {
            // Inherited birth colors remain fixed on each child. Reuse the
            // palette atlas so alpha-blended children retain painter order.
            draw.base.col_anm_type = 4;
            let mut prm_table = Vec::new();
            let mut env_table = Vec::new();
            for i in 0..=jpa.base.col_anm_max.max(0) as usize {
                let (mut prm, mut env) = (child.prm, child.env);
                if child.inherit_flags & 4 != 0 {
                    let parent_prm = jpa.base.prm_table.get(i).copied().unwrap_or(jpa.base.prm);
                    let parent_env = jpa.base.env_table.get(i).copied().unwrap_or(jpa.base.env);
                    for k in 0..3 {
                        prm[k] = (parent_prm[k] as f32 * child.rgb_inherit) as u8;
                        env[k] = (parent_env[k] as f32 * child.rgb_inherit) as u8;
                    }
                }
                // Alpha inheritance/out is per particle, carried by vertex alpha.
                prm[3] = 255;
                env[3] = 255;
                prm_table.push(prm);
                env_table.push(env);
            }
            draw.base.prm_table = prm_table;
            draw.base.env_table = env_table;
        }
        draw.extra = Some(ExtraShape {
            rotate: child.rotate,
            ..Default::default()
        });
        draw.ex_tex = None;
        let paint = bake_paint(textures, &draw);
        (draw, paint)
    })
}

impl Particles {
    /// Loads the named effects from `dir/particle/<name>.jpa`; missing or unreadable ones are
    /// skipped (and never drawn).
    pub fn load(dir: &Path, names: &[&str]) -> Particles {
        let mut p = Particles {
            seed: Rng(0),
            ..Particles::default()
        };
        for name in names {
            let Ok(data) = std::fs::read(dir.join("particle").join(format!("{name}.jpa"))) else {
                continue;
            };
            let Ok(jpa) = Jpa::parse(&data) else {
                continue;
            };
            p.add(name, jpa);
        }
        p
    }

    /// Adds an effect under `name`.
    pub fn add(&mut self, name: &str, jpa: Jpa) {
        let paint = bake_paint(&mut self.textures, &jpa);
        let child_draw = bake_child_draw(&mut self.textures, &jpa);
        self.by_name.insert(name.to_owned(), self.effects.len());
        self.effects.push(Effect {
            name: name.to_owned(),
            jpa,
            paint,
            child_draw,
            untinted: None,
            world_space: false,
            fixed_height: false,
            speed: 1.0,
            rate_scale: 1.0,
            life: None,
        });
    }

    /// A skin's palette: `on` turns the colours of the named effects green by `rule` (Luigi's
    /// alt-skin keeps Mario's effects but not his red); `off` restores the file's own colours. A
    /// palette-kind effect bakes its colours into a texture, so that is rebuilt.
    pub fn retint_to_green(&mut self, names: &[&str], on: bool, rule: GreenRule) {
        self.retint(names, on, |jpa| turn_green(jpa, rule));
    }

    /// A skin's palette from colours: `on` turns the warm entries of the named effects (by `rule`)
    /// into `warm` and, if given, the blue ones into `cool` (its primary-colour and its
    /// environment-colour entries get one each), each as bright as the entry's
    /// strongest channel (255 is the colour itself); `off` restores the file's own colours.
    pub fn retint_to_colors(
        &mut self,
        names: &[&str],
        on: bool,
        rule: GreenRule,
        warm: [u8; 3],
        cool: Option<[[u8; 3]; 2]>,
    ) {
        self.retint(names, on, |jpa| turn_to_colors(jpa, rule, warm, cool));
    }

    /// The effect's primary colour as it is now (after any skin's tint).
    #[cfg(test)]
    pub(crate) fn prm_of(&self, name: &str) -> Option<[u8; 4]> {
        self.by_name.get(name).map(|&i| self.effects[i].jpa.base.prm)
    }

    fn retint(&mut self, names: &[&str], on: bool, tint: impl Fn(&mut Jpa)) {
        for &name in names {
            let Some(&i) = self.by_name.get(name) else {
                continue;
            };
            if on {
                if self.effects[i].untinted.is_none() {
                    self.effects[i].untinted = Some(self.effects[i].jpa.clone());
                } else {
                    self.effects[i].jpa = self.effects[i].untinted.clone().unwrap();
                }
                tint(&mut self.effects[i].jpa);
            } else if let Some(orig) = self.effects[i].untinted.take() {
                self.effects[i].jpa = orig;
            } else {
                continue;
            }
            if matches!(self.effects[i].paint, Paint::Palette { .. }) {
                let jpa = self.effects[i].jpa.clone();
                self.effects[i].paint = bake_paint(&mut self.textures, &jpa);
            }
            if self.effects[i].child_draw.is_some() {
                let jpa = self.effects[i].jpa.clone();
                self.effects[i].child_draw = bake_child_draw(&mut self.textures, &jpa);
            }
        }
    }

    /// Host presentation override: emitted puffs stay in the world while the emitter
    /// follows Mario to place new particles. This doesn't flatten the puff's own motion.
    pub fn set_world_space(&mut self, name: &str, world_space: bool) {
        if let Some(&i) = self.by_name.get(name) {
            self.effects[i].world_space = world_space;
        }
    }

    /// Host presentation override for ground dust: keep each particle's birth height,
    /// including between renders, while its horizontal motion and look still advance.
    pub fn set_fixed_height(&mut self, name: &str, fixed_height: bool) {
        if let Some(&i) = self.by_name.get(name) {
            self.effects[i].fixed_height = fixed_height;
        }
    }

    /// `JPABaseEmitter::setRate`: the effect's emitters spawn this many times as many
    /// particles as the file says (the Shine's rays are asked for at a fraction of theirs).
    pub fn set_rate_scale(&mut self, name: &str, rate: f32) {
        if let Some(&i) = self.by_name.get(name) {
            self.effects[i].rate_scale = rate.max(0.0);
        }
    }

    /// `JPABaseEmitter::setLifeTime`: its particles live this many frames, whatever the file
    /// says.
    pub fn set_life_time(&mut self, name: &str, frames: f32) {
        if let Some(&i) = self.by_name.get(name) {
            self.effects[i].life = Some(frames.max(1.0));
        }
    }

    /// Host presentation playback rate. Disc timing is unchanged at 1.
    pub fn set_speed(&mut self, name: &str, speed: f32) {
        if let Some(&i) = self.by_name.get(name) {
            self.effects[i].speed = if speed.is_finite() {
                speed.clamp(0.25, 4.0)
            } else {
                1.0
            };
        }
    }

    /// Smooth between the last two particle frames using the host's rendered sim time.
    pub fn render_fraction(&self, tick: f64) -> f32 {
        self.last_tick.map_or(1.0, |t| {
            ((tick - t as f64) / FRAME_TICKS as f64).clamp(0.0, 1.0) as f32
        })
    }

    /// World-anchored textures must not receive the host's current character draw offset.
    pub fn texture_world_space(&self, texture: u32) -> bool {
        self.effects.iter().any(|e| {
            let owns = |paint: &Paint| match paint {
                Paint::Baked { texture: t }
                | Paint::Refraction { texture: t }
                | Paint::Palette { texture: t, .. } => *t == texture,
                Paint::Basis { first, count, over } => first
                    .values()
                    .any(|&t| texture >= t && texture < t + count + *over as u32),
            };
            e.world_space && (owns(&e.paint) || e.child_draw.as_ref().is_some_and(|(_, p)| owns(p)))
        })
    }

    pub fn has(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    /// Asks for effect `name` this game frame, bound to `mtx` (world space; its rotation and
    /// translation are used, `setGlobalRTMatrix`), as `TMarioParticleManager::emitAndBindToMtxPtr`
    /// with a persistent slot: the emitter starts on the first frame asked and is let go
    /// (`becomeInvalidEmitter`, its particles living out) on the first frame not asked.
    /// `slot` tells requests apart.
    pub fn request(&mut self, slot: u32, name: &str, mtx: Mat4) {
        self.request_scaled(slot, name, mtx, 1.0);
    }

    pub fn request_scaled(&mut self, slot: u32, name: &str, mtx: Mat4, scale: f32) {
        if let Some(&e) = self.by_name.get(name) {
            if let Some(w) = self.wanted.iter_mut().find(|w| w.0 == slot) {
                *w = (slot, e, scale);
            } else {
                self.wanted.push((slot, e, scale));
            }
            self.binds.insert(slot, mtx);
        }
    }

    /// `TMarioParticleManager::emit` / `emitWithRotate`: an independent emitter at a fixed
    /// transform, alive for the duration in its JPA. Unlike a held request it needs no
    /// refresh, and another emission of the same effect doesn't replace it.
    pub fn emit(&mut self, name: &str, mtx: Mat4, scale: f32) {
        if let Some(&effect) = self.by_name.get(name) {
            let mut e = self.make(effect, 0);
            e.slot = None;
            e.bound_slot = None;
            e.global_scale = scale;
            e.bind(mtx);
            self.emitters.push(e);
        }
    }

    /// `emitAndBindToPosPtr(.., 0, ..)`: an independent emitter, alive for its JPA's duration like
    /// [`Self::emit`], that follows the matrix bound to `slot` (kept up to date with [`Self::rebind`]).
    /// `slot` must be one no held request uses.
    pub fn emit_following(&mut self, slot: u32, name: &str, mtx: Mat4, scale: f32) {
        if let Some(&effect) = self.by_name.get(name) {
            let mut e = self.make(effect, 0);
            e.slot = None;
            e.bound_slot = Some(slot);
            e.global_scale = scale;
            e.bind(mtx);
            self.emitters.push(e);
            self.binds.insert(slot, mtx);
        }
    }

    /// Updates the bound matrices without asking for anything (between game frames, and for
    /// emitters already let go that still follow).
    pub fn rebind(&mut self, slot: u32, mtx: Mat4) {
        if let Some(b) = self.binds.get_mut(&slot) {
            *b = mtx;
        }
    }

    /// Any particles alive.
    pub fn active(&self) -> bool {
        self.emitters.iter().any(|e| {
            (!e.particles.is_empty() || !e.children.is_empty()) || e.status & ENABLE_DELETE == 0
        })
    }

    /// Runs the game frames due at sim tick `tick` (one per [`FRAME_TICKS`]; at most four at
    /// once). Requests made since the last call count for the first.
    pub fn advance(&mut self, tick: u64) {
        let frames = match self.last_tick {
            Some(t) if tick >= t => ((tick - t) / FRAME_TICKS).min(4),
            _ => 1,
        };
        if frames == 0 {
            return;
        }
        self.last_tick = Some(match self.last_tick {
            Some(t) if tick >= t => t + frames * FRAME_TICKS,
            _ => tick,
        });
        // Frames caught up together share the requests.
        for _ in 0..frames {
            self.frame();
        }
        self.wanted.clear();
    }

    /// One game frame: `TMarioParticleManager::perform` then `JPAEmitterManager::calc`.
    fn frame(&mut self) {
        // Start emitters newly asked for; let go of those no longer asked for.
        for (slot, effect, scale) in self.wanted.clone() {
            let live = self.emitters.iter_mut().find(|e| e.slot == Some(slot));
            match live {
                Some(e)
                    if e.effect == effect
                        && !(e.status & ENABLE_DELETE != 0 && e.particles.is_empty()) =>
                {
                    e.global_scale = scale;
                }
                Some(e) => {
                    e.max_frame = -1;
                    e.status |= STOP_EMIT;
                    e.slot = None;
                    let mut em = self.make(effect, slot);
                    em.global_scale = scale;
                    self.emitters.push(em);
                }
                None => {
                    let mut em = self.make(effect, slot);
                    em.global_scale = scale;
                    self.emitters.push(em);
                }
            }
        }
        let asked: Vec<u32> = self.wanted.iter().map(|w| w.0).collect();
        for e in &mut self.emitters {
            if let Some(s) = e.slot
                && !asked.contains(&s)
            {
                e.max_frame = -1;
                e.status |= STOP_EMIT;
                e.slot = None;
            }
        }
        // Bind, then calc; emitters done and empty go.
        let binds = &self.binds;
        let effects = &self.effects;
        self.emitters.retain_mut(|e| {
            // Let-go emitters keep their last slot's matrix (`userWork` still points at it).
            let bind = e.bound_slot.and_then(|s| binds.get(&s)).copied();
            if let Some(m) = bind {
                e.bind(m);
            }
            let effect = &effects[e.effect];
            for p in e.particles.iter_mut().chain(&mut e.children) {
                p.previous_local = p.local;
                p.previous_global = p.global;
                p.previous_look = p.look;
            }
            e.time += effect.speed;
            while e.time >= 1.0 {
                e.time -= 1.0;
                if !e.check_start_frame() {
                    continue;
                }
                if e.check_max_frame() && e.particles.is_empty() && e.children.is_empty() {
                    return false;
                }
                e.calc(&effect.jpa);
            }
            true
        });
    }

    fn make(&mut self, effect: usize, slot: u32) -> Emitter {
        let jpa = &self.effects[effect].jpa;
        let em = &jpa.emitter;
        let base = &jpa.base;
        let white = [255u8; 4];
        let prm = if base.prm_on { base.prm } else { white };
        let env = if base.env_on { base.env } else { white };
        Emitter {
            effect,
            slot: Some(slot),
            bound_slot: Some(slot),
            frame: 0.0,
            start_frame: 0.0,
            start: em.start_frame,
            status: FIRST_EMIT | EMIT_NEXT_FRAME,
            max_frame: em.max_frame,
            rate: em.rate * self.effects[effect].rate_scale,
            volume_size: em.volume_size as f32,
            volume_sweep: em.volume_sweep,
            lifetime: self.effects[effect].life.unwrap_or(em.lifetime as f32),
            spawn_timer: 0.0,
            emit_timer: 0.0,
            emit_rate: if em.emit_interval == 0 {
                1.0
            } else {
                1.0 / (em.emit_interval as f32 + 1.0) + 1e-7
            },
            rng: Rng(self.seed.get()),
            global_rot: Mat3::IDENTITY,
            global_pos: Vec3::ZERO,
            global_scale: 1.0,
            draw_scale: 1.0,
            keyed: [None; 10],
            world_space: self.effects[effect].world_space,
            fixed_height: self.effects[effect].fixed_height,
            time: 0.0,
            prm,
            env,
            texture: base.texture,
            particles: Vec::new(),
            children: Vec::new(),
        }
    }

    /// Builds this frame's quads. `right`, `up`: the camera's axes (sim world) for billboards.
    pub fn build(&mut self, right: Vec3, up: Vec3) {
        self.build_filtered(right, up, |_| true);
    }

    /// Drawing switches don't stop simulation or change emitter lifetimes.
    pub fn build_filtered(&mut self, right: Vec3, up: Vec3, shown: impl Fn(&str) -> bool) {
        self.build_interpolated(right, up, 1.0, shown);
    }

    pub fn build_interpolated(
        &mut self,
        right: Vec3,
        up: Vec3,
        fraction: f32,
        shown: impl Fn(&str) -> bool,
    ) {
        let fraction = fraction.clamp(0.0, 1.0);
        self.vertices.clear();
        self.batches.clear();
        let right = right.normalize_or(Vec3::X);
        let up = up.normalize_or(Vec3::Y);
        let mut baked: Vec<(u32, Vec<u8>)> = Vec::new();
        for e in &self.emitters {
            let fx = &self.effects[e.effect];
            if !shown(&fx.name) {
                continue;
            }
            let children_first = fx.jpa.base.flags & 2 != 0;
            for child_pass in [children_first, !children_first] {
                let (jpa, paint, particles) = if child_pass {
                    let Some((jpa, paint)) = &fx.child_draw else {
                        continue;
                    };
                    (jpa, paint, &e.children)
                } else {
                    if fx.jpa.child.as_ref().is_some_and(|c| c.flags & 1 == 0) {
                        continue;
                    }
                    (&fx.jpa, &fx.paint, &e.particles)
                };
                let base = &jpa.base;
                // Child stripes need their own linked-list ribbon builder. Do not
                // accidentally draw the parent's list as the child's geometry.
                if child_pass && matches!(base.kind, 5 | 6) {
                    continue;
                }
                if base.kind == 5 || base.kind == 6 {
                    let quads = stripe_quads(e, jpa);
                    if quads.is_empty() {
                        continue;
                    }
                    match paint {
                        Paint::Refraction { texture } => {
                            let first = self.vertices.len() as u32;
                            for q in &quads {
                                for (p, uv) in q {
                                    self.vertices.push(FxVertex {
                                        pos: *p,
                                        u: uv[0],
                                        v: uv[1],
                                        color: [1.0; 4],
                                    });
                                }
                            }
                            self.batches.push(FxBatch {
                                texture: *texture,
                                first_vertex: first,
                                vertex_count: self.vertices.len() as u32 - first,
                                blend: blend::REFRACTION,
                            });
                        }
                        Paint::Baked { texture } => {
                            let t = &self.textures[*texture as usize];
                            baked.push((*texture, bake_stripe(e, jpa, t.width, t.height)));
                            let first = self.vertices.len() as u32;
                            for q in &quads {
                                for (p, uv) in q {
                                    self.vertices.push(FxVertex {
                                        pos: *p,
                                        u: uv[0],
                                        v: uv[1],
                                        color: [1.0; 4],
                                    });
                                }
                            }
                            self.batches.push(FxBatch {
                                texture: *texture,
                                first_vertex: first,
                                vertex_count: self.vertices.len() as u32 - first,
                                blend: blend::ADDITIVE,
                            });
                        }
                        Paint::Palette { .. } => unreachable!("palette is only for billboards"),
                        Paint::Basis { first, count, over } => {
                            let Some(&t0) = first.get(&e.texture) else {
                                continue;
                            };
                            let (prm, env) = registers(e, None, jpa);
                            let colors = paint_colors(base, prm, env, *count);
                            let m = stripe_tex_mtx(base, e.frame as i32);
                            let additive = if *count == 1 {
                                blend::ALPHA
                            } else {
                                blend::ADDITIVE
                            };
                            // The coverage goes first: the colour adds to what it leaves.
                            let coverage = (t0 + *count, [0.0, 0.0, 0.0, prm[3]], blend::ALPHA);
                            let layers = std::iter::once(coverage).filter(|_| *over).chain(
                                colors
                                    .iter()
                                    .enumerate()
                                    .take(*count as usize)
                                    .filter(|(_, c)| {
                                        *count == 1 || c[0] + c[1] + c[2] > 1.0 / 512.0
                                    })
                                    .map(|(k, c)| (t0 + k as u32, *c, additive)),
                            );
                            for (texture, color, blend) in layers {
                                let first = self.vertices.len() as u32;
                                for q in &quads {
                                    for (p, uv) in q {
                                        let uv = m.map_or(*uv, |m| tex_mtx_apply(m, *uv));
                                        self.vertices.push(FxVertex {
                                            pos: *p,
                                            u: uv[0],
                                            v: uv[1],
                                            color,
                                        });
                                    }
                                }
                                self.batches.push(FxBatch {
                                    texture,
                                    first_vertex: first,
                                    vertex_count: self.vertices.len() as u32 - first,
                                    blend,
                                });
                            }
                        }
                    }
                    continue;
                }
                if let Paint::Palette {
                    texture,
                    colors,
                    columns,
                    tile,
                } = paint
                {
                    let atlas = &self.textures[*texture as usize];
                    let first_vertex = self.vertices.len() as u32;
                    let order: Box<dyn Iterator<Item = &Particle>> = if base.list_order() {
                        Box::new(particles.iter())
                    } else {
                        Box::new(particles.iter().rev())
                    };
                    for particle in order {
                        let p = sample_particle(particle, fraction);
                        let Some(index) = colors.iter().position(|c| {
                            c.0[..3] == p.look.prm[..3] && c.1[..3] == p.look.env[..3]
                        }) else {
                            continue;
                        };
                        let pos = if p.follow {
                            e.bound_slot
                                .and_then(|slot| self.binds.get(&slot))
                                .map(|m| {
                                    let mut bound = e.clone_bind();
                                    bound.bind(*m);
                                    p.local * e.global_scale + bound.global_params(jpa).pos
                                })
                                .unwrap_or(p.global)
                        } else {
                            p.global
                        };
                        let Some(corners) = particle_quad(e, &p, pos, jpa, right, up, 0) else {
                            continue;
                        };
                        for (position, uv) in corners {
                            self.vertices.push(FxVertex {
                                pos: position,
                                u: (index as u32 % columns * tile[0] + 1) as f32
                                    / atlas.width as f32
                                    + uv[0] * (tile[0] - 2) as f32 / atlas.width as f32,
                                v: (index as u32 / columns * tile[1] + 1) as f32
                                    / atlas.height as f32
                                    + uv[1] * (tile[1] - 2) as f32 / atlas.height as f32,
                                color: [
                                    1.,
                                    1.,
                                    1.,
                                    p.look.alpha * p.look.prm[3] as f32
                                        / colors[index].0[3].max(1) as f32,
                                ],
                            });
                        }
                    }
                    self.batches.push(FxBatch {
                        texture: *texture,
                        first_vertex,
                        vertex_count: self.vertices.len() as u32 - first_vertex,
                        blend: blend::ALPHA,
                    });
                    continue;
                }
                let Paint::Basis { first, count, over } = paint else {
                    continue;
                };
                // Particles that follow the emitter sit where it is now (the host draws between
                // game frames), not where it was at the last one.
                let now = e.bound_slot.and_then(|s| self.binds.get(&s)).map(|m| {
                    let mut b = e.clone_bind();
                    b.bind(*m);
                    b.global_params(jpa).pos
                });
                let order: Box<dyn Iterator<Item = &Particle>> = if base.list_order() {
                    Box::new(particles.iter())
                } else {
                    Box::new(particles.iter().rev())
                };
                // One batch per (texture, basis image); particles append to theirs.
                // The coverage draws (`over`) are flagged and go first.
                let mut runs: Vec<(u32, bool, Vec<FxVertex>)> = Vec::new();
                for p in order {
                    let sampled = sample_particle(p, fraction);
                    let p = &sampled;
                    let tex = if base.global_tex_anm {
                        e.texture
                    } else {
                        p.look.texture
                    };
                    let Some(&t0) = first.get(&tex) else {
                        continue;
                    };
                    let pos = match now {
                        Some(at) if p.follow => p.local * e.global_scale + at,
                        _ => p.global,
                    };
                    let (prm, env) = registers(e, Some(p), jpa);
                    let colors = paint_colors(base, prm, env, *count);
                    // JPADrawExecDirectionalCross / RotDirectionalCross draw two perpendicular
                    // quads. MS_M_WATSLIDE_C uses this for the wet slide's side spray.
                    let passes = if base.kind == 4 { 2 } else { 1 };
                    for pass in 0..passes {
                        let Some(corners) = particle_quad(e, p, pos, jpa, right, up, pass) else {
                            continue;
                        };
                        let coverage = (t0 + *count, true, [0.0, 0.0, 0.0, prm[3]]);
                        let layers = std::iter::once(coverage).filter(|_| *over).chain(
                            colors
                                .iter()
                                .enumerate()
                                .take(*count as usize)
                                .filter(|(_, c)| *count == 1 || c[0] + c[1] + c[2] > 1.0 / 512.0)
                                .map(|(k, c)| (t0 + k as u32, false, *c)),
                        );
                        for (texture, cover, c) in layers {
                            let run = match runs.iter_mut().find(|r| r.0 == texture) {
                                Some(r) => r,
                                None => {
                                    runs.push((texture, cover, Vec::new()));
                                    runs.last_mut().unwrap()
                                }
                            };
                            for (pos, uv) in &corners {
                                run.2.push(FxVertex {
                                    pos: *pos,
                                    u: uv[0],
                                    v: uv[1],
                                    color: c,
                                });
                            }
                        }
                    }
                }
                // The colour adds to what the coverage leaves of the scene, so it draws first.
                runs.sort_by_key(|r| !r.1);
                for (texture, cover, verts) in runs {
                    let first = self.vertices.len() as u32;
                    self.vertices.extend(verts);
                    self.batches.push(FxBatch {
                        texture,
                        first_vertex: first,
                        vertex_count: self.vertices.len() as u32 - first,
                        blend: if *count == 1 || cover {
                            blend::ALPHA
                        } else {
                            blend::ADDITIVE
                        },
                    });
                }
            }
        }
        for (t, rgba) in baked {
            let tex = &mut self.textures[t as usize];
            tex.rgba = rgba;
            tex.version = tex.version.wrapping_add(1);
        }
    }

    /// Effects loaded, by name (for logs).
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.effects.iter().map(|e| e.name.as_str())
    }
}

/// The colour registers a particle (or a stripe, with `None`) draws with, 0..1: prm (alpha
/// included) and env. The `JPADrawExecRegister*` visitors `setDrawExecVisitorsBeforeCB` picks:
/// per-particle tables unless the shape animates colour emitter-wide, prm alpha times the
/// particle's alpha where `ESP1` drives it.
fn registers(e: &Emitter, p: Option<&Particle>, jpa: &Jpa) -> ([f32; 4], [f32; 3]) {
    let base = &jpa.base;
    let stripe = base.kind == 5 || base.kind == 6;
    let alpha_on = jpa.extra.as_ref().is_some_and(|x| x.alpha_on()) && !stripe;
    let (prm, env, alpha) = match p {
        Some(p) if !base.global_col_anm => (
            if base.prm_anm { p.look.prm } else { e.prm },
            if base.env_anm { p.look.env } else { e.env },
            if base.prm_anm || alpha_on {
                p.look.alpha
            } else {
                1.0
            },
        ),
        Some(p) => (e.prm, e.env, if alpha_on { p.look.alpha } else { 1.0 }),
        None => (e.prm, e.env, 1.0),
    };
    let u = |c: u8| c as f32 / 255.0;
    // `prm.a = mAlpha * prm.a`, stored back into a u8.
    let a = (alpha * prm[3] as f32).clamp(0.0, 255.0) as u8;
    (
        [u(prm[0]), u(prm[1]), u(prm[2]), u(a)],
        [u(env[0]), u(env[1]), u(env[2])],
    )
}

/// The vertex colour for each basis image: its coefficient times prm alpha (additive), or for
/// see-through effects (`count` 1) prm itself.
fn paint_colors(base: &BaseShape, prm: [f32; 4], env: [f32; 3], count: u32) -> [[f32; 4]; 3] {
    if count == 1 {
        return [prm, [0.0; 4], [0.0; 4]];
    }
    let (k, _) = tev_coefficients(base, Vec3::new(prm[0], prm[1], prm[2]), Vec3::from(env));
    k.map(|c| {
        let c = c * prm[3];
        [c.x, c.y, c.z, 1.0]
    })
}

/// `JPADrawExecSetTexMtx` (the emitter's or a particle's): `[[a, b, tx], [c, d, ty]]`, or none
/// when the shape doesn't scroll.
fn tex_mtx(base: &BaseShape, tick: i32) -> [[f32; 3]; 2] {
    let t = tick as f32;
    let (tx, ty) = (0.5 * base.tiling[0], 0.5 * base.tiling[1]);
    let trans_x = t * base.tex_scroll_trans[0] + base.tex_static_trans[0];
    let trans_y = t * base.tex_scroll_trans[1] + base.tex_static_trans[1];
    let sx = t * base.tex_scroll_scale[0] + base.tex_static_scale[0];
    let sy = t * base.tex_scroll_scale[1] + base.tex_static_scale[1];
    let angle = (t * base.tex_scroll_rotate * 32768.0) as i32 as i16;
    let (s, c) = (sin(angle), cos(angle));
    [
        [
            sx * c,
            -sx * s,
            tx + sx * (s * (ty + trans_y) - c * (tx + trans_x)),
        ],
        [
            sy * s,
            sy * c,
            ty - sy * (s * (tx + trans_x) + c * (ty + trans_y)),
        ],
    ]
}

fn tex_mtx_apply(m: [[f32; 3]; 2], uv: [f32; 2]) -> [f32; 2] {
    [
        m[0][0] * uv[0] + m[0][1] * uv[1] + m[0][2],
        m[1][0] * uv[0] + m[1][1] * uv[1] + m[1][2],
    ]
}

/// A stripe's texture matrix (`setDrawExecVisitorsBeforeCB`: `mDrawExecSetTexMtx` emitter-wide
/// when it scrolls, without projection).
fn stripe_tex_mtx(base: &BaseShape, frame: i32) -> Option<[[f32; 3]; 2]> {
    (base.tex_scroll && !base.projection).then(|| tex_mtx(base, frame))
}

/// A stripe's whole TEV baked over its `(u, v)`: the main texture through the scrolling matrix
/// mixed with the registers, times the second texture at the raw coordinates, premultiplied
/// for additive blending.
fn bake_stripe(e: &Emitter, jpa: &Jpa, w: u32, h: u32) -> Vec<u8> {
    let base = &jpa.base;
    let main = jpa.textures.get(e.texture as usize);
    let second = jpa
        .ex_tex
        .as_ref()
        .filter(|x| x.second_on())
        .and_then(|x| jpa.textures.get(x.second_texture as usize));
    let m = stripe_tex_mtx(base, e.frame as i32);
    let (prm, env) = registers(e, None, jpa);
    let p = Vec3::new(prm[0], prm[1], prm[2]);
    let env = Vec3::from(env);
    let [(a0, a1), (b0, b1), (c0, c1), (d0, d1)] = tev_forms(base, p, env);
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let uv = [(x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32];
            let muv = m.map_or(uv, |m| tex_mtx_apply(m, uv));
            let t = main.map_or([1.0; 4], |t| sample(t, muv[0], muv[1]));
            let tc = Vec3::new(t[0], t[1], t[2]);
            let (a, b, c, d) = (a0 + a1 * tc, b0 + b1 * tc, c0 + c1 * tc, d0 + d1 * tc);
            let mut color = (d + a * (Vec3::ONE - c) + b * c).clamp(Vec3::ZERO, Vec3::ONE);
            let mut alpha = t[3] * prm[3];
            if let Some(s) = second {
                let s = sample(s, uv[0], uv[1]);
                color *= Vec3::new(s[0], s[1], s[2]);
                alpha *= s[3];
            }
            let o = color * alpha;
            out.extend([o.x, o.y, o.z].map(|c| (c * 255.0).round().clamp(0.0, 255.0) as u8));
            out.push(255);
        }
    }
    out
}

/// `TMarioParticleManager::emitTry` swaps the main image for the current screen on group-3
/// effects. `JPADrawSetupTev::setupTev` offsets that screen sample with ETX1's matrix and
/// indirect texture. Keep those signed offsets under the disc's second-texture wave mask
/// and hand them to the host's scene-refraction pass as Unity's packed normal channels.
fn refraction_image(jpa: &Jpa) -> FxTexture {
    let ext = jpa.ex_tex.as_ref().unwrap();
    let indirect = jpa.textures.get(ext.ind_texture as usize);
    let mask = jpa.textures.get(ext.second_texture as usize);
    let size = mask.map_or(64, |t| t.image.width.max(t.image.height).clamp(8, 128));
    let exponent = 2.0f32.powi(ext.ind_exp as i32);
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (u, v) = (
                (x as f32 + 0.5) / size as f32,
                (y as f32 + 0.5) / size as f32,
            );
            let tex = indirect.map_or([0.5; 4], |t| sample(t, u, v));
            // GX indirect STU components are alpha, blue, green, biased by 128.
            let stu = [tex[3], tex[2], tex[1]].map(|c| (c * 255.0 - 128.0) / 128.0);
            let coverage = mask.map_or(0.0, |t| {
                let c = sample(t, u, v);
                c[0] * c[3]
            });
            let offset = ext.ind_matrix.map(|row| {
                row.iter().zip(stu).map(|(a, b)| a * b).sum::<f32>()
                    * exponent
                    * blend::REFRACTION_NORMAL_GAIN
            });
            let pack = |v: f32| ((v * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8;
            // Keep mask coverage separate from normals. RoR2 discards near-flat normals
            // before compositing: pre-multiplying by the narrow wave mask erased all crests.
            rgba.extend_from_slice(&[
                255,
                pack(offset[1]),
                (coverage * 255.0).round() as u8,
                pack(offset[0]),
            ]);
        }
    }
    FxTexture {
        width: size,
        height: size,
        rgba,
        wrap: [Wrap::Clamp; 2],
        version: 1,
    }
}

/// The particle's direction for directional shapes and stripes (`dirType*`).
fn dir_of(e: &Emitter, p: &Particle, jpa: &Jpa, prev: Option<&Particle>) -> Vec3 {
    match jpa.base.dir_type {
        1 => p.local,
        2 => -p.local,
        3 => Vec3::from(jpa.emitter.direction),
        4 => prev.map_or(e.global_pos, |q| q.global) - p.global,
        _ => p.vel,
    }
}

/// `rotType*`.
fn rot_type(kind: u8, s: f32, c: f32) -> Mat3 {
    match kind {
        1 => rows([[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]]),
        2 => rows([[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]]),
        3 => {
            let a = (1.0 - c) * 0.333333;
            let (cc, b) = (a + 0.57735 * s, a - 0.57735 * s);
            let a = a + c;
            rows([[a, b, cc], [cc, a, b], [b, cc, a]])
        }
        4 => rows([
            [c, -s * 0.207912, -s * 0.978148],
            [0.0, 0.978148, -0.207912],
            [s, c * 0.207912, c * 0.978148],
        ]),
        _ => rows([[c, 0.0, -s], [0.0, 1.0, 0.0], [s, 0.0, c]]),
    }
}

/// `setParticleClipBoard`: half the quad's size (`unk4`) and the pivot shift (`unkC`).
fn clip_board(jpa: &Jpa, scale: f32) -> ([f32; 2], [f32; 2]) {
    let base = &jpa.base;
    let mut size = [25.0 * base.size_x * scale, 25.0 * base.size_y * scale];
    match base.kind {
        0 => size = size.map(|s| s * 1.02),
        1 => size = [size[0] * 1.02, size[1] * 0.4],
        _ => {}
    }
    let pivot = match &jpa.extra {
        Some(x) if x.scale_on() => [
            size[0] * (x.pivot_x as f32 - 1.0),
            size[1] * (x.pivot_y as f32 - 1.0),
        ],
        _ => [0.0, 0.0],
    };
    (size, pivot)
}

/// One particle's quad (`JPADrawExecBillBoard`, `RotBillBoard`, `Directional`,
/// `RotDirectional`, ...): four corners and their texture coordinates.
fn particle_quad(
    e: &Emitter,
    p: &Particle,
    pos: Vec3,
    jpa: &Jpa,
    right: Vec3,
    up: Vec3,
    pass: usize,
) -> Option<[(Vec3, [f32; 2]); 4]> {
    let base = &jpa.base;
    let (size, pivot) = clip_board(jpa, e.global_scale);
    let l = &p.look;
    let x0 = -l.scale_x * (size[0] + pivot[0]);
    let y0 = l.scale_y * (size[1] + pivot[1]);
    let x1 = l.scale_x * (size[0] - pivot[0]);
    let y1 = -l.scale_y * (size[1] - pivot[1]);
    let tiling = base.tiling;
    let mut uvs = [
        [0.0, 0.0],
        [tiling[0], 0.0],
        [tiling[0], tiling[1]],
        [0.0, tiling[1]],
    ];
    if base.tex_scroll && !base.projection {
        let m = tex_mtx(base, p.age as i32);
        uvs = uvs.map(|uv| tex_mtx_apply(m, uv));
    }
    let rotate = jpa.extra.as_ref().is_some_and(|x| x.rotate);
    let (s, c) = (sin(l.angle), cos(l.angle));
    let offs: [Vec3; 4] = match base.kind {
        3 | 4 | 9 => {
            // Directional: x along the remembered up, y along the direction. The game keeps
            // `unk0` from frame to frame; recomputed here from the initial one each time.
            let d = dir_of(e, p, jpa, None);
            if d.length_squared() < 1e-12 {
                return None;
            }
            let d = d.normalize();
            let side = l.up.cross(d);
            if side.length_squared() < 1e-12 {
                return None;
            }
            let side = side.normalize();
            let nup = d.cross(side).normalize();
            let frame = Mat3::from_cols(nup, d, side);
            let m = if rotate {
                frame * rot_type(base.rot_type, s, c)
            } else {
                frame
            };
            let corners = if pass == 0 {
                [
                    Vec3::new(x0, y0, 0.0),
                    Vec3::new(x1, y0, 0.0),
                    Vec3::new(x1, y1, 0.0),
                    Vec3::new(x0, y1, 0.0),
                ]
            } else {
                // The second plane shares the first plane's pivot and direction. Rotate
                // both with the same particle frame (JPADrawVisitor.cpp).
                let middle = (x0 + x1) * 0.5;
                let half_width = (x1 - x0) * 0.5;
                [
                    Vec3::new(middle, y0, half_width),
                    Vec3::new(middle, y0, -half_width),
                    Vec3::new(middle, y1, -half_width),
                    Vec3::new(middle, y1, half_width),
                ]
            };
            corners.map(|v| m * v)
        }
        _ => {
            // Billboards: in the camera's plane.
            let corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
            corners.map(|(x, y)| {
                let (x, y) = if rotate {
                    (x * c - y * s, y * c + x * s)
                } else {
                    (x, y)
                };
                right * x + up * y
            })
        }
    };
    Some([
        (pos + offs[0], uvs[0]),
        (pos + offs[1], uvs[1]),
        (pos + offs[2], uvs[2]),
        (pos + offs[3], uvs[3]),
    ])
}

/// A stripe's quads (`JPADrawExecStripe` / `StripeCross`): consecutive particles joined, newest
/// first, `v` running 0 to 1 along the list.
fn stripe_quads(e: &Emitter, jpa: &Jpa) -> Vec<[(Vec3, [f32; 2]); 4]> {
    let n = e.particles.len();
    if n < 2 {
        return Vec::new();
    }
    let base = &jpa.base;
    let (size, pivot) = clip_board(jpa, e.global_scale);
    let order: Vec<&Particle> = if base.list_order() {
        e.particles.iter().collect()
    } else {
        e.particles.iter().rev().collect()
    };
    let step = 1.0 / (n - 1) as f32;
    let mut out = Vec::new();
    let passes = if base.kind == 6 { 2 } else { 1 };
    for pass in 0..passes {
        let mut prev: Option<(Vec3, Vec3, f32)> = None;
        let mut up_carry: Vec<Vec3> = order.iter().map(|p| p.look.up).collect();
        for (i, p) in order.iter().enumerate() {
            let l = &p.look;
            let (s, c) = (sin(l.angle), cos(l.angle));
            let (v1, v2) = if pass == 0 {
                let x = -l.scale_x * (size[0] + pivot[0]);
                let y = l.scale_x * (size[0] - pivot[0]);
                (Vec3::new(x * c, 0.0, x * s), Vec3::new(y * c, 0.0, y * s))
            } else {
                let s = -s;
                let x = l.scale_y * (size[1] + pivot[1]);
                let y = -l.scale_y * (size[1] - pivot[1]);
                (Vec3::new(x * s, 0.0, x * c), Vec3::new(y * s, 0.0, y * c))
            };
            let prev_p = if i > 0 { Some(order[i - 1]) } else { None };
            let d = if pass == 0 {
                dir_of(e, p, jpa, prev_p)
            } else {
                p.vel
            };
            let d = if d.length_squared() < 1e-12 {
                Vec3::Y
            } else {
                d.normalize()
            };
            let side = up_carry[i].cross(d);
            let side = if side.length_squared() < 1e-12 {
                if pass == 0 { Vec3::X } else { Vec3::Y }
            } else {
                side.normalize()
            };
            let nup = d.cross(side).normalize_or(Vec3::Z);
            up_carry[i] = nup;
            let m = Mat3::from_cols(side, d, nup);
            let (a, b) = (p.global + m * v1, p.global + m * v2);
            let v = if base.list_order() {
                i as f32 * step
            } else {
                1.0 - i as f32 * step
            };
            if let Some((pa, pb, pv)) = prev {
                out.push([
                    (pa, [0.0, pv]),
                    (pb, [1.0, pv]),
                    (b, [1.0, v]),
                    (a, [0.0, v]),
                ]);
            }
            prev = Some((a, b, v));
        }
    }
    out
}

impl Emitter {
    /// Just the bound transform, for working out where the emitter is now.
    fn clone_bind(&self) -> Emitter {
        Emitter {
            particles: Vec::new(),
            children: Vec::new(),
            ..self.clone()
        }
    }

    /// `setGlobalRTMatrix`: the rotation (columns normalised) and translation of `m`.
    fn bind(&mut self, m: Mat4) {
        let cols = [
            m.x_axis.truncate(),
            m.y_axis.truncate(),
            m.z_axis.truncate(),
        ];
        let n = cols.map(|c| {
            let l = c.length();
            if l != 0.0 { c / l } else { c }
        });
        self.global_rot = Mat3::from_cols(n[0], n[1], n[2]);
        self.global_pos = m.w_axis.truncate();
    }

    /// `checkStartFrame`.
    fn check_start_frame(&mut self) -> bool {
        if self.start_frame as i16 >= self.start {
            return true;
        }
        self.start_frame += 1.0;
        false
    }

    /// `checkMaxFrame`.
    fn check_max_frame(&mut self) -> bool {
        if self.max_frame == 0 {
            return false;
        }
        if self.max_frame < 0 {
            self.status |= ENABLE_DELETE;
            return true;
        }
        if self.frame as i32 >= self.max_frame {
            self.status |= ENABLE_DELETE;
            return true;
        }
        false
    }

    /// `calcEmitterGlobalParams`.
    fn global_params(&self, jpa: &Jpa) -> Info {
        let em = &jpa.emitter;
        let local_rot = rot_xyz(
            (em.rot[0] as i32 * 182) as i16,
            (em.rot[1] as i32 * 182) as i16,
            (em.rot[2] as i32 * 182) as i16,
        );
        let local_scale = Vec3::from(em.scale);
        let scale = local_scale * self.global_scale;
        let rot = self.global_rot * local_rot;
        let rot_scale = rot * Mat3::from_diagonal(local_scale);
        let pos = self.global_pos + self.global_rot * (Vec3::from(em.trans) * self.global_scale);
        let dir = Vec3::from(em.direction).normalize_or(Vec3::Z);
        let dir_rot = if dir == Vec3::Z {
            Mat3::IDENTITY
        } else {
            vec_to_rota(dir, Vec3::Z)
        };
        Info {
            global_rot: self.global_rot,
            rot,
            rot_scale,
            scale,
            pos,
            axis_y: rot.y_axis,
            dir,
            dir_rot,
            ..Info::default()
        }
    }

    /// `JPABaseEmitter::calc`.
    fn calc(&mut self, jpa: &Jpa) {
        // `JPABaseEmitter::calcKeyFrameAnime` runs before drawing/emission. The wet-slide
        // ribbon's type-10 curve pulses its birth width; old particles retain that width.
        for key in &jpa.keys {
            if jpa.emitter.key_anm_mask & (1u32 << key.kind) == 0 {
                continue;
            }
            let Some(v) = key.value(self.frame) else {
                continue;
            };
            match key.kind {
                10 => self.draw_scale = v,
                0 => self.rate = v,
                1 => self.volume_size = v.max(0.0).floor(),
                2 => self.volume_sweep = v,
                4 => self.lifetime = v,
                k @ (3 | 5..=9) => self.keyed[k as usize] = Some(v),
                _ => {}
            }
        }
        let mut info = self.global_params(jpa);
        self.calc_draw_emitter(jpa);
        // `calcFieldParams`: each field's push this frame.
        let fields: Vec<(Vec3, &libsms_assets::jpa::Field)> = jpa
            .fields
            .iter()
            .map(|f| {
                let dir = Vec3::from(f.dir);
                let dir = if f.status & 0x2 != 0 {
                    dir
                } else {
                    info.global_rot * dir
                };
                (dir * f.magnitude, f)
            })
            .collect();
        if self.status & ENABLE_DELETE == 0 {
            self.create_particles(jpa, &mut info);
        }
        // `doParticle`.
        let mut i = 0;
        while i < self.particles.len() {
            let p = &mut self.particles[i];
            let newborn = p.just_born;
            if p.just_born {
                p.just_born = false;
            } else {
                p.age += 1.0;
            }
            if p.age >= p.life {
                p.progress = 1.0;
                p.dead = true;
            } else {
                p.progress = p.age / p.life;
            }
            if p.dead {
                self.particles.remove(i);
                continue;
            }
            move_particle(p, &info, &fields, self.fixed_height);
            // `JPADraw::calcParticle`.
            p.look.angle = p.look.angle.wrapping_add(p.look.spin);
            calc_draw_particle(self.prm, self.env, p, jpa);
            // The initial draw parameters must already use ESP1's birth scale/alpha;
            // interpolating from uninitialised defaults makes new smoke flash full size.
            if newborn {
                p.previous_look = p.look;
            }
            // JPAEmitter::doParticle spawns children after draw calc, before moving
            // the parent. A zero-alpha newborn parent may still have an opaque child.
            let spawn = jpa
                .child
                .as_ref()
                .is_some_and(|c| {
                    let time = if p.life > 1.0 {
                        p.age / (p.life - 1.0)
                    } else {
                        1.0
                    };
                    time >= c.timing && (c.step == 0 || p.age as i32 % (c.step as i32 + 1) == 0)
                })
                .then_some(*p);
            // `calcGlobalPosition`.
            p.local += p.vel;
            // JPAParticle::calcGlobalPosition applies the global dynamics scale after
            // integrating in local space; fields and velocity shrink with the effect too.
            p.global = p.local * self.global_scale + p.anchor;
            if let Some(parent) = spawn {
                self.create_children(&parent, jpa.child.as_ref().unwrap(), jpa);
            }
            i += 1;
        }
        // Children keep the emitter alive after its parents expire. JPAEmitter::
        // doChildParticle skips velocity integration on the birth frame only.
        if let Some(c) = &jpa.child {
            self.children.retain_mut(|p| {
                let newborn = p.just_born;
                if newborn {
                    p.just_born = false;
                } else {
                    p.age += 1.0;
                }
                if p.age >= p.life {
                    return false;
                }
                p.progress = p.age / p.life;
                if !newborn {
                    move_particle(
                        p,
                        &info,
                        if c.fields { &fields } else { &[] },
                        self.fixed_height,
                    );
                }
                p.look.angle = p.look.angle.wrapping_add(p.look.spin);
                if c.alpha_out {
                    p.look.alpha = 1.0 - p.progress;
                }
                if c.scale_out {
                    p.look.scale_x = p.look.base_scale * (1.0 - p.progress);
                    p.look.scale_y = p.look.wave * (1.0 - p.progress);
                }
                if newborn {
                    p.previous_look = p.look;
                }
                p.local += p.vel;
                p.global = p.local * self.global_scale + p.anchor;
                true
            });
        }
        self.frame += 1.0;
    }

    /// JPAEmitter::createChildParticle and JPADraw::initChild. The sweep shape's
    /// inheritance bits select authored child values independently of the parent.
    fn create_children(&mut self, parent: &Particle, c: &ChildShape, jpa: &Jpa) {
        for _ in 0..c.rate.max(0) {
            let mut p = *parent;
            p.local = parent.local;
            if c.pos_random != 0.0 {
                p.local += rot_xy(self.rng.angle(), self.rng.angle())
                    * Vec3::new(0.0, 0.0, self.rng.f() * c.pos_random);
            }
            p.base_vel = parent.base_vel * c.vel_inherit;
            p.field_accel = parent.field_vel * c.vel_inherit;
            if c.base_vel != 0.0 {
                let speed = c.base_vel * (1.0 + c.vel_random * self.rng.rf());
                let random = Vec3::new(self.rng.sf(), self.rng.sf(), self.rng.sf());
                p.base_vel += set_length(random, speed);
            }
            p.field_vel = Vec3::ZERO;
            p.vel = (p.base_vel + p.field_accel) * p.weight;
            p.dir = Vec3::NEG_Y;
            p.moment = c.moment;
            p.age = 0.0;
            p.life = c.lifetime as f32;
            p.progress = 0.0;
            p.just_born = true;
            p.dead = false;
            p.follow = !self.world_space && jpa.emitter.flags & 0x10 != 0;
            let (mut prm, mut env) = (c.prm, c.env);
            if c.inherit_flags & 4 != 0 {
                for k in 0..3 {
                    prm[k] = (parent.look.prm[k] as f32 * c.rgb_inherit) as u8;
                    env[k] = (parent.look.env[k] as f32 * c.rgb_inherit) as u8;
                }
            }
            if c.inherit_flags & 2 != 0 {
                let alpha = parent.look.alpha * c.alpha_inherit;
                prm[3] = (parent.look.prm[3] as f32 * alpha) as u8;
                env[3] = (parent.look.env[3] as f32 * alpha) as u8;
            }
            let scale = if c.inherit_flags & 1 != 0 {
                [
                    parent.look.scale_x * c.scale_inherit,
                    parent.look.scale_y * c.scale_inherit,
                ]
            } else {
                [1.0; 2]
            };
            p.look = Look {
                up: parent.look.up,
                prm,
                env,
                alpha: 1.0,
                scale_x: scale[0],
                scale_y: scale[1],
                base_scale: scale[0],
                wave: scale[1],
                loop_offset: 0,
                angle: parent.look.angle,
                spin: if c.rotate {
                    (c.rotate_speed * 32768.0) as i32 as i16
                } else {
                    0
                },
                texture: c.texture,
            };
            p.global = p.local * self.global_scale + p.anchor;
            p.previous_local = p.local;
            p.previous_global = p.global;
            p.previous_look = p.look;
            self.children.insert(0, p);
        }
    }

    /// The emitter-wide draw calc (`JPADraw::calc`): colour tables and texture keys animated
    /// by the emitter's frame.
    fn calc_draw_emitter(&mut self, jpa: &Jpa) {
        let base = &jpa.base;
        let tick = self.frame as i32;
        if base.global_col_anm && (base.prm_anm || base.env_anm) {
            let max = base.col_anm_max as i32;
            let frame = match base.col_anm_type {
                0 => tick.min(max),
                1 => tick.rem_euclid(max + 1),
                2 if max > 0 => {
                    let odd = (tick / max) & 1;
                    let f = tick % max;
                    f + odd * max - 2 * odd * f
                }
                _ => 0,
            };
            let f = frame.max(0) as usize;
            if base.prm_anm
                && let Some(c) = base.prm_table.get(f)
            {
                self.prm = *c;
            }
            if base.env_anm
                && let Some(c) = base.env_table.get(f)
            {
                self.env = *c;
            }
        }
        if !base.tex_empty && base.tex_anm && base.global_tex_anm && !base.tex_keys.is_empty() {
            let n = base.tex_keys.len() as i32;
            let i = match base.tex_anm_type {
                0 => tick.min(n - 1),
                1 => tick.rem_euclid(n),
                2 if n > 1 => {
                    let max = n - 1;
                    let odd = (tick / max) & 1;
                    let f = tick % max;
                    f + odd * max - 2 * odd * f
                }
                _ => 0,
            };
            self.texture = base.tex_keys[i.clamp(0, n - 1) as usize];
        } else if !base.tex_anm {
            self.texture = base.texture;
        }
    }

    /// `calcCreateParticle`.
    fn create_particles(&mut self, jpa: &Jpa, info: &mut Info) {
        let em = &jpa.emitter;
        let mut count = 0i32;
        if self.status & EMIT_NEXT_FRAME != 0 {
            if em.flags & FIXED_INTERVAL == 0 {
                let mut rate = self.rate;
                if em.rate_random != 0.0 {
                    rate += rate * em.rate_random * self.rng.rf();
                }
                self.spawn_timer += rate;
                if self.spawn_timer >= 1.0 {
                    count = self.spawn_timer as i32;
                    self.spawn_timer -= count as f32;
                } else if rate > 0.0 && self.status & FIRST_EMIT != 0 {
                    count = 1;
                }
            } else if em.volume_type == 1 {
                info.sphere_done_in_layer = 0;
                info.sphere_pitch = 0;
                info.sphere_yaw = 0;
                info.hemisphere_flip = true;
                info.sphere_in_layer = 0;
                count = 2;
                for _ in 0..em.volume_subdivision {
                    count += info.sphere_in_layer;
                    info.sphere_in_layer += 4;
                    count += info.sphere_in_layer;
                }
                info.sphere_yaw_step = if info.sphere_in_layer != 0 {
                    ((65536.0 * self.volume_sweep) as i32 / info.sphere_in_layer) as i16
                } else {
                    0
                };
                info.sphere_pitch_step = if em.volume_subdivision != 0 {
                    (65536 / 4 / em.volume_subdivision as i32) as i16
                } else {
                    0
                };
            } else if em.volume_type == 5 || em.volume_type == 6 {
                count = em.volume_subdivision as i32;
            }
            info.emit_count = count as i16 as i32;
            if self.status & STOP_EMIT != 0 {
                count = 0;
            }
            // `JPAEmitterManager` has a fixed pool; this keeps a sane cap.
            let room = 1024usize.saturating_sub(self.particles.len()) as i32;
            for i in 0..count.min(room) {
                info.volume_index = i;
                let p = self.create_particle(jpa, info);
                self.particles.insert(0, p);
            }
        }
        if self.emit_rate == 1.0 {
            self.status |= EMIT_NEXT_FRAME;
        } else {
            self.emit_timer += self.emit_rate;
            if self.emit_timer >= 1.0 {
                self.emit_timer -= 1.0;
                self.status |= EMIT_NEXT_FRAME;
            } else {
                self.status &= !EMIT_NEXT_FRAME;
            }
        }
        self.status &= !FIRST_EMIT;
    }

    /// The file's emitter with this frame's key-frame curves that aren't kept as emitter state applied
    /// (minimum radius, moment and the four initial velocities).
    fn keyed<'a>(&self, jpa: &'a Jpa) -> std::borrow::Cow<'a, libsms_assets::jpa::Emitter> {
        if !self.keyed.iter().any(Option::is_some) {
            return std::borrow::Cow::Borrowed(&jpa.emitter);
        }
        let mut em = jpa.emitter.clone();
        let k = &self.keyed;
        em.volume_min_radius = k[3].unwrap_or(em.volume_min_radius);
        em.moment = k[5].unwrap_or(em.moment);
        em.vel_omni = k[6].unwrap_or(em.vel_omni);
        em.vel_axis = k[7].unwrap_or(em.vel_axis);
        em.vel_random = k[8].unwrap_or(em.vel_random);
        em.vel_dir = k[9].unwrap_or(em.vel_dir);
        std::borrow::Cow::Owned(em)
    }

    /// `createParticle`.
    fn create_particle(&mut self, jpa: &Jpa, info: &mut Info) -> Particle {
        let em = &*self.keyed(jpa);
        let fixed = em.flags & FIXED_INTERVAL != 0;
        let (index, emit_count) = (info.volume_index, info.emit_count.max(1));
        let (mut pitch, mut yaw) = (0i16, 0i16);
        if fixed && em.volume_type == 1 {
            pitch = info.sphere_pitch;
            yaw = info.sphere_yaw;
            info.sphere_done_in_layer += 1;
            info.sphere_yaw = info.sphere_yaw.wrapping_add(info.sphere_yaw_step);
            if info.sphere_done_in_layer == info.sphere_in_layer {
                info.sphere_done_in_layer = 0;
                if info.hemisphere_flip {
                    info.sphere_in_layer -= 4;
                    if info.sphere_in_layer == 0 {
                        info.sphere_in_layer = 1;
                    } else {
                        info.sphere_yaw_step =
                            ((self.volume_sweep * 65536.0) as i32 / info.sphere_in_layer) as i16;
                    }
                    info.sphere_pitch = info.sphere_pitch.wrapping_neg();
                    info.sphere_pitch = info.sphere_pitch.wrapping_add(info.sphere_pitch_step);
                    info.hemisphere_flip = false;
                } else {
                    info.sphere_pitch = info.sphere_pitch.wrapping_neg();
                    info.hemisphere_flip = true;
                }
            }
            info.sphere_yaw = 0;
        }
        let size = self.volume_size;
        let sweep = self.volume_sweep;
        let min_r = em.volume_min_radius;
        let density = em.flags & FIXED_DENSITY != 0;
        let radius = |rng: &mut Rng| {
            let d = rng.f();
            let d = if density { d.max(0.0).sqrt() } else { d };
            size * (min_r + d * (1.0 - min_r))
        };
        let rng = &mut self.rng;
        let mut torus_ring = Vec3::ZERO;
        let point = match em.volume_type {
            4 => Vec3::ZERO,
            6 => {
                let z = if fixed {
                    size * index as f32 / (emit_count - 1).max(1) as f32 - size * 0.5
                } else {
                    size * rng.sf()
                };
                Vec3::new(0.0, 0.0, z)
            }
            5 => {
                let angle = if fixed {
                    ((index << 16) / emit_count) as i16
                } else {
                    let a = rng.get() as i16;
                    rng.get();
                    a
                };
                let m = if sweep < 1.0 {
                    rot_y((angle as f32 * sweep) as i32 as i16)
                } else {
                    rot_y(angle)
                };
                m * Vec3::new(0.0, 0.0, radius(rng))
            }
            0 => Vec3::new(rng.sf() * size, rng.sf() * size, rng.sf() * size),
            1 => {
                let m = if fixed {
                    rot_xy(pitch, yaw)
                } else if sweep < 1.0 {
                    let y = (sweep * rng.ss()) as i32 as i16;
                    rot_xy(rng.rs(), y)
                } else {
                    let (a, b, c) = (rng.angle(), rng.angle(), rng.angle());
                    rot_xyz(a, b, c)
                };
                m * Vec3::new(0.0, 0.0, radius(rng))
            }
            2 => {
                let a = if sweep < 1.0 {
                    (sweep * rng.ss()) as i32 as i16
                } else {
                    rng.angle()
                };
                let r = radius(rng);
                Vec3::new(r * sin(a), 2.0 * size * rng.sf(), r * cos(a))
            }
            3 => {
                let theta = if sweep < 1.0 {
                    (sweep * rng.ss()) as i32 as i16
                } else {
                    rng.angle()
                };
                let m = rot_y(theta);
                let ring = Vec3::new(size * sin(theta), 0.0, size * cos(theta));
                let rad = size * min_r;
                let phi = rng.angle();
                torus_ring = m * Vec3::new(0.0, rad * sin(phi), rad * cos(phi));
                ring + torus_ring
            }
            _ => Vec3::ZERO,
        };
        let flat = if em.volume_type == 3 {
            torus_ring
        } else {
            Vec3::new(point.x, 0.0, point.z)
        };
        let scaled = point * info.scale;
        let local = info.rot_scale * point;
        let speed_scale = em.speed_random * rng.rf() + 1.0;
        let mut omni = Vec3::ZERO;
        let mut axis = Vec3::ZERO;
        let mut random = Vec3::ZERO;
        let mut along = Vec3::ZERO;
        if em.vel_omni != 0.0 {
            omni = if em.volume_type == 4 {
                set_length(Vec3::new(rng.sf(), rng.sf(), rng.sf()), em.vel_omni)
            } else {
                set_length(scaled, em.vel_omni)
            };
        }
        if em.vel_axis != 0.0 {
            axis = if em.volume_type == 4 {
                let v = if fixed {
                    let a = ((index << 16) / emit_count) as i16;
                    Vec3::new(sin(a), 0.0, cos(a))
                } else {
                    Vec3::new(rng.sf(), 0.0, rng.sf())
                };
                set_length(v, em.vel_axis)
            } else {
                set_length(flat, em.vel_axis)
            };
        }
        if em.vel_random != 0.0 {
            random = Vec3::new(rng.sf(), rng.sf(), rng.sf()) * em.vel_random;
        }
        if em.vel_dir != 0.0 {
            along = if em.spread > 0.0 {
                let y = (32768.0 * rng.rf() * em.spread) as i32 as i16;
                let z = rng.angle();
                info.dir_rot * rot_yz(y, z) * Vec3::Z
            } else {
                info.dir
            } * em.vel_dir;
        }
        let mut vel = along + random + omni + axis;
        if em.speed_random != 0.0 {
            vel *= speed_scale;
        }
        if em.flags & INHERIT_SCALE != 0 {
            vel *= Vec3::from(em.scale);
        }
        let vel = info.rot * vel;
        let moment = if em.moment != 0.0 {
            em.moment * (em.moment_random * rng.rf() + 1.0)
        } else {
            0.0
        };
        let air = (em.air_resistance + em.air_resistance_random * rng.sf()).min(1.0);
        let weight = em.weight * (1.0 - em.weight_random * rng.f());
        let mut life = self.lifetime;
        if em.lifetime_random != 0.0 {
            life *= 1.0 - em.lifetime_random * rng.f();
        }
        let anchor = info.pos;
        let look = init_look(self, jpa, info);
        Particle {
            local,
            global: local * self.global_scale + anchor,
            anchor,
            base_vel: vel,
            field_accel: Vec3::ZERO,
            field_vel: Vec3::ZERO,
            vel: Vec3::ZERO,
            dir: vel.normalize_or_zero(),
            moment,
            air,
            weight,
            age: 0.0,
            life,
            progress: 0.0,
            just_born: true,
            dead: false,
            follow: !self.world_space && em.flags & FOLLOW_EMITTER != 0,
            look,
            previous_local: local,
            previous_global: local * self.global_scale + anchor,
            previous_look: look,
        }
    }
}

/// JPAParticle::calcVelocity shared by parent and child lists.
fn move_particle(
    p: &mut Particle,
    info: &Info,
    fields: &[(Vec3, &libsms_assets::jpa::Field)],
    fixed_height: bool,
) {
    p.field_vel = Vec3::ZERO;
    if p.follow {
        p.anchor = info.pos;
    }
    if p.moment != 0.0 {
        p.base_vel += p.dir * p.moment;
    }
    for (push, f) in fields {
        let mut v = match f.kind {
            0 | 1 => *push,
            // JPAField.cpp: JPAMagnetField / JPANewtonField::affect.
            2 | 3 => {
                let target = if f.status & 2 != 0 {
                    Vec3::from(f.pos)
                } else {
                    info.global_rot * Vec3::from(f.pos)
                };
                let from = if f.status & 2 != 0 { p.global } else { p.local };
                let delta = target - from;
                let distance = delta.length_squared();
                if f.status & 0x80 != 0 && distance >= f.max_distance * f.max_distance {
                    continue;
                }
                let strength = if f.kind == 2 || f.status & 0x100 != 0 {
                    f.magnitude
                } else {
                    let radius = f.param[0] * f.param[0];
                    f.magnitude
                        * if distance > radius {
                            radius * 10. / distance
                        } else {
                            10.
                        }
                };
                delta.normalize_or_zero() * strength
            }
            // JPAVortexField::set/affect: tangent around the authored
            // axis, interpolating inner/outer speeds by squared radius.
            4 => {
                let axis = (info.rot * Vec3::from(f.dir)).normalize_or_zero();
                let radial = p.local - axis * axis.dot(p.local);
                let radius = f.pos[2] * f.pos[2];
                let share = if radius > 0. {
                    radial.length_squared().min(radius) / radius
                } else {
                    0.
                };
                radial.normalize_or_zero().cross(axis)
                    * (f.magnitude * (1. - share) + f.magnitude_random * share)
            }
            _ => continue,
        };
        let scale = fade_scale(f, p.progress);
        if scale <= 0.0 {
            v = Vec3::ZERO;
        } else if scale < 1.0 {
            v *= scale;
        }
        match f.add_type {
            0 => p.field_accel += v,
            1 => p.base_vel += v,
            _ => p.field_vel += v,
        }
    }
    if p.air < 1.0 {
        p.base_vel *= p.air;
    }
    p.field_vel += p.field_accel;
    p.vel = (p.base_vel + p.field_vel) * p.weight;
    if fixed_height {
        p.vel.y = 0.0;
    }
}

/// Presentation only: keep JPA's discrete integration, interpolate motion, size, alpha and
/// spin for high refresh displays. Births begin at their emitted position, not Mario's.
fn sample_particle(p: &Particle, t: f32) -> Particle {
    let mut out = *p;
    out.local = p.previous_local.lerp(p.local, t);
    out.global = p.previous_global.lerp(p.global, t);
    let (a, b) = (p.previous_look, p.look);
    out.look.alpha = a.alpha + (b.alpha - a.alpha) * t;
    out.look.scale_x = a.scale_x + (b.scale_x - a.scale_x) * t;
    out.look.scale_y = a.scale_y + (b.scale_y - a.scale_y) * t;
    out.look.angle = a
        .angle
        .wrapping_add((b.angle.wrapping_sub(a.angle) as f32 * t) as i16);
    for k in 0..4 {
        out.look.prm[k] = (a.prm[k] as f32 + (b.prm[k] as f32 - a.prm[k] as f32) * t).round() as u8;
        out.look.env[k] = (a.env[k] as f32 + (b.env[k] as f32 - a.env[k] as f32) * t).round() as u8;
    }
    out
}

/// `JPADraw::initParticle`.
fn init_look(e: &mut Emitter, jpa: &Jpa, info: &Info) -> Look {
    let base = &jpa.base;
    let rng = &mut e.rng;
    let loop_offset = (rng.f() * base.loop_offset as f32) as i32;
    let (angle, spin, scale, wave) = match &jpa.extra {
        Some(x) => {
            let (angle, spin) = if x.rotate {
                let angle = (x.rotate_angle * 32768.0 + rng.sf() * x.rotate_random_angle * 65536.0)
                    as i32 as i16;
                let speed = x.rotate_speed * (x.rotate_random_speed * rng.rf() + 1.0) * 32768.0;
                let spin = if rng.rf() < x.rotate_direction {
                    speed
                } else {
                    -speed
                };
                (angle, spin as i32 as i16)
            } else {
                (0, 0)
            };
            let scale = rng.rf() * x.random_scale + 1.0;
            let wave = rng.rf() * x.alpha_wave_random + 1.0;
            (angle, spin, scale, wave)
        }
        None => (0, 0, 1.0, 1.0),
    };
    let scale = scale * e.draw_scale;
    Look {
        up: info.axis_y,
        prm: e.prm,
        env: e.env,
        alpha: 1.0,
        scale_x: scale,
        scale_y: scale,
        base_scale: scale,
        wave,
        loop_offset,
        angle,
        spin,
        texture: e.texture,
    }
}

/// `calcFieldFadeScale`.
fn fade_scale(f: &libsms_assets::jpa::Field, progress: f32) -> f32 {
    let st = f.status;
    let mut cut = false;
    let mut out = 1.0;
    if st & 0x8 != 0 && progress < f.fade_in_start {
        out = 0.0;
        cut = true;
    }
    if st & 0x10 != 0 && progress >= f.fade_out_end {
        out = 0.0;
        cut = true;
    }
    if !cut {
        if st & 0x40 != 0 && progress >= f.fade_out_start {
            let d = f.fade_out_end - f.fade_out_start;
            if d > 0.0 {
                out = (f.fade_out_end - progress) / d;
            }
        } else if st & 0x20 != 0 && progress < f.fade_in_end {
            let d = f.fade_in_end - f.fade_in_start;
            if d > 0.0 {
                out = (progress - f.fade_in_start) / d;
            }
        }
    }
    out
}

/// The per-particle draw calc (`setDrawCalcVisitors`' particle visitors, in their order).
fn calc_draw_particle(prm: [u8; 4], env: [u8; 4], p: &mut Particle, jpa: &Jpa) {
    let base = &jpa.base;
    let stripe = base.kind == 5 || base.kind == 6;
    let age = p.age as i32;
    if let Some(x) = jpa.extra.as_ref().filter(|x| x.scale_on()) {
        let timing = |cycle: i16, reverse: bool| -> f32 {
            let cycle = cycle.max(1) as i32;
            let f = (age % cycle) as f32 / cycle as f32;
            if reverse {
                let odd = ((age / cycle) & 1) as f32;
                odd + (f - odd * 2.0 * f)
            } else {
                f
            }
        };
        let mut t = if x.scale_anm_x() {
            timing(x.anm_cycle_x, x.anm_type_x != 0)
        } else {
            p.progress
        };
        let speed = p.vel.length() * 0.01;
        p.look.scale_x = scale_at(x, t, true, p.look.base_scale);
        if x.scale_by_speed_x() && base.kind != 1 {
            p.look.scale_x *= speed;
        }
        if base.kind != 0 {
            if !x.diff_xy() {
                p.look.scale_y = p.look.scale_x;
            } else {
                if x.scale_anm_y() {
                    t = timing(x.anm_cycle_y, x.anm_type_y != 0);
                } else if x.scale_anm_x() {
                    t = p.progress;
                }
                p.look.scale_y = scale_at(x, t, false, p.look.base_scale);
                if x.scale_by_speed_y() {
                    p.look.scale_y *= speed;
                }
            }
        }
    }
    if !base.global_col_anm {
        if base.prm_anm || base.env_anm {
            let max = base.col_anm_max as i32;
            let start = if base.col_loop { p.look.loop_offset } else { 0 };
            let frame = match base.col_anm_type {
                0 => age.min(max),
                1 => (start + age).rem_euclid(max + 1),
                2 if max > 0 => {
                    let t = start + age;
                    let odd = (t / max) & 1;
                    let f = t % max;
                    f + odd * max - 2 * odd * f
                }
                3 => ((start as f32 + (max + 1) as f32 * p.progress) as i32).rem_euclid(max + 1),
                4 => start.rem_euclid(max + 1),
                _ => 0,
            };
            let f = frame.max(0) as usize;
            if base.prm_anm
                && let Some(c) = base.prm_table.get(f)
            {
                p.look.prm = *c;
            }
            if base.env_anm
                && let Some(c) = base.env_table.get(f)
            {
                p.look.env = *c;
            }
        }
    } else {
        p.look.prm = prm;
        p.look.env = env;
    }
    if let Some(x) = jpa.extra.as_ref().filter(|x| x.alpha_on() && !stripe) {
        let t = p.progress;
        let mut a = if t < x.alpha_in_timing {
            t * x.alpha_increase + x.alpha_in_value
        } else if t > x.alpha_out_timing {
            x.alpha_base_value + x.alpha_decrease * (t - x.alpha_out_timing)
        } else {
            x.alpha_base_value
        };
        if x.sin_wave() {
            let theta = age as f32 * 16384.0 * p.look.wave;
            let s = |k: f32| sin((theta * (1.0 - k)) as i32 as i16);
            match x.alpha_wave_type {
                0 => a *= p.look.wave * ((s(x.alpha_wave[0]) - 1.0) * 0.5) * x.alpha_wave[2] + 1.0,
                1 => {
                    a *= (p.look.wave
                        * (((s(x.alpha_wave[0]) + s(x.alpha_wave[1])) - 2.0) * 0.5)
                        * x.alpha_wave[2]
                        + 2.0)
                        * 0.5
                }
                _ => {
                    let m = x.alpha_wave[2] * 0.5 * p.look.wave;
                    a *= (m * (s(x.alpha_wave[0]) - 1.0) + 1.0)
                        * (m * (s(x.alpha_wave[1]) - 1.0) + 1.0);
                }
            }
            a = a.max(0.0);
        }
        p.look.alpha = a;
    }
    if !base.tex_empty && base.tex_anm && !base.global_tex_anm && !base.tex_keys.is_empty() {
        let n = base.tex_keys.len() as i32;
        let start = if base.tex_loop { p.look.loop_offset } else { 0 };
        let i = match base.tex_anm_type {
            0 => age.min(n - 1),
            1 => (start + age).rem_euclid(n),
            2 if n > 1 => {
                let max = n - 1;
                let t = start + age;
                let odd = (t / max) & 1;
                let f = t % max;
                f + odd * max - 2 * odd * f
            }
            3 => ((start as f32 + n as f32 * p.progress) as i32).rem_euclid(n),
            4 => start.rem_euclid(n),
            _ => 0,
        };
        p.look.texture = base.tex_keys[i.clamp(0, n - 1) as usize];
    }
}

/// `JPADrawCalcScaleX` / `ScaleY` at timer `t`.
fn scale_at(x: &ExtraShape, t: f32, is_x: bool, base: f32) -> f32 {
    let (rate_in, value_in, rate_out) = if is_x {
        (x.increase_x, x.scale_in_x, x.decrease_x)
    } else {
        (x.increase_y, x.scale_in_y, x.decrease_y)
    };
    if t < x.scale_in_timing {
        base * (rate_in * t + value_in)
    } else if t > x.scale_out_timing {
        base * (rate_out * (t - x.scale_out_timing) + 1.0)
    } else {
        base
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirrored_sampler_preserves_rgba_and_independent_axes() {
        let tex = FxTexture {
            width: 2,
            height: 2,
            rgba: vec![
                10, 11, 12, 13, 20, 21, 22, 23, 30, 31, 32, 33, 40, 41, 42, 43,
            ],
            wrap: [Wrap::Mirror, Wrap::Clamp],
            version: 1,
        };
        for (u, expected) in [
            (-0.75, 20),
            (-0.25, 10),
            (0.25, 10),
            (0.75, 20),
            (1.25, 20),
            (1.75, 10),
            (2.25, 10),
        ] {
            assert_eq!(
                tex.sample_nearest(u, -1.0),
                [expected, expected + 1, expected + 2, expected + 3]
            );
            assert_eq!(tex.sample_nearest(u, 2.0)[3], expected + 23);
        }
    }

    #[test]
    fn all_current_mirrored_effect_layers_keep_both_sampler_axes() {
        let Some(dir) = libsms::assets::find(None) else {
            eprintln!("SKIP: no SMS assets");
            return;
        };
        for name in [
            "ms_coinget_a",
            "ms_coinget_b",
            "ms_m_sphipd_d",
            "ms_shine_bow",
        ] {
            let fx = Particles::load(&dir, &[name]);
            assert_eq!(fx.effects.len(), 1, "missing {name}");
            assert!(
                fx.effects[0]
                    .jpa
                    .textures
                    .iter()
                    .any(|t| t.wrap_s == Wrap::Mirror && t.wrap_t == Wrap::Mirror)
            );
            assert!(
                fx.textures.iter().any(|t| t.wrap_modes() == [2, 2]),
                "lost mirrored layer: {name}"
            );
        }
        // Rocket D contains an unused mirrored texture at index 2, but its
        // unanimated parent uses clamped texture 0 and has no child shape.
        let rocket = Particles::load(&dir, &["ms_m_rocket_d"]);
        assert_eq!(rocket.effects[0].jpa.base.texture, 0);
        assert!(rocket.effects[0].jpa.child.is_none());
        assert!(rocket.textures.iter().all(|t| t.wrap_modes() == [1,1]));
    }

    #[test]
    fn children_follow_source_timing_inheritance_and_outlive_parent() {
        let mut jpa = puff().effects[0].jpa.clone();
        jpa.emitter.rate = 1.0;
        jpa.emitter.max_frame = 1;
        jpa.emitter.lifetime = 3;
        jpa.child = Some(ChildShape {
            kind: 2,
            lifetime: 6,
            rate: 1,
            timing: 0.5,
            step: 1,
            flags: 1,
            size_x: 2.0,
            size_y: 3.0,
            scale_out: true,
            alpha_out: true,
            inherit_flags: 7,
            scale_inherit: 0.5,
            alpha_inherit: 0.5,
            rgb_inherit: 0.5,
            vel_inherit: 1.0,
            ..Default::default()
        });
        let mut fx = Particles::default();
        fx.add("fixture", jpa);
        fx.emit("fixture", Mat4::IDENTITY, 1.0);
        for frame in 0..2 {
            fx.advance(frame * 4);
            assert!(fx.emitters[0].children.is_empty());
        }
        fx.advance(8);
        let parent = fx.emitters[0].particles[0];
        let child = fx.emitters[0].children[0];
        assert_eq!(child.age, 0.0);
        assert_eq!(
            child.look.prm,
            parent.look.prm.map(|v| (v as f32 * 0.5) as u8)
        );
        assert_eq!(child.look.scale_x, parent.look.scale_x * 0.5);
        assert_eq!(child.base_vel, parent.base_vel);
        fx.advance(12);
        assert!(fx.emitters[0].particles.is_empty());
        assert!(fx.active(), "child keeps emitter alive");
        let child = fx.emitters[0].children[0];
        assert!((child.look.alpha - 5.0 / 6.0).abs() < 1e-6);
        assert!((child.look.scale_x - child.look.base_scale * 5.0 / 6.0).abs() < 1e-6);
        for frame in 4..=8 {
            fx.advance(frame * 4);
        }
        assert!(!fx.active());
    }

    #[test]
    fn coin_child_sparkle_has_authored_opacity_texture_size_and_lifetime() {
        let Some(dir) = libsms::assets::find(None) else {
            eprintln!("SKIP: no SMS assets");
            return;
        };
        let mut fx = Particles::load(&dir, &["ms_coinget_b"]);
        let child = fx.effects[0].jpa.child.as_ref().expect("coin B SSP1");
        assert_eq!(
            (child.texture, child.lifetime, child.rate, child.step),
            (1, 14, 1, 30)
        );
        assert_eq!(child.inherit_flags, 0);
        assert_eq!(child.prm, [255, 200, 100, 200]);
        assert!(child.scale_out && !child.alpha_out);
        fx.emit(
            "ms_coinget_b",
            Mat4::from_translation(Vec3::new(300., 50., -800.)),
            1.0,
        );
        fx.advance(0);
        let e = &fx.emitters[0];
        assert_eq!(e.particles[0].look.alpha, 0.0);
        assert_eq!(e.children.len(), 1);
        assert_eq!(
            e.children[0].look.prm[3], 200,
            "child is visible at birth despite transparent parent"
        );
        fx.build(Vec3::X, Vec3::Y);
        let (draw, paint) = fx.effects[0].child_draw.as_ref().unwrap();
        assert_eq!(draw.base.tiling, [1.0; 2]);
        assert_eq!((draw.base.size_x, draw.base.size_y), (10.0, 8.5));
        let Paint::Basis { first, .. } = paint else {
            panic!("child paint")
        };
        assert_eq!(fx.textures[first[&1] as usize].wrap, [Wrap::Clamp; 2]);
        assert!(fx.textures.iter().any(|t| t.wrap == [Wrap::Mirror; 2]));
        let mut visible_child = false;
        for frame in 1..=18 {
            fx.advance(frame * 4);
            if frame < 14 {
                let p = &fx.emitters[0].children[0];
                visible_child = true;
                assert!((p.look.scale_x - (1.0 - frame as f32 / 14.)).abs() < 1e-6);
                assert_eq!(p.look.prm[3], 200);
                assert_eq!(p.look.alpha, 1.0);
            } else if let Some(e) = fx.emitters.first() {
                assert!(e.children.is_empty());
            }
        }
        assert!(visible_child && !fx.active());
    }

    fn puff() -> Particles {
        let mut p = Particles::default();
        p.add(
            "puff",
            Jpa {
                emitter: libsms_assets::jpa::Emitter {
                    scale: [1.0; 3],
                    trans: [10.0, 20.0, 30.0],
                    volume_type: 4,
                    rate: 2.0,
                    max_frame: 2,
                    lifetime: 5,
                    weight: 1.0,
                    air_resistance: 1.0,
                    vel_dir: 3.0,
                    direction: [0.0, 1.0, 0.0],
                    ..Default::default()
                },
                base: BaseShape {
                    size_x: 1.0,
                    size_y: 1.0,
                    ..Default::default()
                },
                fields: vec![],
                extra: None,
                ex_tex: None,
                keys: vec![],
                child: None,
                textures: vec![Texture {
                    name: "white".into(),
                    format: 0,
                    image: libsms_assets::texture::Image {
                        width: 1,
                        height: 1,
                        rgba: vec![255; 4],
                    },
                    wrap_s: Wrap::Clamp,
                    wrap_t: Wrap::Clamp,
                    min_filter: 1,
                    mag_filter: 1,
                }],
            },
        );
        p
    }

    #[test]
    fn directional_cross_keeps_both_planes_with_pivot_spin_and_yaw() {
        let mut fx = puff();
        let jpa = &mut fx.effects[0].jpa;
        jpa.base.kind = 4;
        jpa.base.tiling = [1.0; 2];
        jpa.emitter.direction = [0.0, 0.0, 1.0];
        jpa.extra = Some(ExtraShape {
            scale_flags: 1,
            pivot_x: 2,
            pivot_y: 0,
            rotate: true,
            rotate_angle: 0.25,
            ..Default::default()
        });
        let at = Mat4::from_rotation_translation(
            glam::Quat::from_rotation_y(0.8),
            Vec3::new(100.0, 20.0, -40.0),
        );
        fx.emit("puff", at, 1.4);
        fx.advance(0);
        fx.build(Vec3::X, Vec3::Y);
        assert_eq!(fx.vertices.len(), 8 * fx.emitters[0].particles.len());
        for pair in fx.vertices.chunks_exact(8) {
            let [a, b] = [&pair[..4], &pair[4..]];
            let width_a = a[1].pos - a[0].pos;
            let width_b = b[1].pos - b[0].pos;
            assert!((width_a.length() - 70.0).abs() < 0.001);
            assert!((width_b.length() - 70.0).abs() < 0.001);
            assert!(width_a.normalize().dot(width_b.normalize()).abs() < 0.001);
            let center = |q: &[FxVertex]| q.iter().map(|v| v.pos).sum::<Vec3>() * 0.25;
            assert!((center(a) - center(b)).length() < 0.001);
            for (a, b) in a.iter().zip(b) {
                assert_eq!((a.u, a.v, a.color), (b.u, b.v, b.color));
            }
        }
    }

    #[test]
    fn disc_wet_slide_spray_draws_crossed_planes_and_expires() {
        let Some(dir) = libsms::assets::find(None) else {
            return;
        };
        let name = "ms_m_watslide_c";
        let mut cross = Particles::load(&dir, &[name]);
        assert!(cross.has(name));
        assert_eq!(cross.effects[0].jpa.base.kind, 4);
        let mut single = cross.clone();
        single.effects[0].jpa.base.kind = 3;
        let mut saw_spray = false;
        for tick in (0..80).step_by(4) {
            let at = Mat4::from_rotation_translation(
                glam::Quat::from_rotation_y(0.8),
                Vec3::new(0.0, 0.0, tick as f32 * 10.0),
            );
            cross.request_scaled(0, name, at, 1.4);
            single.request_scaled(0, name, at, 1.4);
            cross.advance(tick);
            single.advance(tick);
            cross.build(Vec3::X, Vec3::Y);
            single.build(Vec3::X, Vec3::Y);
            assert_eq!(cross.vertices.len(), 2 * single.vertices.len());
            saw_spray |= !cross.vertices.is_empty();
            assert!(cross.vertices.iter().all(|v| v.pos.is_finite()));
        }
        assert!(saw_spray);
        for tick in (80..160).step_by(4) {
            cross.advance(tick);
        }
        cross.build(Vec3::X, Vec3::Y);
        assert!(!cross.active());
        assert!(cross.vertices.is_empty());
    }

    #[test]
    fn disc_slide_waves_refract_both_sides_with_a_mask_and_authored_scale() {
        let Some(dir) = libsms::assets::find(None) else {
            return;
        };
        let name = "ms_m_watslide_a";
        let mut waves = Particles::load(&dir, &[name]);
        assert!(matches!(waves.effects[0].paint, Paint::Refraction { .. }));
        let tex = &waves.textures[0];
        // A framebuffer placeholder made every pixel invisible before. The replacement
        // must also survive Hopoo's near-flat-normal discard; coverage stays separate.
        let pixels: Vec<_> = tex.rgba.chunks_exact(4).collect();
        let survivors = pixels
            .iter()
            .filter(|p| {
                let x = p[3] as f32 / 255.0 * 2.0 - 1.0;
                let y = p[1] as f32 / 255.0 * 2.0 - 1.0;
                p[2] as f32 / 255.0 >= 0.01 && x * x + y * y >= 1.0 - 0.99f32.powi(2)
            })
            .count();
        assert!(
            survivors > 64,
            "RoR2's distortion shader discards the ribbon ({survivors} visible pixels)"
        );
        assert!(
            pixels
                .iter()
                .any(|p| p[1].abs_diff(128) > 2 || p[3].abs_diff(128) > 2)
        );
        assert!(pixels.iter().any(|p| p[2] == 0));
        assert!(pixels.iter().all(|p| p[0] == 255));
        // The enlarged normal map changes shader eligibility, not the authored distortion:
        // gain compensation and the separate mask reconstruct the disc's signed offset.
        let jpa = &waves.effects[0].jpa;
        let ext = jpa.ex_tex.as_ref().unwrap();
        for (i, p) in pixels.iter().enumerate() {
            let u = ((i as u32 % tex.width) as f32 + 0.5) / tex.width as f32;
            let v = ((i as u32 / tex.width) as f32 + 0.5) / tex.height as f32;
            let input = sample(&jpa.textures[ext.ind_texture as usize], u, v);
            let mask = sample(&jpa.textures[ext.second_texture as usize], u, v);
            let stu = [input[3], input[2], input[1]].map(|c| (c * 255.0 - 128.0) / 128.0);
            for (row, channel) in ext.ind_matrix.iter().zip([3, 1]) {
                let expected = row.iter().zip(stu).map(|(a, b)| a * b).sum::<f32>()
                    * 2.0f32.powi(ext.ind_exp as i32)
                    * mask[0]
                    * mask[3];
                let actual = (p[channel] as f32 / 255.0 * 2.0 - 1.0)
                    / blend::REFRACTION_NORMAL_GAIN
                    * (p[2] as f32 / 255.0);
                assert!((actual - expected).abs() < 0.0015);
            }
        }
        for frame in 0..24 {
            waves.request(
                0,
                name,
                Mat4::from_translation(Vec3::new(0.0, 0.0, frame as f32 * 25.0)),
            );
            waves.advance(frame * FRAME_TICKS);
        }
        waves.build(Vec3::X, Vec3::new(0.0, 0.6, -0.8));
        assert_eq!(waves.batches.len(), 1);
        assert_eq!(waves.batches[0].blend, blend::REFRACTION);
        assert!(waves.vertices.len() >= 8);
        assert!(waves.vertices.iter().all(|v| v.pos.is_finite()));
        assert!(waves.vertices.iter().any(|v| v.pos.x < -25.0));
        assert!(waves.vertices.iter().any(|v| v.pos.x > 25.0));
        let e = &waves.emitters[0];
        let scale = waves.effects[0].jpa.keys[0].value(23.0).unwrap();
        assert!((e.draw_scale - scale).abs() < 1e-6);
        for frame in 24..64 {
            waves.advance(frame * FRAME_TICKS);
        }
        waves.build(Vec3::X, Vec3::Y);
        assert!(!waves.active());
        assert!(waves.vertices.is_empty());
    }

    #[test]
    fn one_shots_overlap_stay_put_and_expire_without_requests() {
        let mut p = puff();
        let at = Mat4::from_translation(Vec3::new(100.0, 200.0, 300.0));
        p.emit("puff", at, 1.0);
        assert!(p.active());
        p.advance(0);
        p.emit("puff", at, 1.0);
        // A held emitter using slot 0 must not move or stop either one-shot.
        p.request(0, "puff", Mat4::from_translation(Vec3::splat(5000.0)));
        p.advance(4);
        assert_eq!(p.emitters.len(), 3);
        assert_eq!(p.emitters[0].particles.len(), 4);
        assert_eq!(p.emitters[1].particles.len(), 2);
        assert_eq!(p.emitters[0].global_pos, at.w_axis.truncate());
        let frame = p.emitters[0].frame;
        p.advance(4);
        assert_eq!(
            p.emitters[0].frame, frame,
            "paused render advanced the effect"
        );
        for tick in (8..48).step_by(4) {
            p.advance(tick);
        }
        p.build(Vec3::X, Vec3::Y);
        assert!(!p.active());
        assert!(p.vertices.is_empty());
    }

    #[test]
    fn global_scale_shrinks_motion_translation_and_quads() {
        let origin = Vec3::new(100.0, 200.0, 300.0);
        let at = Mat4::from_rotation_translation(glam::Quat::from_rotation_y(0.8), origin);
        let mut full = puff();
        let mut small = full.clone();
        full.emit("puff", at, 1.0);
        small.emit("puff", at, 0.7);
        for tick in [0, 4, 8] {
            full.advance(tick);
            small.advance(tick);
            full.build(Vec3::X, Vec3::Y);
            small.build(Vec3::X, Vec3::Y);
            assert!(!full.vertices.is_empty());
            assert_eq!(full.vertices.len(), small.vertices.len());
            for (a, b) in full.vertices.iter().zip(&small.vertices) {
                assert!(((a.pos - origin) * 0.7 - (b.pos - origin)).length() < 0.001);
            }
        }
    }

    #[test]
    fn world_smoke_stays_at_birth_height_when_its_emitter_jumps() {
        let mut p = puff();
        p.effects[0].jpa.emitter.flags |= FOLLOW_EMITTER;
        p.effects[0].jpa.emitter.vel_dir = 0.0;
        p.set_world_space("puff", true);
        p.request(0, "puff", Mat4::IDENTITY);
        p.advance(0);
        let original = p.emitters[0].particles.last().unwrap().global;
        p.request(0, "puff", Mat4::from_translation(Vec3::Y * 800.0));
        p.advance(4);
        assert_eq!(p.emitters[0].particles.last().unwrap().global, original);
        assert!(
            p.emitters[0].particles[0].global.y > 800.0,
            "new smoke didn't follow emitter"
        );
        p.rebind(0, Mat4::from_translation(Vec3::Y * 1600.0));
        p.advance(8); // released, but Mario's matrix continues moving
        assert_eq!(p.emitters[0].particles.last().unwrap().global, original);
        p.build(Vec3::X, Vec3::Y);
        assert!(p.batches.iter().all(|b| p.texture_world_space(b.texture)));
    }

    #[test]
    fn dust_clouds_keep_birth_height_while_spreading_and_fading() {
        let Some(dir) = libsms::assets::find(None) else {
            return;
        };
        let configured = crate::view::View::load(&dir).unwrap();
        for &name in crate::view::DUST_CLOUDS {
            let mut p = configured.fx.clone();
            // The belly slide's and the wall slide's smoke are held (requested every frame).
            let held = matches!(name, "ms_m_slipsmoke" | "ms_mariwallsl1");
            if !held {
                p.emit(
                    name,
                    Mat4::from_translation(Vec3::new(100.0, 20.0, -40.0)),
                    1.0,
                );
            }
            let mut saw_smoke = false;
            let mut saw_spread = false;
            for tick in (0..1200).step_by(4) {
                // Advance through the lifetime, including subframes, while the character
                // rebounds far above the impact. One-shot smoke must stay at birth height.
                let rebound = Mat4::from_translation(Vec3::Y * (tick as f32 * 20.0));
                if held && tick < 200 {
                    p.request(0, name, rebound);
                }
                p.rebind(0, rebound);
                p.advance(tick);
                for e in &p.emitters {
                    for particle in &e.particles {
                        saw_smoke = true;
                        assert!(
                            (particle.global.y - particle.previous_global.y).abs() < 0.001,
                            "{name} rose at tick {tick}: {} -> {}",
                            particle.previous_global.y,
                            particle.global.y
                        );
                        let delta = particle.global - particle.previous_global;
                        saw_spread |= delta.x.abs() + delta.z.abs() > 0.001;
                    }
                }
                p.build_interpolated(Vec3::X, Vec3::Y, 0.5, |_| true);
                assert!(p.batches.iter().all(|b| p.texture_world_space(b.texture)));
            }
            assert!(saw_smoke && saw_spread, "{name} lost its smoke/spread");
            assert!(!p.active(), "{name} did not expire");
        }
    }

    #[test]
    fn gate_particles_curl_then_converge_and_keep_opaque_palette_order() {
        let Some(dir) = libsms::assets::find(None) else {
            eprintln!("SKIP: Sunshine assets absent");
            return;
        };
        let mut p = Particles::load(&dir, &["ms_m_gout_gate"]);
        assert_eq!(
            p.effects[0]
                .jpa
                .fields
                .iter()
                .map(|f| f.kind)
                .collect::<Vec<_>>(),
            [3, 2, 4]
        );
        assert!(matches!(p.effects[0].paint, Paint::Palette { .. }));
        p.set_speed("ms_m_gout_gate", 2.);
        p.emit("ms_m_gout_gate", Mat4::IDENTITY, 1.);
        let mut first = None;
        let mut spread = 0f32;
        let mut late = 0f32;
        for frame in 0..46 {
            p.advance(frame * FRAME_TICKS);
            if let Some(e) = p.emitters.first() {
                if frame == 0 {
                    first = Some(e.particles[0].local);
                }
                if let Some(q) = e.particles.first() {
                    spread = spread.max((q.local - first.unwrap()).length());
                }
                if frame == 40 {
                    let target = Vec3::new(0., 110., 350.);
                    late = e
                        .particles
                        .iter()
                        .map(|q| q.local.distance(target))
                        .sum::<f32>()
                        / e.particles.len() as f32;
                }
            }
            p.build(Vec3::X, Vec3::Y);
            assert!(
                p.batches.iter().all(|b| b.blend == blend::ALPHA),
                "opaque colors must occlude one another instead of adding white"
            );
        }
        assert!(spread > 300., "particles never leave the gate");
        assert!(late < 100., "particles missed Mario's return point: {late}");
        assert!(
            p.emitters.iter().all(|e| e.particles.is_empty()),
            "parent palette particles survive the 89-frame reveal"
        );
        // SSP1 children emitted near the end retain their authored 16-frame
        // lifetime after the 89-frame parent reveal (two source frames per step).
        for frame in 46..56 {
            p.advance(frame * FRAME_TICKS);
            p.build(Vec3::X, Vec3::Y);
            assert!(p.batches.iter().all(|b| b.blend == blend::ALPHA));
        }
        assert!(
            !p.active(),
            "child palette particles survive their authored lifetime"
        );
    }

    #[test]
    fn playback_speed_advances_motion_and_lifetime_together() {
        let mut original = puff();
        let mut fast = original.clone();
        fast.set_speed("puff", 2.0);
        original.emit("puff", Mat4::IDENTITY, 1.0);
        fast.emit("puff", Mat4::IDENTITY, 1.0);
        original.advance(0);
        fast.advance(0);
        assert_eq!(original.emitters[0].frame, 1.0);
        assert_eq!(fast.emitters[0].frame, 2.0);
        assert!(
            fast.emitters[0].particles.last().unwrap().global.y
                > original.emitters[0].particles.last().unwrap().global.y
        );
        for tick in [4, 8, 12] {
            original.advance(tick);
            fast.advance(tick);
        }
        assert!(!fast.active(), "2x smoke lingered");
        assert!(original.active(), "1x smoke expired too early");
    }

    #[test]
    fn rendered_motion_changes_between_particle_frames_without_advancing_simulation() {
        let mut p = puff();
        p.emit("puff", Mat4::IDENTITY, 1.0);
        p.advance(0);
        let mut positions = Vec::new();
        for tick in [0.0, 1.0, 2.0, 3.0] {
            p.build_interpolated(Vec3::X, Vec3::Y, p.render_fraction(tick), |_| true);
            positions.push(p.vertices[0].pos);
        }
        assert!(
            positions.windows(2).all(|w| w[1].y > w[0].y),
            "{positions:?}"
        );
        assert_eq!(p.emitters[0].frame, 1.0, "rendering advanced simulation");
        assert_eq!(p.emitters[0].particles[0].age, 0.0);
    }

    #[test]
    fn every_mario_effect_builds_finite_textured_quads() {
        let Some(dir) = libsms::assets::find(None) else {
            return;
        };
        let all = Particles::load(&dir, crate::view::MARIO_EFFECTS);
        if all.effects.is_empty() {
            return;
        }
        for &name in crate::view::MARIO_EFFECTS {
            assert!(all.has(name), "missing effect {name}");
            let mut p = Particles::load(&dir, &[name]);
            let held = matches!(
                name,
                "ms_m_blur2"
                    | "ms_m_blur2sp"
                    | "ms_m_spinshot_a"
                    | "ms_m_spinshot_b"
                    | "ms_m_blur3"
                    | "ms_m_slipsmoke"
                    | "ms_mariwallsl1"
                    | "ms_m_watslide_a"
                    | "ms_m_watslide_b"
                    | "ms_m_watslide_c"
                    | "ms_nozzlesmoke"
                    | "ms_watboost_a"
                    | "ms_watboost_b"
            ) || crate::view::SUPER_POUND_FLAMES.contains(&name);
            if !held {
                p.emit(name, Mat4::IDENTITY, 1.0);
            }
            let mut most = 0;
            for tick in (0..120).step_by(4) {
                if held {
                    p.request(
                        0,
                        name,
                        Mat4::from_translation(Vec3::new(tick as f32, 0.0, 0.0)),
                    );
                }
                p.advance(tick);
                p.build(Vec3::X, Vec3::Y);
                most = most.max(p.vertices.len());
                assert!(
                    p.vertices
                        .iter()
                        .all(|v| v.pos.is_finite() && v.color.iter().all(|c| c.is_finite())),
                    "bad quad in {name}"
                );
                assert!(
                    p.batches.iter().all(|b| b.texture < p.textures.len() as u32
                        && b.first_vertex + b.vertex_count <= p.vertices.len() as u32
                        && b.vertex_count % 4 == 0),
                    "bad batch in {name}"
                );
            }
            assert!(
                most > 0,
                "{name} never drew (shape {})",
                p.effects[0].jpa.base.kind
            );
            for tick in (120..1200).step_by(4) {
                p.advance(tick);
            }
            assert!(!p.active(), "{name} never expired");
        }
    }

    const FLAMES: [&str; 4] = [
        "ms_m_sphipd_a",
        "ms_m_sphipd_b",
        "ms_m_sphipd_c",
        "ms_m_sphipd_d",
    ];

    fn flames() -> Option<Particles> {
        let dir = libsms::assets::find(None)?;
        let p = Particles::load(&dir, &FLAMES);
        if p.effects.len() < 4 {
            eprintln!("skipping: no particle/ in assets (re-extract)");
            return None;
        }
        Some(p)
    }

    /// Asked for every frame, the flames make particles around the bound point; let go, they
    /// burn out within their lifetimes.
    #[test]
    fn the_super_pound_flames_burn_and_die_out() {
        let Some(mut p) = flames() else {
            return;
        };
        let at = Mat4::from_translation(Vec3::new(100.0, 500.0, -40.0));
        let mut tick = 0u64;
        for _ in 0..30 {
            for (i, name) in FLAMES.iter().enumerate() {
                p.request(i as u32, name, at);
            }
            tick += FRAME_TICKS;
            p.advance(tick);
        }
        p.build(Vec3::X, Vec3::Y);
        assert!(!p.vertices.is_empty(), "no quads");
        assert_eq!(p.vertices.len() % 4, 0);
        for b in &p.batches {
            assert!((b.texture as usize) < p.textures.len());
            assert_eq!(b.vertex_count % 4, 0);
        }
        for v in &p.vertices {
            assert!(v.pos.is_finite(), "{v:?}");
            // Close to where they're bound (Mario's middle): within a few metres.
            assert!(
                (v.pos - Vec3::new(100.0, 500.0, -40.0)).length() < 1500.0,
                "{v:?}"
            );
            assert!(v.color.iter().all(|c| (0.0..=1.0).contains(c)), "{v:?}");
        }
        // Some light: the flames aren't drawn black.
        let lit = p.vertices.iter().any(|v| v.color[0] > 0.2);
        assert!(lit, "all dark");
        // Every texture is the size it says.
        for t in &p.textures {
            assert_eq!(t.rgba.len(), (t.width * t.height * 4) as usize);
        }
        for _ in 0..60 {
            tick += FRAME_TICKS;
            p.advance(tick);
        }
        p.build(Vec3::X, Vec3::Y);
        assert!(p.vertices.is_empty(), "{} corners left", p.vertices.len());
        assert!(!p.active());
    }

    /// The bump stars (`ms_dmg_c`) mix env into their colour: orange edges (and red, late) with a
    /// yellow and then pink core. Check the actual baked colour as well as the alpha blend.
    #[test]
    fn bump_stars_are_orange_then_pink() {
        let Some(dir) = libsms::assets::find(None) else {
            return;
        };
        let mut p = Particles::load(&dir, &["ms_dmg_c"]);
        if p.effects.is_empty() {
            return;
        }
        p.emit("ms_dmg_c", Mat4::IDENTITY, 1.0);
        // Particles start at random places in the colour table, so both show at once.
        let (mut orange, mut pink) = (false, false);
        for tick in (0..120).step_by(4) {
            p.advance(tick);
            p.build(Vec3::X, Vec3::Y);
            let mut colour_seen = false;
            for b in &p.batches {
                let verts = &p.vertices[b.first_vertex as usize..][..b.vertex_count as usize];
                if b.blend == blend::ALPHA {
                    if matches!(p.effects[0].paint, Paint::Palette { .. }) {
                        assert!(verts.iter().all(|v| v.color[..3] == [1.; 3]));
                        for pixel in p.textures[b.texture as usize].rgba.chunks_exact(4) {
                            let [r, g, bl, a] = <[u8; 4]>::try_from(pixel).unwrap();
                            if r < 15 || a < 128 {
                                continue;
                            }
                            let (g, bl) = (g as f32 / r as f32, bl as f32 / r as f32);
                            orange |= (0.45..0.56).contains(&g) && bl < 0.05;
                            pink |= bl > g && bl > 0.4;
                        }
                        continue;
                    }
                    // Coverage: black, and under (before) every colour layer.
                    assert!(!colour_seen, "coverage after colour");
                    assert!(verts.iter().all(|v| v.color[..3] == [0.0; 3]));
                    continue;
                }
                assert_eq!(b.blend, blend::ADDITIVE);
                colour_seen = true;
                for v in verts {
                    let [r, g, bl, _] = v.color;
                    if r < 0.05 {
                        continue;
                    }
                    let (g, bl) = (g / r, bl / r);
                    orange |= (0.45..0.56).contains(&g) && bl < 0.05;
                    pink |= bl > g && bl > 0.4;
                }
            }
        }
        assert!(orange, "no orange");
        assert!(pink, "no pink");
    }

    #[test]
    fn tev_lerp_splits_into_two_basis_images() {
        let base = BaseShape {
            color_mode: 3,
            ..BaseShape::default()
        };
        let (k, n) = tev_coefficients(&base, Vec3::new(1.0, 0.5, 0.0), Vec3::new(0.2, 0.0, 0.0));
        assert_eq!(n, 2);
        // (1 - t) env + t prm.
        assert!((k[0] - Vec3::new(0.2, 0.0, 0.0)).length() < 1e-6);
        assert!((k[1] - Vec3::new(1.0, 0.5, 0.0)).length() < 1e-6);
    }

    /// A skin tint touches only the red: the smear's red prm and the spark's red key turn
    /// green, the blues/whites stay, and the file's colours come back for the other skin.
    #[test]
    fn a_skin_turns_only_the_red_colours_green_and_restores_them() {
        let mut fx = Particles::default();
        fx.add(
            "smear",
            Jpa {
                emitter: libsms_assets::jpa::Emitter {
                    lifetime: 5,
                    ..Default::default()
                },
                base: BaseShape {
                    prm: [255, 39, 15, 255],
                    env: [39, 63, 255, 255],
                    ..BaseShape::default()
                },
                fields: vec![],
                extra: None,
                ex_tex: None,
                keys: vec![],
                child: None,
                textures: vec![],
            },
        );
        fx.add(
            "spark",
            Jpa {
                emitter: libsms_assets::jpa::Emitter {
                    lifetime: 5,
                    ..Default::default()
                },
                base: BaseShape {
                    prm: [63, 87, 255, 255],
                    env: [135, 231, 255, 255],
                    prm_anm: true,
                    col_anm_max: 1,
                    prm_table: vec![[255, 87, 72, 183], [63, 87, 255, 255]],
                    ..BaseShape::default()
                },
                fields: vec![],
                extra: None,
                ex_tex: None,
                keys: vec![],
                child: None,
                textures: vec![],
            },
        );
        let base = |fx: &Particles, name: &str| fx.effects[fx.by_name[name]].jpa.base.clone();
        fx.retint_to_green(&["smear", "spark"], true, GreenRule::Red);
        let smear = base(&fx, "smear");
        assert_eq!(smear.prm, [39, 255, 15, 255], "the smear's red is green");
        assert_eq!(
            smear.env,
            [39, 63, 255, 255],
            "its blue mixing colour is not"
        );
        let spark = base(&fx, "spark");
        assert_eq!(spark.prm, [63, 87, 255, 255], "the spark's blue prm is not");
        assert_eq!(
            spark.prm_table[0],
            [87, 255, 72, 183],
            "its red key is green"
        );
        assert_eq!(
            spark.prm_table[1],
            [63, 87, 255, 255],
            "its blue key is not"
        );
        assert_eq!(spark.env, [135, 231, 255, 255], "its light blue env is not");
        fx.retint_to_green(&["smear", "spark"], false, GreenRule::Red);
        assert_eq!(base(&fx, "smear").prm, [255, 39, 15, 255]);
        assert_eq!(base(&fx, "spark").prm_table[0], [255, 87, 72, 183]);
        // Tinting again from green changes nothing more: the file's colours are restored first.
        fx.retint_to_green(&["smear"], true, GreenRule::Red);
        fx.retint_to_green(&["smear"], true, GreenRule::Red);
        assert_eq!(base(&fx, "smear").prm, [39, 255, 15, 255]);
    }

    /// A skin with colours of its own (Piantissimo's pink and brown): red entries become the warm
    /// colour and blue ones the cool, each as bright as the entry was; whites stay, the file's
    /// colours come back when it is turned off, and a second tint starts from the file's.
    #[test]
    fn a_skin_with_colours_of_its_own_recolours_the_smears_and_the_fire() {
        let jpa = |prm: [u8; 4], env: [u8; 4], env_table: Vec<[u8; 4]>| Jpa {
            emitter: libsms_assets::jpa::Emitter { lifetime: 5, ..Default::default() },
            base: BaseShape {
                prm,
                env,
                env_anm: !env_table.is_empty(),
                col_anm_max: env_table.len().saturating_sub(1) as i16,
                env_table,
                ..BaseShape::default()
            },
            fields: vec![],
            extra: None,
            ex_tex: None,
            keys: vec![],
            child: None,
            textures: vec![],
        };
        let mut fx = Particles::default();
        fx.add("smear", jpa([255, 39, 15, 255], [39, 63, 255, 255], vec![]));
        fx.add("spark", jpa([63, 87, 255, 255], [135, 231, 255, 255], vec![]));
        fx.add(
            "flame",
            jpa([255, 255, 255, 255], [255, 255, 135, 255], vec![[255, 144, 0, 255], [128, 0, 0, 255]]),
        );
        let base = |fx: &Particles, name: &str| fx.effects[fx.by_name[name]].jpa.base.clone();
        let (pink, brown, green) = ([200, 100, 180], [140, 100, 40], [20, 170, 20]);
        fx.retint_to_colors(&["smear", "spark"], true, GreenRule::Red, pink, Some([brown, green]));
        let smear = base(&fx, "smear");
        let spark = base(&fx, "spark");
        assert_eq!(spark.prm, [140, 100, 40, 255], "a blue main colour is his brown");
        assert_eq!(spark.env, [20, 170, 20, 255], "its light blue mixing colour is his green");
        assert_eq!(smear.prm, [200, 100, 180, 255], "the red is his pink");
        assert_eq!(smear.env, [20, 170, 20, 255], "the blue mixing colour is his green");
        fx.retint_to_colors(&["flame"], true, GreenRule::Fire, pink, None);
        let flame = base(&fx, "flame");
        assert_eq!(flame.prm, [255, 255, 255, 255], "the white core stays");
        assert_eq!(flame.env, [200, 100, 180, 255], "the yellow is pink");
        assert_eq!(flame.env_table[0], [200, 100, 180, 255], "the orange is pink");
        assert_eq!(flame.env_table[1], [100, 50, 90, 255], "a dim red is a dim pink");
        fx.retint_to_colors(&["smear", "flame"], false, GreenRule::Red, pink, None);
        assert_eq!(base(&fx, "smear").prm, [255, 39, 15, 255]);
        assert_eq!(base(&fx, "flame").env_table[0], [255, 144, 0, 255]);
        fx.retint_to_colors(&["smear"], true, GreenRule::Red, pink, Some([brown, green]));
        fx.retint_to_colors(&["smear"], true, GreenRule::Red, pink, Some([brown, green]));
        assert_eq!(base(&fx, "smear").prm, [200, 100, 180, 255]);
    }

    /// The pound flames run red through orange, so the fire rule takes every warm entry while
    /// the white core stays; the smears' strongly-red-only rule would have missed the orange.
    #[test]
    fn the_fire_rule_turns_the_pound_flames_green() {
        let mut fx = Particles::default();
        fx.add(
            "flame",
            Jpa {
                emitter: libsms_assets::jpa::Emitter {
                    lifetime: 5,
                    ..Default::default()
                },
                base: BaseShape {
                    prm: [255, 255, 255, 255],
                    env: [255, 144, 0, 255],
                    env_anm: true,
                    col_anm_max: 1,
                    env_table: vec![[255, 144, 0, 255], [255, 0, 0, 255]],
                    ..BaseShape::default()
                },
                fields: vec![],
                extra: None,
                ex_tex: None,
                keys: vec![],
                child: None,
                textures: vec![],
            },
        );
        fx.retint_to_green(&["flame"], true, GreenRule::Fire);
        let e = |fx: &Particles| fx.effects[fx.by_name["flame"]].jpa.base.clone();
        assert_eq!(
            e(&fx).prm,
            [255, 255, 255, 255],
            "the white core stays white"
        );
        assert_eq!(e(&fx).env, [144, 255, 0, 255], "orange becomes green");
        assert_eq!(e(&fx).env_table[0], [144, 255, 0, 255]);
        assert_eq!(e(&fx).env_table[1], [0, 255, 0, 255], "red becomes green");
        // The strict rule leaves that orange alone, which is why fire has its own.
        fx.retint_to_green(&["flame"], false, GreenRule::Fire);
        fx.retint_to_green(&["flame"], true, GreenRule::Red);
        assert_eq!(e(&fx).env, [255, 144, 0, 255]);
    }

    /// The green is not shipped: it comes from the user's own disc. A first run extracts
    /// `particle.szs` from that disc, and the Luigi retint of those files turns the red green.
    /// Set `SMS_DISC` to the Sunshine image the mod extracts on first launch.
    #[test]
    #[ignore = "requires SMS_DISC and extracts the user's disc into a temporary cache"]
    fn a_first_run_extraction_holds_the_effects_the_luigi_tint_turns_green() {
        let disc = std::env::var("SMS_DISC").expect("set SMS_DISC to a Sunshine ISO/GCM");
        let cache = std::env::temp_dir().join(format!("sms-smear-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&cache);
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(cache.clone());
        libsms::extract::extract_assets(std::path::Path::new(&disc), &cache, &mut |_| {})
            .expect("extract the disc");
        let mut fx = Particles::load(&cache, &["ms_m_blur2", "ms_m_blur2sp", "ms_m_blur3"]);
        for name in ["ms_m_blur2", "ms_m_blur2sp", "ms_m_blur3"] {
            assert!(
                fx.has(name),
                "{name} is missing from the first-run extraction"
            );
        }
        let prm = |fx: &Particles, name: &str| fx.effects[fx.by_name[name]].jpa.base.prm;
        assert_eq!(prm(&fx, "ms_m_blur3"), [255, 39, 15, 255], "the file's red");
        fx.retint_to_green(
            &["ms_m_blur2", "ms_m_blur2sp", "ms_m_blur3"],
            true,
            GreenRule::Red,
        );
        assert_eq!(
            prm(&fx, "ms_m_blur3"),
            [39, 255, 15, 255],
            "green, in memory"
        );
        assert_eq!(
            fx.effects[fx.by_name["ms_m_blur2sp"]].jpa.base.prm_table[0],
            [87, 255, 72, 183],
            "the spark's red key is green too"
        );
    }
}
