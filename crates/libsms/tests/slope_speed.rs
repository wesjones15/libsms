//! Mario's ground speed on clean ramps with the disc's tuning. `walkProcess` (MarioPhysics.cpp)
//! scales each step by the floor normal's y, so he keeps his speed along the surface and loses
//! only cos(angle) of his ground progress; `slopeProcess` adds the disc's slope acceleration (zero
//! in the shipped SlipParam). RoR2's terrain is mostly slope, so this guards against the core
//! itself making uphills sticky; host-side stickiness (probes, wall guard) is checked in game.
use libsms::{AnimTable, Host, Pad, Params, Sim, Status, TICK_HZ, Vec2, Vec3};

struct Ramp {
    tan: f32,
    n: Vec3,
}
impl Host for Ramp {
    fn ground(&self, p: Vec3) -> Option<libsms::GroundHit> {
        let h = p.z * self.tan;
        (p.y >= h - 0.01).then_some(libsms::GroundHit { height: h, normal: self.n, flags: libsms::SurfaceFlags::NONE })
    }
}

/// Horizontal speed (units/s) over the third second of running, and the status then.
fn run(params: &Params, anims: &AnimTable, deg: f32, uphill: bool) -> (f32, Status) {
    let a = deg.to_radians();
    let mut host = Ramp { tan: a.tan(), n: Vec3::new(0.0, a.cos(), -a.sin()) };
    let z0 = if uphill { 0.0 } else { 20000.0 };
    let mut sim = Sim::new(params.clone(), anims.clone(), Vec3::new(0.0, z0 * a.tan(), z0), 0);
    sim.mario.face_yaw = if uphill { 0 } else { i16::MIN };
    let pad = Pad { stick: Vec2::new(0.0, if uphill { 1.0 } else { -1.0 }), ..Pad::default() };
    let dt = 1.0 / TICK_HZ as f32;
    for _ in 0..2 * TICK_HZ {
        sim.advance(&mut host, &pad, i16::MIN, dt);
    }
    let z1 = sim.mario.pos.z;
    for _ in 0..TICK_HZ {
        sim.advance(&mut host, &pad, i16::MIN, dt);
    }
    ((sim.mario.pos.z - z1).abs(), sim.mario.status)
}

#[test]
fn uphill_keeps_speed_along_the_surface() {
    let Some(dir) = libsms::assets::find(None) else {
        eprintln!("SKIP: no assets/ (disc tuning)");
        return;
    };
    let (params, _) = Params::from_dir(&dir.join("params_mario"));
    let anims = AnimTable::from_dir(&dir.join("bck"));
    let (flat, _) = run(&params, &anims, 0.0, true);
    for deg in [10.0f32, 20.0, 30.0, 40.0, 50.0] {
        for uphill in [true, false] {
            let (speed, status) = run(&params, &anims, deg, uphill);
            eprintln!("{deg} deg {}: {speed:.0} of {flat:.0} units/s, {status:?}", if uphill { "up" } else { "down" });
            assert_eq!(status, Status::Run, "{deg} deg: he should still be running");
            // cos(angle) of the flat speed, less a margin for the run's own wobble.
            let floor = flat * deg.to_radians().cos() * 0.9;
            assert!(speed >= floor, "{deg} deg {}: {speed} under {floor}", if uphill { "up" } else { "down" });
        }
    }
}
