//! Prints each batch's bind-pose bounding box and skinning, to check where parts sit.
//!
//!     cargo run -p libsms-assets --example bounds -- watergun2/body/wg_mdl1.bmd

use libsms_assets::{Bmd, Pose};

fn main() {
    let rel = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "bmd/ma_mdl1.bmd".into());
    let dir = libsms::assets::find(None).expect("no assets/ folder");
    let m = Bmd::parse(&std::fs::read(dir.join(&rel)).expect("read")).expect("parse");
    let pose = Pose::new(&m, glam::Mat4::IDENTITY, &m.bind_world());
    println!("draw mtx {:?}", m.draw_mtx);
    let bind = m.bind_world();
    for (j, jt) in m.joints.iter().enumerate() {
        let check = m
            .inverse_bind
            .get(j)
            .map(|ib| (bind[j] * *ib).abs_diff_eq(glam::Mat4::IDENTITY, 1e-3));
        if std::env::var("FULL").is_ok()
            && let Some(ib) = m.inverse_bind.get(j)
        {
            println!(
                "  bind {:?}
  inv(ib) {:?}",
                bind[j].to_cols_array(),
                ib.inverse().to_cols_array()
            );
        }
        println!(
            "joint {j} {} rot {:.3} t {:.1} s {:.2} comp {} bind*inv=I {:?}",
            jt.name, jt.rotation, jt.translation, jt.scale, jt.scale_compensate, check
        );
    }
    for b in m.batches().expect("batches") {
        let (mut lo, mut hi) = (glam::Vec3::splat(f32::MAX), glam::Vec3::splat(f32::MIN));
        let mut skins = std::collections::BTreeSet::new();
        for v in &b.vertices {
            let p = pose.skin(v).0;
            lo = lo.min(p);
            hi = hi.max(p);
            skins.insert(format!("{:?}", v.skin));
        }
        println!(
            "shape {} {}: {lo:.1} .. {hi:.1} {skins:?}",
            b.shape, m.materials[b.material].name
        );
    }
}
