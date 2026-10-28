//! A Sunshine Pianta as a body for another game's character: the 20-joint NPC model baked for
//! a host, plus the retargets that pose it from Mario's clips or from any other skeleton's
//! world matrices (Olimar's, through libpikmin-present).
//!
//! Spec: `TMonteMBaseManager` / `TMonteWBaseManager` and `sMonteM*Data` / `sMonteW*Data`
//! (NpcInitData.cpp) for the model, its straw skirt and its body colours; `TBaseNPC::loadAfter`
//! (NpcInitPrg.cpp) for the clean-NPC material values. The Pianta has no foot contact model of its
//! own, so the host anchors the lowest skinned vertex to the ground.
//!
//! Host adaptation: the retarget transfers world-rotation changes from the source rig and keeps the
//! Pianta's own bone lengths ([`libsms_assets::retarget::Retarget`]), as the viewer's `pianta.rs`.

use std::path::Path;

use glam::Mat4;
use std::collections::HashMap;

use libsms::{Clip, ClipLibrary, Skeleton, Srt};
use libsms_assets::Bmd;
use libsms_assets::retarget::Retarget;

use crate::view::{Model, RIG_KEY_BASE, Textures, build};

/// (Pianta joint, Mario joint). The Pianta has no waist/chest split: arms, legs and head hang
/// off `body_jnt`, so each limb follows Mario's world rotation for it.
pub const MARIO_ALIASES: &[(&str, &str)] = &[
    ("body_jnt", "center"), ("koshi_null", "jnt_waist"),
    ("armRT_jnt", "jnt_arm_R1"), ("armRB_jnt", "jnt_arm_R2"), ("handR_jnt", "jnt_hand_R"),
    ("armLT_jnt", "jnt_arm_L1"), ("armLB_jnt", "jnt_arm_L2"), ("handL_jnt", "jnt_hand_L"),
    ("legRT_jnt", "jnt_leg_R1"), ("legRB_jnt", "jnt_leg_R2"), ("footR_jnt", "jnt_foot_R"),
    ("legLT_jnt", "jnt_leg_L1"), ("legLB_jnt", "jnt_leg_L2"), ("footL_jnt", "jnt_foot_L"),
    ("kubi", "jnt_head"),
];

/// `sMonteM_BodyColorBuf` (NpcInitData.cpp); GXColorS10, so past 255 is fine.
pub const MALE_BODY: [[i16; 4]; 10] = [
    [100, 255, 300, 255], [120, 120, 300, 255], [350, 300, 0, 255], [200, 70, 0, 255], [300, 130, 255, 255],
    [255, 350, 0, 255], [400, 255, 255, 255], [320, 140, 0, 255], [200, 255, 400, 255], [400, 250, 100, 255],
];
/// `sMonteW_ColorBuf`.
pub const FEMALE_BODY: [[i16; 4]; 6] = [
    [300, 100, 200, 255], [400, 150, 0, 255], [300, 330, 0, 255], [400, 330, 0, 255], [330, 40, 0, 255],
    [400, 200, 255, 255],
];

/// Files a Pianta needs under the Sunshine assets folder (the standard extraction has them), plus
/// the Mario rig and clips its locomotion borrows.
pub fn pianta_files(female: bool) -> [&'static str; 3] {
    if female {
        ["npc/montew/mow_model.bmd", "npc/montewcommon/i_mow_mino_rgba.bti", "bmd/ma_mdl1.bmd"]
    } else {
        ["npc/montem/mom_model.bmd", "npc/montemcommon/i_mom_mino_rgba.bti", "bmd/ma_mdl1.bmd"]
    }
}

/// A Pianta ready to draw: its model with baked textures, its rig and Mario's clips.
pub struct Pianta {
    pub model: Model,
    pub textures: Textures,
    pub rig: Skeleton,
    mario: Skeleton,
    clips: ClipLibrary,
    /// The Pianta's own clips (`mom_wait`, `mow_walk`, ...), by name without the prefix.
    own: HashMap<String, Clip>,
    from_mario: Retarget,
}

/// Joints Mario and Olimar have no counterpart for: the palm on the head, its two leaves and the
/// nose. Their motion always comes from one of the Pianta's own clips.
pub const PLANT_JOINTS: [&str; 4] = ["yashi_jnt", "leafL_jnt", "leafR_jnt", "nose_jnt"];

/// Which of the Pianta's own clips animates the plant, and at which frame.
pub type Plant<'a> = Option<(&'a str, f32)>;

impl Pianta {
    /// Loads a Pianta from the Sunshine assets folder. `female` picks `montew`; `colour` indexes
    /// the game's body colour table (out of range: the game's pick for the first Pianta).
    pub fn load(dir: &Path, female: bool, colour: usize) -> Result<Pianta, String> {
        let [model_rel, straw_rel, mario_rel] = pianta_files(female);
        let read = |rel: &str| std::fs::read(dir.join(rel)).map_err(|e| format!("{rel}: {e}"));
        let data = read(model_rel)?;
        let mut bmd = Bmd::parse(&data).map_err(|e| format!("{model_rel}: {e}"))?;
        // TMonte*BaseManager::changeTextureToStraw_: the 8x8 dummy becomes the real straw.
        let straw = libsms_assets::texture::read_bti(&read(straw_rel)?, straw_rel.to_owned())
            .map_err(|e| format!("{straw_rel}: {e}"))?;
        for t in bmd.textures.iter_mut().filter(|t| t.name.ends_with("_mino_dummyI4")) {
            let name = std::mem::take(&mut t.name);
            *t = straw.clone();
            t.name = name;
        }
        // TBaseNPC::loadAfter: every non-eye material's KColor0 is white with alpha = the
        // pollution amount; a clean NPC has 0, which keeps the pollution mask off.
        for m in bmd.materials.iter_mut().filter(|m| m.name != "_eye_mat") {
            m.konst[0] = [255, 255, 255, 0];
        }
        // sMonte*_BodyColor (NpcInitData.cpp): TEV register 0 of `_hand_mat`, skin and body.
        let table: &[[i16; 4]] = if female { &FEMALE_BODY } else { &MALE_BODY };
        let pick = if colour < table.len() { colour } else if female { 1 } else { 7 };
        for m in bmd.materials.iter_mut().filter(|m| m.name == "_hand_mat") {
            m.tev_colors[0] = table[pick];
        }
        let rig = Skeleton::from_bmd(&data).ok_or_else(|| format!("{model_rel}: invalid JNT1/INF1"))?;
        let mario = Skeleton::from_bmd(&read(mario_rel)?).ok_or("ma_mdl1.bmd: invalid JNT1/INF1")?;
        let from_mario = Retarget::new(&mario, &rig, MARIO_ALIASES);
        let mut textures = Textures::default();
        let model = build(bmd, &|_, _| None, RIG_KEY_BASE + 0x20 + female as u32, &mut textures)?;
        let clips = ClipLibrary::from_dir(&dir.join("bck"));
        // The NPC's own clips: `montemcommon/mom_*.bck` (`montewcommon/mow_*.bck`).
        let (folder, prefix) = if female { ("npc/montewcommon", "mow_") } else { ("npc/montemcommon", "mom_") };
        let mut own = HashMap::new();
        if let Ok(rd) = std::fs::read_dir(dir.join(folder)) {
            for e in rd.flatten() {
                let path = e.path();
                let Some(name) = path.file_stem().and_then(|s| s.to_str()).and_then(|s| s.strip_prefix(prefix)) else { continue };
                if path.extension().and_then(|s| s.to_str()) != Some("bck") { continue }
                if let Some(c) = std::fs::read(&path).ok().and_then(|b| Clip::parse(&b)) {
                    own.insert(name.to_owned(), c);
                }
            }
        }
        Ok(Pianta { model, textures, rig, mario, clips, own, from_mario })
    }

    /// Standing height in the model's own units (bind pose).
    pub fn height(&self) -> f32 {
        self.model.hi.y - self.model.lo.y
    }

    /// Length in frames of Mario's `ma_<name>.bck` (60 per second), if it was extracted.
    pub fn mario_clip_frames(&self, name: &str) -> Option<f32> {
        self.clips.get(name).map(|c| c.frames)
    }

    /// Length in frames of the Pianta's own clip (`wait`, `walk`, ...; 60 per second).
    pub fn own_clip_frames(&self, name: &str) -> Option<f32> {
        self.own.get(name).map(|c| c.frames)
    }

    /// The Pianta's own clip `name` at `frame`, every joint, in model space.
    pub fn pose_own(&self, name: &str, frame: f32) -> Option<Vec<Mat4>> {
        let clip = self.own.get(name)?;
        Some(self.rig.pose(clip, frame))
    }

    /// Local transforms for the joints no source reaches: the bind pose, with the plant joints
    /// taken from the Pianta's own clip when one is named.
    fn base(&self, plant: Plant) -> Vec<Srt> {
        let mut base = self.rig.bind.clone();
        if let Some(clip) = plant.and_then(|(name, frame)| self.own.get(name).map(|c| (c, frame))) {
            for name in PLANT_JOINTS {
                if let Some(j) = self.rig.find(name) {
                    if let Some(srt) = clip.0.sample(j, clip.1) { base[j] = srt; }
                }
            }
        }
        base
    }

    /// The Pianta's joints posed by Mario's `name` clip at `frame`, in model space, with the
    /// plant animated by `plant`.
    pub fn pose_mario(&self, name: &str, frame: f32, plant: Plant) -> Option<Vec<Mat4>> {
        let clip = self.clips.get(name)?;
        Some(self.from_mario.pose_over(&self.rig, &self.mario.pose(clip, frame), &self.base(plant)))
    }

    /// The bind pose in model space.
    pub fn bind_world(&self) -> Vec<Mat4> {
        self.rig.bind_world()
    }

    /// A retarget from another rig (`source`) onto this one; `aliases` are (Pianta joint,
    /// source joint). Pose it with [`Pianta::pose_from`].
    pub fn retarget_from(&self, source: &Skeleton, aliases: &[(&str, &str)]) -> Retarget {
        Retarget::new(source, &self.rig, aliases)
    }

    /// The Pianta's joints following `source_world` through `retarget`, in model space.
    pub fn pose_from(&self, retarget: &Retarget, source_world: &[Mat4], plant: Plant) -> Vec<Mat4> {
        retarget.pose_over(&self.rig, source_world, &self.base(plant))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assets() -> Option<std::path::PathBuf> {
        let dir = std::env::var_os("SMS_ASSETS").map(std::path::PathBuf::from)
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets"));
        pianta_files(false).iter().all(|f| dir.join(f).is_file()).then_some(dir)
    }

    #[test]
    fn pianta_poses_from_mario_clips_keeping_its_bones() {
        let Some(dir) = assets() else { eprintln!("SKIP: Sunshine assets absent"); return };
        for female in [false, true] {
            let p = Pianta::load(&dir, female, usize::MAX).expect("load");
            assert!(p.height() > 50.0 && p.height() < 400.0, "height {}", p.height());
            assert!(p.model.batches.iter().all(|b| !b.indices.is_empty()));
            assert!(p.model.batch_texture.iter().any(|&t| t >= 0), "no baked texture");
            let bind = p.bind_world();
            // The Pianta's own idle is there, and it moves the palm.
            let idle = p.pose_own("wait", 20.0).expect("mom_wait");
            let leaf = p.rig.find("yashi_jnt").unwrap();
            assert!(idle[leaf].abs_diff_eq(bind[leaf], 1e-3) == false, "palm does not move in the idle");
            for clip in ["wait", "walk", "run2", "jump", "dive_wait", "hipat"] {
                let frames = p.mario_clip_frames(clip).expect(clip);
                for frame in [0.0, frames * 0.4] {
                    let world = p.pose_mario(clip, frame, Some(("wait", frame))).unwrap();
                    assert!(world.iter().all(|m| m.is_finite()), "{clip}");
                    for (i, parent) in p.rig.parents.iter().enumerate() {
                        let Some(parent) = *parent else { continue };
                        // The plant plays the Pianta's own clip, which may stretch it.
                        if PLANT_JOINTS.contains(&p.rig.names[i].as_str()) { continue }
                        let rest = bind[i].w_axis.truncate().distance(bind[parent].w_axis.truncate());
                        let posed = world[i].w_axis.truncate().distance(world[parent].w_axis.truncate());
                        assert!((rest - posed).abs() < 0.01 * rest.max(1.), "{clip} bone {i}");
                    }
                }
            }
        }
    }
}

/// Files Louie needs under the Pikmin 2 cache (`libsms::extract::p2`).
pub const LOUIE_FILES: [&str; 3] = ["piki/orima_model/orima3.bmd", "piki/motion/wait.bca", "piki/motion/walk.bca"];

/// Purple and White Pikmin (Pikmin 2): `PikiMgr::loadBmd(Purple, "piki_p2_black")`, `White`, "piki_p2_white".
pub const PIKMIN2_FILES: [&str; 2] = ["piki/piki_model/piki_p2_black.bmd", "piki/piki_model/piki_p2_white.bmd"];

/// Pikmin 2's Louie (`NaviMgr::loadResources_float`: `orima_model/orima3.bmd`) with his own motions
/// (`motion/*.bca`, driven by `animmgr.txt` in the game), from the cache `libsms::extract::p2` writes.
/// His rig is the one the motions were made for, so they pose him directly, no retarget.
pub struct Louie {
    pub model: Model,
    pub textures: Textures,
    pub rig: Skeleton,
    clips: HashMap<String, Clip>,
}

impl Louie {
    pub fn load(dir: &Path) -> Result<Louie, String> {
        Self::load_from(dir, LOUIE_FILES[0])
    }

    /// Any Pikmin 2 model whose rig the cache's `piki/motion/*.bca` fit: Louie, or a Pikmin (`PIKMIN2_FILES`),
    /// which play their own `wait`/`walk`/`run2`/`nuku`... by the same names. `rel` is relative to `dir`.
    pub fn load_from(dir: &Path, rel: &str) -> Result<Louie, String> {
        Self::load_with(dir, rel, "piki/motion")
    }

    /// A Pikmin 2 model with the motions in `motions` (a folder under `dir`; `.bca` and `.bck` files whose joint count
    /// fits the rig). The Hocotate Ship is `ufo/ufo.bmd` with `ufo/*.bck`.
    pub fn load_with(dir: &Path, rel: &str, motions: &str) -> Result<Louie, String> {
        let data = std::fs::read(dir.join(rel)).map_err(|e| format!("{rel}: {e}"))?;
        let bmd = Bmd::parse(&data).map_err(|e| format!("{rel}: {e}"))?;
        let rig = Skeleton::from_bmd(&data).ok_or_else(|| format!("{rel}: invalid JNT1/INF1"))?;
        let mut clips = HashMap::new();
        let rd = std::fs::read_dir(dir.join(motions)).map_err(|e| format!("{motions}: {e}"))?;
        for e in rd.flatten() {
            let path = e.path();
            let ext = path.extension().and_then(|s| s.to_str());
            let Some(name) = path.file_stem().and_then(|s| s.to_str()) else { continue };
            let clip = match ext {
                Some("bca") => std::fs::read(&path).ok().and_then(|b| Clip::parse_bca(&b)),
                Some("bck") => std::fs::read(&path).ok().and_then(|b| Clip::parse(&b)),
                _ => None,
            };
            if let Some(c) = clip {
                if c.joint_count() == rig.len() { clips.insert(name.to_owned(), c); }
            }
        }
        if clips.is_empty() { return Err(format!("no motion in {motions} fits {rel}'s rig")) }
        let mut textures = Textures::default();
        let key = RIG_KEY_BASE + 0x100 + (rel.bytes().fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32)) & 0xfff);
        let model = build(bmd, &|_, _| None, key, &mut textures)?;
        Ok(Louie { model, textures, rig, clips })
    }

    /// Standing height in the model's units (bind pose).
    pub fn height(&self) -> f32 {
        self.model.hi.y - self.model.lo.y
    }

    /// Length in frames of motion `name` (`wait`, `walk`, `run2`, ...; 30 per second).
    pub fn clip_frames(&self, name: &str) -> Option<f32> {
        self.clips.get(name).map(|c| c.frames)
    }

    /// His joints in motion `name` at `frame`, model space.
    pub fn pose(&self, name: &str, frame: f32) -> Option<Vec<Mat4>> {
        Some(self.rig.pose(self.clips.get(name)?, frame))
    }

    pub fn bind_world(&self) -> Vec<Mat4> {
        self.rig.bind_world()
    }
}

#[cfg(test)]
mod louie_tests {
    use super::*;

    fn assets() -> Option<std::path::PathBuf> {
        let dir = std::env::var_os("SMS_P2_ASSETS").map(std::path::PathBuf::from)
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/p2"));
        LOUIE_FILES.iter().all(|f| dir.join(f).is_file()).then_some(dir)
    }

    #[test]
    fn louie_poses_from_his_own_motions() {
        let Some(dir) = assets() else { eprintln!("SKIP: no Pikmin 2 cache"); return };
        let l = Louie::load(&dir).expect("load");
        assert!(l.height() > 5.0 && l.height() < 400.0, "height {}", l.height());
        assert!(l.model.batch_texture.iter().any(|&t| t >= 0), "no baked texture");
        let bind = l.bind_world();
        for clip in ["wait", "walk", "run2", "throw", "nuku", "fue", "damage", "dead", "jump", "rolljmp"] {
            let frames = l.clip_frames(clip).unwrap_or_else(|| panic!("no {clip}"));
            for frame in [0.0, frames * 0.5, frames] {
                let w = l.pose(clip, frame).unwrap();
                assert_eq!(w.len(), bind.len());
                assert!(w.iter().all(|m| m.is_finite()), "{clip}");
            }
        }
    }
}

#[cfg(test)]
mod p2_pikmin_tests {
    use super::*;

    #[test]
    fn purple_and_white_pikmin_pose_from_their_own_motions() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/p2");
        for rel in PIKMIN2_FILES {
            if !dir.join(rel).is_file() { eprintln!("SKIP: no Pikmin 2 cache ({rel})"); return }
            let b = Louie::load_from(&dir, rel).expect("load");
            assert_eq!(b.rig.len(), 11, "{rel}");
            assert!(b.height() > 5.0 && b.height() < 60.0, "{rel} height {}", b.height());
            let bind = b.bind_world();
            for clip in ["wait", "walk", "run2", "nigeru", "throw", "nuku", "dead", "attack1", "kuttuku", "jump"] {
                let frames = b.clip_frames(clip).unwrap_or_else(|| panic!("no {clip}"));
                for frame in [0.0, frames * 0.5, frames] {
                    let w = b.pose(clip, frame).unwrap();
                    assert_eq!(w.len(), bind.len());
                    assert!(w.iter().all(|m| m.is_finite()), "{clip}");
                }
            }
        }
    }
}

#[cfg(test)]
mod ufo_probe {
    use super::*;

    #[test]
    fn probe_the_hocotate_ship() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/p2");
        let b = match Louie::load_with(&dir, "ufo/ufo.bmd", "ufo") { Ok(b) => b, Err(e) => { eprintln!("SKIP or failed: {e}"); return } };
        eprintln!("joints {:?} height {} lo {:?} hi {:?}", b.rig.names, b.height(), b.model.lo, b.model.hi);
        let mut names: Vec<_> = b.clips.iter().map(|(k, c)| (k.clone(), c.frames)).collect();
        names.sort_by(|a, b| a.0.cmp(&b.0));
        eprintln!("clips {names:?}");
    }
}
