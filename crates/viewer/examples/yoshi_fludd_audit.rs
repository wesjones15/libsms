//! Local-disc identity audit for Mari-Yoshi's belt and mouth spray.
use glam::{Mat4,Vec3};
use libsms_assets::{Bmd,Pose};
fn main()->Result<(),Box<dyn std::error::Error+Send+Sync>> {
    let dir=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let b=Bmd::parse(&std::fs::read(dir.join("watergun2/body/wg_mdl1.bmd"))?)?;
    println!("wg_mdl1.bmd joints:");
    for (i,j) in b.joints.iter().enumerate() {println!("{i}: {} parent={:?}",j.name,j.parent);}
    let pose=Pose::new(&b,Mat4::IDENTITY,&b.bind_world());
    for batch in b.batches()? {
        let (mut lo,mut hi)=(Vec3::splat(f32::INFINITY),Vec3::splat(f32::NEG_INFINITY));
        let mut skins=std::collections::BTreeSet::new();
        for v in &batch.vertices {let p=pose.skin(v).0;lo=lo.min(p);hi=hi.max(p);skins.insert(format!("{:?}",v.skin));}
        println!("shape {} material {} bounds {lo:?}..{hi:?} skins {skins:?}",batch.shape,b.materials[batch.material].name);
    }
    let (params,_)=libsms::Params::from_dir(&dir.join("params_mario"));
    println!("Mouth params: {:?}",params.nozzle(libsms::NozzleKind::Yoshi));
    let y=Bmd::parse(&std::fs::read(dir.join("yoshi/yoshi_model.bmd"))?)?;
    for name in ["chn_head","jnt_head","null_tongue","null_nose","jnt_mouth"] {
        let i=y.joint(name).unwrap();println!("Yoshi {i}: {name} bind={:?}",y.bind_world()[i]);
    }
    Ok(())
}
