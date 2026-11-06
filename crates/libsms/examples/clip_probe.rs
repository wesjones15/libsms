//! Prints where a joint goes over a Mario clip (relative to his root, facing +Z), to pick
//! frames by eye: `cargo run -p libsms --example clip_probe -- draw jnt_hand_L`.
use libsms::skeleton::{Clip, Skeleton};

fn main() {
    let mut args = std::env::args().skip(1);
    let clip = args.next().unwrap_or_else(|| "draw".into());
    let joint = args.next().unwrap_or_else(|| "jnt_hand_L".into());
    let dir = libsms::assets::find(None).expect("no assets/ folder");
    let skel = Skeleton::from_bmd(&std::fs::read(dir.join("bmd/ma_mdl1.bmd")).unwrap()).unwrap();
    let c = Clip::parse(&std::fs::read(dir.join(format!("bck/ma_{clip}.bck"))).unwrap()).unwrap();
    let j = skel.find(&joint).expect("joint");
    let len = c.frames;
    println!("{clip}: {len} frames");
    let mut f = 0.0;
    while f <= len {
        let w = skel.pose(&c, f);
        let p = w[j].w_axis;
        let d = w[j].z_axis;
        println!("{f:5.0}  pos {:7.1} {:7.1} {:7.1}   z {:5.2} {:5.2} {:5.2}", p.x, p.y, p.z, d.x, d.y, d.z);
        f += 4.0;
    }
}
