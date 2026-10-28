//! A Pikmin 2 enemy as a body for another game's host: its J3D model (`enemy/data/<Name>/model/enemy.bmd`, from
//! `libsms::extract::p2_enemy`), its `.bca` clips and, for the variants that reuse a model, the texture the enemy's
//! manager swaps in (`EnemyMgrBase::loadTexData`, `Koganemushi::Obj::changeMaterial`: `changeImage(tex, 0)`).
//!
//! Same job as [`crate::body::Louie`] with a texture override. Host adaptation: the variants' `karada` material
//! also gets a TEV konst colour tweak in the game (`setTevKColor(0, ..)`), which the baked material does not apply.

use std::collections::HashMap;
use std::path::Path;

use glam::Mat4;
use libsms::{Clip, Skeleton};
use libsms_assets::Bmd;
use libsms_assets::texture::read_bti;

use crate::tev::Tex;
use crate::view::{Model, RIG_KEY_BASE, Textures, build};

pub struct EnemyBody {
    pub model: Model,
    pub textures: Textures,
    pub rig: Skeleton,
    clips: HashMap<String, Clip>,
}

impl EnemyBody {
    /// Loads `model_rel` and the `.bca`/`.bck` clips in `clips_rel` (both relative to `dir`, the Pikmin 2 cache) whose
    /// joint count fits the rig; the model's first texture is replaced by `texture_rel` when given.
    pub fn load(dir: &Path, model_rel: &str, clips_rel: &str, texture_rel: Option<&str>) -> Result<EnemyBody, String> {
        let data = std::fs::read(dir.join(model_rel)).map_err(|e| format!("{model_rel}: {e}"))?;
        let bmd = Bmd::parse(&data).map_err(|e| format!("{model_rel}: {e}"))?;
        let rig = Skeleton::from_bmd(&data).ok_or_else(|| format!("{model_rel}: invalid JNT1/INF1"))?;
        let mut clips = HashMap::new();
        let rd = std::fs::read_dir(dir.join(clips_rel)).map_err(|e| format!("{clips_rel}: {e}"))?;
        for e in rd.flatten() {
            let path = e.path();
            let Some(name) = path.file_stem().and_then(|s| s.to_str()) else { continue };
            let bytes = std::fs::read(&path).ok();
            let clip = match path.extension().and_then(|s| s.to_str()) {
                Some("bca") => bytes.and_then(|b| Clip::parse_bca(&b)),
                Some("bck") => bytes.and_then(|b| Clip::parse(&b)),
                _ => None,
            };
            if let Some(c) = clip.filter(|c| c.joint_count() == rig.len()) {
                clips.insert(name.to_owned(), c);
            }
        }
        if clips.is_empty() {
            return Err(format!("no clip in {clips_rel} fits {model_rel}'s rig"));
        }
        let swap = match texture_rel {
            Some(rel) => {
                let b = std::fs::read(dir.join(rel)).map_err(|e| format!("{rel}: {e}"))?;
                let t = read_bti(&b, rel.to_owned()).map_err(|e| format!("{rel}: {e}"))?;
                Some(Tex::new(t.image, t.wrap_s, t.wrap_t))
            }
            None => None,
        };
        let hash = (model_rel.bytes().chain(texture_rel.unwrap_or("").bytes())).fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
        let key = RIG_KEY_BASE + 0x2000 + (hash & 0xfff);
        let mut textures = Textures::default();
        let model = build(bmd, &|i, _| if i == 0 { swap.clone() } else { None }, key, &mut textures)?;
        Ok(EnemyBody { model, textures, rig, clips })
    }

    /// Standing height in the model's units (bind pose).
    pub fn height(&self) -> f32 {
        self.model.hi.y - self.model.lo.y
    }

    /// Length in frames of clip `name` (30 a second).
    pub fn clip_frames(&self, name: &str) -> Option<f32> {
        self.clips.get(name).map(|c| c.frames)
    }

    /// The joints in clip `name` at `frame`, model space.
    pub fn pose(&self, name: &str, frame: f32) -> Option<Vec<Mat4>> {
        Some(self.rig.pose(self.clips.get(name)?, frame))
    }

    pub fn bind_world(&self) -> Vec<Mat4> {
        self.rig.bind_world()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> Option<std::path::PathBuf> {
        let dir = std::env::var_os("SMS_P2_ASSETS").map(std::path::PathBuf::from)
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../libpikmin/assets/p2"));
        dir.join("enemy/data/Kogane/model/enemy.bmd").is_file().then_some(dir)
    }

    #[test]
    fn the_kogane_poses_on_its_clips_and_variants_differ_in_texture() {
        let Some(dir) = dir() else { eprintln!("SKIP: no Pikmin 2 enemy cache"); return };
        let load = |tex: &str| EnemyBody::load(&dir, "enemy/data/Kogane/model/enemy.bmd", "enemy/data/Kogane/anim", Some(tex)).expect("load");
        let flint = load("enemy/data/Kogane/kogane_s3tc.bti");
        let glint = load("enemy/data/Wealthy/oogane_s3tc.bti");
        assert!(flint.height() > 5.0 && flint.height() < 200.0, "{}", flint.height());
        for clip in ["move", "wait", "damage"] {
            let frames = flint.clip_frames(clip).unwrap_or_else(|| panic!("no {clip}"));
            for f in [0.0, frames * 0.5, frames] {
                let joints = flint.pose(clip, f).unwrap();
                assert_eq!(joints.len(), flint.rig.len());
                assert!(joints.iter().all(|m| m.is_finite()));
            }
        }
        // The variants differ in the picture the body's texture bakes to.
        let first = |b: &EnemyBody| b.textures.list.first().map(|t| t.rgba.clone());
        assert!(first(&flint).is_some() && first(&flint) != first(&glint), "the Glint Beetle should wear another texture");
    }
}
