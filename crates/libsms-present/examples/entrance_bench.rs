//! CPU timings for the native pod view; no graphics device or audio output required.
use glam::Vec3;
use libsms_present::{View, view::Entrance};
use std::{path::Path, time::Instant};
fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "assets".into());
    let start = Instant::now();
    let fast = std::env::args().any(|a| a == "--fast");
    if fast {
        libsms_present::portal_cache::prepare(Path::new(&dir)).unwrap();
    }
    let mut view = View::load(Path::new(&dir)).unwrap();
    let mut entrance = if fast {
        Entrance::load_fast(Path::new(&dir), &mut view).unwrap()
    } else {
        Entrance::load(Path::new(&dir), &mut view).unwrap()
    };
    println!("load {:.1}ms", start.elapsed().as_secs_f64() * 1000.);
    for (label, offset, active) in [
        ("idle", 0., false),
        ("exit", 0., true),
        ("after exit", 10., true),
    ] {
        let mut worst = 0f64;
        let start = Instant::now();
        let mut uploaded = 0;
        for i in 0..120 {
            let at = Instant::now();
            let t = offset + i as f32 / 60.;
            if fast {
                entrance.update_fast(
                    &mut view,
                    t,
                    if !active {
                        0
                    } else if t < libsms_present::portal_cache::COMPLETE {
                        1
                    } else {
                        2
                    },
                    Vec3::X,
                    Vec3::Y,
                );
            } else {
                entrance.update(&mut view, t, active, Vec3::X, Vec3::Y);
            }
            uploaded += view.positions.len();
            worst = worst.max(at.elapsed().as_secs_f64() * 1000.);
        }
        println!(
            "{label}: mean {:.3}ms, max {worst:.3}ms, mean uploaded vertices {}",
            start.elapsed().as_secs_f64() * 1000. / 120.,
            uploaded / 120
        );
    }
}
