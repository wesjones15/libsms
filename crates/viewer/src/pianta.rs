//! Piantas wearing Mario's motion, viewer-only like `peach.rs`.
//! Spec: TNpcParts / sMonteM*Data (NpcInitData.cpp) for the 20-joint Pianta rigs
//! (`montem/mom_model.bmd`, `montew/mow_model.bmd`); TEnemyMario (enemyMario.cpp), which
//! animates Piantissimo's `monteman_model.bmd` with Mario's own joint matrices.
use std::path::{Path, PathBuf};
use glam::Mat4;
use libsms::Skeleton;
use libsms_assets::{Pose, retarget::Retarget};
use crate::{j3d::{Lights, Look, Pass, Renderer}, mario_model::Part};

pub struct Pianta {
    renderer: Renderer,
    body: Part,
    rig: Skeleton,
    /// `None`: the rig is Mario's (Piantissimo), so his joint matrices apply as they are.
    retarget: Option<Retarget>,
    chest: usize,
    head: usize,
    head_align: Mat4,
    chest_align: Mat4,
    pub label: &'static str,
}

/// Mario joint -> Pianta joint. Pianta has no waist or chest split: arms, legs and head hang
/// straight off `body_jnt`, so each limb follows Mario's world rotation for that limb.
const ALIASES: &[(&str, &str)] = &[
    ("body_jnt", "center"), ("koshi_null", "jnt_waist"),
    ("armRT_jnt", "jnt_arm_R1"), ("armRB_jnt", "jnt_arm_R2"), ("handR_jnt", "jnt_hand_R"),
    ("armLT_jnt", "jnt_arm_L1"), ("armLB_jnt", "jnt_arm_L2"), ("handL_jnt", "jnt_hand_L"),
    ("legRT_jnt", "jnt_leg_R1"), ("legRB_jnt", "jnt_leg_R2"), ("footR_jnt", "jnt_foot_R"),
    ("legLT_jnt", "jnt_leg_L1"), ("legLB_jnt", "jnt_leg_L2"), ("footL_jnt", "jnt_foot_L"),
    ("kubi", "jnt_head"),
];

/// sMonteM_BodyColorBuf / sMonteW_ColorBuf (NpcInitData.cpp); GXColorS10, so past 255 is fine.
const MALE_BODY: [[i16; 4]; 10] = [
    [100, 255, 300, 255], [120, 120, 300, 255], [350, 300, 0, 255], [200, 70, 0, 255], [300, 130, 255, 255],
    [255, 350, 0, 255], [400, 255, 255, 255], [320, 140, 0, 255], [200, 255, 400, 255], [400, 250, 100, 255],
];
const FEMALE_BODY: [[i16; 4]; 6] = [
    [300, 100, 200, 255], [400, 150, 0, 255], [300, 330, 0, 255], [400, 330, 0, 255], [330, 40, 0, 255],
    [400, 200, 255, 255],
];

pub fn ensure_assets(dir: &Path, disc_dirs: &[PathBuf]) -> Result<(), String> {
    // The Pianta bodies come with the standard extraction; Piantissimo lives in a stage archive.
    for f in ["npc/montem/mom_model.bmd", "npc/montew/mow_model.bmd"] {
        if !dir.join(f).is_file() { return Err(format!("{f} missing: re-run the asset extraction")); }
    }
    crate::peach::extract_from_stage(&dir.join("npc/monteman"), disc_dirs, "mamma4.szs",
        "map/map/pad/", &["monteman_model.bmd"])
}

impl Pianta {
    pub fn load_all(dir: &Path, source: &Skeleton) -> Result<Vec<Pianta>, String> {
        let mut all = Vec::new();
        // (label, model, Mario's rig as is, joints, straw texture, sMonte*_BodyColorBuf index)
        let colour: usize = std::env::var("SMS_PIANTA_COLOR").ok().and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
        for (label, rel, direct, joints, straw, body_colour) in [
            ("PIANTA (male) | montem/mom_model.bmd | 20 joints, retargeted", "npc/montem/mom_model.bmd", false, 20,
                Some("npc/montemcommon/i_mom_mino_rgba.bti"), Some((MALE_BODY[7], 7))),
            ("PIANTA (female) | montew/mow_model.bmd | 20 joints, retargeted", "npc/montew/mow_model.bmd", false, 20,
                Some("npc/montewcommon/i_mow_mino_rgba.bti"), Some((FEMALE_BODY[1], 1))),
            ("PIANTISSIMO | map/pad/monteman_model.bmd | Mario's 29-joint rig, no retarget", "npc/monteman/monteman_model.bmd", true, 29,
                None, None),
        ] {
            let mut renderer = Renderer::new();
            let (parent, file) = rel.rsplit_once('/').unwrap();
            let straw = straw.and_then(|p| std::fs::read(dir.join(p)).ok()
                .and_then(|b| libsms_assets::texture::read_bti(&b, p.to_owned()).ok()));
            let table = if rel.contains("montew") { &FEMALE_BODY[..] } else { &MALE_BODY[..] };
            let body_colour = body_colour.map(|(c, _)| if colour < table.len() { table[colour] } else { c });
            let body = Part::load_edited(&mut renderer, &dir.join(parent), file, |bmd| {
                // TMonte*BaseManager::changeTextureToStraw_: the 8x8 dummy becomes the real straw.
                if let Some(real) = &straw {
                    for t in bmd.textures.iter_mut().filter(|t| t.name.ends_with("_mino_dummyI4")) {
                        let name = std::mem::take(&mut t.name);
                        *t = real.clone();
                        t.name = name;
                    }
                }
                // TBaseNPC::loadAfter (NpcInitPrg.cpp): every non-eye material gets KColor0 =
                // unk174, white with alpha = pollution amount; a clean NPC has 0, which keeps
                // the pollution mask's compare stage off (NpcBase.cpp sets a = 0).
                if !direct {
                    for m in bmd.materials.iter_mut().filter(|m| m.name != "_eye_mat") {
                        m.konst[0] = [255, 255, 255, 0];
                    }
                }
                // sMonte*_BodyColor (NpcInitData.cpp): TEV register 0 of `_hand_mat`, skin and body.
                if let Some(c) = body_colour {
                    for m in bmd.materials.iter_mut().filter(|m| m.name == "_hand_mat") {
                        m.tev_colors[0] = c;
                    }
                }
            })?;
            let bytes = std::fs::read(dir.join(rel)).map_err(|e| e.to_string())?;
            let rig = Skeleton::from_bmd(&bytes).ok_or_else(|| format!("{rel}: invalid JNT1/INF1"))?;
            if rig.len() != joints { return Err(format!("{rel}: expected {joints} joints, got {}", rig.len())); }
            let mario_chest = source.find("jnt_chest").ok_or("Mario chest missing")?;
            let (chest, retarget) = if direct {
                if rig.parents != source.parents { return Err("Piantissimo's hierarchy differs from Mario's".into()); }
                (mario_chest, None)
            } else {
                (rig.find("body_jnt").ok_or("Pianta body_jnt missing")?, Some(Retarget::new(source, &rig, ALIASES)))
            };
            let rot = |m: Mat4| m.to_scale_rotation_translation().1;
            let chest_align = if direct { Mat4::IDENTITY } else {
                Mat4::from_quat(rot(rig.bind_world()[chest]).inverse() * rot(source.bind_world()[mario_chest]))
            };
            let head = rig.find(if direct { "jnt_head" } else { "kubi" }).ok_or("Pianta head joint missing")?;
let head_align = if direct { Mat4::IDENTITY } else {
                let mario_head = source.find("jnt_head").ok_or("Mario head missing")?;
                Mat4::from_quat(rot(rig.bind_world()[head]).inverse() * rot(source.bind_world()[mario_head]))
            };
            all.push(Pianta { renderer, body, rig, retarget, chest, head, head_align, chest_align, label });
        }
        Ok(all)
    }

    pub fn pose(&self, source_world: &[Mat4], source_lowest: f32) -> Vec<Mat4> {
        let mut world = match &self.retarget {
            Some(r) => r.pose(&self.rig, source_world),
            None => source_world.to_vec(),
        };
        let pose = Pose::new(&self.body.model.bmd, Mat4::IDENTITY, &world);
        let low = self.body.model.bmd.batches().expect("Pianta geometry").iter()
            .flat_map(|b| &b.vertices).map(|v| pose.skin(v).0.y).fold(f32::INFINITY, f32::min);
        let offset = source_lowest - low;
        for m in &mut world { m.w_axis.y += offset; }
        world
    }

    pub fn chest(&self, world: &[Mat4]) -> Mat4 { world[self.chest] * self.chest_align }

    /// The `jnt_head` matrix (the helmet's parent on Mario's own rig).
    pub fn head(&self, world: &[Mat4]) -> Mat4 { world[self.head] * self.head_align }

    pub fn draw(&mut self, world: &[Mat4], root: Mat4, view: Mat4, proj: Mat4) {
        let lights = Lights::daylight(view);
        let look = Look::default();
        let pose = Pose::new(&self.body.model.bmd, root, world);
        for pass in [Pass::Opaque, Pass::Translucent] {
            self.renderer.draw(&mut self.body.model, &pose, view, proj, &lights, &look, pass);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skeleton(dir: &Path, rel: &str) -> Option<Skeleton> {
        Skeleton::from_bmd(&std::fs::read(dir.join(rel)).ok()?)
    }

    #[test]
    fn pianta_retarget_maps_every_limb_and_keeps_bone_lengths() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        let Some(mario) = skeleton(&dir, "bmd/ma_mdl1.bmd") else { eprintln!("SKIP: assets absent"); return };
        for rel in ["npc/montem/mom_model.bmd", "npc/montew/mow_model.bmd"] {
            let Some(rig) = skeleton(&dir, rel) else { eprintln!("SKIP: {rel} absent"); continue };
            for (target, source) in ALIASES {
                assert!(rig.find(target).is_some(), "{rel}: {target}");
                assert!(mario.find(source).is_some(), "Mario: {source}");
            }
            let retarget = Retarget::new(&mario, &rig, ALIASES);
            let bind = rig.bind_world();
            for clip in ["wait", "run2", "jump", "dive_wait", "hipat"] {
                let c = libsms::Clip::parse(&std::fs::read(dir.join(format!("bck/ma_{clip}.bck"))).unwrap()).unwrap();
                for frame in [0., c.frames * 0.3, c.frames * 0.7] {
                    let world = retarget.pose(&rig, &mario.pose(&c, frame));
                    assert!(world.iter().all(|m| m.is_finite()), "{rel} {clip}:{frame}");
                    for (i, p) in rig.parents.iter().enumerate() {
                        let Some(p) = *p else { continue };
                        let rest = bind[i].w_axis.truncate().distance(bind[p].w_axis.truncate());
                        let posed = world[i].w_axis.truncate().distance(world[p].w_axis.truncate());
                        assert!((rest - posed).abs() < 0.01 * rest.max(1.), "{rel} {clip}:{frame} bone {i}");
                    }
                }
            }
        }
    }

    #[test]
    fn piantissimo_shares_mario_hierarchy() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        let (Some(mario), Some(king)) = (skeleton(&dir, "bmd/ma_mdl1.bmd"), skeleton(&dir, "npc/monteman/monteman_model.bmd"))
            else { eprintln!("SKIP: Piantissimo cache absent"); return };
        assert_eq!(mario.parents, king.parents);
        // Only two attachment joints are spelled differently (enemyMario.cpp copies by index).
        let differing: Vec<_> = mario.names.iter().zip(&king.names).filter(|(a, b)| a != b).collect();
        assert_eq!(differing.len(), 2, "{differing:?}");
    }
}
