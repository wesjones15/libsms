//! Two sims given the same inputs end in the same state, step for step.
//!
//! This is what multiplayer rests on: another player's Mario is a shadow sim that replays the calls
//! his own machine made on the real one (`ror2-mario`'s MarioFrames), so what comes out of libsms
//! must depend on those and nothing else (no clock, no randomness the host can't see). Different
//! machines get the same inputs and the same ground, so they must get the same Mario.

use libsms::{FlatHost, Pad, Params, Sim, TICK_HZ, Vec2, Vec3};

/// A small deterministic generator for the pad (not the sim's own randomness).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 33) as f32) / (u32::MAX >> 1) as f32
    }
}

fn pad(rng: &mut Lcg) -> Pad {
    let mut p = Pad::default();
    let r = rng.next();
    p.stick = if r < 0.15 { Vec2::ZERO } else { Vec2::new(rng.next() * 2.0 - 1.0, rng.next() * 2.0 - 1.0) };
    p.a = rng.next() < 0.08;
    p.b = rng.next() < 0.05;
    p.l = rng.next() < 0.04;
    p.r = rng.next() < 0.5;
    p
}

#[test]
fn the_same_inputs_give_the_same_mario() {
    let mut a = Sim::new(Params::default(), Default::default(), Vec3::ZERO, 0);
    let mut b = Sim::new(Params::default(), Default::default(), Vec3::ZERO, 0);
    a.mario.give_fludd();
    b.mario.give_fludd();
    let mut host_a = FlatHost::default();
    let mut host_b = FlatHost::default();
    let mut rng = Lcg(12345);
    let mut held = Pad::default();
    for tick in 0..(TICK_HZ as usize * 60) {
        // A new input every few ticks, as a player's would be.
        if tick % 6 == 0 {
            held = pad(&mut rng);
        }
        let yaw = ((tick / 240) as i16).wrapping_mul(3000);
        a.advance(&mut host_a, &held, yaw, 1.0 / TICK_HZ as f32);
        b.advance(&mut host_b, &held, yaw, 1.0 / TICK_HZ as f32);
        if tick % 120 == 0 {
            assert_eq!(format!("{:?}", a.mario), format!("{:?}", b.mario), "diverged at tick {tick}");
        }
    }
    assert_eq!(format!("{:?}", a.mario), format!("{:?}", b.mario));
}
