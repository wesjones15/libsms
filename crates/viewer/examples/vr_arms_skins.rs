//! VR arms proof for every Mario skin (libcommon's cut, through `View::vr_bodies`): Mario,
//! Piantissimo and Yoshi from the Sunshine assets, Luigi from the Melee cache. One picture per
//! body (libcommon's `vr_arms::capture`): front and side in the bind pose, the right arm red and
//! the left blue, then each arm alone in its arm space.
//!   cargo run --release -p viewer --example vr_arms_skins -- <sms assets> <melee cache> <out dir>

use std::path::PathBuf;

use libcommon::vr_arms;
use libsms_present::View;
use libsms_present::view::Skin;

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().unwrap_or_else(|| "assets".into()));
    let melee = PathBuf::from(args.next().unwrap_or_else(|| "../libssbm/assets".into()));
    let out = PathBuf::from(args.next().unwrap_or_else(|| "vr_arms".into()));
    std::fs::create_dir_all(&out).unwrap();
    let mut v = View::load(&dir).unwrap();
    for skin in Skin::ALL {
        if !v.load_skin(skin, Some(&melee)) {
            println!("SKIP {skin:?}: not loadable");
        }
    }
    for (mesh, arms) in v.vr_bodies() {
        let label = format!("SUNSHINE {}", arms.name);
        let (w, h, px) = vr_arms::capture(&mesh, &arms, &label);
        let path = out.join(format!("{}.png", arms.name));
        std::fs::write(&path, vr_arms::png(w, h, &px)).unwrap();
        let tris: Vec<usize> = arms.arms.iter().map(|a| a.triangles.len()).collect();
        println!("{}: {} arms {tris:?} of {} -> {}", arms.name, arms.arms.len(), mesh.triangles.len(), path.display());
    }
}
