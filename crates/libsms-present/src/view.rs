//! Mario as the game draws him, for engines that draw themselves: the disc's body, hands, cap,
//! FLUDD and nozzles, posed from the sim every frame and CPU-skinned into one vertex buffer,
//! with each material's TEV baked into one texture ([`crate::tev`]).
//!
//! What's drawn follows the viewer (`viewer/src/mario_model.rs`), which follows the game:
//! the body clip with the pumping arm layered on `jnt_chest` (`M3UModelMario`), the hand
//! models `changeHand` picks per clip, the cap on `M_head` (`TMarioCap`), FLUDD on
//! `jnt_chest` and the fitted nozzle on FLUDD's `nozzle_center` with the game's aiming
//! callbacks (`TWaterGun::init`, WaterGun.cpp), and the eyes' blink pattern (`ma_wink_tx`).
//!
//! Every part is posed in Mario's model space (sim units, +Z forward, +Y up), so all of a
//! frame's draws share Mario's position and facing. Models are right-handed: a left-handed
//! host mirrors each instance across its own X.

use std::collections::HashMap;
use std::path::Path;

use glam::{Mat4, Quat, Vec3};
use libsms::{Clip, ClipLibrary, Event, FxAt, Mario, NozzleKind, ParticleCue, Sim, Skeleton};
use libsms_assets::bmd::{Blend, Cull};
use libsms_assets::btp::TexPattern;
use libsms_assets::mario::{BODY_HAND_SHAPES, HIDDEN_SHAPE, Hands, SHIRT_SHAPE, hands_for_clip};
use libsms_assets::{Batch, Bmd, Pose, paths};

use crate::hud::{Gauge, Hud};
use crate::coins::{CoinMeter, Kind as CoinKind};
use crate::democam::DemoCamera;
use crate::life::LifeMeter;
use crate::screenfx::ScreenFx;
use crate::particles::Particles;
use crate::tev::{Baked, C4, Inputs, Source, Tex, env_uv};

/// Model indices.
pub mod models {
    pub const BODY: u32 = 0;
    /// Open hands, right then left (`mHandModels[0]`).
    pub const HAND_OPEN: [u32; 2] = [1, 2];
    /// Fists, right then left (`mHandModels[1]`).
    pub const FIST: [u32; 2] = [3, 4];
    pub const CAP: u32 = 5;
    pub const FLUDD: u32 = 6;
    /// Spray, Hover, Rocket, Turbo, Yoshi (`dummy_wg`).
    pub const NOZZLES: [u32; 5] = [7, 8, 9, 10, 11];
    /// The Turbo dash's water-boost cone, there when the disc's files are.
    pub const WATERBOOST: u32 = 12;
    pub const COUNT: usize = 13;
}

/// Largest baked texture side.
const MAX_BAKE: u32 = 512;
/// Most texture variants (blink frames and the like) baked before reusing the plain one.
const MAX_TEXTURES: usize = 256;

/// Batch blend classes for hosts.
pub mod blend {
    pub const OPAQUE: u8 = 0;
    /// Alpha tested: a cutout at alpha one half.
    pub const CUTOUT: u8 = 1;
    /// Alpha blended.
    pub const TRANSLUCENT: u8 = 2;
    /// Added to the screen.
    pub const ADDITIVE: u8 = 3;
}

pub struct Model {
    pub bmd: Bmd,
    pub batches: Vec<Batch>,
    pub vertex_count: usize,
    /// Bind-pose bounds, model space.
    pub lo: Vec3,
    pub hi: Vec3,
    pub bind: Vec<Mat4>,
    textures: Vec<Option<Tex>>,
    channels: Vec<[C4; 2]>,
    /// Per batch: plain texture id or -1, blend class, lit bits (1 colour, 2 alpha), cull
    /// (0 back, 1 front, 2 none), environment map matrix if env mapped.
    pub batch_texture: Vec<i32>,
    pub batch_blend: Vec<u8>,
    pub batch_lit: Vec<u8>,
    pub batch_cull: Vec<u8>,
    pub batch_env: Vec<Option<Option<[[f32; 3]; 2]>>>,
    /// Colour for untextured batches (white for textured ones).
    batch_color: Vec<C4>,
    /// Per vertex, in vertex order: the alpha a see-through batch takes from its vertices (the
    /// water-boost cone fades along its length that way), 1 elsewhere. A batch that has some
    /// carries bit 4 in `batch_lit`; a host multiplies it into the batch's colour.
    pub vertex_alpha: Vec<f32>,
}

impl Model {
    /// The colour an untextured batch is drawn with (white for textured ones).
    pub fn batch_color_of(&self, batch: usize) -> C4 {
        self.batch_color.get(batch).copied().unwrap_or([1.0; 4])
    }

    fn source(&self) -> Source<'_> {
        Source {
            bmd: &self.bmd,
            textures: &self.textures,
            channels: &self.channels,
        }
    }
}

/// Baked textures, every model's.
#[derive(Default)]
pub struct Textures {
    pub list: Vec<Baked>,
    by_key: HashMap<(u32, usize, Inputs), i32>,
    /// Every bake and plain colour recoloured ([`Look::Shadow`]).
    shade: Option<Shade>,
    versions: HashMap<i32, u32>,
}

/// Which Mario a view draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Look {
    #[default]
    Mario,
    /// Mario's models in Shadow Mario's colours, for a host's dark double. The game draws
    /// Shadow Mario (`TEMario`, emario.cpp) with his own model, `kagemario/default.bmd`,
    /// whose material lays a dark-to-light ramp (TEV registers 0 and 1) over a warped copy of
    /// the screen; here Mario's own textures are put through that ramp by brightness.
    Shadow,
    /// The disc's yellow coin, a static model for hosts to instance and spin themselves.
    Coin,
    /// The disc's blue coin (`coin_blue.bmd`), drawn like [`Look::Coin`].
    BlueCoin,
    /// The Shine Sprite (`TShine`): its model, animations and glow effects, and the Shine Get
    /// demo's camera path, for a host to place and drive with [`View::update_shine`].
    Shine,
}

/// [`Look::Shadow`]'s ramp: dark where Mario's colours are dark, light where they're light.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shade {
    pub dark: [f32; 3],
    pub light: [f32; 3],
}

impl Shade {
    /// Shadow Mario's ramp from his model on the disc (`_mat_head`'s TEV registers 0 and 1),
    /// or a dark blue stand-in without it.
    pub fn load(dir: &Path) -> Shade {
        let reg = |c: [i16; 4]| [0, 1, 2].map(|i| (c[i] as f32 / 255.0).clamp(0.0, 1.0));
        load_bmd(dir, paths::SHADOW_MARIO)
            .ok()
            .and_then(|b| {
                let m = b.material("_mat_head").map(|i| &b.materials[i])?;
                Some(Shade {
                    dark: reg(m.tev_colors[0]),
                    light: reg(m.tev_colors[1]),
                })
            })
            .unwrap_or(Shade {
                dark: [0.1, 0.08, 0.3],
                light: [0.6, 0.75, 1.0],
            })
    }

    /// One colour through the ramp (alpha kept).
    pub fn color(&self, c: C4) -> C4 {
        let l = (0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]).clamp(0.0, 1.0);
        // Squared: Mario's mid tones (red cap and shirt, blue overalls) read dark, white
        // gloves and eyes light.
        let t = l * l;
        let mix = |i: usize| self.dark[i] + (self.light[i] - self.dark[i]) * t;
        [mix(0), mix(1), mix(2), c[3]]
    }

    fn apply(&self, rgba: &mut [u8]) {
        for p in rgba.as_chunks_mut::<4>().0 {
            let c = self.color([0, 1, 2, 3].map(|i| p[i] as f32 / 255.0));
            for i in 0..3 {
                p[i] = (c[i] * 255.0).round() as u8;
            }
        }
    }
}

/// One posed instance.
#[derive(Debug, Clone, Copy)]
pub struct Draw {
    /// Stable while the part is shown (the part's kind in the top byte).
    pub key: u64,
    pub model: u32,
    /// Which part: the model index.
    pub kind: u8,
    /// 1: flashing from a hit (`addDamageFog`).
    pub flags: u8,
    /// Mario's feet, sim units (render-interpolated).
    pub pos: Vec3,
    pub rot: Quat,
    pub first_vertex: u32,
    pub first_color: u32,
    pub alpha: f32,
}

/// The Turbo dash's water-boost cone and where it is in its clips (`TMarioEffect::perform`'s
/// `unk7C`: 0 not shown, 1 `_in` while the dash lasts, 2 `_out` until it completes).
struct Boost {
    /// `01_waterboost_in`, `01_waterboost_out`.
    clips: [Clip; 2],
    state: u8,
    frame: f32,
    /// The game frame (a quarter of the 120 Hz ticks) it last stepped at.
    game_frame: u64,
    texture: Option<libsms_assets::btk::TexSrt>,
    texture_frame: f32,
}

impl Boost {
    /// Clip frames per game frame: `setRate(SMSGetAnmFrameRate())`, 60 over the 30 fps the game
    /// runs at, so the clips' 60 fps frames play in real time.
    const RATE: f32 = 2.0;

    /// One game frame of the cone's state machine; `dashing` is `MARIO_FLAG_FLUDD_EMITTING`.
    fn step(&mut self, dashing: bool) {
        if self.state == 0 && dashing { self.texture_frame = 0.; }
        else if self.state != 0 { self.texture_frame += Self::RATE; }
        match self.state {
            0 => {
                if dashing {
                    self.state = 1;
                    self.frame = 0.0;
                }
            }
            1 => {
                if dashing {
                    self.frame += Self::RATE;
                } else {
                    self.state = 2;
                    self.frame = 0.0;
                }
            }
            _ => {
                self.frame += Self::RATE;
                if self.frame >= self.clips[1].frames {
                    self.state = 0;
                }
            }
        }
    }
}

#[cfg(test)]
mod texture_motion_tests {
    use super::*;
    #[test]
    fn turbo_texture_animation_moves_and_reuses_its_texture_slots() {
        let Some(dir) = libsms::assets::find(None) else { eprintln!("SKIP: Sunshine assets absent"); return };
        let mut view = View::load(&dir).unwrap();
        let boost = view.boost.as_ref().unwrap();
        let texture = boost.texture.as_ref().expect("water boost BTK must load").clone();
        let model = &mut view.models[models::WATERBOOST as usize];
        let ids = model.batch_texture.clone();
        let before: Vec<_> = ids.iter().map(|&id| view.textures.list[id as usize].rgba.clone()).collect();
        let count = view.textures.list.len();
        texture.apply(&mut model.bmd, 12.);
        view.textures.animate(model, 2);
        assert!(ids.iter().enumerate().any(|(i, &id)| before[i] != view.textures.list[id as usize].rgba), "BTK did not move the cone's texture layers");
        for frame in [24., 48., 96.] { texture.apply(&mut model.bmd, frame); view.textures.animate(model, frame as u32 + 2); }
        assert_eq!(count, view.textures.list.len(), "animation allocated texture variants");
        texture.apply(&mut model.bmd, 0.);
        let start: Vec<_> = model.bmd.materials.iter().map(|m| m.tex_mtx.clone()).collect();
        texture.apply(&mut model.bmd, texture.frames);
        assert_eq!(start, model.bmd.materials.iter().map(|m| m.tex_mtx.clone()).collect::<Vec<_>>(), "BTK does not wrap");
    }
}

/// The Shine Sprite's clips, glow effects and demo camera ([`Look::Shine`]).
struct Shine {
    /// `shine_float`, `shine_demo_shine_get`.
    clips: [Clip; 2],
    camera: Option<DemoCamera>,
    /// Joint `body`, where the game binds the glow (`getAnmMtx(2)`).
    body: usize,
    /// 120 Hz ticks since it began, for the particle frames.
    ticks: f32,
}

pub struct View {
    pub models: Vec<Model>,
    pub textures: Textures,
    skeleton: Skeleton,
    clips: ClipLibrary,
    fludd_clips: HashMap<String, Clip>,
    boost: Option<Boost>,
    wink: Option<TexPattern>,
    eye_materials: Vec<usize>,
    hand_joints: [Option<usize>; 2],
    head: Option<usize>,
    chest: Option<usize>,
    center: Option<usize>,
    /// The game's particle effects on Mario: dust, stars, smears, spray and pound flames.
    pub fx: Particles,
    shine: Option<Shine>,
    /// Movement effects and super-pound flames have independent host switches.
    pub movement_effects: bool,
    pub pound_flames: bool,
    fx_ticks: Vec<FxTick>,
    fx_binds: Vec<(u32, FxAt)>,
    /// FLUDD's water gauge (the HUD's tank), redrawn as the tank changes; None without the
    /// HUD's files. `hud_shown` says whether to show it this frame (Mario has FLUDD).
    pub hud: Option<Hud>,
    pub hud_shown: bool,
    /// Mario's life meter: `[0]` as the HUD draws it (with the splash and the "LIFE" text, the
    /// game's eight rays), `[1]` the sun alone with its ring filled to ten rays, for a host's
    /// lock-on marker. None without the HUD's files.
    pub life: [Option<LifeMeter>; 2],
    /// The HUD's counters, `[0]` the yellow coin's, `[1]` the blue coin's, `[2]` Mario's lives; None
    /// without the HUD's files.
    pub coins: [Option<CoinMeter>; 3],
    /// The losing screens ("TOO BAD!", GAME OVER); None without their pictures.
    pub screenfx: Option<ScreenFx>,
    /// The sim tick the gauge last stepped at, for its 30 Hz flash.
    hud_tick: u64,
    /// Wearing the Shine Shirt (off at the start of the game).
    pub shirt: bool,
    time: f32,
    blink_at: f32,
    /// Camera right and up, sim axes, for environment maps.
    right: Vec3,
    up: Vec3,
    /// How many 120 Hz ticks behind the sim the host draws Mario (its own smoothing between
    /// the batches of ticks it runs). The pose is taken that far back too, so a host running
    /// the sim at 60 Hz on a faster display doesn't show each pose for several frames.
    pub lag: f32,
    /// The model yaw at the last two sim ticks seen, for in-between facings.
    yaw_seen: [(u64, i16); 2],

    pub draws: Vec<Draw>,
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub uvs: Vec<[f32; 2]>,
    pub colors: Vec<C4>,
    pub texture_ids: Vec<i32>,
}

/// `PARTICLE_MS_M_SPHIPD_A..D` (`TMario::emitBlurHipDropSuper`).
pub const SUPER_POUND_FLAMES: [&str; 4] = [
    "ms_m_sphipd_a",
    "ms_m_sphipd_b",
    "ms_m_sphipd_c",
    "ms_m_sphipd_d",
];

/// The Shine's glow (`TShine::calc`, Item.cpp): the rays, sparkles, the halo and the rainbow
/// arc, bound to its `body` joint. The halo and arc go with a Shine not yet collected.
pub const SHINE_EFFECTS: [&str; 4] = [
    "ms_shine_senko",
    "ms_shine_kira",
    "ms_shine_promi",
    "ms_shine_bow",
];

/// TMario::emitGetCoinEffect (MarioParticle.cpp), shared by all three coin colours.
pub const COIN_EFFECTS: [&str; 2] = ["ms_coinget_a", "ms_coinget_b"];

/// MarioParticle.cpp and the effects requested by Mario's movement in the sim.
pub const MARIO_EFFECTS: &[&str] = &[
    "ms_m_sphipd_a",
    "ms_m_sphipd_b",
    "ms_m_sphipd_c",
    "ms_m_sphipd_d",
    "ms_mariwalk1_a",
    "ms_mariwalk1_b",
    "ms_mariwalk1_c",
    "ms_jump_ed_a",
    "ms_jump_ed_b",
    "ms_dmg_c",
    "ms_mariwallsl1",
    "ms_wallkick_a",
    "ms_wallkick_b",
    "ms_m_blur3",
    "ms_hipdrop_a",
    "ms_hipdrop_b",
    "ms_hipdrop_c",
    "ms_m_sphipd_hit_a",
    "ms_m_sphipd_hit_b",
    "ms_m_sphipd_hit_c",
    "ms_m_sphipd_hit_d",
    "ms_m_blur2",
    "ms_m_blur2sp",
    // The ground spin's water shot (`emitRotateShootEffect`).
    "ms_m_spinshot_a",
    "ms_m_spinshot_b",
    "ms_m_slipsmoke",
    "ms_m_watslide_a",
    "ms_m_watslide_b",
    "ms_m_watslide_c",
    "ms_fumi_a",
    "ms_fumi_b",
    "ms_fumi_c",
    "ms_coinget_a",
    "ms_coinget_b",
    // The nozzles: the Rocket's launch (`rocketEffectStart`) and every muzzle's smoke.
    "ms_m_rocket_a",
    "ms_m_rocket_b",
    "ms_m_rocket_b2",
    "ms_m_rocket_c",
    "ms_m_rocket_d",
    "ms_nozzlesmoke",
    // The Turbo's dash (`TMarioEffect::perform`): bound to the muzzle while it lasts.
    "ms_watboost_a",
    "ms_watboost_b",
];

/// Host presentation: dust/smoke trails remain where they were emitted when Mario jumps.
pub const SMOKE_EFFECTS: &[&str] = &[
    "ms_mariwalk1_a",
    "ms_mariwalk1_b",
    "ms_mariwalk1_c",
    "ms_jump_ed_a",
    "ms_jump_ed_b",
    "ms_m_slipsmoke",
    "ms_mariwallsl1",
    "ms_wallkick_a",
    "ms_wallkick_b",
    "ms_hipdrop_a",
    "ms_hipdrop_b",
    "ms_hipdrop_c",
    "ms_m_sphipd_hit_b",
];

/// Every effect that is a smoke or dust cloud (the `kemuball` billboard; the others in
/// [`SMOKE_EFFECTS`] are debris, shock rings and marks). Host preference: its puffs spread and
/// fade at their birth height instead of drifting up, whether from a pound, a landing, a run's
/// first step, a belly slide or a wall.
pub const DUST_CLOUDS: &[&str] = &[
    "ms_mariwalk1_a",
    "ms_jump_ed_b",
    "ms_m_slipsmoke",
    "ms_mariwallsl1",
    "ms_wallkick_a",
    "ms_hipdrop_b",
    "ms_m_sphipd_hit_b",
];

// MarioJump.cpp, TMario::hipAttacking: these are one-shot impacts at the floor,
// distinct from the four body-bound flames during the fall. A host's smoothed
// rebound position must not lift their fading rings/smoke off the landing point.
const SUPER_POUND_IMPACTS: &[&str] = &[
    "ms_m_sphipd_hit_a",
    "ms_m_sphipd_hit_b",
    "ms_m_sphipd_hit_c",
    "ms_m_sphipd_hit_d",
];

/// Slots for effects started once that then follow a matrix (see `Particles::emit_following`).
const FOLLOW_SLOTS: u32 = 1000;

struct FxTick {
    tick: u64,
    pos: Vec3,
    yaw: i16,
    /// The corkscrew's tip and roll (`Mario::model_pitch` / `model_roll`), so effects born this tick sit in the
    /// tipped frame, not upright.
    pitch: i16,
    roll: i16,
    pounding: u8,
    /// `Mario::flames`: 1 a corkscrew that has spun for 4 seconds, 2 a rising spin out of the Rocket's climb.
    flames: u8,
    cues: Vec<ParticleCue>,
}

fn fx_matrix(at: FxAt, root: Mat4, joints: &[Mat4], center: Option<usize>, muzzles: &[Mat4]) -> Option<Mat4> {
    match at {
        FxAt::Once { pos, yaw } => Some(Mat4::from_rotation_translation(
            Quat::from_rotation_y(yaw as f32 * std::f32::consts::PI / 32768.0),
            pos,
        )),
        FxAt::Center => center.and_then(|j| joints.get(j)).map(|j| root * *j),
        FxAt::Joint(j) => joints.get(j as usize).map(|j| root * *j),
        // emitAndBindToPosPtr follows position only, not Mario's rotation.
        FxAt::Feet => Some(Mat4::from_translation(root.w_axis.truncate())),
        // FLUDD's muzzle as drawn this frame (the nozzle's `null_G_muzzle` joints).
        FxAt::Muzzle(i) => muzzles.get(i as usize).map(|m| root * *m),
        // `rocketEffectStart`: emitAndBindToPosPtr(.., &mWaterGun->getEmitPos0()) follows the muzzle's
        // position only. The muzzle joint points forward, so taking its rotation too turns the
        // exhaust, which is authored to blow downward, sideways: it pools about his middle.
        FxAt::MuzzleFollow(i) => muzzles
            .get(i as usize)
            .map(|m| Mat4::from_translation((root * *m).w_axis.truncate())),
    }
}

fn load_bmd(dir: &Path, rel: &str) -> Result<Bmd, String> {
    let data = std::fs::read(dir.join(rel)).map_err(|e| format!("{rel}: {e}"))?;
    Bmd::parse(&data).map_err(|e| format!("{rel}: {e}"))
}

/// Builds a model: its batches, texture images (`replace` swaps some first: the caps' and
/// hands' texture 0 is the body's, `TMarioCap`; the nozzles' env-map stand-in is FLUDD's),
/// unlit channel colours and plain bakes.
fn build(
    bmd: Bmd,
    replace: &dyn Fn(usize, &str) -> Option<Tex>,
    index: u32,
    textures: &mut Textures,
) -> Result<Model, String> {
    let inputs = base_inputs(index);
    let batches: Vec<Batch> = bmd
        .batches()
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|b| !b.indices.is_empty())
        .collect();
    let images: Vec<Option<Tex>> = bmd
        .textures
        .iter()
        .enumerate()
        .map(|(i, t)| {
            replace(i, &t.name).or_else(|| Some(Tex::new(t.image.clone(), t.wrap_s, t.wrap_t)))
        })
        .collect();
    // The colour each channel feeds TEV as baked: the material colour (or the average of the
    // vertex colours, where the channel takes them from the vertices) fully lit, and nothing
    // from a specular channel (`GX_AF_SPEC`), whose highlight depends on the view. Mario's
    // materials add their own lighting in TEV (a toon ramp read at the lit colour, then the
    // highlight), so fully lit is where they show their plain colours.
    let unit = |c: [u8; 4]| c.map(|v| v as f32 / 255.0);
    let channels: Vec<[C4; 2]> = bmd
        .materials
        .iter()
        .enumerate()
        .map(|(mi, m)| {
            let avg = |set: usize| -> C4 {
                let mut s = [0.0f32; 4];
                let mut n = 0.0f32;
                for b in batches.iter().filter(|b| b.material == mi) {
                    for v in &b.vertices {
                        for (acc, &c) in s.iter_mut().zip(&v.colors[set]) {
                            *acc += c as f32 / 255.0;
                        }
                        n += 1.0;
                    }
                }
                if n > 0.0 { s.map(|v| v / n) } else { [1.0; 4] }
            };
            let chan = |i: usize| -> C4 {
                let (c, a) = (&m.chans[2 * i], &m.chans[2 * i + 1]);
                let mat = unit(m.mat_colors[i]);
                let spec = |ch: &libsms_assets::bmd::ColorChan| ch.lighting && ch.attn_fn == 0;
                let rgb = if spec(c) {
                    [0.0; 4]
                } else if c.mat_from_vertex {
                    avg(i)
                } else {
                    mat
                };
                let al = if spec(a) {
                    0.0
                } else if a.mat_from_vertex && matches!(m.blend(), Blend::AlphaBlend | Blend::Additive) {
                    // A see-through batch's vertex alpha reaches hosts per vertex
                    // ([`Model::vertex_alpha`]); the bake is the texture's alone.
                    1.0
                } else if a.mat_from_vertex {
                    avg(i)[3]
                } else {
                    mat[3]
                };
                [rgb[0], rgb[1], rgb[2], al]
            };
            [chan(0), chan(1)]
        })
        .collect();
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    let bind = bmd.bind_world();
    let pose = Pose::new(&bmd, Mat4::IDENTITY, &bind);
    for v in batches.iter().flat_map(|b| &b.vertices) {
        let (p, _) = pose.skin(v);
        lo = lo.min(p);
        hi = hi.max(p);
    }
    if lo.x > hi.x {
        (lo, hi) = (Vec3::ZERO, Vec3::ZERO);
    }
    let mut model = Model {
        vertex_count: batches.iter().map(|b| b.vertices.len()).sum(),
        lo,
        hi,
        bind,
        textures: images,
        channels,
        batch_texture: Vec::new(),
        batch_blend: Vec::new(),
        batch_lit: Vec::new(),
        batch_cull: Vec::new(),
        batch_env: Vec::new(),
        batch_color: Vec::new(),
        vertex_alpha: Vec::new(),
        batches,
        bmd,
    };
    for bi in 0..model.batches.len() {
        let mi = model.batches[bi].material;
        let m = &model.bmd.materials[mi];
        let tex = textures.get(&model, index, mi, &inputs);
        let baked = usize::try_from(tex).ok().map(|i| &textures.list[i]);
        // Where a channel takes its alpha from the vertices (the water-boost cone fades along its
        // length that way), the batch's vertex alpha: lowest and mean.
        let alphas: Option<Vec<f32>> = {
            let set = if m.chans[1].mat_from_vertex {
                Some(0)
            } else if m.chans[3].mat_from_vertex {
                Some(1)
            } else {
                None
            };
            set.map(|s| {
                model.batches[bi]
                    .vertices
                    .iter()
                    .map(|v| v.colors[s][3] as f32 / 255.0)
                    .collect()
            })
            .filter(|a: &Vec<f32>| !a.is_empty())
        };
        let vertex_min = alphas.as_ref().map_or(1.0, |a| a.iter().copied().fold(1.0, f32::min));
        let blend = match m.blend() {
            Blend::Opaque => blend::OPAQUE,
            Blend::AlphaTest => blend::CUTOUT,
            // Nearly opaque blending reads as opaque; the deferred shader can't blend. Both the
            // texture's alpha and the vertices' have to be.
            Blend::AlphaBlend if baked.is_some_and(|b| b.min_alpha() >= 0.7) && vertex_min >= 0.7 => {
                blend::OPAQUE
            }
            Blend::AlphaBlend => blend::TRANSLUCENT,
            Blend::Additive => blend::ADDITIVE,
        };
        let see_through = matches!(blend, blend::TRANSLUCENT | blend::ADDITIVE);
        let vertex_faded = see_through && vertex_min < 0.999;
        let lit = (m.chans[0].lighting as u8) | ((m.chans[1].lighting as u8) << 1) | ((vertex_faded as u8) << 2);
        match alphas.filter(|_| vertex_faded) {
            Some(a) => model.vertex_alpha.extend(a),
            None => {
                let n = model.batches[bi].vertices.len();
                model.vertex_alpha.extend(std::iter::repeat_n(1.0, n));
            }
        }
        let cull = match m.cull {
            Cull::Back => 0,
            Cull::Front => 1,
            _ => 2,
        };
        let env = baked.and_then(|b| b.env);
        let color = if tex >= 0 {
            [1.0; 4]
        } else {
            let c = model.source().flat_color(mi, &inputs);
            textures.shade.map_or(c, |s| s.color(c))
        };
        model.batch_texture.push(tex);
        model.batch_blend.push(blend);
        model.batch_lit.push(lit);
        model.batch_cull.push(cull);
        model.batch_env.push(env);
        model.batch_color.push(color);
    }
    Ok(model)
}

/// A model's material inputs before per-draw changes: Mario's body, cap and hands are clean
/// (konst 0's alpha is the dirt level, `TMario::addDirty`).
fn base_inputs(model: u32) -> Inputs {
    Inputs {
        k0_alpha: matches!(model, models::BODY | models::CAP | 1..=4).then_some(0),
        ..Inputs::default()
    }
}

impl Textures {
    pub fn version(&self, id: u32) -> u32 { self.versions.get(&(id as i32)).copied().unwrap_or(0) }

    fn animate(&mut self, model: &Model, version: u32) {
        for (bi, batch) in model.batches.iter().enumerate() {
            let id = model.batch_texture[bi];
            if id < 0 || self.version(id as u32) == version { continue; }
            if let Some(mut baked) = model.source().bake(batch.material, &base_inputs(models::WATERBOOST), MAX_BAKE) {
                if let Some(shade) = self.shade { shade.apply(&mut baked.rgba); }
                self.list[id as usize] = baked;
                self.versions.insert(id, version);
            }
        }
    }
    /// The texture for material `mi` of `model` with `inputs`, baking it the first time; -1
    /// for an untextured material. Past [`MAX_TEXTURES`], new variants fall back to the plain
    /// one.
    fn get(&mut self, model: &Model, index: u32, mi: usize, inputs: &Inputs) -> i32 {
        if let Some(&id) = self.by_key.get(&(index, mi, *inputs)) {
            return id;
        }
        let base = base_inputs(index);
        if self.list.len() >= MAX_TEXTURES && *inputs != base {
            return self.get(model, index, mi, &base);
        }
        let src = model.source();
        let id = if src.textured(mi, inputs) {
            match src.bake(mi, inputs, MAX_BAKE) {
                Some(mut b) => {
                    if let Some(s) = self.shade {
                        s.apply(&mut b.rgba);
                    }
                    self.list.push(b);
                    let id = self.list.len() as i32 - 1;
                    if index == models::WATERBOOST { self.versions.insert(id, 1); }
                    id
                }
                None => -1,
            }
        } else {
            -1
        };
        self.by_key.insert((index, mi, *inputs), id);
        id
    }
}

/// Air Pump composition spans in native centimetres: the wide Hover needs twice
/// the span to give its shorter body the same visual weight as Turbo and Rocket.
pub const NOZZLE_PORTRAIT_SPANS: [f32; 3] = [210.0, 105.0, 105.0];

impl View {
    /// Air Pump still life, from WaterGun.cpp's nozzleBmdData. Same bind geometry,
    /// normalized size, triangle placement and orientation approved in the Rust viewer.
    pub fn nozzle_portrait(&mut self) {
        self.draws.clear(); self.positions.clear(); self.normals.clear();
        self.uvs.clear(); self.colors.clear(); self.texture_ids.clear();
        let places = [Vec3::new(0.0, 55.0, 0.0), Vec3::new(-66.0, -52.0, 0.0), Vec3::new(66.0, -52.0, 0.0)];
        for (i, model) in [models::NOZZLES[1], models::NOZZLES[3], models::NOZZLES[2]].into_iter().enumerate() {
            let mdl = &self.models[model as usize];
            let bind = Pose::new(&mdl.bmd, Mat4::IDENTITY, &mdl.bind);
            let mut lo = Vec3::splat(f32::INFINITY);
            let mut hi = Vec3::splat(f32::NEG_INFINITY);
            for b in &mdl.batches { for v in &b.vertices {
                let p = bind.skin(v).0; lo = lo.min(p); hi = hi.max(p);
            } }
            let scale = NOZZLE_PORTRAIT_SPANS[i] / (hi - lo).max_element().max(0.001);
            let root = Mat4::from_translation(places[i]) * Mat4::from_rotation_y(2.54)
                * Mat4::from_scale(Vec3::splat(scale)) * Mat4::from_translation(-(lo + hi) * 0.5);
            let pose = Pose::new(&mdl.bmd, root, &mdl.bind);
            let first_vertex = self.positions.len() as u32;
            let first_color = self.colors.len() as u32;
            for (bi, b) in mdl.batches.iter().enumerate() {
                for v in &b.vertices {
                    let (p, n) = pose.skin(v);
                    let n = n.normalize_or_zero();
                    self.positions.push(p); self.normals.push(n);
                    self.uvs.push(match mdl.batch_env[bi] {
                        Some(mtx) => { let w = Vec3::new(-n.x, n.y, n.z); env_uv(w.dot(self.right), w.dot(self.up), mtx) },
                        None => [v.uv[0].x, v.uv[0].y],
                    });
                }
                self.texture_ids.push(mdl.batch_texture[bi]);
                self.colors.push(mdl.batch_color[bi]);
            }
            self.draws.push(Draw { key: (model as u64) << 56, model, kind: model as u8,
                flags: 0, pos: Vec3::ZERO, rot: Quat::IDENTITY, first_vertex, first_color, alpha: 1.0 });
        }
    }

    /// Loads Mario's models, rig and clips from the extracted `assets/` folder.
    pub fn load(dir: &Path) -> Result<View, String> {
        View::load_look(dir, Look::Mario)
    }

    /// [`View::load`] drawing `look`.
    pub fn load_look(dir: &Path, look: Look) -> Result<View, String> {
        if look == Look::Coin {
            return Self::load_coin(dir, "coin.bmd");
        }
        if look == Look::BlueCoin {
            return Self::load_coin(dir, "coin_blue.bmd");
        }
        if look == Look::Shine {
            return Self::load_shine(dir);
        }
        let assets = libsms::Assets::load(dir);
        let skeleton = assets.skeleton.clone().ok_or("no rig (bmd/ma_mdl1.bmd)")?;
        let mut textures = Textures {
            shade: (look == Look::Shadow).then(|| Shade::load(dir)),
            ..Textures::default()
        };
        let none = |_: usize, _: &str| None;
        let body = build(
            load_bmd(dir, paths::BODY)?,
            &none,
            models::BODY,
            &mut textures,
        )?;
        let body_tex0 = body
            .bmd
            .textures
            .first()
            .map(|t| Tex::new(t.image.clone(), t.wrap_s, t.wrap_t));
        let fludd_bmd = load_bmd(dir, paths::FLUDD_BODY)?;
        let fludd_tex1 = fludd_bmd
            .textures
            .get(1)
            .map(|t| Tex::new(t.image.clone(), t.wrap_s, t.wrap_t));
        let body_first = |i: usize, _: &str| if i == 0 { body_tex0.clone() } else { None };
        // The nozzles ship an 8x8 stand-in for FLUDD's env-map texture; the game points it at
        // FLUDD's own (`SMS_ChangeTextureAll(.., "H_watergun_main_dummy", ..)`, `TWaterGun::init`).
        let fludd_env = |_: usize, name: &str| {
            if name == "H_watergun_main_dummy" {
                fludd_tex1.clone()
            } else {
                None
            }
        };
        let mut list = vec![body];
        for (i, rel) in [
            paths::HANDS_OPEN[0],
            paths::HANDS_OPEN[1],
            paths::HANDS_FIST[0],
            paths::HANDS_FIST[1],
            paths::CAPS[0],
        ]
        .into_iter()
        .enumerate()
        {
            list.push(build(
                load_bmd(dir, rel)?,
                &body_first,
                i as u32 + 1,
                &mut textures,
            )?);
        }
        list.push(build(fludd_bmd, &none, models::FLUDD, &mut textures)?);
        for (i, rel) in [
            paths::NOZZLE_SPRAY,
            paths::NOZZLE_HOVER,
            paths::NOZZLE_ROCKET,
            paths::NOZZLE_TURBO,
            paths::NOZZLE_YOSHI,
        ]
        .into_iter()
        .enumerate()
        {
            list.push(build(
                load_bmd(dir, rel)?,
                &fludd_env,
                models::NOZZLES[i],
                &mut textures,
            )?);
        }

        // The Turbo dash's cone; without its files (an older extraction) there is none.
        let mut boost = None;
        if let Ok(bmd) = load_bmd(dir, paths::WATERBOOST) {
            let clip = |rel: &str| std::fs::read(dir.join(rel)).ok().and_then(|d| Clip::parse(&d));
            if let (Some(i), Some(o)) = (clip(paths::WATERBOOST_IN), clip(paths::WATERBOOST_OUT)) {
                list.push(build(bmd, &none, models::WATERBOOST, &mut textures)?);
                boost = Some(Boost {
                    clips: [i, o],
                    state: 0,
                    frame: 0.0,
                    game_frame: 0,
                    texture: std::fs::read(dir.join("01_waterboost/01_waterboost.btk")).ok().and_then(|data| libsms_assets::btk::TexSrt::parse(&data).ok()),
                    texture_frame: 0.,
                });
            }
        }

        let mut fludd_clips = HashMap::new();
        for folder in std::fs::read_dir(dir.join("watergun2"))
            .into_iter()
            .flatten()
            .flatten()
        {
            for f in std::fs::read_dir(folder.path())
                .into_iter()
                .flatten()
                .flatten()
            {
                let path = f.path();
                if path.extension().and_then(|e| e.to_str()) != Some("bck") {
                    continue;
                }
                let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                if let Some(c) = std::fs::read(&path).ok().and_then(|d| Clip::parse(&d)) {
                    fludd_clips.insert(stem.to_owned(), c);
                }
            }
        }
        let wink = std::fs::read(dir.join(paths::WINK))
            .ok()
            .and_then(|d| TexPattern::parse(&d).ok());
        let b = &list[0].bmd;
        let eye_materials = wink
            .as_ref()
            .map(|w| {
                w.tracks
                    .iter()
                    .map(|t| b.material(&t.material).unwrap_or(usize::MAX))
                    .collect()
            })
            .unwrap_or_default();
        let mut fx = Particles::load(dir, MARIO_EFFECTS);
        for &name in SMOKE_EFFECTS
            .iter()
            .chain(SUPER_POUND_IMPACTS)
            .chain(&COIN_EFFECTS)
        {
            fx.set_world_space(name, true);
        }
        for &name in DUST_CLOUDS {
            fx.set_fixed_height(name, true);
        }
        Ok(View {
            shine: None,
            hand_joints: [b.joint("jnt_hand_R"), b.joint("jnt_hand_L")],
            head: b.joint("M_head"),
            chest: b.joint("jnt_chest"),
            center: b.joint("center"),
            fx,
            movement_effects: true,
            pound_flames: true,
            fx_ticks: Vec::new(),
            fx_binds: (0..4).map(|s| (s, FxAt::Center)).collect(),
            hud: Hud::load(dir).ok(),
            hud_shown: false,
            life: [
                LifeMeter::load(dir, 8, true).ok(),
                LifeMeter::load(dir, 10, false).ok(),
            ],
            coins: [
                CoinMeter::load(dir, CoinKind::Yellow).ok(),
                CoinMeter::load(dir, CoinKind::Blue).ok(),
                CoinMeter::load(dir, CoinKind::Mario).ok(),
            ],
            screenfx: ScreenFx::load(dir).ok(),
            hud_tick: 0,
            models: list,
            textures,
            skeleton,
            clips: assets.clips,
            fludd_clips,
            boost,
            wink,
            eye_materials,
            shirt: false,
            time: 0.0,
            blink_at: 2.0,
            lag: 0.0,
            yaw_seen: [(0, 0); 2],
            right: Vec3::X,
            up: Vec3::Y,
            draws: Vec::new(),
            positions: Vec::new(),
            normals: Vec::new(),
            uvs: Vec::new(),
            colors: Vec::new(),
            texture_ids: Vec::new(),
        })
    }

    /// TCoin's original model and materials, through the same model ABI as Mario. Hosts
    /// instance this bind pose; they do not call Mario's animation update on this view.
    fn load_coin(dir: &Path, file: &str) -> Result<View, String> {
        let data = std::fs::read(dir.join("scene/mapobj").join(file)).map_err(|e| e.to_string())?;
        let skeleton = Skeleton::from_bmd(&data).ok_or("coin has no rig")?;
        let mut textures = Textures::default();
        let model = build(
            Bmd::parse(&data).map_err(|e| e.to_string())?,
            &|_, _| None,
            models::COUNT as u32,
            &mut textures,
        )?;
        let pose = Pose::new(&model.bmd, Mat4::IDENTITY, &model.bind);
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut uvs = Vec::new();
        for v in model.batches.iter().flat_map(|b| &b.vertices) {
            let (p, n) = pose.skin(v);
            positions.push(p);
            normals.push(n.normalize_or_zero());
            uvs.push([v.uv[0].x, v.uv[0].y]);
        }
        let colors = model.batch_color.clone();
        let texture_ids = model.batch_texture.clone();
        Ok(View {
            shine: None,
            models: vec![model],
            textures,
            skeleton,
            clips: ClipLibrary::default(),
            fludd_clips: HashMap::new(),
            boost: None,
            wink: None,
            eye_materials: Vec::new(),
            hand_joints: [None; 2],
            head: None,
            chest: None,
            center: None,
            fx: Particles::default(),
            movement_effects: false,
            pound_flames: false,
            fx_ticks: Vec::new(),
            fx_binds: Vec::new(),
            hud: None,
            hud_shown: false,
            life: [None, None],
            coins: [None, None, None],
            screenfx: ScreenFx::load(dir).ok(),
            hud_tick: 0,
            shirt: false,
            time: 0.0,
            blink_at: 0.0,
            right: Vec3::X,
            up: Vec3::Y,
            lag: 0.0,
            yaw_seen: [(0, 0); 2],
            draws: vec![Draw {
                key: 1,
                model: 0,
                kind: 0,
                flags: 0,
                pos: Vec3::ZERO,
                rot: Quat::IDENTITY,
                first_vertex: 0,
                first_color: 0,
                alpha: 1.0,
            }],
            positions,
            normals,
            uvs,
            colors,
            texture_ids,
        })
    }

    /// The Shine Sprite: `scene/mapobj/shine.bmd` with its two clips, its glow effects and the
    /// Shine Get camera. The host draws one instance and drives it with [`Self::update_shine`].
    fn load_shine(dir: &Path) -> Result<View, String> {
        let data = std::fs::read(dir.join("scene/mapobj/shine.bmd")).map_err(|e| e.to_string())?;
        let skeleton = Skeleton::from_bmd(&data).ok_or("the Shine has no rig")?;
        let body = skeleton.find("body").ok_or("the Shine has no body joint")?;
        let clip = |name: &str| {
            std::fs::read(dir.join(format!("scene/mapobj/{name}.bck")))
                .ok()
                .and_then(|d| Clip::parse(&d))
                .ok_or(format!("{name}.bck didn't load"))
        };
        let clips = [clip("shine_float")?, clip("shine_demo_shine_get")?];
        let mut textures = Textures::default();
        let model = build(
            Bmd::parse(&data).map_err(|e| e.to_string())?,
            &|_, _| None,
            models::COUNT as u32,
            &mut textures,
        )?;
        let mut fx = Particles::load(dir, &SHINE_EFFECTS);
        // `TShine::calc` asks for them at these rates and lives near the camera (`mKiraRate`,
        // `mSenkoRate`, `mBowRate`, `mPromiLife`: the first of each table).
        fx.set_rate_scale("ms_shine_senko", 0.15);
        fx.set_life_time("ms_shine_promi", 30.0);
        for name in SHINE_EFFECTS {
            fx.set_world_space(name, true);
        }
        let mut view = View {
            shine: Some(Shine {
                clips,
                camera: DemoCamera::load(dir),
                body,
                ticks: 0.0,
            }),
            models: vec![model],
            textures,
            skeleton,
            clips: ClipLibrary::default(),
            fludd_clips: HashMap::new(),
            boost: None,
            wink: None,
            eye_materials: Vec::new(),
            hand_joints: [None; 2],
            head: None,
            chest: None,
            center: None,
            fx,
            movement_effects: true,
            pound_flames: false,
            fx_ticks: Vec::new(),
            fx_binds: Vec::new(),
            hud: None,
            hud_shown: false,
            life: [None, None],
            coins: [None, None, None],
            screenfx: ScreenFx::load(dir).ok(),
            hud_tick: 0,
            shirt: false,
            time: 0.0,
            blink_at: 0.0,
            right: Vec3::X,
            up: Vec3::Y,
            lag: 0.0,
            yaw_seen: [(0, 0); 2],
            draws: vec![Draw {
                key: 1,
                model: 0,
                kind: 0,
                flags: 0,
                pos: Vec3::ZERO,
                rot: Quat::IDENTITY,
                first_vertex: 0,
                first_color: 0,
                alpha: 1.0,
            }],
            positions: Vec::new(),
            normals: Vec::new(),
            uvs: Vec::new(),
            colors: Vec::new(),
            texture_ids: Vec::new(),
        };
        view.update_shine(0.0, Vec3::ZERO, 0, 0, 0.0, 1.0, false);
        Ok(view)
    }

    /// The Shine's clips' lengths in frames (`shine_float`, `shine_demo_shine_get`), None if this
    /// isn't a [`Look::Shine`] view.
    pub fn shine_clip_frames(&self) -> Option<[f32; 2]> {
        self.shine.as_ref().map(|s| [s.clips[0].frames, s.clips[1].frames])
    }

    /// The Shine Get camera at clip frame `frame` of the outside or inside path, in sim units
    /// with Mario's feet at the origin (not yet turned by the Shine's yaw); None without the
    /// camera's files.
    pub fn demo_camera(&self, inside: bool, frame: f32) -> Option<crate::democam::Shot> {
        self.shine.as_ref()?.camera.as_ref().map(|c| c.shot(inside, frame))
    }

    /// The demo camera path's length in clip frames, 0 without it.
    pub fn demo_camera_frames(&self, inside: bool) -> f32 {
        self.shine
            .as_ref()
            .and_then(|s| s.camera.as_ref())
            .map_or(0.0, |c| c.frames(inside))
    }

    /// Poses the Shine and runs its glow, `dt` seconds after the last call. `clip` is 0 for the
    /// model as it waits (it only turns, which the host does), 1 for `shine_float` (it floats up
    /// as Mario touches it) or 2 for `shine_demo_shine_get` (it rises over his head), at `frame`
    /// of it (60 a second). The instance stands at `pos` turned `yaw` (a 16-bit angle) and the
    /// glow follows it; `scale` is the glow's size (`setGlobalScale`: 1, or one half once
    /// taken). The posed vertices replace the view's buffers, for the host's one instance. A host
    /// that draws its instance mirrored across X (the models are right-handed) says so with
    /// `mirror`, so the glow lands on the joint as drawn.
    pub fn update_shine(
        &mut self,
        dt: f32,
        pos: Vec3,
        yaw: i16,
        clip: u8,
        frame: f32,
        scale: f32,
        mirror: bool,
    ) {
        let Some(shine) = self.shine.as_mut() else {
            return;
        };
        let model = &self.models[0];
        let joints = match clip {
            1 | 2 => {
                let c = &shine.clips[(clip - 1) as usize];
                model.bmd.pose(|j| c.sample(j, frame), |_| None)
            }
            _ => model.bind.clone(),
        };
        self.positions.clear();
        self.normals.clear();
        self.uvs.clear();
        let pose = Pose::new(&model.bmd, Mat4::IDENTITY, &joints);
        for b in &model.batches {
            let hide = model.bmd.materials[b.material].cull == Cull::All;
            for v in &b.vertices {
                let (p, n) = pose.skin(v);
                self.positions.push(if hide { Vec3::ZERO } else { p });
                self.normals.push(n.normalize_or_zero());
                self.uvs.push([v.uv[0].x, v.uv[0].y]);
            }
        }
        self.colors = model.batch_color.clone();
        self.texture_ids = model.batch_texture.clone();

        // The glow, asked for every game frame (30 a second, four ticks of 120).
        let turn = Quat::from_rotation_y(yaw as f32 * std::f32::consts::PI / 32768.0);
        let joint = joints[shine.body].w_axis.truncate();
        let joint = if mirror { Vec3::new(-joint.x, joint.y, joint.z) } else { joint };
        let at = Mat4::from_rotation_translation(turn, pos + turn * joint);
        for (slot, name) in SHINE_EFFECTS.iter().enumerate() {
            self.fx.request_scaled(slot as u32, name, at, scale);
        }
        shine.ticks += dt.max(0.0) * 120.0;
        let tick = shine.ticks as u64;
        for slot in 0..SHINE_EFFECTS.len() as u32 {
            self.fx.rebind(slot, at);
        }
        self.fx.advance(tick);
        let fraction = self.fx.render_fraction(shine.ticks as f64);
        self.fx
            .build_interpolated(self.right, self.up, fraction, |_| true);
    }

    /// A host-owned coin was collected. The two disc effects stay at the collection point.
    pub fn coin_get(&mut self, pos: Vec3) {
        for name in COIN_EFFECTS {
            self.fx.emit(name, Mat4::from_translation(pos), 1.0);
        }
    }

    /// The camera's right and up (sim axes, as the host's world), for environment maps.
    pub fn set_camera(&mut self, right: Vec3, up: Vec3) {
        self.right = right.normalize_or(Vec3::X);
        self.up = up.normalize_or(Vec3::Y);
    }

    /// Host tuning for smoke only; 1 preserves disc timing, 2 plays it twice as fast.
    pub fn set_smoke_speed(&mut self, speed: f32) {
        for &name in SMOKE_EFFECTS {
            self.fx.set_speed(name, speed);
        }
    }

    /// Observe every sim tick (`Sim::advance_observed`), even without sound or rendering.
    /// Queue effects until the next pose is available, preserving one-shots and short held
    /// effects between rendered frames. Empty ticks release held emitters at 30 Hz too.
    pub fn events(&mut self, events: &[Event], mario: &Mario) {
        self.fx_ticks.push(FxTick {
            tick: mario.ticks,
            pos: mario.pos,
            yaw: mario.model_yaw,
            pitch: mario.model_pitch,
            roll: mario.model_roll,
            pounding: mario.pounding(),
            flames: mario.flames(),
            cues: events
                .iter()
                .filter_map(|e| match e {
                    Event::Particle(c) => Some(*c),
                    _ => None,
                })
                .collect(),
        });
    }

    /// Poses Mario for this frame, `dt` seconds after the last, and hands FLUDD's posed
    /// muzzles back to the sim (`Mario::set_emitters`) so its water leaves the drawn nozzle.
    pub fn update(&mut self, sim: &mut Sim, dt: f32) {
        self.time += dt.max(0.0);
        self.draws.clear();
        self.positions.clear();
        self.normals.clear();
        self.uvs.clear();
        self.colors.clear();
        self.texture_ids.clear();

        let alpha = sim.alpha();
        let m = &sim.mario;
        let pos = m.prev_pos.lerp(m.pos, alpha);
        // Drawn `lag - alpha` ticks before the sim's latest state: the clip that far back, and
        // the facing between the last two states seen.
        let back = (self.lag - alpha).max(0.0);
        if self.yaw_seen[1].0 != m.ticks {
            self.yaw_seen = [self.yaw_seen[1], (m.ticks, m.model_yaw)];
        }
        let [(t0, y0), (t1, y1)] = self.yaw_seen;
        let yaw = if t1 > t0 && back > 0.0 {
            let t = (1.0 - back / (t1 - t0) as f32).clamp(0.0, 1.0);
            y0 as f32 + y1.wrapping_sub(y0) as f32 * t
        } else {
            m.model_yaw as f32
        };
        let yaw = yaw * std::f32::consts::PI / 32768.0;
        let rot = Quat::from_rotation_y(yaw);
        // `addDamageFog` while invincible; hosts flash instead.
        let flags = (m.invincible > 0) as u8;

        // The body clip, with the pumping arm over the chest while pumping.
        let upper = m
            .upper_layer()
            .and_then(|(c, f)| Some((self.clips.get(c)?, f, self.skeleton.find("jnt_chest")?)));
        let look = m.fludd_look();
        // `MarioHeadCtrl`: the head turns what hangs on it (the cap). Rocketing, with water to spray, it tips
        // back by `mHoverHeadAngle`; otherwise it follows a FLUDD aimed upward, by half the gun's angle.
        // (The game's other head turns, toward a sign or a wall or deep water, aren't here.)
        let head_tilt = self.head.and_then(|h| {
            let angle = if m.status == libsms::Status::Rocket {
                let can_spray = m.fludd.as_ref().is_some_and(|f| f.can_spray(&m.params));
                can_spray.then_some(m.params.upper_body.hover_head_angle as f32)
            } else {
                look.map(|l| l.gun_angle * 0.5).filter(|a| *a < 0.0)
            }?;
            Some((h, Mat4::from_rotation_z(angle * std::f32::consts::PI / 32768.0)))
        });
        let joints = self.skeleton.pose_layered_with(
            self.clips
                .get(m.anim.clip)
                .map(|c| (c, m.anim.frame_before(back))),
            upper,
            |j| head_tilt.filter(|t| t.0 == j).map(|t| t.1),
        );

        // Blink every few seconds (the game runs `ma_wink_tx` on idle; 30 frames a second).
        if self.time > self.blink_at + 2.0 {
            self.blink_at = self.time + 2.0 + (self.time * 7.13).fract() * 2.5;
        }
        let mut body_inputs: HashMap<usize, Inputs> = HashMap::new();
        if let Some(w) = &self.wink {
            let frame = (self.time - self.blink_at) * 30.0;
            if frame >= 0.0 {
                for (i, t) in w.tracks.iter().enumerate() {
                    if let (Some(&mat), Some(tex)) = (self.eye_materials.get(i), w.tex_no(i, frame))
                    {
                        let e = body_inputs.entry(mat).or_default();
                        if (t.tex_map as usize) < 8 {
                            e.tex[t.tex_map as usize] = Some(tex);
                        }
                    }
                }
            }
        }
        let mut hidden = vec![HIDDEN_SHAPE];
        if !self.shirt {
            hidden.push(SHIRT_SHAPE);
        }
        let hands = hands_for_clip(m.anim.clip);
        if hands != Hands::Body {
            hidden.extend(BODY_HAND_SHAPES);
        }

        // FLUDD on the chest, tilted back as the chest leans (`TWaterGun::setBaseTRMtx`:
        // `|m[1][0]| * (unk1D04 - unk1D06) + unk1D06`, 0 and -0x1800), posed by the sim's clip;
        // the nozzle on `nozzle_center` with its own clip and the aiming callbacks.
        let deg = |a: f32| a * std::f32::consts::PI / 32768.0;
        let fludd_base = look
            .and(self.chest)
            .and_then(|c| joints.get(c))
            .map(|chest| {
                let lean = chest.x_axis.truncate().normalize_or_zero().y.abs();
                *chest * Mat4::from_rotation_z(deg(lean * 6144.0 - 6144.0))
            });
        let fludd = &self.models[models::FLUDD as usize].bmd;
        let fludd_joints = fludd.pose(
            |j| {
                look.and_then(|l| self.fludd_clips.get(l.body_clip))
                    .and_then(|c| c.sample(j, look.map_or(0.0, |l| l.body_frame)))
            },
            |_| None,
        );
        let slot = look.map(|l| match l.nozzle {
            NozzleKind::Spray => 0,
            NozzleKind::Hover | NozzleKind::Underwater => 1,
            NozzleKind::Rocket => 2,
            NozzleKind::Turbo => 3,
            // The water leaves Yoshi's mouth; FLUDD shows `dummy_wg`.
            NozzleKind::Yoshi => 4,
        });
        let nozzle_at = fludd_base
            .zip(fludd.joint("nozzle_center"))
            .map(|(base, c)| base * fludd_joints[c]);
        let mut muzzles = Vec::new();
        let mut muzzle_mtx: Vec<Mat4> = Vec::new();
        let nozzle = match (look, slot) {
            (Some(l), Some(i)) => {
                let model = models::NOZZLES[i];
                let bmd = &self.models[model as usize].bmd;
                let extra = |name: &str| match name {
                    // `NozzleCtrl`: the neck dips with the aim, never above level.
                    // A host aim (the lock-on) also swivels the head round to the target
                    // and lifts it above level; not in the game. The pose is in the models'
                    // right-handed space and the host draws it mirrored across X, so the
                    // turn goes the other way round here to look right on screen.
                    "chn_muzzle_1" | "jnt_back_nozzle_neck" if l.aimed => Some(
                        Mat4::from_rotation_y(-deg(l.aim_turn as f32))
                            * Mat4::from_rotation_z(deg(l.gun_angle)),
                    ),
                    "chn_muzzle_1" | "jnt_back_nozzle_neck" if l.gun_angle < 0.0 => {
                        Some(Mat4::from_rotation_z(deg(l.gun_angle)))
                    }
                    // `WaterGunDivingCtrlL/R`: the hover jets lean with the steering.
                    "jnt_nozzle_L" => Some(Mat4::from_rotation_z(deg(-l.tilt[0]))),
                    "jnt_nozzle_R" => Some(Mat4::from_rotation_z(deg(-l.tilt[1]))),
                    // `RotateCtrl`: the Turbo propeller.
                    "chn_back_nozzle_prop" => Some(Mat4::from_rotation_x(deg(l.prop_angle))),
                    _ => None,
                };
                let clip = l.nozzle_clip.and_then(|c| self.fludd_clips.get(c));
                let js = bmd.pose(
                    |j| clip.and_then(|c| c.sample(j, l.nozzle_frame)),
                    |j| extra(&bmd.joints[j].name),
                );
                if let Some(at) = nozzle_at.filter(|_| l.nozzle != NozzleKind::Yoshi) {
                    for name in ["null_G_muzzle", "null_G_muzzle2"] {
                        if let Some(j) = bmd.joint(name) {
                            let mm = at * js[j];
                            muzzles.push((mm.w_axis.truncate(), mm.x_axis.truncate()));
                            muzzle_mtx.push(mm);
                        }
                    }
                }
                nozzle_at.map(|at| (model, at, js))
            }
            _ => None,
        };

        // The Turbo corkscrew dive: the spin jump's motion tipped head first and spun about his own length,
        // through about his middle.
        let tilt = Quat::from_rotation_x(m.model_pitch as f32 * std::f32::consts::PI / 32768.0)
            * Quat::from_rotation_y(m.model_roll as f32 * std::f32::consts::PI / 32768.0);
        let middle = Vec3::new(0.0, libsms::mario::HEIGHT * 0.35, 0.0);
        let root = Mat4::from_rotation_translation(rot * tilt, pos + rot * (middle - tilt * middle));
        // Hosts which only pose the view still get the state-driven super pound flames.
        if self.fx_ticks.is_empty() {
            self.fx_ticks.push(FxTick {
                tick: m.ticks,
                pos,
                yaw: m.model_yaw,
                pitch: m.model_pitch,
                roll: m.model_roll,
                pounding: m.pounding(),
                flames: m.flames(),
                cues: Vec::new(),
            });
        }
        for tick in self.fx_ticks.drain(..) {
            // The same tip and roll as the drawn body (the spin swirl is born in the spin pose's frame).
            let yaw_q = Quat::from_rotation_y(tick.yaw as f32 * std::f32::consts::PI / 32768.0);
            let tick_tilt = Quat::from_rotation_x(tick.pitch as f32 * std::f32::consts::PI / 32768.0)
                * Quat::from_rotation_y(tick.roll as f32 * std::f32::consts::PI / 32768.0);
            let tick_root = Mat4::from_rotation_translation(
                yaw_q * tick_tilt,
                tick.pos + yaw_q * (middle - tick_tilt * middle),
            );
            // Let-go emitters still follow their animation matrix, as in MarioParticle.cpp.
            for &(slot, at) in &self.fx_binds {
                if let Some(mtx) = fx_matrix(at, tick_root, &joints, self.center, &muzzle_mtx) {
                    self.fx.rebind(slot, mtx);
                }
            }
            for cue in tick.cues {
                let Some(mtx) = fx_matrix(cue.at, tick_root, &joints, self.center, &muzzle_mtx) else {
                    continue;
                };
                if matches!(cue.at, FxAt::Once { .. }) {
                    self.fx.emit(cue.name, mtx, cue.scale);
                } else if let FxAt::MuzzleFollow(_) = cue.at {
                    // Its own slot (past the held effects'), kept bound to the muzzle as it moves.
                    if let Some(i) = MARIO_EFFECTS.iter().position(|&n| n == cue.name) {
                        let slot = FOLLOW_SLOTS + i as u32;
                        self.fx.emit_following(slot, cue.name, mtx, cue.scale);
                        if !self.fx_binds.iter().any(|b| b.0 == slot) {
                            self.fx_binds.push((slot, cue.at));
                        }
                    }
                } else if let Some(slot) = MARIO_EFFECTS.iter().position(|&n| n == cue.name) {
                    let slot = slot as u32;
                    if !self.fx_binds.iter().any(|b| b.0 == slot) {
                        self.fx_binds.push((slot, cue.at));
                    }
                    self.fx.request_scaled(slot, cue.name, mtx, cue.scale);
                }
            }
            // emitBlurHipDropSuper, bound to getCenterAnmMtx during the flaming fall.
            if tick.pounding == 2 {
                if let Some(center) = fx_matrix(FxAt::Center, tick_root, &joints, self.center, &muzzle_mtx) {
                    for (slot, name) in SUPER_POUND_FLAMES.iter().enumerate() {
                        self.fx.request(slot as u32, name, center);
                    }
                }
            }
            // The pound's flames on a flying Mario, from about his middle, streaming back along the corkscrew's
            // flight or down behind the Rocket spin's climb (they trail up behind a pounder, as authored).
            if tick.flames != 0 {
                if let Some(centre) = fx_matrix(FxAt::Center, root, &joints, self.center, &muzzle_mtx) {
                    let streaming = if tick.flames == 1 {
                        Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)
                    } else {
                        Quat::from_rotation_x(std::f32::consts::PI)
                    };
                    let mtx = Mat4::from_rotation_translation(rot * streaming, centre.w_axis.truncate());
                    for (slot, name) in SUPER_POUND_FLAMES.iter().enumerate() {
                        self.fx.request(slot as u32, name, mtx);
                    }
                }
            }
            self.fx.advance(tick.tick);
        }
        for &(slot, at) in &self.fx_binds {
            if let Some(mtx) = fx_matrix(at, root, &joints, self.center, &muzzle_mtx) {
                self.fx.rebind(slot, mtx);
            }
        }
        let movement = self.movement_effects;
        let flames = self.pound_flames;
        let fraction = self
            .fx
            .render_fraction(m.ticks as f64 + alpha as f64 - self.lag as f64);
        self.fx
            .build_interpolated(self.right, self.up, fraction, |name| {
                // Host preference: keep the pound's smoke/ring, omit its star spray.
                if name == "ms_m_sphipd_hit_d" {
                    false
                } else if name.starts_with("ms_m_sphipd_") {
                    flames
                } else {
                    movement
                }
            });

        // The water gauge, stepped once a game frame (4 ticks).
        let gauge = Gauge::of(m);
        self.hud_shown = gauge.is_some() && self.hud.is_some();
        if let (Some(hud), Some(g)) = (self.hud.as_mut(), gauge) {
            let frames = (m.ticks / 4).saturating_sub(self.hud_tick / 4) as u32;
            hud.update(&g, frames);
        }
        self.hud_tick = m.ticks;

        // (model, its root in Mario's model space, its joints)
        // The Turbo dash's water-boost cone, on FLUDD's muzzle (`TMarioEffect::perform`, which
        // sets its base matrix to `getEmitMtx(0)`): `_in` while the dash lasts, then `_out`.
        let boost_part = self.boost.as_mut().and_then(|b| {
            let game_frame = m.ticks / 4;
            for _ in 0..game_frame.saturating_sub(b.game_frame).min(8) {
                b.step(m.dashing());
            }
            b.game_frame = game_frame;
            if b.state == 0 {
                return None;
            }
            let at = *muzzle_mtx.first()?;
            let model = self.models.get_mut(models::WATERBOOST as usize)?;
            if let Some(texture) = &b.texture {
                texture.apply(&mut model.bmd, b.texture_frame);
                self.textures.animate(model, game_frame as u32 + 2);
            }
            let bmd = &model.bmd;
            let clip = &b.clips[(b.state == 2) as usize];
            let js = bmd.pose(|j| clip.sample(j, b.frame), |_| None);
            Some((models::WATERBOOST, at, js))
        });

        let mut parts: Vec<(u32, Mat4, &[Mat4])> =
            vec![(models::BODY, Mat4::IDENTITY, &joints)];
        if let Some(h) = self.head.and_then(|h| joints.get(h)) {
            parts.push((
                models::CAP,
                *h,
                &self.models[models::CAP as usize].bind,
            ));
        }
        let set = match hands {
            Hands::Open => Some(models::HAND_OPEN),
            Hands::Fists => Some(models::FIST),
            Hands::Body => None,
        };
        if let Some(set) = set {
            for (model, joint) in set.into_iter().zip(self.hand_joints) {
                if let Some(at) = joint.and_then(|j| joints.get(j)) {
                    parts.push((model, *at, &self.models[model as usize].bind));
                }
            }
        }
        if let Some(base) = fludd_base {
            parts.push((models::FLUDD, base, &fludd_joints));
        }
        if let Some((model, at, js)) = &nozzle {
            parts.push((*model, *at, js));
        }
        if let Some((model, at, js)) = &boost_part {
            parts.push((*model, *at, js));
        }

        // The corkscrew tips and rolls the whole drawn body about his middle (as `root` does for the effects).
        let draw_pos = pos + rot * (middle - tilt * middle);
        let draw_rot = rot * tilt;
        for (model, root, js) in parts {
            let first_vertex = self.positions.len() as u32;
            let first_color = self.colors.len() as u32;
            let mi = model as usize;
            let mdl = &self.models[mi];
            let pose = Pose::new(&mdl.bmd, root, &js);
            let mirror_rot = |n: Vec3| rot * Vec3::new(-n.x, n.y, n.z);
            for (bi, b) in mdl.batches.iter().enumerate() {
                let hide = (model == models::BODY && hidden.contains(&b.shape))
                    || mdl.bmd.materials[b.material].cull == Cull::All;
                let env = mdl.batch_env[bi];
                for v in &b.vertices {
                    // Hidden shirt/hand shapes collapse to zero; skinning them is wasted work
                    // on every run/jump pose. Keep vertex offsets and UV layout unchanged.
                    if hide {
                        self.positions.push(Vec3::ZERO);
                        self.normals.push(Vec3::Y);
                        self.uvs.push([v.uv[0].x, v.uv[0].y]);
                        continue;
                    }
                    let (p, n) = pose.skin(v);
                    let n = n.normalize_or_zero();
                    // A hidden shape collapses to a point.
                    self.positions.push(p);
                    self.normals.push(n);
                    self.uvs.push(match env {
                        Some(mtx) => {
                            let w = mirror_rot(n);
                            env_uv(w.dot(self.right), w.dot(self.up), mtx)
                        }
                        None => [v.uv[0].x, v.uv[0].y],
                    });
                }
                // The material's texture for this draw (the eyes' blink frame), and colour.
                let mut inputs = base_inputs(model);
                if model == models::BODY
                    && let Some(i) = body_inputs.get(&b.material)
                {
                    inputs.tex = i.tex;
                }
                let tex = if inputs == base_inputs(model) {
                    mdl.batch_texture[bi]
                } else {
                    self.textures.get(mdl, model, b.material, &inputs)
                };
                self.texture_ids.push(tex);
                self.colors.push(mdl.batch_color[bi]);
            }
            self.draws.push(Draw {
                key: (model as u64) << 56,
                model,
                kind: model as u8,
                flags,
                pos: draw_pos,
                rot: draw_rot,
                first_vertex,
                first_color,
                alpha: 1.0,
            });
        }
        if let Some(f) = &sim.mario.fludd {
            let kind = f.nozzle;
            sim.mario.set_emitters(kind, &muzzles);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> Option<View> {
        let dir = libsms::assets::find(None)?;
        match View::load(&dir) {
            Ok(v) => Some(v),
            Err(e) => panic!("loading Mario's models: {e}"),
        }
    }

    fn shine() -> Option<View> {
        let dir = libsms::assets::find(None)?;
        if !dir.join("scene/mapobj/shine.bmd").is_file() {
            eprintln!("skipped: needs the Shine's files (sms_extract_extras)");
            return None;
        }
        Some(View::load_look(&dir, Look::Shine).expect("loading the Shine"))
    }

    #[test]
    fn the_shine_loads_its_model_clips_and_camera() {
        let Some(v) = shine() else { return };
        assert_eq!(v.models.len(), 1);
        let n = v.models[0].vertex_count;
        assert!(n > 0);
        assert_eq!(v.positions.len(), n);
        assert_eq!(v.normals.len(), n);
        assert_eq!(v.uvs.len(), n);
        assert_eq!(v.shine_clip_frames(), Some([79.0, 419.0]));
        assert!(v.demo_camera_frames(false) > 400.0);
        assert!(v.demo_camera(false, 10.0).is_some());
    }

    #[test]
    fn the_shine_floats_up_and_holds_its_pose_with_the_clips() {
        let Some(mut v) = shine() else { return };
        let top = |v: &View| v.positions.iter().map(|p| p.y).fold(f32::MIN, f32::max);
        v.update_shine(0.0, Vec3::ZERO, 0, 0, 0.0, 1.0, false);
        let waiting = top(&v);
        // `shine_float` lifts it about 170 units at its middle, back down at its end.
        v.update_shine(0.0, Vec3::ZERO, 0, 1, 40.0, 1.0, false);
        assert!(top(&v) > waiting + 100.0, "{} -> {}", waiting, top(&v));
        v.update_shine(0.0, Vec3::ZERO, 0, 1, 79.0, 1.0, false);
        assert!((top(&v) - waiting).abs() < 5.0);
        // The demo's clip starts above Mario's head (the model's local origin is his feet).
        v.update_shine(0.0, Vec3::ZERO, 0, 2, 0.0, 1.0, false);
        assert!(top(&v) > 200.0, "{}", top(&v));
    }

    #[test]
    fn the_shine_glows_with_its_disc_particles() {
        let Some(mut v) = shine() else { return };
        assert!(v.fx.has("ms_shine_kira") && v.fx.has("ms_shine_bow"));
        for _ in 0..90 {
            v.update_shine(1.0 / 30.0, Vec3::new(0.0, 100.0, 0.0), 0, 0, 0.0, 1.0, false);
        }
        assert!(!v.fx.vertices.is_empty(), "sparkles and rays");
        // Every vertex is in the world, around where it's placed.
        let far = v.fx.vertices.iter().any(|f| f.pos.distance(Vec3::new(0.0, 170.0, 0.0)) > 3000.0);
        assert!(!far);
    }

    #[test]
    fn blue_coin_loads_its_disc_mesh_in_blue() {
        let Some(dir) = libsms::assets::find(None) else {
            return;
        };
        let v = View::load_look(&dir, Look::BlueCoin).unwrap();
        assert_eq!(v.models.len(), 1);
        assert!(!v.positions.is_empty() && v.positions.iter().all(|p| p.is_finite()));
        assert!(v.models[0].hi.y - v.models[0].lo.y > 90.0);
        assert!(
            (0..v.models[0].batches.len()).any(|b| {
                let c = shown(&v, 0, b);
                c[2] > 0.3 && c[2] > c[0] * 1.5
            }),
            "blue coin baked without its blue colour"
        );
    }

    #[test]
    fn yellow_coin_loads_its_disc_mesh_and_gold_materials_without_marios_models() {
        let Some(dir) = libsms::assets::find(None) else {
            return;
        };
        let v = View::load_look(&dir, Look::Coin).unwrap();
        assert_eq!(v.models.len(), 1);
        assert_eq!(v.positions.len(), v.models[0].vertex_count);
        assert!(!v.positions.is_empty() && v.positions.iter().all(|p| p.is_finite()));
        assert_eq!(v.draws.len(), 1);
        assert!(v.models[0].hi.y - v.models[0].lo.y > 90.0);
        assert!(
            (0..v.models[0].batches.len()).any(|b| {
                let c = shown(&v, 0, b);
                c[0] > 0.4 && c[1] > 0.2 && c[0] > c[2] * 1.5
            }),
            "yellow coin baked without its gold colour"
        );
    }

    #[test]
    fn coin_pickup_sparks_stay_at_collection_point_and_expire() {
        let Some(mut v) = view() else { return };
        let at = Vec3::new(2000.0, 30.0, -4000.0);
        v.coin_get(at);
        let mut saw_sparks = false;
        for tick in (0..200).step_by(4) {
            v.fx.advance(tick);
            v.fx.build(Vec3::X, Vec3::Y);
            saw_sparks |= !v.fx.vertices.is_empty();
            assert!(v.fx.vertices.iter().all(|p| p.pos.distance(at) < 1000.0));
            assert!(
                v.fx.batches
                    .iter()
                    .all(|b| v.fx.texture_world_space(b.texture))
            );
        }
        assert!(saw_sparks);
        assert!(!v.fx.active());
    }

    /// The average colour a model's batch shows: its bake read at each of its vertices'
    /// texture coordinates.
    fn shown(v: &View, model: u32, batch: usize) -> C4 {
        let m = &v.models[model as usize];
        let Ok(t) = usize::try_from(m.batch_texture[batch]) else {
            return m.batch_color_of(batch);
        };
        let t = &v.textures.list[t];
        let mut s = [0.0f32; 4];
        let verts = &m.batches[batch].vertices;
        for vx in verts {
            let x = ((vx.uv[0].x.rem_euclid(1.0) * t.width as f32) as u32).min(t.width - 1);
            let y = ((vx.uv[0].y.rem_euclid(1.0) * t.height as f32) as u32).min(t.height - 1);
            let o = ((y * t.width + x) * 4) as usize;
            for (sc, px) in s.iter_mut().zip(&t.rgba[o..o + 4]) {
                *sc += *px as f32 / 255.0;
            }
        }
        s.map(|c| c / verts.len().max(1) as f32)
    }

    /// The bakes come out the colours Mario is: a red cap, the body's atlas with every colour
    /// in it, and not washed out by the materials' own lighting terms.
    #[test]
    fn bakes_are_marios_colours() {
        let Some(v) = view() else {
            eprintln!("skipping: no assets/");
            return;
        };
        let cap = shown(&v, models::CAP, 0);
        assert!(
            cap[0] > 0.45 && cap[0] > 2.0 * cap[1] && cap[0] > 2.0 * cap[2],
            "cap {cap:?}"
        );
        // The overalls (the body's biggest batch by vertices) are blue somewhere: the body
        // has a batch whose colour is mostly blue.
        let body = &v.models[models::BODY as usize];
        let blue = (0..body.batches.len())
            .map(|b| shown(&v, models::BODY, b))
            .any(|c| c[2] > 0.4 && c[2] > 1.5 * c[0]);
        assert!(blue, "no blue batch on the body");
        for t in &v.textures.list {
            assert_eq!(t.rgba.len(), (t.width * t.height * 4) as usize);
        }
    }

    /// Shadow Mario's look: the red cap comes out in his dark blue, and nothing keeps red.
    #[test]
    fn a_shadow_view_is_shadow_marios_blues() {
        let Some(dir) = libsms::assets::find(None) else {
            eprintln!("skipping: no assets/");
            return;
        };
        let v = View::load_look(&dir, Look::Shadow).unwrap();
        let cap = shown(&v, models::CAP, 0);
        assert!(cap[2] > cap[0] && cap[0] < 0.3, "cap {cap:?}");
        for t in &v.textures.list {
            for p in t.rgba.as_chunks::<4>().0 {
                assert!(p[2] >= p[0], "{p:?}");
            }
        }
    }

    #[test]
    fn a_running_mario_poses_every_part_once() {
        let Some(mut v) = view() else {
            eprintln!("skipping: no assets/");
            return;
        };
        let a = libsms::Assets::discover(None).unwrap();
        let mut sim = Sim::new(a.params, a.anim_table, Vec3::ZERO, 0);
        sim.mario.give_fludd();
        let mut host = libsms::FlatHost::default();
        let pad = libsms::Pad {
            stick: libsms::Vec2::new(0.0, 1.0),
            ..Default::default()
        };
        for _ in 0..30 {
            sim.advance(&mut host, &pad, i16::MIN, 1.0 / 60.0);
            v.update(&mut sim, 1.0 / 60.0);
        }
        let kinds: Vec<u8> = v.draws.iter().map(|d| d.kind).collect();
        assert!(kinds.contains(&(models::BODY as u8)), "{kinds:?}");
        assert!(kinds.contains(&(models::CAP as u8)), "{kinds:?}");
        assert!(kinds.contains(&(models::FLUDD as u8)), "{kinds:?}");
        let total: usize = v
            .draws
            .iter()
            .map(|d| v.models[d.model as usize].vertex_count)
            .sum();
        assert_eq!(v.positions.len(), total);
        assert_eq!(v.uvs.len(), total);
        let batches: usize = v
            .draws
            .iter()
            .map(|d| v.models[d.model as usize].batches.len())
            .sum();
        assert_eq!(v.colors.len(), batches);
        assert_eq!(v.texture_ids.len(), batches);
        // Mario stands about 160 tall: the body's vertices reach well above his feet.
        let body = &v.draws[0];
        let n = v.models[0].vertex_count;
        let top = v.positions[body.first_vertex as usize..][..n]
            .iter()
            .map(|p| p.y)
            .fold(f32::MIN, f32::max);
        assert!((120.0..220.0).contains(&top), "top of Mario at {top}");
        assert!(v.positions.iter().all(|p| p.is_finite()));
    }

    #[test]
    fn a_host_aim_swivels_the_nozzle_head_toward_it() {
        let Some(mut v) = view() else {
            return;
        };
        let a = libsms::Assets::discover(None).unwrap();
        let mut sim = Sim::new(a.params, a.anim_table, Vec3::ZERO, 0);
        sim.mario.give_fludd();
        let mut host = libsms::FlatHost::default();
        // Spraying, so the nozzle is out of its housing.
        let pad = libsms::Pad {
            r_analog: 140,
            ..Default::default()
        };
        let heading = |sim: &Sim| {
            let f = sim.mario.fludd.as_ref().unwrap();
            let (_, d) = f.emitter(0).expect("muzzle reported");
            d.x.atan2(d.z)
        };
        sim.mario.aim = Some(libsms::Aim {
            yaw: sim.mario.model_yaw,
            pitch: 0,
        });
        for _ in 0..60 {
            sim.advance(&mut host, &pad, i16::MIN, 1.0 / 60.0);
            v.update(&mut sim, 1.0 / 60.0);
        }
        let ahead = heading(&sim);
        // A quarter turn to one side of where he faces (the heading increases toward +X). The
        // pose is mirrored across X when drawn, so its own heading swings the other way.
        let yaw = sim.mario.model_yaw;
        sim.mario.aim = Some(libsms::Aim {
            yaw: yaw.wrapping_add(0x4000),
            pitch: 0,
        });
        v.update(&mut sim, 1.0 / 60.0);
        let turned = heading(&sim);
        let swing = (turned - ahead + std::f32::consts::PI).rem_euclid(2.0 * std::f32::consts::PI)
            - std::f32::consts::PI;
        println!("ahead {ahead:.2} turned {turned:.2} swing {swing:.2}");
        assert!(
            (swing + std::f32::consts::FRAC_PI_2).abs() < 0.2,
            "swing {swing}"
        );
    }

    #[test]
    fn a_host_aim_pitches_the_water_the_way_it_points() {
        let Some(mut v) = view() else {
            return;
        };
        let a = libsms::Assets::discover(None).unwrap();
        let mut sim = Sim::new(a.params, a.anim_table, Vec3::ZERO, 0);
        sim.mario.give_fludd();
        let mut host = libsms::FlatHost::default();
        let pad = libsms::Pad {
            r_analog: 140,
            ..Default::default()
        };
        let rise = |sim: &Sim| {
            let f = sim.mario.fludd.as_ref().unwrap();
            let (_, d) = f.emitter(0).expect("muzzle reported");
            d.normalize().y
        };
        let mut at = |sim: &mut Sim, yaw_turn: i16, pitch: f32| {
            let yaw = sim.mario.model_yaw.wrapping_add(yaw_turn);
            let units = (pitch * (32768.0 / std::f32::consts::PI)) as i16;
            sim.mario.aim = Some(libsms::Aim { yaw, pitch: units });
            for _ in 0..30 {
                sim.advance(&mut host, &pad, i16::MIN, 1.0 / 60.0);
                v.update(sim, 1.0 / 60.0);
            }
            rise(sim)
        };
        for turn in [0i16, 0x4000, -0x4000, i16::MIN] {
            // The drawn head points where it is aimed (the water itself is aimed exactly).
            for pitch in [0.6f32, 0.3, 0.0, -0.3, -0.6] {
                let rise = at(&mut sim, turn, pitch);
                println!("turn {turn:#x} pitch {pitch}: head rises {rise:.2}");
                assert!(
                    (rise - pitch.sin()).abs() < 0.12,
                    "aimed {pitch}, head rose {rise} (turn {turn:#x})"
                );
            }
        }
    }

    #[test]
    fn pound_impacts_stay_at_landing_when_mario_rebounds() {
        let Some(mut v) = view() else {
            return;
        };
        let a = libsms::Assets::discover(None).unwrap();
        let mut sim = Sim::new(a.params, a.anim_table, Vec3::ZERO, 0);
        let landing = Vec3::new(120.0, 0.0, -80.0);
        let names = [
            "ms_hipdrop_a",
            "ms_hipdrop_b",
            "ms_hipdrop_c",
            "ms_m_sphipd_hit_a",
            "ms_m_sphipd_hit_b",
            "ms_m_sphipd_hit_c",
            "ms_m_sphipd_hit_d",
        ];
        let cues: Vec<_> = names
            .iter()
            .map(|&name| {
                Event::Particle(ParticleCue {
                    name,
                    at: FxAt::Once {
                        pos: landing,
                        yaw: 0,
                    },
                    scale: 1.0,
                })
            })
            .collect();
        v.events(&cues, &sim.mario);
        v.update(&mut sim, 0.0);
        assert!(!v.fx.batches.is_empty());
        // This metadata is what makes the host omit Mario's smoothed draw offset.
        assert!(
            v.fx.batches
                .iter()
                .all(|b| v.fx.texture_world_space(b.texture)),
            "a landing effect would receive Mario's bounce draw offset"
        );
        let at_landing: Vec<_> = v.fx.vertices.iter().map(|v| v.pos).collect();
        sim.mario.pos = landing + Vec3::new(300.0, 800.0, -200.0);
        v.update(&mut sim, 0.0);
        let at_rebound: Vec<_> = v.fx.vertices.iter().map(|v| v.pos).collect();
        assert_eq!(at_landing, at_rebound, "rebound moved the landing effects");
    }

    /// The nozzle's effects are on the muzzle FLUDD's drawn nozzle has, and the Rocket's exhaust
    /// stays with it as Mario moves.
    #[test]
    fn nozzle_effects_sit_on_the_muzzle_and_the_rockets_exhaust_follows_it() {
        let Some(mut v) = view() else {
            return;
        };
        let a = libsms::Assets::discover(None).unwrap();
        let mut sim = Sim::new(a.params, a.anim_table, Vec3::ZERO, 0);
        sim.mario.give_fludd();
        let tick = |sim: &mut Sim| sim.mario.ticks;
        let cue = |name, at| Event::Particle(ParticleCue { name, at, scale: 1.0 });
        let first = tick(&mut sim);
        for i in 0..30 {
            v.events(&[cue("ms_nozzlesmoke", FxAt::Muzzle(0))], &sim.mario);
            sim.mario.ticks = first + i * 4;
            v.update(&mut sim, 0.0);
        }
        assert!(!v.fx.vertices.is_empty(), "no nozzle smoke was drawn");
        // On FLUDD's nozzle: at Mario's chest, not at the world's origin or his feet.
        let high = v.fx.vertices.iter().filter(|p| p.pos.y > 40.0 && p.pos.distance(sim.mario.pos) < 300.0).count();
        assert!(high > 0, "the smoke isn't on the nozzle");
        // The exhaust: started once, then it goes where the muzzle goes.
        v.events(&[cue("ms_m_rocket_b", FxAt::MuzzleFollow(0))], &sim.mario);
        sim.mario.ticks += 4;
        v.update(&mut sim, 0.0);
        let before = v.fx.vertices.iter().map(|p| p.pos.y).fold(f32::MIN, f32::max);
        sim.mario.pos += Vec3::Y * 600.0;
        sim.mario.prev_pos = sim.mario.pos;
        for _ in 0..4 {
            v.events(&[], &sim.mario);
            sim.mario.ticks += 4;
            v.update(&mut sim, 0.0);
        }
        let after = v.fx.vertices.iter().map(|p| p.pos.y).fold(f32::MIN, f32::max);
        assert!(after > before + 300.0, "the exhaust stayed behind: {before} -> {after}");
    }

    #[test]
    fn movement_and_flames_switch_independently_without_advancing_time() {
        let Some(mut v) = view() else {
            return;
        };
        let a = libsms::Assets::discover(None).unwrap();
        let mut sim = Sim::new(a.params, a.anim_table, Vec3::ZERO, 0);
        v.fx.emit("ms_dmg_c", Mat4::IDENTITY, 1.0);
        v.fx.emit("ms_m_sphipd_hit_b", Mat4::IDENTITY, 1.0);
        v.update(&mut sim, 0.0);
        let all = v.fx.vertices.len();
        v.pound_flames = false;
        v.update(&mut sim, 0.0);
        let movement = v.fx.vertices.len();
        v.pound_flames = true;
        v.movement_effects = false;
        v.update(&mut sim, 0.0);
        let flames = v.fx.vertices.len();
        assert!(movement > 0 && flames > 0);
        assert_eq!(all, movement + flames);
        v.pound_flames = false;
        v.update(&mut sim, 0.0);
        assert!(v.fx.vertices.is_empty());
        v.movement_effects = true;
        v.pound_flames = true;
        v.update(&mut sim, 0.0);
        assert_eq!(v.fx.vertices.len(), all);
    }

    /// Collects the sim's events over flat ground, for stepping a launch.
    struct Collect(Vec<Event>);

    impl libsms::Host for Collect {
        fn ground(&self, p: Vec3) -> Option<libsms::GroundHit> {
            libsms::Host::ground(&libsms::FlatHost::default(), p)
        }
        fn event(&mut self, e: Event) {
            self.0.push(e);
        }
    }

    /// The Rocket's exhaust is bound to the muzzle's position only (`emitAndBindToPosPtr`), so it
    /// comes out of the nozzle on his back and trails below as he climbs; bound with the muzzle's
    /// rotation as well it pooled about his middle.
    #[test]
    fn the_rockets_exhaust_leaves_the_nozzle_on_his_back_as_he_climbs() {
        let dir = libsms::assets::find(None).expect("assets not found (set SMS_ASSETS)");
        let a = libsms::Assets::discover(None).unwrap();
        for only in ["ms_m_rocket_a", "ms_m_rocket_b", "ms_m_rocket_b2"] {
            let mut v = View::load(&dir).unwrap();
            let mut sim = Sim::new(a.params.clone(), a.anim_table.clone(), Vec3::ZERO, 0);
            sim.mario.give_fludd();
            {
                let p = sim.mario.params.clone();
                let f = sim.mario.fludd.as_mut().unwrap();
                f.change_nozzle(libsms::NozzleKind::Rocket, false, &p);
                f.refill(&p);
            }
            let mut host = Collect(Vec::new());
            let pad = libsms::Pad { r: true, r_analog: 255, ..Default::default() };
            let mut sampled = false;
            for tick in 0..205 {
                sim.mario.tick(&mut host, &pad, 0);
                let events: Vec<Event> = std::mem::take(&mut host.0)
                    .into_iter()
                    .filter(|e| matches!(e, Event::Particle(c) if c.name == only))
                    .collect();
                v.events(&events, &sim.mario);
                if tick % 4 == 3 {
                    v.update(&mut sim, 0.0);
                    // A moment into the climb.
                    if tick == 195 {
                        let m = sim.mario.pos;
                        let n = v.fx.vertices.len() as f32;
                        assert!(n > 0.0, "{only} drew nothing");
                        let z = v.fx.vertices.iter().map(|p| p.pos.z - m.z).sum::<f32>() / n;
                        assert!(z < -30.0, "{only} is about his middle, not at the nozzle behind him: z {z:.0}");
                        let high = v.fx.vertices.iter().map(|p| p.pos.y - m.y).fold(f32::MIN, f32::max);
                        assert!(high < 110.0, "{only} reaches up his body: {high:.0}");
                        sampled = true;
                    }
                }
            }
            assert!(sampled);
        }
    }

    #[test]
    fn the_turbo_dash_draws_the_water_boost_cone_and_its_particles() {
        let dir = libsms::assets::find(None).expect("assets not found (set SMS_ASSETS)");
        let mut v = View::load(&dir).unwrap();
        assert!(v.models.len() > models::WATERBOOST as usize, "the cone didn't load");
        let a = libsms::Assets::discover(None).unwrap();
        let mut sim = Sim::new(a.params.clone(), a.anim_table.clone(), Vec3::ZERO, 0);
        sim.mario.give_fludd();
        {
            let p = sim.mario.params.clone();
            let f = sim.mario.fludd.as_mut().unwrap();
            f.change_nozzle(libsms::NozzleKind::Turbo, false, &p);
            f.refill(&p);
        }
        let mut host = Collect(Vec::new());
        let mut pad = libsms::Pad { r_analog: 255, ..Default::default() };
        pad.stick = libsms::Vec2::new(0.0, 1.0);
        let mut cone_ticks = 0;
        let mut boost_cues = 0;
        let mut after_release = 0;
        for tick in 0..120 * 12 {
            if tick == 120 * 6 {
                pad.r_analog = 0;
            }
            sim.mario.tick(&mut host, &pad, 0);
            let events = std::mem::take(&mut host.0);
            boost_cues += events.iter().filter(|e| matches!(e, Event::Particle(c) if c.name == "ms_watboost_a")).count();
            v.events(&events, &sim.mario);
            if tick % 4 == 3 {
                v.update(&mut sim, 0.0);
                let cone = v.draws.iter().find(|d| d.model == models::WATERBOOST);
                if let Some(d) = cone {
                    cone_ticks += 1;
                    if cone_ticks == 20 {
                        let (lo, hi) = (d.first_vertex as usize, v.models[models::WATERBOOST as usize].vertex_count);
                        let pts = &v.positions[lo..lo + hi];
                        let n = pts.len() as f32;
                        let mean = pts.iter().fold(Vec3::ZERO, |s, p| s + *p) / n;
                        // Out the back of his pack (model space: -z is behind him), at the nozzle's height.
                        assert!(mean.z < -80.0, "the cone isn't behind him: {mean:?}");
                        assert!(mean.y > 40.0 && mean.y < 160.0, "the cone isn't at the nozzle's height: {mean:?}");
                    }
                    if tick > 120 * 6 + 600 {
                        after_release += 1;
                    }
                }
            }
        }
        assert!(cone_ticks > 30, "the cone was drawn on only {cone_ticks} updates");
        assert!(boost_cues > 100, "the dash asked for {boost_cues} watboost cues");
        assert_eq!(after_release, 0, "the cone stays after the dash ends");
        assert!(!v.fx.vertices.is_empty() || boost_cues > 0);
    }

    /// The water-boost cone's layers are alpha blended and fade along its length by their vertex
    /// alpha (the texture's alpha is solid), so hosts must get them as translucent, through the
    /// see-through path, not as an opaque shell.
    #[test]
    fn the_water_boost_cone_is_translucent_not_an_opaque_shell() {
        let dir = libsms::assets::find(None).expect("assets not found (set SMS_ASSETS)");
        let v = View::load(&dir).unwrap();
        let m = &v.models[models::WATERBOOST as usize];
        for (i, b) in m.batches.iter().enumerate() {
            assert_ne!(m.batch_blend[i], blend::OPAQUE, "{} is an opaque shell", m.bmd.materials[b.material].name);
        }
        // Their fade is per vertex, flagged for the host, and the bake carries none of it.
        let faded = (0..m.batches.len()).filter(|&i| m.batch_lit[i] & 4 != 0).count();
        assert!(faded >= 2, "the faded layers don't carry their vertex alpha");
        assert_eq!(m.vertex_alpha.len(), m.vertex_count);
        let (lo, hi) = m.vertex_alpha.iter().fold((1.0f32, 0.0f32), |(l, h), &a| (l.min(a), h.max(a)));
        assert!(lo < 0.1 && hi > 0.9, "the vertex alpha doesn't run from clear to solid: {lo}..{hi}");
        for i in (0..m.batches.len()).filter(|&i| m.batch_lit[i] & 4 != 0) {
            assert_eq!(m.batch_color_of(i), [1.0; 4], "the mean alpha is applied twice");
        }
    }

    /// The cone's clips play in real time (`setRate(SMSGetAnmFrameRate())`, two 60 fps frames a
    /// game frame): the `_out` clip's 21 frames take eleven game frames, not twenty-one.
    #[test]
    fn the_water_boost_cone_plays_its_clips_in_real_time() {
        let dir = libsms::assets::find(None).expect("assets not found (set SMS_ASSETS)");
        let v = View::load(&dir).unwrap();
        let mut b = v.boost.expect("the cone's files are there");
        b.step(true);
        assert_eq!(b.state, 1);
        b.step(false);
        assert_eq!(b.state, 2);
        let mut steps = 0;
        while b.state == 2 {
            b.step(false);
            steps += 1;
        }
        assert_eq!(steps, 11, "the cone fades out over {steps} game frames");
    }

    #[test]
    fn a_spin_jumps_nozzle_turns_with_him_whatever_the_host_aims() {
        let Some(mut v) = view() else {
            return;
        };
        let a = libsms::Assets::discover(None).unwrap();
        let mut sim = Sim::new(a.params, a.anim_table, Vec3::ZERO, 0);
        sim.mario.give_fludd();
        // A flat floor that keeps the events it is told.
        #[derive(Default)]
        struct Rec {
            flat: libsms::FlatHost,
            events: Vec<libsms::Event>,
        }
        impl libsms::Host for Rec {
            fn ground(&self, pos: Vec3) -> Option<libsms::host::GroundHit> {
                self.flat.ground(pos)
            }
            fn event(&mut self, e: libsms::Event) {
                self.events.push(e);
            }
        }
        let mut host = Rec::default();
        let mut stick_turn = 0.0f32;
        let mut pad = libsms::Pad::default();
        // Two quick turns of the stick, then A: a spin jump.
        for _ in 0..24 {
            stick_turn += std::f32::consts::TAU / 12.0;
            pad.stick = glam::Vec2::new(stick_turn.cos(), stick_turn.sin());
            sim.advance(&mut host, &pad, i16::MIN, 1.0 / 60.0);
            v.update(&mut sim, 1.0 / 60.0);
        }
        pad.a = true;
        sim.advance(&mut host, &pad, i16::MIN, 1.0 / 60.0);
        pad.a = false;
        assert!(matches!(
            sim.mario.status,
            libsms::Status::LeftRotateJump | libsms::Status::RightRotateJump
        ));
        // The host aims dead ahead the whole time, as the crosshair does.
        let yaw = sim.mario.model_yaw;
        sim.mario.aim = Some(libsms::Aim { yaw, pitch: 0 });
        pad.stick = glam::Vec2::ZERO;
        pad.r_analog = 140;
        host.events.clear();
        let mut headings = Vec::new();
        for _ in 0..40 {
            sim.advance(&mut host, &pad, i16::MIN, 1.0 / 60.0);
            v.update(&mut sim, 1.0 / 60.0);
            for e in host.events.drain(..) {
                if let libsms::Event::WaterEmit(w) = e {
                    headings.push(w.dir.x.atan2(w.dir.z));
                }
            }
        }
        // The water leaves in the world along the posed nozzle, which turns with the spin: not all
        // along the host's fixed aim.
        let (lo, hi) = (
            headings.iter().cloned().fold(f32::MAX, f32::min),
            headings.iter().cloned().fold(f32::MIN, f32::max),
        );
        assert!(headings.len() > 10 && hi - lo > 1.5, "the spin's water only covered {:.2} rad", hi - lo);
    }

    #[test]
    fn a_ground_spins_water_turns_round_in_the_world_whatever_the_host_aims() {
        let Some(mut v) = view() else {
            return;
        };
        let a = libsms::Assets::discover(None).unwrap();
        let mut sim = Sim::new(a.params, a.anim_table, Vec3::ZERO, 0);
        sim.mario.give_fludd();
        #[derive(Default)]
        struct Rec {
            flat: libsms::FlatHost,
            events: Vec<libsms::Event>,
        }
        impl libsms::Host for Rec {
            fn ground(&self, pos: Vec3) -> Option<libsms::host::GroundHit> {
                self.flat.ground(pos)
            }
            fn event(&mut self, e: libsms::Event) {
                self.events.push(e);
            }
        }
        let mut host = Rec::default();
        let mut pad = libsms::Pad::default();
        // Spraying, with the host's aim dead ahead (the crosshair), then two turns of the stick.
        let yaw = sim.mario.model_yaw;
        sim.mario.aim = Some(libsms::Aim { yaw, pitch: 0 });
        pad.r_analog = 200;
        for _ in 0..10 {
            sim.advance(&mut host, &pad, i16::MIN, 1.0 / 60.0);
            v.update(&mut sim, 1.0 / 60.0);
        }
        // The ground spin, which a stick circle starts from a turn or a stop: he is put in it.
        sim.mario.status = libsms::Status::RotateL;
        host.events.clear();
        let mut headings = Vec::new();
        for _ in 0..40 {
            sim.advance(&mut host, &pad, i16::MIN, 1.0 / 60.0);
            v.update(&mut sim, 1.0 / 60.0);
            for e in host.events.drain(..) {
                if let libsms::Event::WaterEmit(w) = e {
                    headings.push(w.dir.x.atan2(w.dir.z));
                }
            }
        }
        let (lo, hi) = (
            headings.iter().cloned().fold(f32::MAX, f32::min),
            headings.iter().cloned().fold(f32::MIN, f32::max),
        );
        assert!(headings.len() > 10 && hi - lo > 1.5, "the ground spin's water only covered {:.2} rad", hi - lo);
    }

    #[test]
    fn a_running_marios_stream_starts_at_his_nozzle_not_behind_it() {
        let Some(mut v) = view() else {
            return;
        };
        let a = libsms::Assets::discover(None).unwrap();
        let params = a.params.clone();
        #[derive(Default)]
        struct Rec {
            flat: libsms::FlatHost,
            events: Vec<libsms::Event>,
        }
        impl libsms::Host for Rec {
            fn ground(&self, pos: Vec3) -> Option<libsms::host::GroundHit> {
                self.flat.ground(pos)
            }
            fn event(&mut self, e: libsms::Event) {
                self.events.push(e);
            }
        }
        // Standing or running forward while spraying, the display `lag` ticks behind the sim, with
        // the host telling WaterLook where the nozzle is drawn (`with_nozzle`) or not.
        for (label, stick, lag, with_nozzle) in [
            ("standing", 0.0f32, 1.0f32, true),
            ("running", 1.0, 1.0, true),
            ("running, nozzle known, lag 2", 1.0, 2.0, true),
            ("running, nozzle unknown", 1.0, 1.0, false),
        ] {
            let mut sim = Sim::new(a.params.clone(), a.anim_table.clone(), Vec3::ZERO, 0);
            sim.mario.give_fludd();
            let mut host = Rec::default();
            let mut water = libsms::water::WaterManager::new(&params);
            let mut look = crate::water::WaterLook::default();
            let mut ticks = 0u32;
            let pad = libsms::Pad { stick: glam::Vec2::new(0.0, stick), r_analog: 140, ..Default::default() };
            let (mut behind, mut n_frames) = (0.0f32, 0u32);
            let mut worst = 0.0f32;
            let mut speed = 0.0f32;
            for frame in 0..200 {
                let n = sim.advance(&mut host, &pad, i16::MIN, 1.0 / 60.0);
                let mut muzzle = None;
                for e in host.events.drain(..) {
                    if let libsms::Event::WaterEmit(w) = e {
                        muzzle = Some((w.pos, w.dir));
                        water.emit(&w);
                    }
                }
                ticks += n;
                while ticks >= 4 {
                    ticks -= 4;
                    water.step(&host, sim.mario.pos);
                    water.events.clear();
                }
                v.update(&mut sim, 1.0 / 60.0);
                let Some((m, d)) = muzzle else { continue };
                if frame < 60 {
                    continue;
                }
                let vel = Vec3::new(sim.mario.vel.x, 0.0, sim.mario.vel.z);
                speed = vel.length();
                // Mario and his nozzle are drawn `lag` ticks behind the sim.
                let drawn = m - vel * lag * 0.25;
                let frac = (ticks as f32 + sim.alpha() - lag) / 4.0;
                let eye = sim.mario.pos + Vec3::new(500.0, 150.0, 0.0);
                look.nozzle = if with_nozzle { Some(drawn) } else { None };
                look.build(&water, frac, eye, Vec3::Z, Vec3::Y, 0);
                let d = d.normalize();
                let mut least = f32::MAX;
                // Only the newest drops: those still within 6 frames of the full lifetime.
                let alive = water.particles.iter().map(|p| p.life).fold(0.0f32, f32::max);
                let mut quad = 0usize;
                for p in water.particles.iter().take(libsms::water::SLOTS) {
                    if p.state != libsms::water::State::Flying || !water.drawn(p) { continue; }
                    if p.life >= alive - 6.0 {
                        for q in &look.vertices[quad * 4..quad * 4 + 4] {
                            least = least.min((q.pos - drawn).dot(d));
                        }
                    }
                    quad += 1;
                }
                if least != f32::MAX {
                    behind += least;
                    worst = worst.min(least);
                    n_frames += 1;
                }
            }
            let _ = (behind, speed);
            assert!(n_frames > 50, "{label}: no stream was drawn");
            if with_nozzle {
                // No part of the newest drops' streaks reaches back behind the nozzle as drawn.
                assert!(worst > -20.0, "{label}: the stream starts {:.0} units behind the nozzle", -worst);
            } else {
                // Without it, a runner leaves the water behind: the streaks reach well back.
                assert!(label != "running, nozzle unknown" || worst < -50.0, "{label}: {worst}");
            }
        }
    }
}
