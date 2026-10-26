//! Sunshine Peach as a viewer-only Mario skin; no RoR2 adapter changes.
//! Exact body: airport0.szs:peach/peach_model.bmd, 26 joints.
//! Spec: TPeachManager::createModelData (NpcManager.cpp), sPeach_*Data
//! (NpcInitData.cpp), TNpcParts texture replacement/partsPerform (NpcParts.cpp).
use std::path::Path;
use glam::{Mat4,Vec3};
use libsms::{Clip, Skeleton};
use libsms_assets::{Pose, retarget::Retarget};
use crate::{j3d::{Lights, Look, Pass, Renderer}, mario_model::Part};

pub struct Peach {
    renderer: Renderer,
    body: Part,
    parts: Vec<(Part, usize, Option<Clip>)>,
    rig: Skeleton,
    retarget: Retarget,
    chest_align: Mat4,
    dress_clip: Clip,
    dress_profile: (Vec3,f32,Vec3,Vec3),
}

/// Uses the existing ISO/Yaz0/RARC readers; data remains in the ignored local cache.
pub fn ensure_assets(dir: &Path, disc_dirs: &[std::path::PathBuf]) -> Result<(), String> {
    let required = ["peach_model.bmd", "peach_hair_normal.bmd", "peach_hand1_l.bmd", "peach_hand1_r.bmd"];
    let dest = dir.join("npc/peach");
    if required.iter().all(|p| dest.join(p).is_file()) { return Ok(()); }
    let mut candidates = Vec::new();
    if let Some(p) = std::env::var_os("SMS_DISC") { candidates.push(p.into()); }
    for folder in disc_dirs {
        for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
            let p = entry.path();
            if p.extension().and_then(|s| s.to_str()).is_some_and(|s|
                ["iso", "gcm", "ciso"].iter().any(|x| s.eq_ignore_ascii_case(x))) {
                candidates.push(p);
            }
        }
    }
    for path in candidates {
        let Ok(mut disc) = libsms::extract::Disc::open(&path) else { continue };
        if !disc.game_id.starts_with("GMS") { continue }
        let Some(arc) = disc.paths_under("").into_iter().find(|p| p.ends_with("/airport0.szs")) else { continue };
        let raw = disc.read_file(&arc).map_err(|e| e.to_string())?;
        let raw = if libsms::extract::yaz0::is_yaz0(&raw) {
            libsms::extract::yaz0::decompress(&raw).map_err(|e| e.to_string())?
        } else { raw };
        let archive = libsms::extract::rarc::Archive::parse(raw).map_err(|e| e.to_string())?;
        for (rel, bytes) in archive.under("peach/") {
            if !Path::new(rel).components().all(|c| matches!(c, std::path::Component::Normal(_))) {
                return Err("invalid Peach archive path".into());
            }
            let output = dest.join(rel);
            std::fs::create_dir_all(output.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::write(output, bytes).map_err(|e| e.to_string())?;
        }
        if required.iter().all(|p| dest.join(p).is_file()) {
            eprintln!("Peach: {}:{} -> {} (TPeachManager::createModelData; 26 joints)", path.display(), arc, dest.display());
            return Ok(());
        }
    }
    Err("Peach needs your Sunshine ISO (pass --iso <path>) to extract airport0.szs".into())
}

impl Peach {
    pub fn load(dir: &Path, source: &Skeleton) -> Result<Self, String> {
        let dir = dir.join("npc/peach");
        let bytes = std::fs::read(dir.join("peach_model.bmd")).map_err(|e| e.to_string())?;
        let rig = Skeleton::from_bmd(&bytes).ok_or("Peach's JNT1/INF1 rig is invalid")?;
        if rig.len() != 26 { return Err(format!("expected Peach's 26-joint rig, got {}", rig.len())); }
        let retarget = Retarget::new(source, &rig, &[("kubi", "jnt_head")]);
        let s = source.bind_world()[source.find("jnt_chest").ok_or("Mario chest missing")?];
        let t = rig.bind_world()[rig.find("jnt_chest").ok_or("Peach chest missing")?];
        let chest_align = Mat4::from_quat(t.to_scale_rotation_translation().1.inverse() * s.to_scale_rotation_translation().1);
        let mut renderer = Renderer::new();
        let body = Part::load(&mut renderer, &dir, "peach_model.bmd")?;
        let dress_clip = Clip::parse(&std::fs::read(dir.join("peach_wait.bck")).map_err(|e|e.to_string())?)
            .ok_or("invalid Peach dress donor clip")?;
        let bind_pose = Pose::new(&body.model.bmd,Mat4::IDENTITY,&body.bind);
        let mut points = Vec::new();
        for batch in body.model.bmd.batches().map_err(|e|e.to_string())? {
            for v in batch.vertices {
                let dress = match v.skin {
                    libsms_assets::Skin::Joint(j) => [9,10,11].contains(&j),
                    libsms_assets::Skin::Envelope(e) => body.model.bmd.envelopes[e as usize].iter().any(|&(j,_)|[9,10,11].contains(&j)),
                };
                if dress { points.push(bind_pose.skin(&v).0); }
            }
        }
        let pivot = body.bind[10].w_axis.truncate();
        let height = pivot.y-points.iter().map(|p|p.y).fold(f32::INFINITY,f32::min);
        let extent = |upper:bool| points.iter().filter(|p| if upper {p.y>pivot.y-height*0.2} else {p.y<pivot.y-height*0.75})
            .fold(Vec3::splat(1.),|a,p|a.max(Vec3::new(p.x.abs(),1.,p.z.abs())));
        let bottom = extent(false);
        let top = extent(true).max(bottom*0.30);
        let dress_profile = (pivot,height,top,bottom);
        eprintln!("Sunshine dress: joints 9/10/11; peach_wait.bck + TMtxTimeLag on koshi_null (NpcInitPrg.cpp); profile={dress_profile:?}");
        let mut parts = Vec::new();
        for (file, joint, animation) in [
            ("peach_hair_normal.bmd", "kubi", Some("peach_hair_normal_wait.bck")),
            ("peach_hand1_l.bmd", "jnt_hand_L", None),
            ("peach_hand1_r.bmd", "jnt_hand_R", None),
        ] {
            let mut part = Part::load(&mut renderer, &dir, file)?;
            // TNpcParts::init: replace H_peach_main_dummy by the body's H_peach_main_s3tc.
            if let Some(host) = body.model.bmd.textures.iter().position(|t| t.name == "H_peach_main_s3tc") {
                for (i, tex) in part.model.bmd.textures.iter().enumerate() {
                    if tex.name == "H_peach_main_dummy" { part.model.textures[i] = body.model.textures[host]; }
                }
            }
            let clip = animation.and_then(|a| std::fs::read(dir.join(a)).ok()).and_then(|b| Clip::parse(&b));
            parts.push((part, rig.find(joint).ok_or_else(|| format!("Peach missing {joint}"))?, clip));
        }
        Ok(Self { renderer, body, parts, rig, retarget, chest_align, dress_clip, dress_profile })
    }

    pub fn pose(&self, source_world: &[Mat4], source_lowest: f32, clip: &str, phase: f32) -> Vec<Mat4> {
        let gait = clip.starts_with("run") || clip.contains("walk");
        let mut world = self.retarget.pose_weighted(&self.rig, source_world,|n|libsms_assets::dress::gait_weight(n,gait));
        if std::env::var("SMS_DRESS").as_deref()!=Ok("off") {
            let donor = self.rig.pose(&self.dress_clip,phase*self.dress_clip.frames);
            libsms_assets::dress::graft(&self.rig,&mut world,&donor,9);
            let (pivot,height,top,bottom) = self.dress_profile;
            libsms_assets::dress::cone(&self.rig,&mut world,1,[10,11],pivot,height,top,bottom);
        }
        let pose = Pose::new(&self.body.model.bmd, Mat4::IDENTITY, &world);
        let low = self.body.model.bmd.batches().expect("Peach geometry").iter()
            .flat_map(|b| &b.vertices).map(|v| pose.skin(v).0.y).fold(f32::INFINITY, f32::min);
        let offset = source_lowest - low;
        for m in &mut world { m.w_axis.y += offset; }
        world
    }

    pub fn chest(&self, world: &[Mat4]) -> Mat4 {
        world[self.rig.find("jnt_chest").unwrap()] * self.chest_align
    }

    pub fn draw(&mut self, world: &[Mat4], root: Mat4, view: Mat4, proj: Mat4, time: f32) {
        let lights = Lights::daylight(view);
        let look = Look::default();
        let pose = Pose::new(&self.body.model.bmd, root, world);
        for pass in [Pass::Opaque, Pass::Translucent] {
            self.renderer.draw(&mut self.body.model, &pose, view, proj, &lights, &look, pass);
            for (part, joint, clip) in &mut self.parts {
                let local = part.posed(clip.as_ref(), time * 30.0, |_| None);
                let attached = Pose::new(&part.model.bmd, root * world[*joint], &local);
                self.renderer.draw(&mut part.model, &attached, view, proj, &lights, &look, pass);
            }
        }
    }
}
