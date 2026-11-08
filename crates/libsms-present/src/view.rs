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
use std::path::{Path, PathBuf};

use glam::{Mat4, Quat, Vec3};
use libsms::{Clip, ClipLibrary, Event, FxAt, Mario, NozzleKind, ParticleCue, Sim, Skeleton};
use libsms_assets::bmd::{Blend, Cull};
use libsms_assets::btp::TexPattern;
use libsms_assets::mario::{BODY_HAND_SHAPES, HIDDEN_SHAPE, Hands, SHIRT_SHAPE, hands_for_clip};
use libsms_assets::{Batch, Bmd, Pose, paths};

use crate::hud::{Gauge, Hud};

/// Clips with a face pattern of their own, in place of the blink: the burning Mario's hot face
/// (`gMarioAnimeData`'s pattern index per clip, `marioAnimeTexPatternFilenames`, MarioDraw.cpp:
/// `hot_wait` 8, `firejmp` 9, `fjpend` 10).
const FACE_PATTERNS: [(&str, &str); 3] = [
    ("hot_wait", "ma_hot_wait_tx.btp"),
    ("firejmp", "ma_firejmp_tx.btp"),
    ("fjpend", "ma_fjpend_tx.btp"),
];

/// Piantissimo's own colours, the two most common in his texture (`monteman_model.bmd`): the
/// purply pink of his mask and the brown of his skin. They recolour Mario's smears and the super
/// pound's fire: red to pink, and the blue half brown (the primary colour) and half green (the
/// mixing colour).
pub const PIANTISSIMO_PINK: [u8; 3] = [213, 106, 188];
pub const PIANTISSIMO_BROWN: [u8; 3] = [140, 99, 40];
/// The green of his leaf (`monteman_model.bmd`'s atlas runs from (3, 147, 1) to (180, 209, 67)).
pub const PIANTISSIMO_GREEN: [u8; 3] = [24, 176, 20];

/// Piantissimo's model has his face painted into one atlas, with no pattern to swap, so a burning
/// Piantissimo flushes: his colours times this (red up, green and blue down).
const PIANTISSIMO_HOT_FLUSH: [f32; 3] = [1.0, 0.62, 0.55];

/// A face pattern and the body materials its tracks name (`usize::MAX`: not in this body).
struct FacePattern {
    clip: &'static str,
    pat: TexPattern,
    mats: Vec<usize>,
}
use crate::coins::{CoinMeter, Kind as CoinKind};
use crate::democam::DemoCamera;
use crate::life::LifeMeter;
use crate::screenfx::ScreenFx;
use crate::particles::{GreenRule, Particles};
use crate::tev::{Baked, C4, Inputs, Source, Tex, env_uv};

#[path = "entrance.rs"]
mod entrance;
pub use entrance::{Entrance, entrance_fall, entrance_pose, entrance_fast_pose, ENTRANCE_RETURN};

#[path = "luigi_skin.rs"]
mod luigi_skin;
pub use luigi_skin::{HsdBatch, Luigi, LUIGI_HANG_OFFSET, LUIGI_ROD_SHAPE, extract_melee_files as extract_luigi_melee_files, melee_files_complete as luigi_melee_files_complete};

#[path = "yoshi_skin.rs"]
mod yoshi_skin;
pub use yoshi_skin::Yoshi;

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

    pub(crate) fn source(&self) -> Source<'_> {
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
    /// Sambo flowers (hanasambo.cpp), each a posed prop like [`Look::Shine`] without the glow:
    /// `samboflower/flower.bmd` (the plain flower), `sambo/sambo.bmd` (the tower) and
    /// `sambohead/sambohead.bmd` (the hopping head), all under `enemy/`. A host places one
    /// instance per view and poses it with [`View::update_prop`].
    Flower,
    Sambo,
    SamboHead,
    /// No body to draw: the sambo flowers' particle effects (pollen, rock and smoke bursts,
    /// vanishing puffs), emitted by name with [`View::emit_effect`] and stepped by
    /// [`View::step_effects`]; the quads come from the usual `fx` readers.
    Effects,
    /// Bowser Jr. (`TLimitKoopaJr`, limitkoopajr.cpp): `koopajr/koopajr_model.bmd` and his four
    /// clips (`damage`, `shoot`, `wait`, `yahoo`), posed like the flowers ([`View::update_prop`]);
    /// [`libsms::koopajr::KoopaJr::jr_clip`] says which.
    KoopaJr,
    /// The Swoopin' Stu (`TNameKuri`, namekuri.cpp): `namekuri2/name.bmd` and its clips. Its
    /// body shows goop through the dirt mask at its own konst alpha.
    NameKuri,
    /// The brain a Stu wears (`namekuri2/brain.bmd`), drawn on the Stu's top joint by the host.
    NameKuriBrain,
    /// The goop Piranha (`TPakkun`, pakkun.cpp): `pakkun/pakun.bmd` and its clips.
    Pakkun,
    /// The Piranha's goop ball / seed (`pakkun/seed.bmd`), static: the shot and the seed.
    PakkunSeed,
    /// Bowser Jr.'s sign that Shadow Mario paints (`stamp_koopa_sign_model1.bmd` with its
    /// `_draw1` clip; enemyMario.cpp draws it at three times his scale, turned half round).
    KoopaSign,
    /// The big goop Piranha Plant (`TBiancoGateKeeper`, gatekeeper.cpp):
    /// `gatekeeper/gene_pakkun_model1.bmd` and its `gene_pakkun_*` clips, the goop mask in place
    /// of its `Q_kepper_dummy_128IA4` (`SMS_ChangeTextureAll` in `init`).
    GateKeeper,
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

/// The sambo flowers' effects (`THanaSambo`, `TSamboFlower`, `TNerveHanaSamboDie`): pollen when a
/// flower opens (`ms_smb_kafun_ow`) or a tower bites (`_oa`), the rock and smoke of a tower
/// rising (`ms_smb_ap_*`) and sinking (`ms_smb_hd_*`), the puffs of one dying (`ms_enm_disap_*`).
pub const FLOWER_EFFECTS: [&str; 8] = [
    "ms_smb_kafun_ow",
    "ms_smb_kafun_oa",
    "ms_smb_ap_rock",
    "ms_smb_ap_smoke",
    "ms_smb_hd_rock",
    "ms_smb_hd_smoke",
    "ms_enm_disap_a",
    "ms_enm_disap_b",
];

/// The goop's, its critters' and Bowser Jr.'s effects, with the folder each loads from (the
/// scene's and the enemies' own `jpa/` folders, else the common `particle/`): the goop's
/// burning (`ms_newfire_*`, pollution.cpp) and electric (`ms_thunder_s`) layers, a cleaned
/// patch's sparkle (`ms_raku_indawa`, `ms_raku_kie`), Swoopin' Stu's and the goop Piranha's
/// deaths (`ms_deadnamekli_*`, `ms_packn_hd_smoke`), their water hits (`ms_enm_wathit`), Shadow
/// Mario's change (`ms_kgm_change`), Bowser Jr.'s blast (`ms_koopajr_killer`) and the
/// Gatekeeper's smoke, drool and splats (`ms_gkpa_*`, gatekeeper.cpp).
pub const GOOP_EFFECTS: [(&str, &str); 20] = [
    ("ms_newfire_a", "scene/map/pollution"),
    ("ms_newfire_b", "scene/map/pollution"),
    ("ms_thunder_s", "particle"),
    ("ms_raku_indawa", "particle"),
    ("ms_raku_kie", "particle"),
    ("ms_deadnamekli_n", "particle"),
    ("ms_deadnamekli_o", "particle"),
    ("ms_packn_hd_smoke", "particle"),
    ("ms_enm_wathit", "particle"),
    ("ms_kgm_change", "enemy/kagemario/jpa"),
    ("ms_koopajr_killer", "enemy/koopajr/jpa"),
    ("ms_gkpa_kemuri", "enemy/gatekeeper/jpa"),
    ("ms_gkpa_kemuri_w", "enemy/gatekeeper/jpa"),
    ("ms_gkpa_bota", "enemy/gatekeeper/jpa"),
    ("ms_gkpa_yodare_l", "enemy/gatekeeper/jpa"),
    ("ms_gkpa_yodare_s", "enemy/gatekeeper/jpa"),
    ("ms_gkpa_dead", "enemy/gatekeeper/jpa"),
    ("ms_gkpa_deadsmoke", "enemy/gatekeeper/jpa"),
    ("ms_smb_hd_smoke", "particle"),
    ("ms_smb_ap_smoke", "particle"),
];

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

/// Which body the view draws. The ids (0, 1, 2) are the host ABI's (`sms_view_skin`); a new
/// skin appends one. A skin's body is one of two kinds ([`SkinBody`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Skin {
    #[default]
    Mario,
    /// Luigi's HSD mesh (Melee assets), posed by the same Sunshine clips through the retarget,
    /// with his own FLUDD.
    Luigi,
    /// Piantissimo's `monteman_model.bmd`: Mario's 29 joints in Mario's order, so Mario's own
    /// pose drives it as it does in the game (`TEnemyMario`, enemyMario.cpp), with Mario's FLUDD.
    Piantissimo,
    /// Sunshine Yoshi (`yoshi.szs:yoshi_model.bmd`, 38 joints, `TYoshi::init`): his own
    /// wait/walk/run and flutter clips on his own rig, Mario's air clips retargeted onto it,
    /// with FLUDD. The skin's flutter is [`libsms::Options::yoshi_skin`].
    Yoshi,
    /// Shadow Mario (`TEMario`, emario.cpp): `kagemario/default.bmd` on Mario's 29 joints, its
    /// swirling material baked each game frame ([`crate::shadow_skin`]) and the paintbrush in
    /// his left hand. Bowser Jr.'s Shadow form (`libsms::koopajr`); he has no FLUDD.
    Shadow,
}

pub use libsms_audio::VoiceSet;

/// How a skin gets its body, so adding one is a row and not a code path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkinBody {
    /// Mario's own J3D body.
    Mario,
    /// A J3D model on Mario's exact skeleton (same joints, same parents): it is drawn instead
    /// of `ma_mdl1.bmd` with Mario's joint matrices, no retarget and no cap or hands (the
    /// model carries them). The file is under the Sunshine assets folder.
    SameRig(&'static str),
    /// A model from the Melee cache posed through the retarget ([`Luigi`]).
    Hsd,
    /// Yoshi's J3D body on his own 38-joint rig: his own clips plus Mario's retargeted.
    Yoshi,
}

impl Skin {
    /// Every skin, in ABI order.
    pub const ALL: [Skin; 5] = [Skin::Mario, Skin::Luigi, Skin::Piantissimo, Skin::Yoshi, Skin::Shadow];

    pub const fn id(self) -> u32 {
        self as u32
    }

    /// The skin for a host's id; anything unknown is Mario.
    pub fn from_id(id: u32) -> Skin {
        Skin::ALL.get(id as usize).copied().unwrap_or(Skin::Mario)
    }

    /// Whose voice the skin speaks with: Piantissimo's jump voice replaces Mario's jump cues
    /// (`MSound::startMarioVoice`, voice status 6); the rest keep Mario's lines.
    pub const fn voice_set(self) -> VoiceSet {
        match self {
            Skin::Piantissimo => VoiceSet::Piantissimo,
            // `TMario::getVoiceStatus`: PLAYER_TYPE_SHADOW_MARIO answers 2.
            Skin::Shadow => VoiceSet::Shadow,
            _ => VoiceSet::Mario,
        }
    }

    pub const fn body(self) -> SkinBody {
        match self {
            Skin::Mario => SkinBody::Mario,
            Skin::Luigi => SkinBody::Hsd,
            // `OPTIONAL_EXTRACTS`: mamma4.szs:map/map/pad/monteman_model.bmd.
            Skin::Piantissimo => SkinBody::SameRig("npc/monteman/monteman_model.bmd"),
            Skin::Yoshi => SkinBody::Yoshi,
            Skin::Shadow => SkinBody::SameRig(crate::shadow_skin::MODEL),
        }
    }
}

/// Texture-cache keys for same-rig bodies sit above any model index.
pub(crate) const RIG_KEY_BASE: u32 = 0x1000;
/// Model key of the diving helmet (not a rig body: its konst 0 alpha is the glass's, not dirt).
const HELMET_KEY: u32 = 0x2000;
/// Model key of Shadow Mario's brush (its konst 0 alpha is its own, not Mario's dirt).
const BRUSH_KEY: u32 = 0x2001;
/// Mario's dirt mask (`cDirtyFileName`, `/scene/map/pollution/H_ma_rak.bti`): the game puts it
/// in place of every `H_ma_rak_dummy` (`SMS_ChangeTextureAll`, `TMario::initModel` and
/// `TEnemyMario::init`), and konst 0's alpha (`TMario::addDirty`) blends it in.
const DIRT_TEXTURE: &str = "scene/map/pollution/h_ma_rak.bti";
const DIRT_DUMMY: &str = "H_ma_rak_dummy";
/// The Gatekeeper's stand-in for the same mask (`TBiancoGateKeeper::init`).
const GATEKEEPER_DUMMY: &str = "Q_kepper_dummy_128IA4";

/// The dirt mask from the disc, if extracted.
pub(crate) fn dirt_texture(dir: &Path) -> Option<Tex> {
    let data = std::fs::read(dir.join(DIRT_TEXTURE)).ok()?;
    let t = libsms_assets::texture::read_bti(&data, DIRT_DUMMY.into()).ok()?;
    Some(Tex::new(t.image, t.wrap_s, t.wrap_t))
}

/// `replace` for [`build`]: the dirt mask where the model has the dummy.
pub(crate) fn swap_dirt(name: &str, dirt: &Option<Tex>) -> Option<Tex> {
    (name == DIRT_DUMMY || name == GATEKEEPER_DUMMY).then(|| dirt.clone()).flatten()
}

/// Dirt levels baked (konst 0 alpha steps): each level is one more texture per material.
const DIRT_LEVELS: f32 = 5.0;

/// [`libsms::Mario::dirt_share`] as the konst alpha a bake uses, in [`DIRT_LEVELS`] steps.
fn dirt_alpha(share: f32) -> u8 {
    ((share.clamp(0.0, 1.0) * DIRT_LEVELS).round() / DIRT_LEVELS * 255.0).round() as u8
}

pub struct View {
    pub models: Vec<Model>,
    pub textures: Textures,
    /// Whose body the body draws are: Mario's own, or an alt-skin (Luigi's HSD mesh posed by
    /// the same Sunshine clips and wearing his own FLUDD). Set by the host from the skin it
    /// is drawing; the rest of the view (clips, FLUDD look, effects) is shared.
    pub skin: Skin,
    /// A host-supplied body pose (one model-space matrix per Sunshine joint).
    /// The Mario A-attack port uses it to draw Melee Mario's clip on this rig;
    /// `None` poses from the sim as usual.
    pub pose_override: Option<Vec<Mat4>>,
    skeleton: Skeleton,
    clips: ClipLibrary,
    fludd_clips: HashMap<String, Clip>,
    boost: Option<Boost>,
    wink: Option<TexPattern>,
    eye_materials: Vec<usize>,
    /// The face a clip wears instead of the blink (`gMarioAnimeData`'s pattern per clip).
    faces: Vec<FacePattern>,
    hand_joints: [Option<usize>; 2],
    head: Option<usize>,
    chest: Option<usize>,
    center: Option<usize>,
    /// The game's particle effects on Mario: dust, stars, smears, spray and pound flames.
    pub fx: Particles,
    shine: Option<Shine>,
    /// A prop view's clips by lower-case name ([`Look::Flower`] and its kin).
    props: Option<Vec<(String, Clip)>>,
    return_parts: Option<[u32; 2]>,
    /// Movement effects and super-pound flames have independent host switches.
    pub movement_effects: bool,
    pub pound_flames: bool,
    fx_ticks: Vec<FxTick>,
    fx_binds: Vec<(u32, FxAt)>,
    /// Model-space muzzle matrices a host drew itself this frame (an alt-skin's FLUDD): the
    /// mist and the water take these instead of the ones this view would pose.
    host_muzzles: Vec<Mat4>,
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
    /// Luigi's HSD mesh, when the Melee cache was loaded ([`Self::set_luigi`]).
    pub luigi: Option<Luigi>,
    /// Yoshi's body and clips, when the skin was loaded ([`Self::load_skin`]).
    pub yoshi: Option<Yoshi>,
    /// Same-rig skin bodies loaded into [`Self::models`] ([`Self::load_skin`]): skin, model.
    rig_bodies: Vec<(Skin, u32)>,
    /// `diver_helm.bmd`'s index in `models`, once [`View::set_helmet`] has loaded it.
    helmet: Option<u32>,
    /// Draw the diving helmet on the head (`TMarioCap` model 2, flag `E_CAP_MODEL_HELMET`).
    pub helmet_on: bool,
    /// Shadow Mario's swirl ([`Skin::Shadow`], once loaded): his model, the texture id all his
    /// batches share, and the baker.
    shadow: Option<(u32, i32, crate::shadow_skin::ShadowSwirl)>,
    /// A prop's texture scrolls (the Gatekeeper's `TMultiBtk`), and the game frame last baked.
    scroll: Vec<libsms_assets::btk::TexSrt>,
    scroll_frame: Option<u64>,
    /// `kagemario_brush.bmd`'s index in `models` (drawn in Shadow Mario's left hand).
    brush: Option<u32>,
    /// The Sunshine assets folder, kept for loading Luigi's rig and clips later.
    assets_dir: PathBuf,
    /// The body's named attachment points (head, chest, hands...) posed this frame, in
    /// model space, for a host that mounts its own props on the body. Indexed by
    /// [`Anchor`]; `None` where the current skin has no such part.
    anchors: [Option<Mat4>; ANCHOR_COUNT],
    /// The host's VR arms ([`View::set_vr_arms`]), and each body's cut once read (by model;
    /// Luigi's under [`LUIGI_CUT`]).
    vr_arms: bool,
    vr_cuts: Vec<(u32, Option<VrCut>)>,
    /// A VR host's eye height above Mario's feet (decomp units; 0: none): the spin jump's swirl
    /// ([`VR_EYE_EFFECTS`]) is centred there ([`View::set_vr_eye`]).
    vr_eye: f32,
}

/// Host-facing body attachment points shared by every skin (the FFI's `sms_view_anchor`
/// ids, in this order). Olimar's and Pikmin's views use the same order, so one host class
/// mounts item displays on any of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    Head = 0,
    Chest = 1,
    Pelvis = 2,
    ShoulderL = 3,
    ShoulderR = 4,
    ElbowL = 5,
    ElbowR = 6,
    HandL = 7,
    HandR = 8,
    LegL = 9,
    LegR = 10,
    KneeL = 11,
    KneeR = 12,
    FootL = 13,
    FootR = 14,
}
pub const ANCHOR_COUNT: usize = 15;
/// The Stu's `jnt_brain`, where its brain rides ([`View::prop_joint`]).
pub const NAMEKURI_BRAIN_JOINT: usize = 2;

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
    // MarioReceiveMsg.cpp getNozzle -> MarioParticle.cpp emitGetEffect, JPA 0x0E.
    "ms_itemget1_a",
    // The nozzles: the Rocket's launch (`rocketEffectStart`) and every muzzle's smoke.
    "ms_m_rocket_a",
    "ms_m_rocket_b",
    "ms_m_rocket_b2",
    "ms_m_rocket_c",
    "ms_m_rocket_d",
    "ms_nozzlesmoke",
    // MarDirectorLoadResource.cpp: emitter 6; MarioReceiveMsg.cpp fire damage.
    "ms_firedmg_a",
    // The Turbo's dash (`TMarioEffect::perform`): bound to the muzzle while it lasts.
    "ms_watboost_a",
    "ms_watboost_b",
    // Enemy water-hit marker, TSmallEnemy/TChuuHana::receiveMessage, cue 0xE7.
    crate::water_hit::EFFECT,
];

/// The effects a VR host's eye recentres ([`View::set_vr_eye`]): the spin jump's swirl and its
/// spark (`TMario::emitBlurSpinJump`, MarioParticle.cpp:564).
pub const VR_EYE_EFFECTS: &[&str] = &["ms_m_blur2", "ms_m_blur2sp"];

/// The slot's effect is one [`VR_EYE_EFFECTS`] centres: a held slot is its index in
/// [`MARIO_EFFECTS`], a following one that past `FOLLOW_SLOTS`.
fn eye_slot(slot: u32) -> bool {
    let i = if slot >= FOLLOW_SLOTS { slot - FOLLOW_SLOTS } else { slot } as usize;
    MARIO_EFFECTS.get(i).is_some_and(|n| VR_EYE_EFFECTS.contains(n))
}

/// `mtx` moved up or down so its origin is `eye` above the feet at `feet` (0: unchanged).
fn at_eye(mut mtx: Mat4, feet: Vec3, eye: f32) -> Mat4 {
    if eye > 0.0 {
        mtx.w_axis.y = feet.y + eye;
    }
    mtx
}

/// Luigi's alt-skin: Mario's effects are his, but the red parts turn green. From
/// `TMario::emitBlurSpinJump` (MarioParticle.cpp:564: the spin jump's swirl `ms_m_blur2` and
/// its spark `ms_m_blur2sp`) and `TMario::emitBlurHipDrop` (:546: the ground pound's smear
/// `ms_m_blur3`). Their textures are grey, so all their colour is in the JPA's prm/env.
pub const LUIGI_RED_EFFECTS: &[&str] = &["ms_m_blur2", "ms_m_blur2sp", "ms_m_blur3"];

/// The super ground pound's fireball (`TMario::emitBlurHipDropSuper`, MarioParticle.cpp:552:
/// `PARTICLE_MS_M_SPHIPD_A..D`). Its fire runs red through orange to yellow, so every warm
/// entry turns green ([`GreenRule::Fire`]) rather than only the saturated red ones.
pub const LUIGI_FIRE_EFFECTS: &[&str] = &[
    "ms_m_sphipd_a",
    "ms_m_sphipd_b",
    "ms_m_sphipd_c",
    "ms_m_sphipd_d",
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

/// FLUDD water thrown by Mario's movement rather than the nozzle: the wet belly slide's spray.
/// Host pacing may run these at the same rate as the Spray stream (the rest of the water, the
/// runoff drips, is in the sim's water manager).
pub const WATER_EFFECTS: &[&str] = &[
    "ms_m_watslide_a",
    "ms_m_watslide_b",
    "ms_m_watslide_c",
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
    burning: bool,
    tick: u64,
    pos: Vec3,
    yaw: i16,
    /// The corkscrew's tip and roll (`Mario::model_pitch` / `model_roll`), so effects born this tick sit in the
    /// tipped frame, not upright.
    pitch: i16,
    roll: i16,
    pounding: u8,
    /// `Mario::flames`: 1 a corkscrew that has spun for 1 second, 2 a rising spin out of the Rocket's climb.
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
        FxAt::JointFollow(j) => joints.get(j as usize)
            .map(|j| Mat4::from_translation((root * *j).w_axis.truncate())),
        // emitAndBindToPosPtr follows position only, not Mario's rotation.
        FxAt::Feet | FxAt::FeetFollow => Some(Mat4::from_translation(root.w_axis.truncate())),
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

pub(crate) fn load_bmd(dir: &Path, rel: &str) -> Result<Bmd, String> {
    let data = std::fs::read(dir.join(rel)).map_err(|e| format!("{rel}: {e}"))?;
    Bmd::parse(&data).map_err(|e| format!("{rel}: {e}"))
}

/// Builds a model: its batches, texture images (`replace` swaps some first: the caps' and
/// hands' texture 0 is the body's, `TMarioCap`; the nozzles' env-map stand-in is FLUDD's),
/// unlit channel colours and plain bakes.
pub(crate) fn build(
    bmd: Bmd,
    replace: &dyn Fn(usize, &str) -> Option<Tex>,
    index: u32,
    textures: &mut Textures,
) -> Result<Model, String> {
    let batches: Vec<Batch> = bmd
        .batches()
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|b| !b.indices.is_empty())
        .collect();
    build_batches(bmd, batches, replace, index, textures)
}

fn build_batches(bmd: Bmd, batches: Vec<Batch>, replace: &dyn Fn(usize, &str) -> Option<Tex>, index: u32, textures: &mut Textures) -> Result<Model, String> {
    let inputs = base_inputs(index);
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
        k0_alpha: (matches!(model, models::BODY | models::CAP | 1..=4)
            || (model >= RIG_KEY_BASE && model != HELMET_KEY && model != BRUSH_KEY))
            .then_some(0),
        ..Inputs::default()
    }
}

impl Textures {
    pub fn version(&self, id: u32) -> u32 { self.versions.get(&(id as i32)).copied().unwrap_or(0) }

    fn animate(&mut self, model: &Model, version: u32) {
        self.animate_as(model, models::WATERBOOST, version);
    }

    /// Re-bakes `model`'s textures (after its texture matrices moved) with `index`'s inputs.
    fn animate_as(&mut self, model: &Model, index: u32, version: u32) {
        for (bi, batch) in model.batches.iter().enumerate() {
            let id = model.batch_texture[bi];
            if id < 0 || self.version(id as u32) == version { continue; }
            if let Some(mut baked) = model.source().bake(batch.material, &base_inputs(index), MAX_BAKE) {
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

/// The lowest skinned point of a J3D model for `joints`, in model space (decomp units), as the
/// viewer's `MarioModel::lowest`: Luigi's alt-skin anchors his own lowest point to this so he
/// stands, dives and hangs where Mario does.
fn body_lowest(model: &Model, joints: &[Mat4]) -> f32 {
    let pose = Pose::new(&model.bmd, Mat4::IDENTITY, joints);
    let mut low = f32::INFINITY;
    for b in &model.batches {
        for v in &b.vertices {
            low = low.min(pose.skin(v).0.y);
        }
    }
    low
}

impl View {
    /// Replaces the body pose drawn this frame (one model-space matrix per
    /// Sunshine joint), or clears it with `None`. False when the count is wrong;
    /// the sim pose then stays in use. Used for the Melee Mario A-attack port.
    pub fn set_pose_override(&mut self, joints: Option<Vec<Mat4>>) -> bool {
        if let Some(j) = &joints
            && j.len() != self.skeleton.len()
        {
            return false;
        }
        self.pose_override = joints;
        true
    }
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
        match look {
            Look::Flower => {
                // The game's flower draws `flower_orange.bmt`, not its own placeholder
                // material (`TSamboFlowerManager::load`, `TSamboFlower::setMActorAndKeeper`).
                return Self::load_prop_table(
                    dir,
                    "samboflower",
                    "flower.bmd",
                    Some("flower_orange.bmt"),
                );
            }
            Look::Sambo => return Self::load_prop(dir, "sambo", "sambo.bmd"),
            Look::SamboHead => return Self::load_prop(dir, "sambohead", "sambohead.bmd"),
            Look::Effects => return Self::load_effects(dir),
            Look::KoopaJr => return Self::load_prop(dir, "koopajr", "koopajr_model.bmd"),
            Look::NameKuri => return Self::load_prop(dir, "namekuri2", "name.bmd"),
            Look::NameKuriBrain => return Self::load_prop(dir, "namekuri2", "brain.bmd"),
            Look::Pakkun => return Self::load_prop(dir, "pakkun", "pakun.bmd"),
            Look::PakkunSeed => return Self::load_prop(dir, "pakkun", "seed.bmd"),
            Look::KoopaSign => {
                return Self::load_prop(dir, "kagemario/stamp_koopa_sign", "stamp_koopa_sign_model1.bmd");
            }
            Look::GateKeeper => {
                // `TBiancoGateKeeper::init`: a `TMultiBtk` of the two scrolls over its goop.
                let mut view = Self::load_prop(dir, "gatekeeper", "gene_pakkun_model1.bmd")?;
                view.scroll = ["gene_pakkun_tex0.btk", "gene_pakkun_tex1.btk"]
                    .iter()
                    .filter_map(|n| std::fs::read(dir.join("enemy/gatekeeper").join(n)).ok())
                    .filter_map(|d| libsms_assets::btk::TexSrt::parse(&d).ok())
                    .collect();
                if !view.scroll.is_empty() {
                    // Its textures change: the host keeps them writable (a nonzero version).
                    for &id in &view.models[0].batch_texture {
                        if id >= 0 {
                            view.textures.versions.insert(id, 1);
                        }
                    }
                }
                return Ok(view);
            }
            _ => {}
        }
        let assets = libsms::Assets::load(dir);
        let skeleton = assets.skeleton.clone().ok_or("no rig (bmd/ma_mdl1.bmd)")?;
        let mut textures = Textures {
            shade: (look == Look::Shadow).then(|| Shade::load(dir)),
            ..Textures::default()
        };
        let none = |_: usize, _: &str| None;
        let dirt = dirt_texture(dir);
        let dirty = |_: usize, name: &str| swap_dirt(name, &dirt);
        let body = build(
            load_bmd(dir, paths::BODY)?,
            &dirty,
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
        let body_first = |i: usize, name: &str| if i == 0 { body_tex0.clone() } else { swap_dirt(name, &dirt) };
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
        let faces: Vec<FacePattern> = FACE_PATTERNS
            .iter()
            .filter_map(|&(clip, file)| {
                let pat = std::fs::read(dir.join("btp").join(file))
                    .ok()
                    .and_then(|d| TexPattern::parse(&d).ok())?;
                let mats = pat
                    .tracks
                    .iter()
                    .map(|t| b.material(&t.material).unwrap_or(usize::MAX))
                    .collect();
                Some(FacePattern { clip, pat, mats })
            })
            .collect();
        let mut fx = Particles::load(dir, MARIO_EFFECTS);
        fx.set_world_space(crate::water_hit::EFFECT, true);
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
            props: None,
            return_parts: None,
            hand_joints: [b.joint("jnt_hand_R"), b.joint("jnt_hand_L")],
            head: b.joint("M_head"),
            chest: b.joint("jnt_chest"),
            center: b.joint("center"),
            fx,
            movement_effects: true,
            pound_flames: true,
            fx_ticks: Vec::new(),
            fx_binds: (0..4).map(|s| (s, FxAt::Center)).collect(),
            host_muzzles: Vec::new(),
            hud: Hud::load(dir).ok(),
            skin: Skin::Mario,
            pose_override: None,
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
            faces,
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
            luigi: None,
            yoshi: None,
            rig_bodies: Vec::new(),
            helmet: None,
            helmet_on: false,
            shadow: None,
            scroll: Vec::new(),
            scroll_frame: None,
            brush: None,
            assets_dir: dir.to_path_buf(),
            anchors: [None; ANCHOR_COUNT],
            vr_arms: false,
            vr_eye: 0.0,
            vr_cuts: Vec::new(),
        })
    }

    /// The body attachment point `id` as the last draw posed it, in model space.
    pub fn anchor(&self, id: usize) -> Option<Mat4> {
        self.anchors.get(id).copied().flatten()
    }

    /// TCoin's original model and materials, through the same model ABI as Mario. Hosts
    /// instance this bind pose; they do not call Mario's animation update on this view.
    fn load_coin(dir: &Path, file: &str) -> Result<View, String> {
        Self::load_static(dir, &dir.join("scene/mapobj").join(file))
    }

    /// A sambo model (`enemy/<folder>/<file>`) with every `.bck` in its folder, posed by
    /// [`Self::update_prop`].
    fn load_prop(dir: &Path, folder: &str, file: &str) -> Result<View, String> {
        Self::load_prop_table(dir, folder, file, None)
    }

    /// The same, with a `.bmt` material table in the same folder swapped in first (the game's
    /// `J3DModelData::setMaterialTable`): the plain flower's own material is a placeholder over
    /// `K_name_dummy`, and `flower_orange.bmt` gives it its texture.
    fn load_prop_table(
        dir: &Path,
        folder: &str,
        file: &str,
        table: Option<&str>,
    ) -> Result<View, String> {
        let root = dir.join("enemy").join(folder);
        let mut view = Self::load_static_table(
            dir,
            &root.join(file),
            table.map(|t| root.join(t)).as_deref(),
        )?;
        let mut clips = Vec::new();
        for e in std::fs::read_dir(&root).map_err(|e| e.to_string())?.flatten() {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("bck") {
                continue;
            }
            let name = path.file_stem().and_then(|x| x.to_str()).unwrap_or("").to_lowercase();
            if let Some(c) = std::fs::read(&path).ok().and_then(|d| Clip::parse(&d)) {
                clips.push((name, c));
            }
        }
        view.props = Some(clips);
        Ok(view)
    }

    /// The flower effects' view: a flower's model (never drawn) carrying the particle runs.
    fn load_effects(dir: &Path) -> Result<View, String> {
        let mut view = Self::load_prop(dir, "samboflower", "flower.bmd")?;
        view.fx = Particles::load(dir, &FLOWER_EFFECTS);
        for name in FLOWER_EFFECTS {
            view.fx.set_world_space(name, true);
        }
        // The goop's and its creatures' (the host emits them all on this view).
        for (name, folder) in GOOP_EFFECTS {
            if FLOWER_EFFECTS.contains(&name) {
                continue;
            }
            if view.fx.add_file(name, &dir.join(folder).join(format!("{name}.jpa"))) {
                view.fx.set_world_space(name, true);
            }
        }
        Ok(view)
    }

    /// Starts one of [`FLOWER_EFFECTS`] at `pos` (`TMarioParticleManager::emit`), `scale` big.
    pub fn emit_effect(&mut self, name: &str, pos: Vec3, scale: f32) {
        self.fx.emit(name, Mat4::from_translation(pos), scale);
    }

    /// Accepted enemy hits from the water simulation, including native flower enemies.
    pub fn water_events(&mut self, events: &[libsms::water::WaterEvent]) {
        crate::water_hit::effects(&mut self.fx, events);
    }

    /// Runs the effects `dt` seconds on and builds their quads.
    pub fn step_effects(&mut self, dt: f32) {
        self.time += dt.max(0.0);
        let ticks = self.time as f64 * 120.0;
        self.fx.advance(ticks as u64);
        let fraction = self.fx.render_fraction(ticks);
        self.fx.build_interpolated(self.right, self.up, fraction, |_| true);
    }

    /// A prop view's clip by name: its index and length in frames.
    pub fn prop_clip(&self, name: &str) -> Option<(usize, f32)> {
        let name = name.to_lowercase();
        self.props
            .as_ref()?
            .iter()
            .enumerate()
            .find(|(_, (n, _))| *n == name)
            .map(|(i, (_, c))| (i, c.frames))
    }

    /// Poses a prop view: clip `clip` (an index from [`Self::prop_clip`]; negative is the bind
    /// pose) at `frame`, turned `yaw` (the host places and scales its own instance); `mirror` is the host's
    /// X flip, for env-mapped coordinates.
    /// A prop view's joint `joint` (model space, unturned) in clip `clip` at `frame` (bind pose
    /// for a clip of -1), for a host that mounts a part on it: the Swoopin' Stu wears its brain
    /// ([`Look::NameKuriBrain`]) on joint [`NAMEKURI_BRAIN_JOINT`] (`TSharedParts(.., 2, ..)`,
    /// namekuri.cpp). None for another view or a joint it doesn't have.
    pub fn prop_joint(&self, clip: i32, frame: f32, joint: usize) -> Option<Mat4> {
        let props = self.props.as_ref()?;
        let model = self.models.first()?;
        let joints = match usize::try_from(clip).ok().and_then(|i| props.get(i)) {
            Some((_, c)) => model.bmd.pose(|j| c.sample(j, frame), |_| None),
            None => model.bind.clone(),
        };
        joints.get(joint).copied()
    }

    /// Runs a prop's texture scrolls to `seconds` (the Gatekeeper's goop flowing over it).
    /// `TMultiBtk::update` steps each at `0.5 * SMSGetAnmFrameRate()` (2), a btk frame a 30 fps
    /// game frame, wrapping at its end (both loop whole tiles and turns, so the wrap is seamless).
    /// Re-bakes the textures every other game frame; their versions change for the host.
    pub fn scroll_textures(&mut self, seconds: f32) {
        if self.scroll.is_empty() || !seconds.is_finite() {
            return;
        }
        let frame = (seconds.max(0.0) * 30.0) as u64 & !1;
        if self.scroll_frame == Some(frame) {
            return;
        }
        self.scroll_frame = Some(frame);
        let Some(model) = self.models.get_mut(0) else { return };
        for s in &self.scroll {
            let f = if s.frames > 0.0 { (frame as f64 % s.frames as f64) as f32 } else { 0.0 };
            s.apply(&mut model.bmd, f);
        }
        let model = &self.models[0];
        self.textures.animate_as(model, models::COUNT as u32, (frame / 2) as u32 + 2);
    }

    pub fn update_prop(&mut self, yaw: i16, clip: i32, frame: f32, mirror: bool) {
        let Some(props) = self.props.as_ref() else { return };
        let model = &self.models[0];
        let joints = match usize::try_from(clip).ok().and_then(|i| props.get(i)) {
            Some((_, c)) => model.bmd.pose(|j| c.sample(j, frame), |_| None),
            None => model.bind.clone(),
        };
        self.positions.clear();
        self.normals.clear();
        self.uvs.clear();
        let pose = Pose::new(&model.bmd, Mat4::IDENTITY, &joints);
        let turn = Quat::from_rotation_y(yaw as f32 * std::f32::consts::PI / 32768.0);
        let world = |n: Vec3| turn * if mirror { Vec3::new(-n.x, n.y, n.z) } else { n };
        for (bi, b) in model.batches.iter().enumerate() {
            let hide = model.bmd.materials[b.material].cull == Cull::All;
            let env = model.batch_env[bi];
            for v in &b.vertices {
                let (p, n) = pose.skin(v);
                let n = n.normalize_or_zero();
                self.positions.push(if hide { Vec3::ZERO } else { p });
                self.normals.push(n);
                self.uvs.push(match env {
                    Some(mtx) => {
                        let w = world(n);
                        env_uv(w.dot(self.right), w.dot(self.up), mtx)
                    }
                    None => [v.uv[0].x, v.uv[0].y],
                });
            }
        }
        self.colors = model.batch_color.clone();
        self.texture_ids = model.batch_texture.clone();
    }

    fn load_static(dir: &Path, path: &Path) -> Result<View, String> {
        Self::load_static_table(dir, path, None)
    }

    /// `table`, if given, is a `.bmt` material table swapped into the model before its
    /// materials are baked (`J3DModelData::setMaterialTable`).
    fn load_static_table(
        dir: &Path,
        path: &Path,
        table: Option<&Path>,
    ) -> Result<View, String> {
        let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let skeleton = Skeleton::from_bmd(&data).ok_or("coin has no rig")?;
        let mut bmd = Bmd::parse(&data).map_err(|e| format!("{}: {e}", path.display()))?;
        if let Some(table) = table {
            let t = std::fs::read(table).map_err(|e| format!("{}: {e}", table.display()))?;
            let t = libsms_assets::MaterialTable::parse(&t)
                .map_err(|e| format!("{}: {e}", table.display()))?;
            bmd.set_material_table(&t);
        }
        let mut textures = Textures::default();
        // Goop enemies show goop through the dirt mask (`H_ma_rak_dummy`), as Mario does.
        let dirt = dirt_texture(dir);
        let model = build(bmd, &|_, name| swap_dirt(name, &dirt), models::COUNT as u32, &mut textures)?;
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
            props: None,
            return_parts: None,
            models: vec![model],
            textures,
            skeleton,
            clips: ClipLibrary::default(),
            fludd_clips: HashMap::new(),
            boost: None,
            wink: None,
            eye_materials: Vec::new(),
            faces: Vec::new(),
            hand_joints: [None; 2],
            head: None,
            chest: None,
            center: None,
            anchors: [None; ANCHOR_COUNT],
            vr_arms: false,
            vr_eye: 0.0,
            vr_cuts: Vec::new(),
            fx: Particles::default(),
            movement_effects: false,
            pound_flames: false,
            fx_ticks: Vec::new(),
            fx_binds: Vec::new(),
            host_muzzles: Vec::new(),
            hud: None,
            skin: Skin::Mario,
            pose_override: None,
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
            luigi: None,
            yoshi: None,
            rig_bodies: Vec::new(),
            helmet: None,
            helmet_on: false,
            shadow: None,
            scroll: Vec::new(),
            scroll_frame: None,
            brush: None,
            assets_dir: PathBuf::new(),
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
            return_parts: None,
            props: None,
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
            faces: Vec::new(),
            hand_joints: [None; 2],
            head: None,
            chest: None,
            center: None,
            anchors: [None; ANCHOR_COUNT],
            vr_arms: false,
            vr_eye: 0.0,
            vr_cuts: Vec::new(),
            fx,
            movement_effects: true,
            pound_flames: false,
            fx_ticks: Vec::new(),
            fx_binds: Vec::new(),
            host_muzzles: Vec::new(),
            hud: None,
            skin: Skin::Mario,
            pose_override: None,
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
            luigi: None,
            yoshi: None,
            rig_bodies: Vec::new(),
            helmet: None,
            helmet_on: false,
            shadow: None,
            scroll: Vec::new(),
            scroll_frame: None,
            brush: None,
            assets_dir: PathBuf::new(),
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
        // The body's gold is environment mapped (`_mat_shine_body`: both texgens from the normal), so
        // its coordinates come from the normal as the camera sees it, turned and mirrored as the host
        // draws it; its vertices carry none (they all read the bake's white-hot middle).
        let turn = Quat::from_rotation_y(yaw as f32 * std::f32::consts::PI / 32768.0);
        let world = |n: Vec3| turn * if mirror { Vec3::new(-n.x, n.y, n.z) } else { n };
        for (bi, b) in model.batches.iter().enumerate() {
            let hide = model.bmd.materials[b.material].cull == Cull::All;
            let env = model.batch_env[bi];
            for v in &b.vertices {
                let (p, n) = pose.skin(v);
                let n = n.normalize_or_zero();
                self.positions.push(if hide { Vec3::ZERO } else { p });
                self.normals.push(n);
                self.uvs.push(match env {
                    Some(mtx) => {
                        let w = world(n);
                        env_uv(w.dot(self.right), w.dot(self.up), mtx)
                    }
                    None => [v.uv[0].x, v.uv[0].y],
                });
            }
        }
        self.colors = model.batch_color.clone();
        self.texture_ids = model.batch_texture.clone();

        // The glow, asked for every game frame (30 a second, four ticks of 120).
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

    /// Host tuning for FLUDD water thrown by movement (the wet slide's spray); 1 preserves disc
    /// timing. The Spray stream and the runoff drips are scaled in the sim's water manager.
    pub fn set_water_speed(&mut self, speed: f32) {
        for &name in WATER_EFFECTS {
            self.fx.set_speed(name, speed);
        }
    }

    /// Observe every sim tick (`Sim::advance_observed`), even without sound or rendering.
    /// Muzzle matrices a host posed itself (an alt-skin's FLUDD), in Mario's model space:
    /// from this frame on, the nozzle mist and the water leave those instead of this view's.
    /// Whose body the view draws (a skin picks the mesh; the clips and FLUDD look are shared).
    /// Luigi's alt-skin also turns Mario's red effects green ([`LUIGI_RED_EFFECTS`]) and his
    /// pound's fireball green ([`LUIGI_FIRE_EFFECTS`]).
    pub fn set_skin(&mut self, skin: Skin) {
        if self.skin == skin {
            return;
        }
        self.skin = skin;
        let luigi = skin == Skin::Luigi;
        let pianta = skin == Skin::Piantissimo;
        // Back to the file's own colours first: the two skins' palettes share one saved original.
        self.fx.retint_to_green(LUIGI_RED_EFFECTS, false, GreenRule::Red);
        self.fx.retint_to_green(LUIGI_FIRE_EFFECTS, false, GreenRule::Fire);
        self.fx
            .retint_to_green(LUIGI_RED_EFFECTS, luigi, GreenRule::Red);
        self.fx
            .retint_to_green(LUIGI_FIRE_EFFECTS, luigi, GreenRule::Fire);
        if pianta {
            // His own smears: Mario's red becomes his purply pink and the blue his skin's brown (the
            // main colour) and his leaf's green (the mixing colour); the super pound's fireball burns pink.
            self.fx.retint_to_colors(
                LUIGI_RED_EFFECTS,
                true,
                GreenRule::Red,
                PIANTISSIMO_PINK,
                Some([PIANTISSIMO_BROWN, PIANTISSIMO_GREEN]),
            );
            self.fx.retint_to_colors(
                LUIGI_FIRE_EFFECTS,
                true,
                GreenRule::Fire,
                PIANTISSIMO_PINK,
                Some([PIANTISSIMO_BROWN, PIANTISSIMO_GREEN]),
            );
        }
    }

    /// Loads Luigi's HSD mesh from the Melee cache (alongside the Sunshine assets this view was
    /// built from), making [`Skin::Luigi`] drawable. False when the cache is incomplete, so a
    /// host can offer the skin only then.
    /// Makes `skin` drawable: Luigi reads the Melee cache at `melee_dir`; a same-rig skin loads
    /// its model from the Sunshine assets (`melee_dir` unused). False when its files are
    /// missing or its rig is not Mario's, so a host offers the skin only then.
    pub fn load_skin(&mut self, skin: Skin, melee_dir: Option<&Path>) -> bool {
        match skin.body() {
            SkinBody::Mario => true,
            SkinBody::Hsd => melee_dir.is_some_and(|d| self.set_luigi(d)),
            SkinBody::Yoshi => {
                if self.yoshi.is_some() {
                    return true;
                }
                let index = self.models.len() as u32;
                match Yoshi::load(&self.assets_dir, &self.skeleton, &mut self.textures, index) {
                    Ok((yoshi, model)) => {
                        self.models.push(model);
                        self.yoshi = Some(yoshi);
                        true
                    }
                    Err(_) => false,
                }
            }
            SkinBody::SameRig(rel) => {
                if self.rig_bodies.iter().any(|(s, _)| *s == skin) {
                    return true;
                }
                let loaded = std::fs::read(self.assets_dir.join(rel))
                    .ok()
                    .and_then(|data| libsms::Skeleton::from_bmd(&data))
                    .filter(|rig| rig.parents == self.skeleton.parents)
                    .and_then(|_| load_bmd(&self.assets_dir, rel).ok())
                    .and_then(|bmd| {
                        let dirt = dirt_texture(&self.assets_dir);
                        build(bmd, &|_, name| swap_dirt(name, &dirt), RIG_KEY_BASE + skin.id(), &mut self.textures).ok()
                    });
                match loaded {
                    Some(mut model) => {
                        let index = self.models.len() as u32;
                        let brush = (skin == Skin::Shadow).then(|| self.attach_shadow(&mut model, index)).flatten();
                        self.models.push(model);
                        self.rig_bodies.push((skin, index));
                        if let Some(b) = brush {
                            self.models.push(b);
                            self.brush = Some(index + 1);
                        }
                        true
                    }
                    None => false,
                }
            }
        }
    }

    /// Wears (or takes off) the diving helmet: `watergun2/body/diver_helm.bmd` on `jnt_head`
    /// (`TMarioCap::TMarioCap`, MarioCap.cpp; on with `TMario::setDivHelm`). Its glass goes
    /// through the same TEV bake and alpha blend class as every other model, so it comes
    /// out translucent. Returns false when the disc's file is missing. Mario and the
    /// same-rig skins wear it; Luigi's HSD mesh and Yoshi's rig have no `jnt_head` here.
    pub fn set_helmet(&mut self, on: bool) -> bool {
        self.helmet_on = false;
        if self.helmet.is_none() {
            let built = load_bmd(&self.assets_dir, "watergun2/body/diver_helm.bmd")
                .ok()
                .and_then(|bmd| build(bmd, &|_, _| None, HELMET_KEY, &mut self.textures).ok());
            let Some(model) = built else { return false };
            self.models.push(model);
            self.helmet = Some(self.models.len() as u32 - 1);
        }
        self.helmet_on = on;
        true
    }

    /// Also draws each forearm and hand on its own, for a host that puts them on a VR
    /// player's controllers (the body itself is then the host's to leave out), for every skin:
    /// Mario, Piantissimo, Yoshi and Luigi. Each is libcommon's `vr_arms` cut of the body, read
    /// from the asset cache (`vr_arms/sms-<skin>.arms`, baked on first run by [`View::bake_vr_arms`]
    /// and cut then if missing). Mario's own glove shapes are kept even while the animation shows
    /// the separate hand models (`changeHand`). Each draw is posed as the body is but given in
    /// its arm space, keyed [`VR_ARM_KEY`] + side (0 right, 1 left) under the body's model.
    pub fn set_vr_arms(&mut self, on: bool) {
        self.vr_arms = on;
    }

    /// A VR host's first person: the eye is `height` above Mario's feet (decomp units; 0 or less
    /// puts it back). The spin jump's swirl wraps round Mario's middle in the game, which seen
    /// from his head is a ring at the feet; with an eye it's centred on the eye instead, so the
    /// player is in the middle of it as Mario is from outside.
    pub fn set_vr_eye(&mut self, height: f32) {
        self.vr_eye = if height.is_finite() { height.max(0.0) } else { 0.0 };
    }

    /// Shadow Mario's swirl while his skin is drawn (for a host's screen-grab pass).
    pub fn shadow_swirl(&self) -> Option<&crate::shadow_skin::ShadowSwirl> {
        self.shadow.as_ref().filter(|_| self.skin == Skin::Shadow).map(|(_, _, s)| s)
    }

    /// Shadow Mario's look on his freshly built body: every batch takes the one swirl texture
    /// (baked now, rebaked each game frame by [`Self::update`]), and his brush is loaded.
    /// Returns the brush to push after the body.
    fn attach_shadow(&mut self, model: &mut Model, index: u32) -> Option<Model> {
        let swirl = crate::shadow_skin::ShadowSwirl::load(&self.assets_dir, &model.bmd)?;
        self.textures.list.push(swirl.bake());
        let id = self.textures.list.len() as i32 - 1;
        self.textures.versions.insert(id, 1);
        for bi in 0..model.batches.len() {
            model.batch_texture[bi] = id;
            model.batch_color[bi] = [1.0; 4];
            model.batch_blend[bi] = blend::OPAQUE;
            model.batch_env[bi] = None;
        }
        self.shadow = Some((index, id, swirl));
        // The brush shows goop through the dirt mask as the game's does (`SMS_ChangeTextureAll`
        // of `H_ma_rak_dummy`, `TEnemyMario::init`).
        let dirt = dirt_texture(&self.assets_dir);
        load_bmd(&self.assets_dir, crate::shadow_skin::BRUSH)
            .ok()
            .and_then(|bmd| build(bmd, &|_, name| swap_dirt(name, &dirt), BRUSH_KEY, &mut self.textures).ok())
    }

    /// The model a same-rig skin draws as its body, when that skin is current and loaded.
    fn rig_body(&self) -> Option<(u32, u32)> {
        self.rig_bodies
            .iter()
            .find(|(s, _)| *s == self.skin)
            .map(|&(s, m)| (m, RIG_KEY_BASE + s.id()))
    }

    pub fn set_luigi(&mut self, melee_dir: &Path) -> bool {
        match Luigi::load(melee_dir, &self.assets_dir, &mut self.textures) {
            Ok(luigi) => {
                self.luigi = Some(luigi);
                true
            }
            Err(_) => {
                self.luigi = None;
                false
            }
        }
    }

    pub fn set_muzzles(&mut self, muzzles: &[Mat4]) {
        self.host_muzzles.clear();
        self.host_muzzles.extend_from_slice(muzzles);
    }

    /// Queue effects until the next pose is available, preserving one-shots and short held
    /// effects between rendered frames. Empty ticks release held emitters at 30 Hz too.
    pub fn events(&mut self, events: &[Event], mario: &Mario) {        self.fx_ticks.push(FxTick {
            burning: mario.on_fire(),
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
        let route = (self.skin == Skin::Yoshi).then(|| yoshi_skin::SprayRoute::for_mario(m));
        let yoshi_spraying = m
            .fludd
            .as_ref()
            .is_some_and(|f| f.spray_amount() > 0.0 && f.water > 0);
        let mut look = m.fludd_look();
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
        let joints = match &self.pose_override {
            Some(p) if p.len() == joints.len() => p.clone(),
            _ => joints,
        };

        // Yoshi's own rig: his wait/walk/run and flutter clips on his skeleton, Mario's air
        // clips through the retarget, anchored feet to feet.
        let mut yoshi_world: Option<Vec<Mat4>> = None;
        let mut yoshi_chest: Option<Mat4> = None;
        let mut yoshi_mouth: Option<Mat4> = None;
        if self.skin == Skin::Yoshi
            && let Some(y) = self.yoshi.as_mut()
        {
            let picked = yoshi_skin::clip_for(m, back, y, &self.clips);
            if let Some((clip, frame, native)) = picked {
                let world = if native {
                    y.pose_native(clip, frame)
                } else {
                    Some(y.pose_retarget(&joints))
                };
                if let Some(mut world) = world {
                    let mario_low = body_lowest(&self.models[models::BODY as usize], &joints);
                    let anchor =
                        mario_low - yoshi_skin::lowest(&self.models[y.model as usize], &world);
                    for mtx in &mut world {
                        mtx.w_axis.y += anchor;
                    }
                    // Mouth juice: open the jaw and turn the head inside the clearance envelope.
                    if let Some(route) = route {
                        world = y.spray_world(world, route, yoshi_spraying);
                    }
                    if route.is_some_and(|r| r.mouth && yoshi_spraying) {
                        yoshi_mouth = Some(y.mouth(&world));
                    }
                    yoshi_chest = look.and(Some(y.chest(&world)));
                    yoshi_world = Some(world);
                }
            }
        }
        // While the juice leaves Yoshi's mouth, FLUDD is at rest (its house clip and nozzle).
        // The head aim above already used the fitted Spray's route.
        let mouth_route = route.is_some_and(|r| r.mouth && yoshi_spraying);
        if let (true, Some(l)) = (mouth_route, look.as_mut()) {
            l.body_clip = libsms::fludd_anim::body_clip::HOUSE;
            l.body_frame = m.anims.get(l.body_clip).frames;
            l.nozzle_clip = Some("normal_wg_house");
            l.nozzle_frame = m.anims.get("normal_wg_house").frames;
            l.gun_angle = 0.0;
            l.aimed = false;
            l.aim_turn = 0;
        }

        // Blink every few seconds (the game runs `ma_wink_tx` on idle; 30 frames a second).
        if self.time > self.blink_at + 2.0 {
            self.blink_at = self.time + 2.0 + (self.time * 7.13).fract() * 2.5;
        }
        let mut body_inputs: HashMap<usize, Inputs> = HashMap::new();
        let dirt = dirt_alpha(m.dirt_share());
        // Shadow Mario's swirl, rebaked when the game frame turns.
        if let Some((sm, id, swirl)) = self.shadow.as_mut()
            && self.skin == Skin::Shadow
            && let Some(baked) = swirl.step(&mut self.models[*sm as usize].bmd, self.time)
        {
            self.textures.list[*id as usize] = baked;
            let v = self.textures.version(*id as u32).wrapping_add(1).max(1);
            self.textures.versions.insert(*id, v);
        }
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
        // A clip with a face of its own wears it for as long as it plays (`setAnimation` starts
        // the clip's `marioAnimeTexPatternFilenames` entry beside it): the burning Mario's hot face.
        // The skins with no pattern to swap show it another way: Luigi's pained eyes, Piantissimo's
        // flushed red.
        let hot = FACE_PATTERNS.iter().any(|&(clip, _)| clip == m.anim.clip);
        if let Some(f) = self.faces.iter().find(|f| f.clip == m.anim.clip) {
            let mut frame = m.anim.frame;
            if f.pat.loop_mode == 2 && f.pat.length > 0 {
                frame %= f.pat.length as f32;
            }
            for (i, t) in f.pat.tracks.iter().enumerate() {
                if let (Some(&mat), Some(tex)) = (f.mats.get(i), f.pat.tex_no(i, frame))
                    && (t.tex_map as usize) < 8
                    && mat != usize::MAX
                {
                    body_inputs.entry(mat).or_default().tex[t.tex_map as usize] = Some(tex);
                }
            }
        }
        let mut hidden = vec![HIDDEN_SHAPE];
        if !self.shirt {
            hidden.push(SHIRT_SHAPE);
        }
        let return_landing = m.anim.clip == "demo_gate_out_get2";
        let hands = if return_landing { Hands::Fists } else { hands_for_clip(m.anim.clip) };
        if hands != Hands::Body {
            hidden.extend(BODY_HAND_SHAPES);
        }

        // FLUDD on the chest, tilted back as the chest leans (`TWaterGun::setBaseTRMtx`:
        // `|m[1][0]| * (unk1D04 - unk1D06) + unk1D06`, 0 and -0x1800), posed by the sim's clip;
        // the nozzle on `nozzle_center` with its own clip and the aiming callbacks.
        let deg = |a: f32| a * std::f32::consts::PI / 32768.0;
        // Luigi's HSD body: posed by the same clip through the retarget, anchored to Mario's
        // lowest point, and his chest frame for FLUDD. None when the skin is Mario or the Melee
        // cache never loaded.
        let mut luigi_world: Option<Vec<Mat4>> = None;
        let mut luigi_chest: Option<Mat4> = None;
        // A pose override (the Melee A attacks on Mario's rig) replaces the clip, so Luigi takes
        // it from the drawn joints rather than the sim's clip, as Yoshi's retarget does.
        let overridden = self
            .pose_override
            .as_ref()
            .is_some_and(|p| p.len() == joints.len());
        if self.skin == Skin::Luigi
            && let Some(luigi) = self.luigi.as_mut()
            && let Some(mut world) = if overridden {
                Some(luigi.pose_joints(&joints))
            } else {
                luigi.pose(m.anim.clip, m.anim.frame_before(back))
            }
        {
            // Luigi's rig carries Mario's clip as rotations only, so it loses the root motion
            // the clips carry (the dive's drop, the hang's drop, the idle's bob). Anchor his
            // lowest point to the drawn Mario's: feet to feet, belly to belly.
            let mario_lowest = body_lowest(&self.models[models::BODY as usize], &joints);
            let anchor = (mario_lowest - luigi.lowest(&world) * 10.0) * 0.1;
            for mtx in &mut world {
                mtx.w_axis.y += anchor;
            }
            // Hanging, he sits a touch lower and further off the wall than the anchor puts him.
            if m.status == libsms::Status::Hanging {
                for mtx in &mut world {
                    mtx.w_axis.y -= LUIGI_HANG_OFFSET.0 * 0.1;
                    mtx.w_axis.z -= LUIGI_HANG_OFFSET.1 * 0.1;
                }
            }
            luigi_chest = Some(luigi.chest(&world));
            luigi_world = Some(world);
        }
        // Mario's chest, Luigi's `WaistN`-aligned chest, or Yoshi's `jnt_neck_1`, carrying FLUDD.
        // Luigi's, like Mario's, only exists while FLUDD is fitted (`look` is None without it): a
        // Luigi with no FLUDD must not keep wearing the pack.
        let chest_frame = yoshi_chest
            .or(luigi_chest.filter(|_| look.is_some()))
            .or_else(|| look.and(self.chest).and_then(|c| joints.get(c)).copied());
        let fludd_base = chest_frame.map(|chest| {
            if self.skin == Skin::Yoshi {
                // The pack follows Yoshi's authored centred neck; the chest lean is Mario's.
                chest
            } else {
                let lean = chest.x_axis.truncate().normalize_or_zero().y.abs();
                chest * Mat4::from_rotation_z(deg(lean * 6144.0 - 6144.0))
            }
        });
        let fludd = &self.models[models::FLUDD as usize].bmd;
        let mut fludd_joints = fludd.pose(
            |j| {
                look.and_then(|l| self.fludd_clips.get(l.body_clip))
                    .and_then(|c| c.sample(j, look.map_or(0.0, |l| l.body_frame)))
            },
            |_| None,
        );
        // The Spray neck leans out of the way of Luigi's taller head, to the side he aims past;
        // the frame rod (shape 3) hides while it is swung out. The game's Mario keeps the
        // authored neck.
        let mut fludd_hidden: Vec<usize> = Vec::new();
        // Only Luigi's taller head needs it; a preloaded Luigi must not lean Mario's or another
        // skin's neck.
        if self.skin == Skin::Luigi
            && let Some(luigi) = self.luigi.as_mut()
        {
            let deployed = m.fludd.as_ref().is_some_and(|f| f.housed < 0.5);
            if luigi.lean_fludd(&mut fludd_joints, look, deployed, dt) {
                fludd_hidden.push(LUIGI_ROD_SHAPE);
            }
        }
        // Yoshi's hips are wider than Mario's: only the belt (`_mat_froater`, shape 1, rigid on
        // `G_center`) is widened in its ring plane, as the viewer does. Tank and straps stay.
        let belt_xform = if self.skin == Skin::Yoshi {
            fludd_base.and_then(|base| {
                fludd_joints.get(1).map(|joint| {
                    let belt = base * *joint;
                    belt * Mat4::from_scale(Vec3::new(1.0, 1.5, 1.5)) * belt.inverse()
                })
            })
        } else {
            None
        };
        let belt_normal = belt_xform.map(|m| m.inverse().transpose());
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
                            // The host draws every model mirrored across its own X, so the
                            // drawn nozzle leaves from -x; the water must leave from the same
                            // place or it sprays out of the other side of the leaning neck.
                            let p = mm.w_axis.truncate();
                            muzzles.push((Vec3::new(-p.x, p.y, p.z), mm.x_axis.truncate()));
                            muzzle_mtx.push(mm);
                        }
                    }
                }
                nozzle_at.map(|at| (model, at, js))
            }
            _ => None,
        };
        // A host that drew FLUDD itself (an alt-skin with its own pack) knows where the muzzles
        // ended up; the mist and the water take those instead of this posed nozzle.
        if !self.host_muzzles.is_empty() {
            muzzle_mtx.clone_from(&self.host_muzzles);
            muzzles = muzzle_mtx
                .iter()
                .map(|m| (m.w_axis.truncate(), m.x_axis.truncate()))
                .collect();
        }
        // The juice leaves Yoshi's `null_tongue` instead of FLUDD's nozzle while the mouth
        // route is active; the fitted Spray still supplies the drop parameters.
        if mouth_route
            && let (Some(mouth), Some(route)) = (yoshi_mouth, route)
        {
            muzzle_mtx = vec![mouth];
            muzzles = vec![(mouth.w_axis.truncate(), route.direction())];
        }

        // The Turbo corkscrew dive: the spin jump's motion tipped head first and spun about his own length,
        // through about his middle.
        let tilt = Quat::from_rotation_x(m.model_pitch as f32 * std::f32::consts::PI / 32768.0)
            * Quat::from_rotation_y(m.model_roll as f32 * std::f32::consts::PI / 32768.0);
        let middle = Vec3::new(0.0, libsms::mario::HEIGHT * 0.35, 0.0);
        let root = Mat4::from_rotation_translation(rot * tilt, pos + rot * (middle - tilt * middle));
        // Hosts which only pose the view still get the state-driven super pound flames.
        if self.fx_ticks.is_empty() {
            self.fx_ticks.push(FxTick {
                burning: m.on_fire(),
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
            let eye = self.vr_eye;
            for &(slot, at) in &self.fx_binds {
                if let Some(mtx) = fx_matrix(at, tick_root, &joints, self.center, &muzzle_mtx) {
                    let mtx = if eye_slot(slot) { at_eye(mtx, tick.pos, eye) } else { mtx };
                    self.fx.rebind(slot, mtx);
                }
            }
            for cue in tick.cues {
                let Some(mtx) = fx_matrix(cue.at, tick_root, &joints, self.center, &muzzle_mtx) else {
                    continue;
                };
                let mtx = if VR_EYE_EFFECTS.contains(&cue.name) { at_eye(mtx, tick.pos, eye) } else { mtx };
                if matches!(cue.at, FxAt::Once { .. }) {
                    self.fx.emit(cue.name, mtx, cue.scale);
                } else if matches!(cue.at, FxAt::MuzzleFollow(_) | FxAt::FeetFollow | FxAt::JointFollow(_)) {
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
            if !tick.burning {
                // Host presentation tail: stop flare/smoke within 0.35 s of recovery.
                self.fx.fade_out("ms_firedmg_a", 10.0);
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

        // Mario's own parts: body, cap and hands. Luigi's body is the HSD mesh drawn below;
        // Yoshi's is his own rig. Their FLUDD and nozzle are the same models, on their chests.
        let luigi_body = self.skin == Skin::Luigi;
        let yoshi_body = self.skin == Skin::Yoshi && yoshi_world.is_some();
        let rig_body = self.rig_body();
        let mut parts: Vec<(u32, Mat4, &[Mat4])> = Vec::new();
        if let Some((model, _)) = rig_body {
            // A same-rig skin: its model on Mario's joints, cap and hands included.
            parts.push((model, Mat4::IDENTITY, &joints));
            if let (true, Some(hm), Some(h)) = (self.helmet_on, self.helmet, self.head.and_then(|h| joints.get(h))) {
                parts.push((hm, *h, &self.models[hm as usize].bind));
            }
            // Shadow Mario's brush on `jnt_hand_L` (`TEnemyMario::perform`).
            if let (Skin::Shadow, Some(bm), Some(h)) = (self.skin, self.brush, self.hand_joints[1].and_then(|h| joints.get(h))) {
                parts.push((bm, *h, &self.models[bm as usize].bind));
            }
        } else if !luigi_body && !yoshi_body {
            parts.push((models::BODY, Mat4::IDENTITY, &joints));
            if let Some(h) = self.head.and_then(|h| joints.get(h)) {
                let cap = if return_landing && m.anim.frame < 157. { self.return_parts.map_or(models::CAP, |p| p[0]) } else { models::CAP };
                parts.push((
                    cap,
                    *h,
                    &self.models[models::CAP as usize].bind,
                ));
            }
            if let (true, Some(hm), Some(h)) = (self.helmet_on, self.helmet, self.head.and_then(|h| joints.get(h))) {
                parts.push((hm, *h, &self.models[hm as usize].bind));
            }
            let set = match hands {
                Hands::Open => Some(models::HAND_OPEN),
                Hands::Fists => Some(models::FIST),
                Hands::Body => None,
            };
            if let Some(set) = set {
                for (model, joint) in set.into_iter().zip(self.hand_joints) {
                    let model = if return_landing && model == models::FIST[0] && m.anim.frame < 158. { self.return_parts.map_or(model, |p| p[1]) } else { model };
                    if let Some(at) = joint.and_then(|j| joints.get(j)) {
                        parts.push((model, *at, &self.models[model as usize].bind));
                    }
                }
            }
        }
        if let (Some(world), Some(y)) = (&yoshi_world, self.yoshi.as_ref()) {
            parts.push((y.model, Mat4::IDENTITY, world));
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

        // The host's attachment points: the same pose the body is skinned from, in model
        // space (the host places the body at its feet and mirrors it, as for the draws).
        let mario = |j: usize| joints.get(j).copied();
        let mut anchors = [None; ANCHOR_COUNT];
        if let Some(world) = &yoshi_world {
            let y = |j: usize| world.get(j).copied();
            anchors[Anchor::Head as usize] = y(22);
            anchors[Anchor::Chest as usize] = y(19);
            anchors[Anchor::Pelvis as usize] = y(3);
            anchors[Anchor::ShoulderR as usize] = y(28);
            anchors[Anchor::ShoulderL as usize] = y(33);
            anchors[Anchor::ElbowR as usize] = y(30);
            anchors[Anchor::ElbowL as usize] = y(35);
            anchors[Anchor::HandR as usize] = y(31);
            anchors[Anchor::HandL as usize] = y(36);
            anchors[Anchor::LegR as usize] = y(6);
            anchors[Anchor::LegL as usize] = y(11);
            anchors[Anchor::KneeR as usize] = y(7);
            anchors[Anchor::KneeL as usize] = y(12);
            anchors[Anchor::FootR as usize] = y(9);
            anchors[Anchor::FootL as usize] = y(14);
        } else if let (Some(world), Some(luigi)) = (&luigi_world, self.luigi.as_ref()) {
            // His body is skinned at ×10 (Melee units into Sunshine decomp units).
            let ten = Mat4::from_scale(Vec3::splat(10.0));
            let l = |part: usize| luigi.parts.get(part).and_then(|&j| world.get(j as usize)).map(|m| ten * *m);
            anchors[Anchor::Head as usize] = l(34);
            anchors[Anchor::Chest as usize] = l(16);
            anchors[Anchor::Pelvis as usize] = l(4);
            anchors[Anchor::ShoulderL as usize] = l(19);
            anchors[Anchor::ShoulderR as usize] = l(37);
            anchors[Anchor::ElbowL as usize] = l(20);
            anchors[Anchor::ElbowR as usize] = l(38);
            anchors[Anchor::HandL as usize] = l(21);
            anchors[Anchor::HandR as usize] = l(39);
            anchors[Anchor::LegL as usize] = l(7);
            anchors[Anchor::LegR as usize] = l(12);
            anchors[Anchor::KneeL as usize] = l(8);
            anchors[Anchor::KneeR as usize] = l(13);
            anchors[Anchor::FootL as usize] = l(10);
            anchors[Anchor::FootR as usize] = l(15);
        } else {
            // Mario and Piantissimo (his model is on Mario's exact 29-joint rig).
            anchors[Anchor::Head as usize] = self.head.and_then(|h| mario(h));
            anchors[Anchor::Chest as usize] = self.chest.and_then(|c| mario(c));
            anchors[Anchor::Pelvis as usize] = mario(2);
            anchors[Anchor::ShoulderR as usize] = mario(15);
            anchors[Anchor::ShoulderL as usize] = mario(20);
            anchors[Anchor::ElbowR as usize] = mario(17);
            anchors[Anchor::ElbowL as usize] = mario(23);
            anchors[Anchor::HandR as usize] = self.hand_joints[0].and_then(|h| mario(h));
            anchors[Anchor::HandL as usize] = self.hand_joints[1].and_then(|h| mario(h));
            anchors[Anchor::LegR as usize] = mario(3);
            anchors[Anchor::LegL as usize] = mario(8);
            anchors[Anchor::KneeR as usize] = mario(4);
            anchors[Anchor::KneeL as usize] = mario(9);
            anchors[Anchor::FootR as usize] = mario(6);
            anchors[Anchor::FootL as usize] = mario(11);
        }
        self.anchors = anchors;

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
                    || (model == models::FLUDD && fludd_hidden.contains(&b.shape))
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
                    let (mut p, mut n) = pose.skin(v);
                    if model == models::FLUDD && b.shape == 1
                        && let (Some(m), Some(nm)) = (belt_xform, belt_normal)
                    {
                        p = m.transform_point3(p);
                        n = nm.transform_vector3(n);
                    }
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
                let key = rig_body.filter(|(m, _)| *m == model).map_or(model, |(_, k)| k);
                let mut inputs = base_inputs(key);
                if model == models::BODY
                    && let Some(i) = body_inputs.get(&b.material)
                {
                    inputs.tex = i.tex;
                }
                // His dirt (`TMario::addDirty`: konst 0's alpha on the body, cap and hands).
                if inputs.k0_alpha.is_some() {
                    inputs.k0_alpha = Some(dirt);
                }
                let swirl = self.shadow.as_ref().is_some_and(|s| s.0 == model);
                let tex = if inputs == base_inputs(key) || swirl {
                    mdl.batch_texture[bi]
                } else {
                    self.textures.get(mdl, key, b.material, &inputs)
                };
                self.texture_ids.push(tex);
                let mut color = mdl.batch_color[bi];
                if hot && rig_body.is_some_and(|(rig, _)| rig == model) {
                    color = [
                        color[0] * PIANTISSIMO_HOT_FLUSH[0],
                        color[1] * PIANTISSIMO_HOT_FLUSH[1],
                        color[2] * PIANTISSIMO_HOT_FLUSH[2],
                        color[3],
                    ];
                }
                self.colors.push(color);
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
        // Luigi's HSD body: CPU-skinned into the same buffers as the parts, ×10 from Melee
        // units into Sunshine's decomp units, drawn as one instance at the index after the J3D
        // models (dynamic, so the entrance's own extra models keep their earlier indices).
        if self.vr_arms && !luigi_body {
            match (&yoshi_world, self.yoshi.as_ref()) {
                (Some(world), Some(y)) if yoshi_body => {
                    let model = y.model;
                    self.push_vr_arms(model, world, flags, None);
                }
                _ => {
                    let body = rig_body.map_or(models::BODY, |(m, _)| m);
                    self.push_vr_arms(body, &joints, flags, rig_body.is_none().then_some(&hidden[..]));
                }
            }
        }
        if let (Some(luigi), Some(world)) = (self.luigi.as_ref(), luigi_world.as_ref()) {
            let (first_vertex, first_color) = luigi.skin_into(
                world,
                &mut self.positions,
                &mut self.normals,
                &mut self.uvs,
                &mut self.colors,
                &mut self.texture_ids,
                hot,
            );
            let luigi_model = self.models.len() as u32;
            self.draws.push(Draw {
                key: (luigi_model as u64) << 56,
                model: luigi_model,
                kind: luigi_model as u8,
                flags,
                pos: draw_pos,
                rot: draw_rot,
                first_vertex,
                first_color,
                alpha: 1.0,
            });
            if self.vr_arms {
                let world = world.clone();
                self.push_vr_arms_luigi(luigi_model, first_vertex, first_color, &world, flags);
            }
        }
        if let Some(f) = &sim.mario.fludd {
            let kind = f.nozzle;
            sim.mario.set_emitters(kind, &muzzles);
        }
        sim.mario.juice_mouth = mouth_route;
    }
}

/// The low byte of a VR arm draw's key ([`View::set_vr_arms`]): + 0 right, + 1 left.
pub const VR_ARM_KEY: u64 = 0x10;
/// The low byte of the VR body draw's key: the body without its head and forearms
/// (libcommon's `vr_arms::torso_map`), drawn where the body is, for a host's first person.
pub const VR_BODY_KEY: u64 = VR_ARM_KEY + 2;
/// [`View::vr_cuts`]' key for Luigi's HSD body (not one of the J3D models).
const LUIGI_CUT: u32 = u32::MAX;

/// A body's VR arms: libcommon's cut, and each side's vertex map over the body's draw.
struct VrCut {
    arms: libcommon::vr_arms::Arms,
    maps: Vec<(u8, Vec<u32>)>,
    /// The VR body's vertex map (no head, no forearms), for a body with a head joint we know.
    torso: Option<Vec<u32>>,
}

impl VrCut {
    fn new(mesh: &libcommon::vr_arms::Mesh, arms: libcommon::vr_arms::Arms, head: Option<usize>) -> VrCut {
        let maps = arms.arms.iter().map(|a| (a.side, a.vertex_map(&mesh.triangles, mesh.positions.len()))).collect();
        let torso = head.map(|h| libcommon::vr_arms::torso_map(mesh, &arms, h));
        VrCut { arms, maps, torso }
    }
}

impl View {
    /// Each skin's VR-arm cut name in the asset cache, and its rig.
    fn vr_cut_name(&self, model: u32) -> Option<(String, crate::vr_arms::Rig)> {
        if model == models::BODY {
            return Some(("sms-mario".into(), crate::vr_arms::MARIO));
        }
        if self.yoshi.as_ref().is_some_and(|y| y.model == model) {
            return Some(("sms-yoshi".into(), crate::vr_arms::MARIO));
        }
        let (skin, _) = self.rig_bodies.iter().find(|(_, m)| *m == model)?;
        Some((format!("sms-{}", format!("{skin:?}").to_ascii_lowercase()), crate::vr_arms::MARIO))
    }

    /// Reads (or cuts) a body's VR arms once.
    fn ensure_vr_cut(&mut self, model: u32) -> bool {
        if let Some((_, c)) = self.vr_cuts.iter().find(|(m, _)| *m == model) {
            return c.is_some();
        }
        let cut = if model == LUIGI_CUT {
            self.luigi.as_ref().map(|l| {
                let (mesh, sides) = l.vr_mesh();
                let (arms, _) = libcommon::vr_arms::Arms::load_or_cut(&self.assets_dir, "sms-luigi", &mesh, sides);
                VrCut::new(&mesh, arms, None)
            })
        } else {
            // Mario's head joint; Piantissimo is on his exact rig. Yoshi's isn't, and keeps no body.
            let head = if self.yoshi.as_ref().is_some_and(|y| y.model == model) { None } else { self.head };
            match (self.vr_cut_name(model), self.models.get(model as usize)) {
                (Some((name, rig)), Some(mdl)) => {
                    let (mesh, arms, _) = crate::vr_arms::load_or_cut(&self.assets_dir, &name, &mdl.bmd, &mdl.batches, rig);
                    Some(VrCut::new(&mesh, arms, head))
                }
                _ => None,
            }
        };
        let ok = cut.is_some();
        self.vr_cuts.push((model, cut));
        ok
    }

    /// First run: reads or cuts every loaded skin's VR arms into the asset cache (load the skins
    /// first: [`View::load_skin`]). One note per body: its name and each side's triangles.
    pub fn bake_vr_arms(&mut self) -> Vec<String> {
        let mut bodies = vec![models::BODY];
        bodies.extend(self.rig_bodies.iter().map(|(_, m)| *m));
        bodies.extend(self.yoshi.as_ref().map(|y| y.model));
        if self.luigi.is_some() {
            bodies.push(LUIGI_CUT);
        }
        let mut notes = Vec::new();
        for model in bodies {
            if self.ensure_vr_cut(model)
                && let Some((_, Some(c))) = self.vr_cuts.iter().find(|(m, _)| *m == model)
            {
                let tris: Vec<usize> = c.arms.arms.iter().map(|a| a.triangles.len()).collect();
                notes.push(format!("{}: {tris:?}", c.arms.name));
            }
        }
        notes
    }

    /// Every loaded skin's body (as cut) and its VR arms, for a viewer's proof pictures.
    pub fn vr_bodies(&mut self) -> Vec<(libcommon::vr_arms::Mesh, libcommon::vr_arms::Arms)> {
        let mut out = Vec::new();
        let mut bodies = vec![models::BODY];
        bodies.extend(self.rig_bodies.iter().map(|(_, m)| *m));
        bodies.extend(self.yoshi.as_ref().map(|y| y.model));
        for model in bodies {
            if let (Some((name, rig)), Some(mdl)) = (self.vr_cut_name(model), self.models.get(model as usize)) {
                let (mesh, arms, _) = crate::vr_arms::load_or_cut(&self.assets_dir, &name, &mdl.bmd, &mdl.batches, rig);
                out.push((mesh, arms));
            }
        }
        if let Some(l) = &self.luigi {
            let (mesh, sides) = l.vr_mesh();
            let (arms, _) = libcommon::vr_arms::Arms::load_or_cut(&self.assets_dir, "sms-luigi", &mesh, sides);
            out.push((mesh, arms));
        }
        out
    }

    /// Takes a body's cut out of the view for a draw (put back with [`View::put_vr_cut`]).
    fn take_vr_cut(&mut self, model: u32) -> Option<VrCut> {
        if !self.ensure_vr_cut(model) {
            return None;
        }
        self.vr_cuts.iter_mut().find(|(m, _)| *m == model).and_then(|(_, c)| c.take())
    }

    fn put_vr_cut(&mut self, model: u32, cut: VrCut) {
        if let Some((_, c)) = self.vr_cuts.iter_mut().find(|(m, _)| *m == model) {
            *c = Some(cut);
        }
    }

    /// One arm draw a side from the body's posed vertices (`skinned`, in the body's vertex order;
    /// None for a vertex that isn't shown), each taken into its arm space by the wrist's matrix
    /// in `joints`; uvs from the body's own draw at `body_vertex`, colours and textures copied
    /// from `body_color` for `colors` batches.
    #[allow(clippy::too_many_arguments)]
    fn push_arm_draws(&mut self, cut: &VrCut, model: u32, skinned: &[Option<(Vec3, Vec3)>], joints: &[Mat4],
        body_vertex: usize, body_color: usize, colors: usize, flags: u8) {
        let body_key = (model as u64) << 56;
        for arm in &cut.arms.arms {
            let (Some(wrist), Some((_, map))) = (joints.get(arm.wrist as usize), cut.maps.iter().find(|(s, _)| *s == arm.side)) else { continue };
            let to_arm = Mat4::from_cols_array(&arm.to_arm(&wrist.to_cols_array()));
            let first_vertex = self.positions.len() as u32;
            let first_color = self.colors.len() as u32;
            for (i, _) in skinned.iter().enumerate() {
                let src = map.get(i).copied().unwrap_or(libcommon::vr_arms::COLLAPSED);
                match skinned.get(src as usize).copied().flatten() {
                    Some((p, n)) => {
                        self.positions.push(to_arm.transform_point3(p));
                        self.normals.push(to_arm.transform_vector3(n).normalize_or_zero());
                    }
                    None => {
                        self.positions.push(Vec3::ZERO);
                        self.normals.push(Vec3::Y);
                    }
                }
                let uv = self.uvs.get(body_vertex + i).copied().unwrap_or([0.0, 0.0]);
                self.uvs.push(uv);
            }
            for c in 0..colors {
                let tex = self.texture_ids.get(body_color + c).copied().unwrap_or(-1);
                let color = self.colors.get(body_color + c).copied().unwrap_or([1.0; 4]);
                self.texture_ids.push(tex);
                self.colors.push(color);
            }
            self.draws.push(Draw {
                key: body_key | (VR_ARM_KEY + arm.side as u64),
                model,
                kind: model as u8,
                flags,
                pos: Vec3::ZERO,
                rot: Quat::IDENTITY,
                first_vertex,
                first_color,
                alpha: 1.0,
            });
        }
    }

    /// The VR arms of the J3D body `model`, posed by `joints`, after its own draw this frame.
    /// `hidden`: Mario's own body hides these shapes as his draw does, but keeps his gloves
    /// (BODY_HAND_SHAPES) whichever hands the clip shows.
    fn push_vr_arms(&mut self, model: u32, joints: &[Mat4], flags: u8, hidden: Option<&[usize]>) {
        let body_key = (model as u64) << 56;
        let Some((body_vertex, body_color, body_pos, body_rot)) = self
            .draws
            .iter()
            .rev()
            .find(|d| d.key == body_key)
            .map(|d| (d.first_vertex as usize, d.first_color as usize, d.pos, d.rot))
        else {
            return;
        };
        let Some(cut) = self.take_vr_cut(model) else { return };
        let (skinned, colors) = match self.models.get(model as usize) {
            Some(mdl) => {
                let bmd = &mdl.bmd;
                let pose = Pose::new(bmd, Mat4::IDENTITY, joints);
                let mut out = Vec::with_capacity(mdl.vertex_count);
                for b in &mdl.batches {
                    let shown = bmd.materials[b.material].cull != Cull::All
                        && hidden.is_none_or(|h| !(h.contains(&b.shape) && !BODY_HAND_SHAPES.contains(&b.shape)));
                    out.extend(b.vertices.iter().map(|v| shown.then(|| pose.skin(v))));
                }
                (out, mdl.batches.len())
            }
            None => (Vec::new(), 0),
        };
        self.push_arm_draws(&cut, model, &skinned, joints, body_vertex, body_color, colors, flags);
        if let Some(map) = &cut.torso {
            self.push_vr_body(model, map, body_vertex, body_color, colors, body_pos, body_rot, flags);
        }
        self.put_vr_cut(model, cut);
    }

    /// The VR body draw ([`VR_BODY_KEY`]): the body's own posed vertices this frame (its draw at
    /// `body_vertex`, as it stands, hidden shapes and all), through `map`, placed where the body is.
    #[allow(clippy::too_many_arguments)]
    fn push_vr_body(&mut self, model: u32, map: &[u32], body_vertex: usize, body_color: usize, colors: usize,
        pos: Vec3, rot: Quat, flags: u8) {
        let first_vertex = self.positions.len() as u32;
        let first_color = self.colors.len() as u32;
        for (i, &src) in map.iter().enumerate() {
            let from = (src != libcommon::vr_arms::COLLAPSED).then_some(body_vertex + src as usize);
            let p = from.and_then(|f| self.positions.get(f).copied());
            let n = from.and_then(|f| self.normals.get(f).copied());
            self.positions.push(p.unwrap_or(Vec3::ZERO));
            self.normals.push(n.unwrap_or(Vec3::Y));
            let uv = self.uvs.get(body_vertex + i).copied().unwrap_or([0.0, 0.0]);
            self.uvs.push(uv);
        }
        for c in 0..colors {
            let tex = self.texture_ids.get(body_color + c).copied().unwrap_or(-1);
            let color = self.colors.get(body_color + c).copied().unwrap_or([1.0; 4]);
            self.texture_ids.push(tex);
            self.colors.push(color);
        }
        self.draws.push(Draw {
            key: ((model as u64) << 56) | VR_BODY_KEY,
            model,
            kind: model as u8,
            flags,
            pos,
            rot,
            first_vertex,
            first_color,
            alpha: 1.0,
        });
    }

    /// Luigi's VR arms from his own skinned draw (`first_vertex`, `first_color`), posed by
    /// `world` (Melee units; his vertices are ×10).
    fn push_vr_arms_luigi(&mut self, model: u32, first_vertex: u32, first_color: u32, world: &[Mat4], flags: u8) {
        let Some(cut) = self.take_vr_cut(LUIGI_CUT) else { return };
        let (count, colors) = self.luigi.as_ref().map_or((0, 0), |l| (l.vertex_count, l.batches.len()));
        let fv = first_vertex as usize;
        let skinned: Vec<Option<(Vec3, Vec3)>> = (0..count)
            .map(|i| Some((self.positions.get(fv + i).copied()?, self.normals.get(fv + i).copied()?)))
            .collect();
        let ten = Mat4::from_scale(Vec3::splat(10.0));
        let joints: Vec<Mat4> = world.iter().map(|m| ten * *m).collect();
        self.push_arm_draws(&cut, model, &skinned, &joints, fv, first_color as usize, colors, flags);
        self.put_vr_cut(LUIGI_CUT, cut);
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

    /// The Melee cache (`SSBM_ASSETS`, or the sibling libssbm checkout's assets), if present.
    fn melee_dir() -> Option<std::path::PathBuf> {
        std::env::var_os("SSBM_ASSETS").map(std::path::PathBuf::from).or_else(|| {
            let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../libssbm/assets");
            p.is_dir().then_some(p)
        })
    }

    #[test]
    fn yoshi_skin_loads_his_rig_and_poses_his_own_ground_clips() {
        let Some(dir) = libsms::assets::find(None) else {
            eprintln!("SKIP: Sunshine assets absent");
            return;
        };
        let mut v = View::load(&dir).unwrap();
        assert!(v.load_skin(Skin::Yoshi, None), "Yoshi's body should load from the Sunshine assets");
        assert!(v.load_skin(Skin::Yoshi, None), "loading a skin twice must stay true");
        let y = v.yoshi.as_mut().unwrap();
        assert_eq!(v.models.len(), models::COUNT as usize + 1, "one body added to the table");
        let wait = y.pose_native("yoshi_wait", 0.0).expect("yoshi_wait");
        assert_eq!(wait.len(), 38, "Yoshi's 38 joints");
        assert!(wait.iter().all(|m| m.is_finite()));
        let run = y.pose_native("yoshi_run", 6.0).expect("yoshi_run");
        assert_eq!(run.len(), 38);
        // The retarget still carries Mario's air clips onto Yoshi.
        let mario = v.skeleton.bind_world();
        let jump = y.pose_retarget(&mario);
        assert_eq!(jump.len(), 38);
        let low = yoshi_skin::lowest(&v.models[y.model as usize], &jump);
        assert!(low.is_finite(), "a finite lowest point");
        assert!(y.chest(&jump).is_finite());
    }

    #[test]
    fn luigi_skin_loads_his_hsd_mesh_and_skins_the_wait_pose() {
        let Some(dir) = libsms::assets::find(None) else {
            eprintln!("SKIP: Sunshine assets absent");
            return;
        };
        let Some(melee) = melee_dir() else {
            eprintln!("SKIP: Melee cache absent");
            return;
        };
        let mut v = View::load(&dir).unwrap();
        assert!(v.set_luigi(&melee), "Luigi's HSD mesh should load from the Melee cache");
        let luigi = v.luigi.as_mut().unwrap();
        assert!(!luigi.batches.is_empty() && luigi.vertex_count > 0, "HSD batches and vertices");
        assert!(!luigi.indices.is_empty(), "HSD triangle indices");
        let world = luigi.pose("wait", 0.0).expect("the wait clip");
        assert!(world.len() >= 40, "Luigi's 61-joint rig, got {}", world.len());
        assert!(luigi.lowest(&world).is_finite(), "a finite lowest point");
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut uvs = Vec::new();
        let mut colors = Vec::new();
        let mut texture_ids = Vec::new();
        let (first_vertex, first_color) =
            luigi.skin_into(&world, &mut positions, &mut normals, &mut uvs, &mut colors, &mut texture_ids, false);
        assert_eq!((first_vertex, first_color), (0, 0));
        assert_eq!(positions.len(), luigi.vertex_count);
        assert_eq!(uvs.len(), luigi.vertex_count);
        assert_eq!(colors.len(), luigi.batches.len());
        assert_eq!(texture_ids.len(), luigi.batches.len());
        assert!(positions.iter().all(|p| p.is_finite()), "skinned vertices are finite");
        assert!(positions.iter().any(|p| p.y.abs() > 10.0), "the body is not collapsed to a point");
    }

    /// The Melee A attacks replace Mario's drawn pose (`set_pose_override`); Luigi is drawn from that
    /// pose too, not from the sim's clip underneath it. Renders a frame with and without an override
    /// and checks Luigi's drawn vertices are the override's retarget, anchored as usual.
    #[test]
    fn luigi_follows_the_pose_override() {
        let Some(dir) = libsms::assets::find(None) else {
            eprintln!("SKIP: Sunshine assets absent");
            return;
        };
        let Some(melee) = melee_dir() else {
            eprintln!("SKIP: Melee cache absent");
            return;
        };
        let mut v = View::load(&dir).unwrap();
        if !v.set_luigi(&melee) {
            eprintln!("SKIP: Luigi's Melee files absent");
            return;
        }
        v.set_skin(Skin::Luigi);
        let a = libsms::Assets::discover(None).unwrap();
        let mut sim = Sim::new(a.params, a.anim_table, Vec3::ZERO, 0);
        let mut host = libsms::FlatHost::default();
        let pad = libsms::Pad::default();
        for _ in 0..30 {
            sim.advance(&mut host, &pad, i16::MIN, 1.0 / 60.0);
        }
        let luigi_verts = |v: &View| -> Vec<Vec3> {
            let model = v.models.len() as u32;
            let d = v.draws.iter().find(|d| d.model == model).expect("Luigi is drawn");
            let n = v.luigi.as_ref().unwrap().vertex_count;
            v.positions[d.first_vertex as usize..d.first_vertex as usize + n].to_vec()
        };

        v.update(&mut sim, 1.0 / 60.0);
        let clip_pose = luigi_verts(&v);

        // Mario's bind pose stands in for a ported attack: a full joint set unlike the idle clip.
        let attack = v.skeleton.bind_world();
        assert!(v.set_pose_override(Some(attack.clone())), "the override has Mario's joint count");
        v.update(&mut sim, 1.0 / 60.0);
        let drawn = luigi_verts(&v);
        let moved = clip_pose.iter().zip(&drawn).map(|(a, b)| (*a - *b).length()).fold(0.0, f32::max);
        assert!(moved > 5.0, "Luigi still shows the clip under the override (max move {moved})");

        // The expected vertices: the override's retarget, skinned; the view only adds the anchor's
        // vertical shift (the same for every vertex).
        let luigi = v.luigi.as_ref().unwrap();
        let world = luigi.pose_joints(&attack);
        let (mut p, mut n, mut u, mut c, mut t) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
        luigi.skin_into(&world, &mut p, &mut n, &mut u, &mut c, &mut t, false);
        assert_eq!(p.len(), drawn.len());
        let shift = drawn[0] - p[0];
        assert!(shift.x.abs() < 1e-3 && shift.z.abs() < 1e-3, "only a vertical anchor: {shift:?}");
        for (d, e) in drawn.iter().zip(&p) {
            assert!((*d - *e - shift).length() < 1e-2, "vertex off the override's pose: {d:?} vs {e:?}");
        }

        // Cleared, he is back on the clip.
        assert!(v.set_pose_override(None));
        v.update(&mut sim, 1.0 / 60.0);
        let back = luigi_verts(&v);
        let off = clip_pose.iter().zip(&back).map(|(a, b)| (*a - *b).length()).fold(0.0, f32::max);
        assert!(off < moved * 0.5, "Luigi didn't return to the clip (max off {off})");
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

    /// Its gold body is environment mapped: its coordinates spread over the bake with the normals,
    /// not all on the bake's white-hot middle (it looked white in RoR2).
    #[test]
    fn the_shine_body_reads_its_env_map_from_its_normals() {
        let Some(mut v) = shine() else { return };
        v.set_camera(Vec3::X, Vec3::Y);
        v.update_shine(0.0, Vec3::ZERO, 0, 0, 0.0, 1.0, true);
        let m = &v.models[0];
        let body = (0..m.batches.len())
            .find(|&i| m.bmd.materials[m.batches[i].material].name.contains("body"))
            .expect("the Shine's body batch");
        assert!(m.batch_env[body].is_some(), "the body isn't env mapped");
        let first: usize = m.batches[..body].iter().map(|b| b.vertices.len()).sum();
        let uvs = &v.uvs[first..first + m.batches[body].vertices.len()];
        let (lo, hi) = uvs.iter().fold(([f32::MAX; 2], [f32::MIN; 2]), |(lo, hi), u| {
            ([lo[0].min(u[0]), lo[1].min(u[1])], [hi[0].max(u[0]), hi[1].max(u[1])])
        });
        assert!(hi[0] - lo[0] > 0.5 && hi[1] - lo[1] > 0.5, "env coordinates bunched: {lo:?}..{hi:?}");
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

    /// The diving helmet (`TMarioCap` model 2): drawn on the head only when worn, and its glass
    /// goes through the shared TEV bake and comes out a translucent batch, not an opaque one.
    #[test]
    fn the_diving_helmet_is_worn_on_the_head_with_translucent_glass() {
        let Some(mut v) = view() else {
            eprintln!("skipping: no assets/");
            return;
        };
        if !v.set_helmet(true) {
            eprintln!("SKIP: watergun2/body/diver_helm.bmd absent");
            return;
        }
        let hm = v.helmet.unwrap();
        let m = &v.models[hm as usize];
        let glass = m.bmd.materials.iter().position(|x| x.name == "_mat_diverhelm_helm").unwrap();
        let bi = m.batches.iter().position(|b| b.material == glass).unwrap();
        let (blend, color) = (m.batch_blend[bi], m.batch_color_of(bi));
        eprintln!("glass blend {blend} colour {color:?}");
        assert_eq!(blend, blend::TRANSLUCENT, "glass must blend");
        // The shared path hands a see-through batch's fade to hosts per vertex.
        let first: usize = m.batches[..bi].iter().map(|b| b.vertices.len()).sum();
        let fade = &m.vertex_alpha[first..first + m.batches[bi].vertices.len()];
        let (lo, hi) = fade.iter().fold((1.0f32, 0.0f32), |(l, h), &a| (l.min(a), h.max(a)));
        eprintln!("glass vertex alpha {lo}..{hi}");
        assert!(lo < 0.9 && color[3] <= 1.0, "glass is not see-through: {lo}..{hi}");
        let a = libsms::Assets::discover(None).unwrap();
        let mut sim = Sim::new(a.params, a.anim_table, Vec3::ZERO, 0);
        let mut host = libsms::FlatHost::default();
        let pad = libsms::Pad::default();
        sim.advance(&mut host, &pad, i16::MIN, 1.0 / 60.0);
        v.update(&mut sim, 1.0 / 60.0);
        assert!(v.draws.iter().any(|d| d.model == hm), "worn helmet draws");
        v.set_helmet(false);
        v.update(&mut sim, 1.0 / 60.0);
        assert!(!v.draws.iter().any(|d| d.model == hm), "taken off");
    }

    #[test]
    fn shadow_mario_swirls_and_holds_his_brush() {
        let Some(mut v) = view() else {
            eprintln!("skipping: no assets/");
            return;
        };
        assert!(v.load_skin(Skin::Shadow, None), "kagemario/default.bmd is on Mario's rig");
        v.set_skin(Skin::Shadow);
        let (body, _) = v.rig_body().expect("his body is loaded");
        let brush = v.brush.expect("his brush is loaded");
        let (_, swirl, _) = v.shadow.as_ref().map(|s| (s.0, s.1, ())).unwrap();
        assert!(v.models[body as usize].batch_texture.iter().all(|&t| t == swirl));
        let a = libsms::Assets::discover(None).unwrap();
        let mut sim = Sim::new(a.params, a.anim_table, Vec3::ZERO, 0);
        let mut host = libsms::FlatHost::default();
        let pad = libsms::Pad::default();
        let mut seen = Vec::new();
        for _ in 0..20 {
            sim.advance(&mut host, &pad, i16::MIN, 1.0 / 60.0);
            v.update(&mut sim, 1.0 / 60.0);
            seen.push((v.textures.version(swirl as u32), v.textures.list[swirl as usize].rgba.clone()));
        }
        let drawn: Vec<u32> = v.draws.iter().map(|d| d.model).collect();
        assert!(drawn.contains(&body) && drawn.contains(&brush), "{drawn:?}");
        assert!(!drawn.contains(&models::BODY) && !drawn.contains(&models::FLUDD), "{drawn:?}");
        // The swirl moves: new pixels, a new version, at the game's 30 frames a second.
        assert!(seen.first().unwrap().0 < seen.last().unwrap().0);
        assert_ne!(seen.first().unwrap().1, seen.last().unwrap().1, "the swirl doesn't move");
        // Mostly his blues: blue over red on average.
        let px = v.textures.list[swirl as usize].rgba.as_chunks::<4>().0;
        let (r, b) = px.iter().fold((0u64, 0u64), |(r, b), p| (r + p[0] as u64, b + p[2] as u64));
        assert!(b > r, "blue {b} red {r}");
        // The host's screen-grab pass: a refraction map that bends both ways and scrolls.
        let swirl_now = v.shadow_swirl().expect("his skin is drawn");
        let (side, map, gain) = swirl_now.refraction();
        assert_eq!(map.len(), (side * side * 4) as usize);
        assert!(gain.is_finite() && gain > 0.0);
        let xs: Vec<u8> = map.as_chunks::<4>().0.iter().map(|p| p[3]).collect();
        assert!(xs.iter().any(|&x| x < 100) && xs.iter().any(|&x| x > 156), "the warp is flat");
        let before = map.clone();
        let mut later = None;
        for _ in 0..10 {
            sim.advance(&mut host, &pad, i16::MIN, 1.0 / 30.0);
            v.update(&mut sim, 1.0 / 30.0);
            later = v.shadow_swirl().map(|s| s.refraction().1);
        }
        assert_ne!(later.unwrap(), before, "the refraction doesn't scroll");
        // The brush rides his left hand.
        let hand = v.anchor(Anchor::HandL as usize).unwrap().w_axis.truncate();
        let d = v.draws.iter().find(|d| d.model == brush).unwrap();
        let n = v.models[brush as usize].vertex_count;
        // A long stick held through the fist (its vertices are at its ends): his hand is inside
        // its posed bounds.
        let pts = &v.positions[d.first_vertex as usize..][..n];
        let lo = pts.iter().fold(Vec3::splat(f32::MAX), |a, p| a.min(*p)) - Vec3::splat(15.0);
        let hi = pts.iter().fold(Vec3::splat(f32::MIN), |a, p| a.max(*p)) + Vec3::splat(15.0);
        assert!(hand.cmpge(lo).all() && hand.cmple(hi).all(), "hand {hand} outside brush {lo}..{hi}");
    }

    #[test]
    fn goop_shows_on_mario_through_the_dirt_mask() {
        let Some(mut v) = view() else {
            eprintln!("skipping: no assets/");
            return;
        };
        let a = libsms::Assets::discover(None).unwrap();
        let mut sim = Sim::new(a.params, a.anim_table, Vec3::ZERO, 0);
        let mut host = libsms::FlatHost::default();
        let pad = libsms::Pad::default();
        let body_textures = |v: &View| {
            let d = v.draws.iter().find(|d| d.model == models::BODY).unwrap();
            let n = v.models[models::BODY as usize].batches.len();
            v.texture_ids[d.first_color as usize..][..n].to_vec()
        };
        sim.advance(&mut host, &pad, i16::MIN, 1.0 / 60.0);
        v.update(&mut sim, 1.0 / 60.0);
        let clean = body_textures(&v);
        sim.mario.add_dirty(255.0);
        sim.advance(&mut host, &pad, i16::MIN, 1.0 / 60.0);
        v.update(&mut sim, 1.0 / 60.0);
        let dirty = body_textures(&v);
        assert_ne!(clean, dirty, "goop changes his bakes");
        let changed = clean.iter().zip(&dirty).find(|(c, d)| c != d && **c >= 0 && **d >= 0).unwrap();
        let (c, d) = (&v.textures.list[*changed.0 as usize], &v.textures.list[*changed.1 as usize]);
        assert_ne!(c.rgba, d.rgba, "the dirty bake differs");
    }

    #[test]
    fn bowser_jr_and_the_goop_critters_load_as_posed_props() {
        let Some(dir) = libsms::assets::find(None) else {
            eprintln!("skipping: no assets/");
            return;
        };
        for (look, clip) in [
            (Look::KoopaJr, Some("koopajr_wait")),
            (Look::NameKuri, Some("name_walk")),
            (Look::NameKuriBrain, None),
            (Look::Pakkun, Some("pakun_shoot")),
            (Look::PakkunSeed, None),
            (Look::KoopaSign, Some("stamp_koopa_sign_draw1")),
            (Look::GateKeeper, Some("gene_pakkun_wait3")),
        ] {
            let mut v = View::load_look(&dir, look).unwrap_or_else(|e| panic!("{look:?}: {e}"));
            let (c, frames) = clip.map_or((-1, 0.0), |n| {
                let (i, f) = v.prop_clip(n).unwrap_or_else(|| panic!("{look:?} has {n}"));
                (i as i32, f)
            });
            v.update_prop(0, c, frames * 0.5, false);
            assert_eq!(v.positions.len(), v.models[0].vertex_count, "{look:?}");
            assert!(v.positions.iter().all(|p| p.is_finite()));
            assert!(v.texture_ids.iter().any(|&t| t >= 0), "{look:?} is textured");
        }
        let stu = View::load_look(&dir, Look::NameKuri).unwrap();
        let (walk, _) = stu.prop_clip("name_walk").unwrap();
        let brain = stu.prop_joint(walk as i32, 10.0, NAMEKURI_BRAIN_JOINT).unwrap();
        assert!(brain.w_axis.y > 20.0, "the brain rides on top: {}", brain.w_axis.y);
    }

    #[test]
    fn piantissimo_wears_marios_pose_on_his_own_model() {
        let Some(mut v) = view() else {
            eprintln!("skipping: no assets/");
            return;
        };
        if !v.load_skin(Skin::Piantissimo, None) {
            eprintln!("SKIP: Piantissimo's model absent (extract the optional pieces)");
            return;
        }
        let (rig_model, _) = {
            v.set_skin(Skin::Piantissimo);
            v.rig_body().expect("his body is loaded")
        };
        let a = libsms::Assets::discover(None).unwrap();
        let mut sim = Sim::new(a.params, a.anim_table, Vec3::ZERO, 0);
        sim.mario.give_fludd();
        let mut host = libsms::FlatHost::default();
        let pad = libsms::Pad { stick: libsms::Vec2::new(0.0, 1.0), ..Default::default() };
        for _ in 0..30 {
            sim.advance(&mut host, &pad, i16::MIN, 1.0 / 60.0);
            v.update(&mut sim, 1.0 / 60.0);
        }
        let models_drawn: Vec<u32> = v.draws.iter().map(|d| d.model).collect();
        assert!(models_drawn.contains(&rig_model), "{models_drawn:?}");
        assert!(!models_drawn.contains(&models::BODY), "Mario's body must not draw: {models_drawn:?}");
        assert!(!models_drawn.contains(&models::CAP), "his cap is part of his model: {models_drawn:?}");
        assert!(models_drawn.contains(&models::FLUDD), "FLUDD stays Mario's: {models_drawn:?}");
        assert!(v.positions.iter().all(|p| p.is_finite()));
        // Same joints as Mario, so he stands about as tall as Mario does.
        let draw = v.draws.iter().find(|d| d.model == rig_model).unwrap();
        let n = v.models[rig_model as usize].vertex_count;
        let top = v.positions[draw.first_vertex as usize..][..n].iter().map(|p| p.y).fold(f32::MIN, f32::max);
        assert!((100.0..260.0).contains(&top), "top of Piantissimo at {top}");
        // The texture cache key must not collide with Mario's body (each is its own bake).
        let total: usize = v.draws.iter().map(|d| v.models[d.model as usize].vertex_count).sum();
        assert_eq!(v.positions.len(), total);
    }

    #[test]
    fn yoshi_skin_draws_his_body_and_routes_a_front_spray_to_his_mouth() {
        let Some(mut v) = view() else {
            eprintln!("skipping: no assets/");
            return;
        };
        if !v.load_skin(Skin::Yoshi, None) {
            eprintln!("SKIP: Yoshi's model absent");
            return;
        }
        v.set_skin(Skin::Yoshi);
        let yoshi_model = v.yoshi.as_ref().unwrap().model;
        let a = libsms::Assets::discover(None).unwrap();
        let mut sim = Sim::new(a.params, a.anim_table, Vec3::ZERO, 0);
        sim.mario.give_fludd();
        sim.mario.options.yoshi_skin = true;
        let mut host = libsms::FlatHost::default();
        let pad = libsms::Pad { r_analog: 255, ..Default::default() };
        for _ in 0..30 {
            sim.advance(&mut host, &pad, i16::MIN, 1.0 / 60.0);
            v.update(&mut sim, 1.0 / 60.0);
        }
        let drawn: Vec<u32> = v.draws.iter().map(|d| d.model).collect();
        assert!(drawn.contains(&yoshi_model), "{drawn:?}");
        assert!(!drawn.contains(&models::BODY), "Mario's body must not draw under Yoshi: {drawn:?}");
        assert!(drawn.contains(&models::FLUDD), "FLUDD stays on Yoshi's back: {drawn:?}");
        assert!(v.positions.iter().all(|p| p.is_finite()));
        // The front Spray leaves the mouth (well above the belt), not FLUDD's nozzle.
        let (pos, _) = sim.mario.fludd.as_ref().unwrap().emitter(0).expect("the mouth emitter");
        assert!(pos.y > 80.0, "emitter at {pos:?} is not the mouth");
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
        let Some(dir) = libsms::assets::find(None) else {
            eprintln!("skipped: needs extracted Sunshine assets (SMS_ASSETS)");
            return;
        };
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

    /// Burning, Mario wears the hot face (`ma_hot_wait_tx`, `ma_firejmp_tx`, `ma_fjpend_tx`) in
    /// place of the open eyes of the idle blink, on the body's own face materials.
    /// Luigi's eyes (the two TObjs with image frames) wear the pained eye while he burns, and nothing
    /// else changes.
    #[test]
    fn the_burning_luigi_wears_the_pained_eye() {
        let Some(dir) = libsms::assets::find(None) else { eprintln!("SKIP: no assets/"); return };
        let Some(melee) = melee_dir() else { eprintln!("SKIP: Melee cache absent"); return };
        let mut v = View::load(&dir).unwrap();
        assert!(v.set_luigi(&melee));
        let luigi = v.luigi.as_mut().unwrap();
        let eyes = luigi.batches.iter().filter(|b| b.hot_texture != b.texture).count();
        assert_eq!(eyes, 2, "both eye batches have a hot image");
        let world = luigi.pose("wait", 0.0).unwrap();
        let skin = |hot| {
            let (mut p, mut n, mut u, mut c, mut t) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
            luigi.skin_into(&world, &mut p, &mut n, &mut u, &mut c, &mut t, hot);
            t
        };
        let (calm, hot) = (skin(false), skin(true));
        assert_eq!(calm.len(), hot.len());
        assert_eq!(calm.iter().zip(&hot).filter(|(a, b)| a != b).count(), 2, "only the two eyes change");
    }

    /// Piantissimo's smears and pound fire are his pink and brown, and Mario's and Luigi's are back
    /// when the skin changes (the palettes share one saved original).
    #[test]
    fn piantissimos_effects_wear_his_colours_and_switch_back() {
        let Some(mut v) = view() else { eprintln!("skipping: no assets/"); return };
        let prm = |v: &View, name: &str| v.fx.prm_of(name).expect("the effect is loaded");
        let mario = (prm(&v, "ms_m_blur3"), prm(&v, "ms_m_sphipd_a"));
        v.set_skin(Skin::Piantissimo);
        assert_eq!(prm(&v, "ms_m_blur3"), [213, 106, 188, 255], "the pound's smear is his pink");
        let fire = prm(&v, "ms_m_sphipd_a");
        assert!(fire[0] > fire[1] && fire[2] > fire[1] && fire[0] > fire[2], "the fire is pink: {fire:?}");
        v.set_skin(Skin::Luigi);
        assert_eq!(prm(&v, "ms_m_blur3")[1], mario.0[0], "Luigi's is green, from Mario's red");
        v.set_skin(Skin::Mario);
        assert_eq!((prm(&v, "ms_m_blur3"), prm(&v, "ms_m_sphipd_a")), mario);
    }

    /// A burning Piantissimo's colours are flushed, and only while a burning clip plays.
    #[test]
    fn the_burning_piantissimo_flushes_red() {
        let Some(mut v) = view() else { eprintln!("skipping: no assets/"); return };
        if !v.load_skin(Skin::Piantissimo, None) {
            eprintln!("SKIP: Piantissimo's model absent");
            return;
        }
        v.set_skin(Skin::Piantissimo);
        let (rig, _) = v.rig_body().expect("his body is loaded");
        let a = libsms::Assets::discover(None).unwrap();
        let mut sim = Sim::new(a.params, a.anim_table, Vec3::ZERO, 0);
        let mut host = libsms::FlatHost::default();
        let pad = libsms::Pad::default();
        let tint = |v: &View| {
            let d = v.draws.iter().find(|d| d.model == rig).expect("his body is drawn");
            let c = v.colors[d.first_color as usize];
            c[1] / c[0].max(0.001)
        };
        for _ in 0..30 {
            sim.advance(&mut host, &pad, i16::MIN, 1.0 / 60.0);
            v.update(&mut sim, 1.0 / 60.0);
        }
        let calm = tint(&v);
        assert!(sim.mario.ignite());
        for _ in 0..30 {
            sim.advance(&mut host, &pad, i16::MIN, 1.0 / 60.0);
            v.update(&mut sim, 1.0 / 60.0);
        }
        assert_eq!(sim.mario.anim.clip, "firejmp");
        assert!(tint(&v) < calm * 0.8, "no flush: {calm} -> {}", tint(&v));
    }

    #[test]
    fn the_burning_mario_wears_the_hot_face() {
        let Some(dir) = libsms::assets::find(None) else {
            eprintln!("skipped: needs extracted Sunshine assets (SMS_ASSETS)");
            return;
        };
        let v = View::load(&dir).unwrap();
        let wink = v.wink.as_ref().expect("no blink pattern");
        assert_eq!(v.faces.len(), FACE_PATTERNS.len(), "a hot face pattern did not load");
        for f in &v.faces {
            let mut changed = false;
            for (i, t) in f.pat.tracks.iter().enumerate() {
                if f.mats[i] == usize::MAX {
                    continue;
                }
                // The same material and slot as the blink's, but not the blink's textures.
                changed |= wink
                    .tracks
                    .iter()
                    .any(|w| w.material == t.material && w.tex_map == t.tex_map && w.frames != t.frames);
            }
            assert!(changed, "{} wears the same eyes as the idle face", f.clip);
        }
    }

    #[test]
    fn fire_damage_loads_and_draws_the_bound_emitter_six() {
        let Some(mut v) = view() else { eprintln!("SKIP: no assets/"); return };
        assert!(v.fx.has("ms_firedmg_a"), "emitter 6 was never loaded");
        let a = libsms::Assets::discover(None).unwrap();
        let mut sim = Sim::new(a.params, a.anim_table, Vec3::ZERO, 0);
        let mut host = Collect(Vec::new());
        sim.mario.options.fire_panic_run = true;
        sim.mario.ignite();
        for _ in 0..36 {
            sim.advance_observed(&mut host, &libsms::Pad::default(), 0, 1. / 120., |m, h| {
                v.events(&std::mem::take(&mut h.0), m);
            });
            v.set_camera(Vec3::X, Vec3::Y);
            v.update(&mut sim, 1. / 120.);
        }
        assert!(!v.fx.vertices.is_empty(), "the fire flare is invisible");
        assert!(v.fx_binds.iter().any(|(_, at)| *at == FxAt::FeetFollow), "the flare was left behind at the hit position");
    }

    #[test]
    fn nozzle_pickup_sparkles_follow_root_joint_for_the_authored_lifetime() {
        let Some(mut v)=view() else {eprintln!("SKIP: Sunshine cache absent");return;};
        let a=libsms::Assets::discover(None).unwrap();
        let mut sim=Sim::new(a.params,a.anim_table,Vec3::ZERO,0);
        sim.mario.give_fludd();sim.mario.equip_nozzle(2);
        let mut host=Collect(Vec::new());let mut saw=false;
        for i in 0..720 {
            if i==120 {sim.mario.pos.x+=600.0;sim.mario.prev_pos=sim.mario.pos;}
            sim.advance_observed(&mut host,&libsms::Pad::default(),0,1.0/120.0,|m,h|v.events(&std::mem::take(&mut h.0),m));
            v.update(&mut sim,1.0/120.0);
            saw|=!v.fx.vertices.is_empty();
        }
        assert!(saw,"no pickup sparkle was drawn");
        assert!(v.fx_binds.iter().any(|(_,at)| *at==FxAt::JointFollow(1)),"pickup did not follow root joint 1");
        assert!(!v.fx.active(),"one pickup's authored emitter never expired");
    }

    #[test]
    fn the_turbo_dash_draws_the_water_boost_cone_and_its_particles() {
        let Some(dir) = libsms::assets::find(None) else {
            eprintln!("skipped: needs extracted Sunshine assets (SMS_ASSETS)");
            return;
        };
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
        let Some(dir) = libsms::assets::find(None) else {
            eprintln!("skipped: needs extracted Sunshine assets (SMS_ASSETS)");
            return;
        };
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
        let Some(dir) = libsms::assets::find(None) else {
            eprintln!("skipped: needs extracted Sunshine assets (SMS_ASSETS)");
            return;
        };
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

    #[test]
    fn sambo_props_load_and_pose() {
        let Some(dir) = libsms::assets::find(None) else { eprintln!("SKIP: Sunshine assets absent"); return };
        if !dir.join("enemy/sambo/sambo.bmd").is_file() { eprintln!("SKIP: sambo models not extracted"); return }
        for (look, clip) in [
            (Look::Flower, "flower_hit"),
            (Look::Flower, "flower_fwait"),
            (Look::Sambo, "sambo_fset"),
            (Look::Sambo, "sambo_fattack"),
            (Look::SamboHead, "sambohead_jump_start"),
        ] {
            let mut v = View::load_look(&dir, look).unwrap();
            let (i, frames) = v.prop_clip(clip).unwrap_or_else(|| panic!("{look:?} lacks {clip}"));
            assert!(frames > 1.0);
            v.update_prop(0, -1, 0.0, false);
            let bind = v.positions.clone();
            v.update_prop(0, i as i32, frames * 0.5, false);
            assert!(!bind.is_empty() && bind.len() == v.positions.len());
            assert!(v.positions.iter().all(|p| p.is_finite()));
            assert!(v.positions != bind, "{clip} moved nothing");
        }
    }

    #[test]
    fn flower_effects_emit_and_run() {
        let Some(dir) = libsms::assets::find(None) else { eprintln!("SKIP: Sunshine assets absent"); return };
        if !dir.join("enemy/samboflower/flower.bmd").is_file() { eprintln!("SKIP: sambo models not extracted"); return }
        let mut v = View::load_look(&dir, Look::Effects).unwrap();
        let at = Vec3::new(100.0, 0.0, 50.0);
        for name in FLOWER_EFFECTS {
            v.emit_effect(name, at, 1.0);
        }
        let mut saw = false;
        for _ in 0..30 {
            v.step_effects(1.0 / 30.0);
            saw |= !v.fx.vertices.is_empty();
        }
        assert!(saw, "pollen, rocks and smoke should draw quads");
        assert!(v.fx.vertices.iter().all(|p| p.pos.is_finite()));
    }

    /// `TMultiBtk`: the goop flows over the Gatekeeper (its textures re-bake as the scrolls
    /// run, in place, with new versions for the host).
    #[test]
    fn the_gatekeepers_goop_scrolls() {
        let Some(dir) = libsms::assets::find(None) else { eprintln!("SKIP: Sunshine assets absent"); return };
        if !dir.join("enemy/gatekeeper/gene_pakkun_tex1.btk").is_file() { eprintln!("SKIP: gatekeeper not extracted"); return }
        let mut v = View::load_look(&dir, Look::GateKeeper).unwrap();
        assert_eq!(v.scroll.len(), 2, "tex0 and tex1");
        let ids: Vec<i32> = v.models[0].batch_texture.iter().copied().filter(|&i| i >= 0).collect();
        assert!(!ids.is_empty());
        assert!(ids.iter().all(|&i| v.textures.version(i as u32) != 0), "animated textures start at a nonzero version");
        let count = v.textures.list.len();
        v.scroll_textures(0.0);
        let before: Vec<Vec<u8>> = ids.iter().map(|&i| v.textures.list[i as usize].rgba.clone()).collect();
        let version = v.textures.version(ids[0] as u32);
        v.scroll_textures(3.0);
        assert!(ids.iter().enumerate().any(|(k, &i)| v.textures.list[i as usize].rgba != before[k]), "the goop should move");
        assert_ne!(version, v.textures.version(ids[0] as u32), "the host is told");
        assert_eq!(count, v.textures.list.len(), "re-baked in place");
    }

    #[test]
    fn goop_effects_load_from_their_folders_and_run() {
        let Some(dir) = libsms::assets::find(None) else { eprintln!("SKIP: Sunshine assets absent"); return };
        if !dir.join("enemy/gatekeeper/jpa/ms_gkpa_kemuri.jpa").is_file() { eprintln!("SKIP: goop effects not extracted"); return }
        let mut v = View::load_look(&dir, Look::Effects).unwrap();
        let mut drew = 0;
        for (name, _) in GOOP_EFFECTS {
            let mut one = View::load_look(&dir, Look::Effects).unwrap();
            one.emit_effect(name, Vec3::ZERO, 1.0);
            let mut saw = false;
            for _ in 0..40 {
                one.step_effects(1.0 / 30.0);
                saw |= !one.fx.vertices.is_empty();
            }
            // Directional ones (`ms_thunder_s`, the two smokes) face their travel and draw
            // nothing from a standing test emitter.
            if !saw { eprintln!("no quads from a standing emitter: {name}"); } else { drew += 1; }
            v.emit_effect(name, Vec3::ZERO, 1.0);
        }
        assert!(drew >= 17, "only {drew} goop effects drew");
        v.step_effects(0.5);
        assert!(v.fx.vertices.iter().all(|p| p.pos.is_finite()));
    }

    /// The host mounts item displays on these: every skin must put the head above the
    /// pelvis and have hands, in the same model space as the draws.
    #[test]
    fn item_anchors_sit_on_every_skins_body() {
        let Some(dir) = libsms::assets::find(None) else {
            eprintln!("skipped: needs extracted Sunshine assets (SMS_ASSETS)");
            return;
        };
        for skin in [Skin::Mario, Skin::Luigi, Skin::Piantissimo, Skin::Yoshi] {
            let a = libsms::Assets::discover(None).unwrap();
            let mut sim = Sim::new(a.params, a.anim_table, Vec3::ZERO, 0);
            let mut v = View::load(&dir).unwrap();
            let melee = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../libssbm/assets");
            let melee = melee.exists().then(|| melee.as_path());
            if skin != Skin::Mario && !v.load_skin(skin, melee) {
                eprintln!("{skin:?}: files missing, skipped");
                continue;
            }
            v.set_skin(skin);
            v.update(&mut sim, 0.0);
            let head = v.anchor(0).unwrap_or_else(|| panic!("{skin:?}: no head anchor"));
            let pelvis = v.anchor(2).unwrap_or_else(|| panic!("{skin:?}: no pelvis anchor"));
            assert!(head.w_axis.y > pelvis.w_axis.y, "{skin:?}: head {head:?} is not above pelvis {pelvis:?}");
            assert!(v.anchor(7).is_some(), "{skin:?}: no left hand anchor");
        }
    }


    /// The VR arms: one draw a side, both cut from the body (most of each glove and forearm kept,
    /// nothing else), the glove in front of the forearm in arm space.
    #[test]
    fn vr_arms_are_the_forearms_and_gloves() {
        let Some(dir) = libsms::assets::find(None) else {
            eprintln!("SKIP: no assets/ (set SMS_ASSETS)");
            return;
        };
        let a = libsms::Assets::load(&dir);
        let mut sim = Sim::new(a.params, a.anim_table, Vec3::ZERO, 0);
        let mut v = View::load(&dir).unwrap();
        v.update(&mut sim, 0.0);
        assert!(v.draws.iter().all(|d| d.key & 0xFF < VR_ARM_KEY), "no arms until asked");
        v.set_vr_arms(true);
        v.update(&mut sim, 0.0);
        let arms: Vec<Draw> = v.draws.iter().filter(|d| d.key & 0xFF >= VR_ARM_KEY && d.key & 0xFF < VR_BODY_KEY).copied().collect();
        assert_eq!(arms.len(), 2);
        let n = (arms[1].first_vertex - arms[0].first_vertex) as usize;
        // The VR body: where the body is, without its head or forearms, but with the legs.
        let body = v.draws.iter().find(|d| d.key == (u64::from(models::BODY) << 56)).copied().unwrap();
        let torso = v.draws.iter().find(|d| d.key & 0xFF == VR_BODY_KEY).copied().expect("no VR body draw");
        assert_eq!((torso.pos, torso.rot), (body.pos, body.rot));
        let all: Vec<Vec3> = v.positions[body.first_vertex as usize..][..n].iter().copied().filter(|p| *p != Vec3::ZERO).collect();
        let kept: Vec<Vec3> = v.positions[torso.first_vertex as usize..][..n].iter().copied().filter(|p| *p != Vec3::ZERO).collect();
        let top = |ps: &[Vec3]| ps.iter().fold(f32::MIN, |a, p| a.max(p.y));
        let bottom = |ps: &[Vec3]| ps.iter().fold(f32::MAX, |a, p| a.min(p.y));
        assert!(kept.len() > all.len() / 3 && kept.len() < all.len(), "{} of {} body vertices kept", kept.len(), all.len());
        assert!(top(&kept) < top(&all) - 20.0, "the head is still there: top {} of {}", top(&kept), top(&all));
        assert!(bottom(&kept) < bottom(&all) + 5.0, "the feet are gone: {} of {}", bottom(&kept), bottom(&all));
        for arm in &arms {
            let kept: Vec<Vec3> = v.positions[arm.first_vertex as usize..][..n].iter().copied().filter(|p| *p != Vec3::ZERO).collect();
            assert!(kept.len() > 200 && kept.len() < n / 4, "{} of {n} vertices kept", kept.len());
            let lo = kept.iter().fold(Vec3::INFINITY, |a, p| a.min(*p));
            let hi = kept.iter().fold(Vec3::NEG_INFINITY, |a, p| a.max(*p));
            assert!(-lo.z > hi.z, "the forearm hangs back along -z: {lo:?}..{hi:?}");
            assert!(hi.z > 5.0 && lo.z < -15.0, "glove ahead, forearm behind: {lo:?}..{hi:?}");
        }
        // Every other skin that loads (Luigi needs the Melee cache beside the libssbm checkout).
        let melee = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../libssbm/assets");
        for skin in [Skin::Piantissimo, Skin::Yoshi, Skin::Luigi] {
            if !v.load_skin(skin, Some(&melee)) {
                eprintln!("SKIP: {skin:?} not loadable");
                continue;
            }
            v.set_skin(skin);
            v.update(&mut sim, 0.0);
            let arms: Vec<Draw> = v.draws.iter().filter(|d| d.key & 0xFF >= VR_ARM_KEY && d.key & 0xFF < VR_ARM_KEY + 2).copied().collect();
            assert_eq!(arms.len(), 2, "{skin:?}");
            assert!(arms.iter().all(|a| a.model == arms[0].model), "{skin:?}");
        }
        let notes = v.bake_vr_arms();
        assert!(notes.len() >= 2, "{notes:?}");
    }
}
