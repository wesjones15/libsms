//! Disc-dependent boundary check for the Sunshine Peach demo; no game data is committed.
use glam::Mat4;
use libsms::{Clip, Skeleton};
use libsms_assets::{Bmd, Pose, retarget::Retarget};

#[test]
fn peach_keeps_her_bind_pose_and_skins_mario_motion_without_invalid_vertices() {
    let Some(dir) = libsms::assets::find(None) else { eprintln!("SKIP: Sunshine cache absent"); return; };
    let path = dir.join("npc/peach/peach_model.bmd");
    if !path.is_file() { eprintln!("SKIP: Peach cache absent (sms-viewer --peach extracts it)"); return; }
    let source = Skeleton::from_bmd(&std::fs::read(dir.join("bmd/ma_mdl1.bmd")).unwrap()).unwrap();
    let bytes = std::fs::read(path).unwrap();
    let target = Skeleton::from_bmd(&bytes).unwrap();
    let model = Bmd::parse(&bytes).unwrap();
    assert_eq!((source.len(),target.len()),(29,26));
    let retarget = Retarget::new(&source,&target,&[("kubi","jnt_head"),("jnt_skirt","jnt_waist")]);
    for (a,b) in retarget.pose(&target,&source.bind_world()).iter().zip(target.bind_world()) {
        assert!(a.abs_diff_eq(b,0.001),"retargeting a rest pose must preserve Peach's proportions");
    }
    let batches = model.batches().unwrap();
    for name in ["wait","run1","run2","jump","dive_wait","hipat","hipsr"] {
        let clip = Clip::parse(&std::fs::read(dir.join(format!("bck/ma_{name}.bck"))).unwrap()).unwrap();
        for frame in [0.,clip.frames*0.25,clip.frames*0.5,clip.frames*0.75] {
            let joints = retarget.pose(&target,&source.pose(&clip,frame));
            let rest = target.bind_world();
            for side in ["L","R"] {
                let chain = [format!("jnt_leg_{side}1"),format!("jnt_leg_{side}2"),format!("jnt_foot_{side}")];
                let ids: Vec<_> = chain.iter().map(|n| target.find(n).unwrap()).collect();
                for pair in ids.windows(2) {
                    let length = |w: &[Mat4]| w[pair[0]].w_axis.truncate().distance(w[pair[1]].w_axis.truncate());
                    assert!((length(&joints)-length(&rest)).abs()<0.001,"Peach leg length changed: {name}:{frame}");
                }
            }
            assert_eq!(joints.len(),26);
            assert!(joints.iter().all(|m| m.is_finite()),"{name}:{frame}");
            let pose = Pose::new(&model,Mat4::IDENTITY,&joints);
            for v in batches.iter().flat_map(|b| &b.vertices) {
                let (p,n) = pose.skin(v);
                assert!(p.is_finite() && n.is_finite() && p.length()<2000.,"{name}:{frame} {p:?}");
            }
        }
    }
}
