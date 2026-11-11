//! Prints the statue pedestal's bounds exactly as the host's collider takes them (`Entrance::pedestal_bounds`), in sim units.
use libsms_present::view::{Entrance, View};

fn main() {
    let dir = libsms::assets::find(None).expect("no assets/ folder");
    let mut view = View::load(&dir).expect("view");
    let e = Entrance::load_fast(&dir, &mut view).expect("entrance");
    let (lo, hi) = e.pedestal_bounds(&view);
    println!("lo {lo:?}\nhi {hi:?}\nsize {:?}\nhost box centre (-(lo.x+hi.x), lo.y+hi.y, lo.z+hi.z)/2 = {:?}", hi - lo,
        glam::Vec3::new(-(lo.x + hi.x), lo.y + hi.y, lo.z + hi.z) * 0.5);
}
