//! The game's own particle effects (JParticle, `libs/JSystem/src/JParticle/`), run on the CPU
//! from the disc's `.jpa` files ([`libsms_assets::jpa`]) and handed to hosts as textured quads
//! in the sim's world space, like the water's.
//!
//! The sim is `JPABaseEmitter::calc` and its helpers, once per 30 Hz game frame: emission
//! (`calcCreateParticle`, `createParticle`), fields (`JPAFieldManager`; gravity and air),
//! particle motion (`JPAParticle::calcVelocity`, `calcGlobalPosition`) and the draw-side
//! per-particle work (`JPADraw::calcParticle` with the visitors `setDrawCalcVisitors` picks:
//! scale, colour tables, alpha, spin). Shapes are built as `JPADrawExec*` draws them:
//! billboards, directional quads and (crossed) stripes.
//!
//! Colour: every effect Mario uses adds light (`GX_BM_BLEND`, source alpha, one). TEV stage 0
//! (`JPADrawSetupTev::setupTev`) mixes the prm and env colours with the texture colour `t`
//! (`d + a (1 - c) + b c`, inputs per `JPABaseShape::unk48`), and its alpha is the texture's
//! times prm's. That is a polynomial in `t` with colour coefficients, so each effect's texture
//! is baked once into basis images (`(1 - t) tA` and `t tA`, or the quadratic Bernstein three
//! when the texture appears twice) and every particle is drawn once per basis image with the
//! coefficient (times prm alpha) as its vertex colour: summed by additive blending, that is
//! the game's colour exactly, for any host shader that multiplies a texture by a vertex colour.
//! A stripe with a second texture (`ETX1`, which reads the texture at other coordinates) can't
//! be split like that; its whole TEV is baked into a texture each frame instead.
//!
//! Not ported: child particles (`SSP1`), key-frame animation (`KFA1`), indirect textures, the
//! fields other than gravity and air, and blending other than additive (drawn as see-through
//! texture times prm colour).

use std::collections::HashMap;
use std::f32::consts::TAU;
use std::path::Path;

use glam::{Mat3, Mat4, Vec3};
use libsms_assets::jpa::{BaseShape, ExtraShape, Jpa};
use libsms_assets::texture::{Texture, Wrap};

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
    pub repeat: bool,
    pub version: u32,
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
    world_space: bool,
    fixed_height: bool,
    time: f32,
    /// `JPADraw::mPrmColor`, `mEnvColor`, `mTexIdx`.
    prm: [u8; 4],
    env: [u8; 4],
    texture: u8,
    /// Newest first, as `mParticleList` (particles are prepended).
    particles: Vec<Particle>,
}

/// How an effect is drawn.
#[derive(Debug, Clone)]
enum Paint {
    /// Static basis textures (per main texture: the first of `count`), coefficients per
    /// particle.
    Basis { first: HashMap<u8, u32>, count: u32 },
    /// One texture rebaked every frame (stripes with a second texture).
    Baked { texture: u32 },
}

/// A loaded `.jpa`.
#[derive(Debug, Clone)]
struct Effect {
    name: String,
    jpa: Jpa,
    paint: Paint,
    world_space: bool,
    fixed_height: bool,
    speed: f32,
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
    let wrap = |i: i32, n: i32, mode: Wrap| -> i32 {
        match mode {
            Wrap::Clamp => i.clamp(0, n - 1),
            Wrap::Repeat => i.rem_euclid(n),
            Wrap::Mirror => {
                let p = i.rem_euclid(2 * n);
                if p < n { p } else { 2 * n - 1 - p }
            }
        }
    };
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

/// The basis images of `tex`: each texel's `t` turned into the basis polynomial, times the
/// texture's alpha (the TEV alpha's texture half). Alpha is 1: the vertex colours carry the
/// rest.
fn basis_images(tex: &Texture, count: u32) -> Vec<FxTexture> {
    let img = &tex.image;
    let repeat = tex.wrap_s != Wrap::Clamp || tex.wrap_t != Wrap::Clamp;
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
                repeat,
                version: 1,
            }
        })
        .collect()
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
        let base = &jpa.base;
        let stripe = base.kind == 5 || base.kind == 6;
        let second = jpa.ex_tex.as_ref().is_some_and(|e| e.second_on());
        let paint = if stripe && second {
            let size = jpa
                .ex_tex
                .as_ref()
                .and_then(|e| jpa.textures.get(e.second_texture as usize))
                .map_or(32, |t| t.image.width.max(t.image.height).clamp(8, 64));
            self.textures.push(FxTexture {
                width: size,
                height: size,
                rgba: vec![0; (size * size * 4) as usize],
                repeat: false,
                version: 0,
            });
            Paint::Baked {
                texture: self.textures.len() as u32 - 1,
            }
        } else {
            let count = if base.blend_dst == 1 && quadratic(base) {
                3
            } else {
                2
            };
            let count = if base.blend_dst == 1 { count } else { 1 };
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
                first.insert(t, self.textures.len() as u32);
                if count == 1 {
                    // See-through effects: the texture itself, times prm.
                    self.textures.push(FxTexture {
                        width: tex.image.width,
                        height: tex.image.height,
                        rgba: tex.image.rgba.clone(),
                        repeat: tex.wrap_s != Wrap::Clamp || tex.wrap_t != Wrap::Clamp,
                        version: 1,
                    });
                } else {
                    self.textures.extend(basis_images(tex, count));
                }
            }
            Paint::Basis { first, count }
        };
        self.by_name.insert(name.to_owned(), self.effects.len());
        self.effects.push(Effect {
            name: name.to_owned(),
            jpa,
            paint,
            world_space: false,
            fixed_height: false,
            speed: 1.0,
        });
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
            e.world_space
                && match &e.paint {
                    Paint::Baked { texture: t } => *t == texture,
                    Paint::Basis { first, count } => {
                        first.values().any(|&t| texture >= t && texture < t + count)
                    }
                }
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

    /// Updates the bound matrices without asking for anything (between game frames, and for
    /// emitters already let go that still follow).
    pub fn rebind(&mut self, slot: u32, mtx: Mat4) {
        if let Some(b) = self.binds.get_mut(&slot) {
            *b = mtx;
        }
    }

    /// Any particles alive.
    pub fn active(&self) -> bool {
        self.emitters
            .iter()
            .any(|e| !e.particles.is_empty() || e.status & ENABLE_DELETE == 0)
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
            for p in &mut e.particles {
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
                if e.check_max_frame() && e.particles.is_empty() {
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
            rate: em.rate,
            volume_size: em.volume_size as f32,
            volume_sweep: em.volume_sweep,
            lifetime: em.lifetime as f32,
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
            world_space: self.effects[effect].world_space,
            fixed_height: self.effects[effect].fixed_height,
            time: 0.0,
            prm,
            env,
            texture: base.texture,
            particles: Vec::new(),
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
            let jpa = &fx.jpa;
            let base = &jpa.base;
            if base.kind == 5 || base.kind == 6 {
                let quads = stripe_quads(e, jpa);
                if quads.is_empty() {
                    continue;
                }
                match &fx.paint {
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
                    Paint::Basis { first, count } => {
                        let Some(&t0) = first.get(&e.texture) else {
                            continue;
                        };
                        let (prm, env) = registers(e, None, jpa);
                        let colors = paint_colors(base, prm, env, *count);
                        let m = stripe_tex_mtx(base, e.frame as i32);
                        for (k, c) in colors.iter().enumerate().take(*count as usize) {
                            if c[0] + c[1] + c[2] <= 1.0 / 512.0 && *count > 1 {
                                continue;
                            }
                            let first = self.vertices.len() as u32;
                            for q in &quads {
                                for (p, uv) in q {
                                    let uv = m.map_or(*uv, |m| tex_mtx_apply(m, *uv));
                                    self.vertices.push(FxVertex {
                                        pos: *p,
                                        u: uv[0],
                                        v: uv[1],
                                        color: *c,
                                    });
                                }
                            }
                            self.batches.push(FxBatch {
                                texture: t0 + k as u32,
                                first_vertex: first,
                                vertex_count: self.vertices.len() as u32 - first,
                                blend: if *count == 1 {
                                    blend::ALPHA
                                } else {
                                    blend::ADDITIVE
                                },
                            });
                        }
                    }
                }
                continue;
            }
            let Paint::Basis { first, count } = &fx.paint else {
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
                Box::new(e.particles.iter())
            } else {
                Box::new(e.particles.iter().rev())
            };
            // One batch per (texture, basis image); particles append to theirs.
            let mut runs: Vec<(u32, Vec<FxVertex>)> = Vec::new();
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
                let Some(corners) = particle_quad(e, p, pos, jpa, right, up) else {
                    continue;
                };
                let (prm, env) = registers(e, Some(p), jpa);
                let colors = paint_colors(base, prm, env, *count);
                for (k, c) in colors.iter().enumerate().take(*count as usize) {
                    if *count > 1 && c[0] + c[1] + c[2] <= 1.0 / 512.0 {
                        continue;
                    }
                    let texture = t0 + k as u32;
                    let run = match runs.iter_mut().find(|r| r.0 == texture) {
                        Some(r) => r,
                        None => {
                            runs.push((texture, Vec::new()));
                            runs.last_mut().unwrap()
                        }
                    };
                    for (pos, uv) in &corners {
                        run.1.push(FxVertex {
                            pos: *pos,
                            u: uv[0],
                            v: uv[1],
                            color: *c,
                        });
                    }
                }
            }
            for (texture, verts) in runs {
                let first = self.vertices.len() as u32;
                self.vertices.extend(verts);
                self.batches.push(FxBatch {
                    texture,
                    first_vertex: first,
                    vertex_count: self.vertices.len() as u32 - first,
                    blend: if *count == 1 {
                        blend::ALPHA
                    } else {
                        blend::ADDITIVE
                    },
                });
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
            [
                Vec3::new(x0, y0, 0.0),
                Vec3::new(x1, y0, 0.0),
                Vec3::new(x1, y1, 0.0),
                Vec3::new(x0, y1, 0.0),
            ]
            .map(|v| m * v)
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
            // `calcVelocity`.
            p.field_vel = Vec3::ZERO;
            if p.follow {
                p.anchor = info.pos;
            }
            if p.moment != 0.0 {
                p.base_vel += p.dir * p.moment;
            }
            for (push, f) in &fields {
                let mut v = *push;
                let scale = fade_scale(f, p.progress);
                if scale <= 0.0 {
                    v = Vec3::ZERO;
                } else if scale < 1.0 {
                    v *= scale;
                }
                // Gravity (0) and air (1) push alike here (air's cone and clamp aren't
                // ported); the others aren't ported.
                if f.kind > 1 {
                    continue;
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
            if self.fixed_height {
                p.vel.y = 0.0;
            }
            // `JPADraw::calcParticle`.
            p.look.angle = p.look.angle.wrapping_add(p.look.spin);
            calc_draw_particle(self.prm, self.env, p, jpa);
            // The initial draw parameters must already use ESP1's birth scale/alpha;
            // interpolating from uninitialised defaults makes new smoke flash full size.
            if newborn {
                p.previous_look = p.look;
            }
            // `calcGlobalPosition`.
            p.local += p.vel;
            // JPAParticle::calcGlobalPosition applies the global dynamics scale after
            // integrating in local space; fields and velocity shrink with the effect too.
            p.global = p.local * self.global_scale + p.anchor;
            i += 1;
        }
        self.frame += 1.0;
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

    /// `createParticle`.
    fn create_particle(&mut self, jpa: &Jpa, info: &mut Info) -> Particle {
        let em = &jpa.emitter;
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
                has_children: false,
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
    fn pound_smoke_keeps_birth_height_while_spreading_and_fading() {
        let Some(dir) = libsms::assets::find(None) else {
            return;
        };
        let configured = crate::view::View::load(&dir).unwrap();
        for name in ["ms_hipdrop_b", "ms_m_sphipd_hit_b"] {
            let mut p = configured.fx.clone();
            p.emit(
                name,
                Mat4::from_translation(Vec3::new(100.0, 20.0, -40.0)),
                1.0,
            );
            let mut saw_smoke = false;
            let mut saw_spread = false;
            for tick in (0..1200).step_by(4) {
                // Advance through the lifetime, including subframes, while the character
                // rebounds far above the impact. One-shot smoke must stay at birth height.
                p.rebind(0, Mat4::from_translation(Vec3::Y * (tick as f32 * 20.0)));
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
                    | "ms_m_blur3"
                    | "ms_m_slipsmoke"
                    | "ms_mariwallsl1"
                    | "ms_m_watslide_a"
                    | "ms_m_watslide_b"
                    | "ms_m_watslide_c"
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
}
