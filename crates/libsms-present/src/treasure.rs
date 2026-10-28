//! Pikmin 2's treasures as props: a config's J3D model baked for a host, with its carry wobble.
//!
//! Spec: `Pellet::onCreateShape` / `Pellet::setupAnim` (plugProjectKandoU/pelletMgr.cpp) load
//! `<config>.bmd` from the treasure's archive and play `carry.bck` (looped between the frames
//! `animmgr.txt` names) while Pikmin carry it; the model has no other motion. The files come from
//! `libsms::extract::p2_treasure`; the table (`libsms_assets::treasure`) says which model is whose.
//!
//! Host adaptation: a model is drawn at the config's scale (Pikmin 2 units, about 1 cm like the
//! other Pikmin games); the host maps units. `Indirect::Yes` treasures (the gems and glass) refract the
//! framebuffer in the game; the bake draws their ordinary texture only.

use std::path::Path;

use glam::{Mat4, Vec3};
use libsms::{Clip, Skeleton};
use libsms_assets::Bmd;
use libsms_assets::treasure::{AnimMgr, Config, Kind, parse_anim_mgr};

use crate::view::{Model, RIG_KEY_BASE, Textures, build};

/// One treasure, baked.
pub struct Treasure {
    pub config: Config,
    pub model: Model,
    pub textures: Textures,
    pub rig: Skeleton,
    /// `carry.bck`, when it fits the rig.
    pub carry: Option<Clip>,
    pub anim: Option<AnimMgr>,
}

impl Treasure {
    /// Loads `config` from the cache's `treasure/` folder (`dir`).
    pub fn load(dir: &Path, config: &Config) -> Result<Treasure, String> {
        let model_dir = dir.join("models").join(config.model_dir());
        let rel = format!("{}/{}", config.model_dir(), config.bmd);
        let data = std::fs::read(model_dir.join(&config.bmd)).map_err(|e| format!("{rel}: {e}"))?;
        let bmd = Bmd::parse(&data).map_err(|e| format!("{rel}: {e}"))?;
        let rig = Skeleton::from_bmd(&data).ok_or_else(|| format!("{rel}: invalid JNT1/INF1"))?;
        let mut textures = Textures::default();
        let model = build(bmd, &|_, _| None, RIG_KEY_BASE + 0x400, &mut textures)?;
        let kind = if config.kind == Kind::Treasure { "otakara" } else { "item" };
        let anim = std::fs::read_to_string(dir.join(format!("texts/{kind}/{}/animmgr.txt", config.name)))
            .ok()
            .and_then(|t| parse_anim_mgr(&t));
        let carry = anim
            .as_ref()
            .and_then(|a| std::fs::read(model_dir.join(&a.file)).ok())
            .and_then(|b| Clip::parse(&b))
            .filter(|c| c.joint_count() == rig.len());
        Ok(Treasure { config: config.clone(), model, textures, rig, carry, anim })
    }

    /// Bind-pose bounding box, model space.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        (self.model.lo, self.model.hi)
    }

    /// Bind-pose size (x, y, z).
    pub fn size(&self) -> Vec3 {
        self.model.hi - self.model.lo
    }

    /// Joints in the carry wobble at `frame` (the whole clip is 30 frames a second; the game loops it between
    /// `anim.loop_start` and `loop_end`), or the bind pose when there is none.
    pub fn pose(&self, frame: f32) -> Vec<Mat4> {
        match &self.carry {
            Some(c) => self.rig.pose(c, frame),
            None => self.rig.bind_world(),
        }
    }

    /// The frame the carry wobble plays at `t` seconds into a carry, looped like `animmgr.txt` says.
    pub fn carry_frame(&self, t: f32) -> f32 {
        let (Some(a), Some(c)) = (&self.anim, &self.carry) else { return 0.0 };
        let (s, e) = if a.loop_end > a.loop_start { (a.loop_start, a.loop_end) } else { (0.0, c.frames) };
        let f = t * 30.0;
        if f < e { f } else { s + (f - s) % (e - s).max(1.0) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libsms_assets::treasure::Table;

    fn dir() -> Option<std::path::PathBuf> {
        let dir = std::env::var_os("SMS_P2_ASSETS")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/p2"))
            .join("treasure");
        dir.join("config/otakara_config.txt").is_file().then_some(dir)
    }

    /// Every treasure and upgrade bakes with a texture, has sane bounds and poses finitely.
    #[test]
    fn every_treasure_bakes_and_is_measured() {
        let Some(dir) = dir() else { eprintln!("SKIP: no Pikmin 2 treasure cache"); return };
        let table = Table::load(&dir).expect("table");
        let mut with_carry = 0;
        let (mut tallest, mut widest) = ((String::new(), 0.0f32), (String::new(), 0.0f32));
        for c in table.all() {
            let t = Treasure::load(&dir, c).unwrap_or_else(|e| panic!("{}: {e}", c.name));
            assert!(!t.model.batches.is_empty() && t.model.batches.iter().all(|b| !b.indices.is_empty()), "{}", c.name);
            assert!(t.model.batch_texture.iter().any(|&x| x >= 0), "{}: no baked texture", c.name);
            let s = t.size();
            assert!(s.is_finite() && s.min_element() >= 0.0 && s.max_element() > 1.0 && s.max_element() < 2000.0, "{} size {s}", c.name);
            if let Some(clip) = &t.carry {
                with_carry += 1;
                for f in [0.0, clip.frames * 0.5, clip.frames] {
                    assert!(t.pose(f).iter().all(|m| m.is_finite()), "{} pose", c.name);
                }
                assert!(t.carry_frame(100.0) < clip.frames.max(1.0), "{}", c.name);
            }
            if s.y > tallest.1 { tallest = (c.name.clone(), s.y) }
            if s.x.max(s.z) > widest.1 { widest = (c.name.clone(), s.x.max(s.z)) }
        }
        eprintln!("{} models, {with_carry} with a carry motion; tallest {tallest:?}, widest {widest:?}", table.all().count());
        assert!(with_carry >= 150, "carry motions: {with_carry}");
    }
}
