//! Sunshine Yoshi wearing Mario's animation, for the standalone viewer.
//! Exact asset: yoshi.szs:yoshi_model.bmd (38 joints), loaded by TYoshi::init
//! in sms-main/src/Player/Yoshi.cpp. Motion: mario.szs:bck/ma_*.bck.
//! Host adaptation: the neck maps onto Mario's torso/head; tail and mouth inherit
//! their authored local transforms. This does not use Yoshi's riding animations.
use std::path::Path;
use glam::Mat4;
use libsms::Skeleton;
use libsms_assets::retarget::Retarget;

const ALIASES: &[(&str,&str)] = &[
    ("chn_neck","chn_chest"),
    ("jnt_neck_1","jnt_chest"),
    ("jnt_neck_2","jnt_chest"),
    ("chn_head","jnt_head"),
];

pub struct YoshiSkin {
    rig: Skeleton,
    retarget: Retarget,
}

impl YoshiSkin {
    pub fn load(dir:&Path,source:&Skeleton)->Result<Self,String> {
        let raw = std::fs::read(dir.join("yoshi/yoshi_model.bmd")).map_err(|e|e.to_string())?;
        let rig = Skeleton::from_bmd(&raw).ok_or("invalid Yoshi JNT1/INF1")?;
        if rig.len()!=38 { return Err(format!("expected Yoshi's 38 joints, got {}",rig.len())); }
        for &(target,name) in ALIASES {
            if rig.find(target).is_none() || source.find(name).is_none() {
                return Err(format!("missing retarget joint: {target} <- {name}"));
            }
        }
        let retarget = Retarget::new(source,&rig,ALIASES);
        eprintln!("Yoshi skin: yoshi.szs:yoshi_model.bmd, 38 joints; TYoshi::init (Player/Yoshi.cpp); Mario's 29-joint clips");
        Ok(Self{rig,retarget})
    }

    pub fn pose(&self,source:&[Mat4])->Vec<Mat4> { self.retarget.pose(&self.rig,source) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libsms_assets::{Bmd,Pose};
    #[test]
    fn mario_motion_keeps_yoshi_lengths_and_skins_finite_vertices() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        if !dir.join("yoshi/yoshi_model.bmd").is_file() { eprintln!("SKIP: Yoshi disc cache absent"); return; }
        let source = Skeleton::from_bmd(&std::fs::read(dir.join("bmd/ma_mdl1.bmd")).unwrap()).unwrap();
        let skin = YoshiSkin::load(&dir,&source).unwrap();
        let bmd = Bmd::parse(&std::fs::read(dir.join("yoshi/yoshi_model.bmd")).unwrap()).unwrap();
        let batches = bmd.batches().unwrap();
        let bind = skin.rig.bind_world();
        for (a,b) in skin.pose(&source.bind_world()).iter().zip(&bind) { assert!(a.abs_diff_eq(*b,0.001)); }
        for name in ["wait","run1","run2","jump","dive_wait","hipat","hipsr"] {
            let c = libsms::Clip::parse(&std::fs::read(dir.join(format!("bck/ma_{name}.bck"))).unwrap()).unwrap();
            for frame in [0.,c.frames*0.25,c.frames*0.5,c.frames*0.75] {
                let world = skin.pose(&source.pose(&c,frame));
                for (j,parent) in skin.rig.parents.iter().enumerate() {
                    let Some(p)=*parent else { continue };
                    let length=|w:&[Mat4]|w[j].w_axis.truncate().distance(w[p].w_axis.truncate());
                    assert!((length(&world)-length(&bind)).abs()<0.01,"{name}:{frame} bone {j}");
                }
                let pose = Pose::new(&bmd,Mat4::IDENTITY,&world);
                for v in batches.iter().flat_map(|b|&b.vertices) {
                    let (p,n)=pose.skin(v);
                    assert!(p.is_finite() && n.is_finite() && p.length()<2000.,"{name}:{frame}");
                }
            }
        }
    }
}
