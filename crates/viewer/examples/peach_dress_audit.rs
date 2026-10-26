//! Headless asset identity and dress influence audit; reads the user's local caches only.
use libssbm_assets::{model::Model,dat::Dat,fighter::Archive};
use std::path::Path;
fn main() -> Result<(),Box<dyn std::error::Error+Send+Sync>> {
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let cache = assets.join("melee-peach");
    let model = Model::load_costume(&cache,"PlPeNr.dat",Archive{file:"PlPe.dat",root:"ftDataPeach"},0,None)?;
    println!("PlPeNr.dat:PlyPeach5K_Share_joint: {} joints",model.joints.len());
    for (i,j) in model.joints.iter().enumerate() {
        let influence = model.batches.iter().flat_map(|b| &b.vertices)
            .filter(|v| v.weights.iter().any(|&(k,w)| k==i && w>0.01)).count();
        println!("{i:3} parent={:?} local={:?} world={:?} influence={influence}",j.parent,j.translation,j.bind.w_axis.truncate());
    }
    let raw = std::fs::read(cache.join("PlPe.dat"))?;
    let d = Dat::parse(&raw)?;
    let root = d.root("ftDataPeach")?;
    if let Some(dyns) = d.ptr(root+0x2c)? {
        println!("DYNAMICS at {dyns:#x} count={} templates={:?}",d.u32(dyns)?,d.ptr(dyns+4)?);
        let table = d.required_ptr(dyns+4)?;
        for i in 0..d.u32(dyns)? as usize {
            let p = table + i*24;
            println!("DYNAMIC {i}: root={} count={} params={:?}",d.u32(p)?,d.u32(p+8)?,d.ptr(p+4)?);
        }
    }
    let raw = std::fs::read(assets.join("npc/peach/peach_model.bmd"))?;
    let bmd = libsms_assets::Bmd::parse(&raw)?;
    println!("Sunshine dress envelopes: {:?}",bmd.envelopes.iter().enumerate().filter(|(_,w)|w.iter().any(|&(i,_)|[9,10,11].contains(&i))).collect::<Vec<_>>());
    for name in ["peach_wait","peach_parasol_wait"] {
        let c = libsms::Clip::parse(&std::fs::read(assets.join(format!("npc/peach/{name}.bck")))?).unwrap();
        for frame in [0.,15.,30.,45.] {
            println!("{name}:{frame} skirt={:?} lag={:?} hem={:?}",c.sample(9,frame),c.sample(10,frame),c.sample(11,frame));
        }
    }
    Ok(())
}
