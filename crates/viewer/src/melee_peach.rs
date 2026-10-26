//! Melee Peach wearing Sunshine motion, using the shared HSD renderer and J3D retarget.
//! Exact asset: PlPeNr.dat:PlyPeach5K_Share_joint (ftPeach/ftpeach.c costume table).
//! Bone correspondence: PlCo.dat FighterPartsTable, ftparts.c::ftParts_GetBoneIndex;
//! Peach kind 9 and semantic Fighter_Part indices (ft/forward.h).
use std::path::{Path, PathBuf};
use glam::Mat4;
use libsms::{Skeleton, Srt, MtxCalc};
use libsms_assets::retarget::Retarget;
use libssbm_assets::{dat::Dat, model::Model, fighter::Archive};
use crate::luigi::HsdRenderer;

pub struct MeleePeach {
    renderer: HsdRenderer,
    rig: Skeleton,
    retarget: Retarget,
    chest: usize,
    chest_align: Mat4,
    dress_anims: libssbm_assets::animation::Animations,
    dress_chains: Vec<Vec<usize>>,
}

pub fn ensure_assets(dir: &Path, disc_dirs: &[PathBuf]) -> Result<PathBuf, String> {
    let out = dir.join("melee-peach");
    let required = ["PlPeNr.dat", "PlPe.dat", "PlCo.dat", "PlPeAJ.dat"];
    if required.iter().all(|n| out.join(n).is_file()) { return Ok(out); }
    let mut candidates = Vec::new();
    if let Some(p) = std::env::var_os("SSBM_DISC") { candidates.push(p.into()); }
    for folder in disc_dirs {
        for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
            let p = entry.path();
            if p.extension().and_then(|s| s.to_str()).is_some_and(|s|
                ["iso", "gcm", "ciso"].iter().any(|x| s.eq_ignore_ascii_case(x))) { candidates.push(p); }
        }
    }
    for path in candidates {
        let Ok(mut disc) = libssbm_assets::disc::Disc::open(&path) else { continue };
        std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        for name in required {
            let raw = disc.file(name).map_err(|e| e.to_string())?;
            if name=="PlPeAJ.dat" { libssbm_assets::animation::Animations::parse(&raw,&[]).map_err(|e|e.to_string())?; }
            else { Dat::parse(&raw).map_err(|e| e.to_string())?; }
            std::fs::write(out.join(name), raw).map_err(|e| e.to_string())?;
        }
        eprintln!("Melee Peach: {} -> PlPeNr.dat:PlyPeach5K_Share_joint (ftPeach/ftpeach.c)", path.display());
        return Ok(out);
    }
    Err("Melee Peach needs your GALE01 Rev 2 ISO/CISO (SSBM_DISC)".into())
}

pub fn load_rig(dir: &Path, source: &Skeleton) -> Result<(Model, Skeleton, usize), String> {
    let mut model = Model::load_costume(dir, "PlPeNr.dat", Archive { file:"PlPe.dat",root:"ftDataPeach" },0,None)
        .map_err(|e| e.to_string())?;
    let peach_bytes = std::fs::read(dir.join("PlPe.dat")).map_err(|e| e.to_string())?;
    let peach_data = Dat::parse(&peach_bytes).map_err(|e| e.to_string())?;
    // ftPe_Init_OnDeath, normal pink costume: no weapon group 3, no Daisy group 5.
    model.set_part_choices(&peach_data,"ftDataPeach",0,
        &[(0,Some(0)),(1,Some(0)),(2,Some(0)),(3,None),(4,Some(0)),(5,None),(6,Some(0))])
        .map_err(|e| e.to_string())?;
    let bytes = std::fs::read(dir.join("PlCo.dat")).map_err(|e| e.to_string())?;
    let d = Dat::parse(&bytes).map_err(|e| e.to_string())?;
    let tables = d.required_ptr(d.root("ftLoadCommonData").map_err(|e| e.to_string())? + 0x10).map_err(|e| e.to_string())?;
    let pe = d.required_ptr(tables + 9*4).map_err(|e| e.to_string())?;
    let common_to_pe = d.required_ptr(pe+4).map_err(|e| e.to_string())?;
    let pe_map = d.bytes(common_to_pe,53).map_err(|e| e.to_string())?;
    let mut names: Vec<_> = (0..model.joints.len()).map(|i| format!("unmapped_{i}")).collect();
    // Explicit body functions, not equal numeric bone indices across characters.
    for (source_name,common) in [
        ("center",4),("jnt_head",34),
        ("jnt_sldr_R",36),("jnt_arm_R1",37),("jnt_arm_R2",38),("jnt_hand_R",39),
        ("jnt_sldr_L",18),("jnt_arm_L1",19),("jnt_arm_L2",20),("jnt_hand_L",21),
        ("jnt_leg_R1",11),("jnt_leg_R2",13),("chn_foot_R",14),("jnt_foot_R",15),
        ("jnt_leg_L1",6),("jnt_leg_L2",8),("chn_foot_L",9),("jnt_foot_L",10),
    ] {
        let target = *pe_map.get(common).ok_or("common part outside table")? as usize;
        if target != 255 {
            let name = names.get_mut(target).ok_or("Peach mapped bone outside skeleton")?;
            source.find(source_name).ok_or_else(|| format!("Mario missing {source_name}"))?;
            *name = source_name.into();
        }
    }
    // Peach has a separate BustN. Split Mario's chest helper and chest across her spine.
    let bust = pe_map[16] as usize;
    let waist = pe_map[5] as usize;
    let chest = if bust != 255 { bust } else { waist };
    if chest >= names.len() { return Err("Peach chest is absent".into()); }
    if bust != 255 && waist != 255 && bust != waist {
        names[waist] = "chn_chest".into();
    }
    names[chest] = "jnt_chest".into();
    let rig = Skeleton { names,
        parents: model.joints.iter().map(|j| j.parent).collect(),
        bind: model.joints.iter().map(|j| Srt { scale:j.scale,rot:j.rotation,trans:j.translation }).collect(),
        calc:MtxCalc::Basic,compensate:vec![false;model.joints.len()] };
    eprintln!("Melee Peach: {} joints; chest={chest}; PlCo kind 9 part_to_joint, ftParts_GetBoneIndex", rig.len());
    Ok((model,rig,chest))
}

impl MeleePeach {
    pub fn load(dir: &Path, source: &Skeleton) -> Result<Self, String> {
        let (model,rig,chest) = load_rig(dir,source)?;
        let retarget = Retarget::new(source,&rig,&[]);
        let t = rig.bind_world()[chest].to_scale_rotation_translation().1;
        let s = source.bind_world()[source.find("jnt_chest").ok_or("Mario chest missing")?].to_scale_rotation_translation().1;
        let chest_align = Mat4::from_quat(t.inverse()*s);
        let dress_anims = libssbm_assets::animation::Animations::parse(
            &std::fs::read(dir.join("PlPeAJ.dat")).map_err(|e|e.to_string())?,&[]).map_err(|e|e.to_string())?;
        for name in ["Run","Wait1"] {
            if !dress_anims.clips.contains_key(name) { return Err(format!("missing Peach dress donor {name}")); }
        }
        let raw = std::fs::read(dir.join("PlPe.dat")).map_err(|e|e.to_string())?;
        let data = Dat::parse(&raw).map_err(|e|e.to_string())?;
        let dynamics = data.required_ptr(data.root("ftDataPeach").map_err(|e|e.to_string())?+0x2c).map_err(|e|e.to_string())?;
        let templates = data.required_ptr(dynamics+4).map_err(|e|e.to_string())?;
        let mut dress_chains = Vec::new();
        for i in 0..data.u32(dynamics).map_err(|e|e.to_string())? as usize {
            let record = templates+i*24;
            let root = data.u32(record).map_err(|e|e.to_string())? as usize;
            let count = data.u32(record+8).map_err(|e|e.to_string())? as usize;
            if root>=rig.len() || !rig.under(root,17) { continue }
            let mut chain = vec![root];
            while chain.len()<count {
                let parent = *chain.last().unwrap();
                let child = rig.parents.iter().position(|&p|p==Some(parent)).ok_or("dress dynamics chain truncated")?;
                chain.push(child);
            }
            dress_chains.push(chain);
        }
        if dress_chains.len()!=8 { return Err(format!("expected eight Peach dress chains, got {}",dress_chains.len())); }
        eprintln!("Melee dress: PlPe.dat ftDataPeach+0x2C, roots {:?}; native Run/Wait1 from PlPeAJ.dat; ftdynamics.c::ftCo_8009CF84",dress_chains.iter().map(|c|c[0]).collect::<Vec<_>>());
        Ok(Self { renderer:HsdRenderer::new(model)?,rig,retarget,chest,chest_align,dress_anims,dress_chains })
    }

    pub fn pose(&self, source: &[Mat4], lowest: f32, clip: &str, phase: f32) -> Vec<Mat4> {
        let gait = clip.starts_with("run") || clip.contains("walk");
        let mut world = self.retarget.pose_weighted(&self.rig,source,|n|libsms_assets::dress::gait_weight(n,gait));
        if std::env::var("SMS_DRESS").as_deref()!=Ok("off") {
            let name = if clip.starts_with("run") { "Run" }
                else if clip.contains("walk") && self.dress_anims.clips.contains_key("WalkFast") { "WalkFast" }
                else { "Wait1" };
            let native = &self.dress_anims.clips[name];
            let donor = self.dress_anims.pose(&self.renderer.model,name,phase*native.frames,true);
            libsms_assets::dress::graft(&self.rig,&mut world,&donor,17);
            libsms_assets::dress::panels(&self.rig,&mut world,4,&self.dress_chains);
        }
        let low = (0..self.renderer.model.batches.len())
            .filter(|&i| self.renderer.model.normal_visible.get(self.renderer.model.batches[i].dobj).copied().unwrap_or(true))
            .flat_map(|i| self.renderer.model.skin(i,&world)).map(|(p,_)| p.y).fold(f32::INFINITY,f32::min);
        let offset = lowest*0.1-low;
        for m in &mut world { m.w_axis.y += offset; }
        world
    }

    pub fn chest(&self, world: &[Mat4]) -> Mat4 {
        let (_,r,t) = (world[self.chest]*self.chest_align).to_scale_rotation_translation();
        Mat4::from_rotation_translation(r,t*10.0)
    }

    pub fn draw(&mut self, world: &[Mat4], root: Mat4, view: Mat4, proj: Mat4) {
        self.renderer.draw(world,root,view,proj);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn peach_disc_skin_has_finite_vertices_across_mario_clips() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        let cache = dir.join("melee-peach");
        if !cache.join("PlPeNr.dat").is_file() { eprintln!("SKIP: Melee Peach cache absent"); return; }
        let source = Skeleton::from_bmd(&std::fs::read(dir.join("bmd/ma_mdl1.bmd")).unwrap()).unwrap();
        let (model,target,_) = load_rig(&cache,&source).unwrap();
        let retarget = Retarget::new(&source,&target,&[]);
        for name in ["wait","run2","jump","dive_wait","hipat"] {
            let clip = libsms::Clip::parse(&std::fs::read(dir.join(format!("bck/ma_{name}.bck"))).unwrap()).unwrap();
            for frame in [0.,clip.frames*0.25,clip.frames*0.5,clip.frames*0.75] {
                let world = retarget.pose(&target,&source.pose(&clip,frame));
                assert!(world.iter().all(|m| m.is_finite()),"{name}:{frame}");
                for batch in 0..model.batches.len() {
                    for (p,n) in model.skin(batch,&world) {
                        assert!(p.is_finite() && n.is_finite() && p.length()<1000.,"{name}:{frame} {p:?}");
                    }
                }
            }
        }
    }
}
