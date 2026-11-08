//! Behavior checks for the code-only baseline, independent of presentation.
use libsms::{
    AnimTable, Event, FlatHost, Host, NozzleKind, Pad, Params, Sim, Status, Vec2, Vec3, TICK_HZ,
};

#[derive(Default)]
struct Floor { events: Vec<Event> }
impl Host for Floor {
    fn ground(&self, p: Vec3) -> Option<libsms::GroundHit> { FlatHost::default().ground(p) }
    fn event(&mut self, event: Event) { self.events.push(event); }
}
fn sim(params: Params) -> Sim {
    let mut sim = Sim::new(params, AnimTable::builtin(), Vec3::ZERO, 0);
    sim.mario.give_fludd();
    sim
}
fn run_jump(params: Params) -> (f32, f32, f32) {
    let mut sim = sim(params);
    let mut floor = Floor::default();
    let mut pad = Pad { stick: Vec2::Y, ..Pad::default() };
    for _ in 0..240 { sim.advance(&mut floor, &pad, 0, 1. / TICK_HZ as f32); }
    let before = sim.mario.forward_vel;
    pad.a = true;
    let at = sim.mario.pos;
    let mut peak = before;
    for tick in 0..30 {
        sim.advance(&mut floor, &pad, 0, 1. / TICK_HZ as f32);
        pad.a = tick < 15;
        peak = peak.max(sim.mario.forward_vel);
    }
    (before, peak, sim.mario.pos.z - at.z)
}
fn nozzle(params: Params, kind: NozzleKind) -> (usize, f32, f32, i32) {
    let mut sim = sim(params);
    let params = sim.mario.params.clone();
    sim.mario.fludd.as_mut().unwrap().change_nozzle(kind, false, &params);
    let mut floor = Floor::default();
    let pad = Pad { r: true, r_analog: 255, ..Pad::default() };
    let mut last_emit = 0;
    let mut peak_y = 0f32;
    for tick in 0..840 {
        floor.events.clear();
        sim.advance(&mut floor, &pad, 0, 1. / TICK_HZ as f32);
        if floor.events.iter().any(|e| matches!(e, Event::WaterEmit(_))) { last_emit = tick; }
        peak_y = peak_y.max(sim.mario.pos.y);
    }
    (last_emit, peak_y, sim.mario.pos.z, sim.mario.fludd.as_ref().unwrap().water)
}
/// Distance until the dive lands, then the belly slide's distance. Both are movement, not
/// animation: the action clocks can still differ without changing the ground covered.
fn dive(params: Params) -> (f32, f32) {
    let mut sim = sim(params);
    let mut floor = Floor::default();
    let pad = Pad { stick: Vec2::Y, ..Pad::default() };
    for _ in 0..120 { sim.advance(&mut floor, &pad, 0, 1. / TICK_HZ as f32); }
    let start = sim.mario.pos;
    let jump = Pad { a: true, ..pad };
    for _ in 0..10 { sim.advance(&mut floor, &jump, 0, 1. / TICK_HZ as f32); }
    let dive = Pad { b: true, ..pad };
    sim.advance(&mut floor, &dive, 0, 1. / TICK_HZ as f32);
    let mut slide_from = None;
    for _ in 0..600 {
        sim.advance(&mut floor, &pad, 0, 1. / TICK_HZ as f32);
        match sim.mario.status {
            Status::JumpCatch => {}
            Status::Catch | Status::CatchDown | Status::CatchLost => {
                slide_from.get_or_insert(sim.mario.pos);
            }
            _ => break,
        }
    }
    let land = slide_from.unwrap_or(sim.mario.pos);
    ((land.z - start.z).abs(), (sim.mario.pos.z - land.z).abs())
}
/// Ticks from the pound's start to its landing, and the fall it reports.
fn pound(params: Params) -> (u32, f32) {
    let mut sim = sim(params);
    let mut floor = Floor::default();
    let jump = Pad { a: true, ..Pad::default() };
    for _ in 0..40 { sim.advance(&mut floor, &jump, 0, 1. / TICK_HZ as f32); }
    let press = Pad { l: true, ..Pad::default() };
    sim.advance(&mut floor, &press, 0, 1. / TICK_HZ as f32);
    let mut ticks = 0;
    let mut drop = 0.;
    for _ in 0..600 {
        sim.advance(&mut floor, &Pad::default(), 0, 1. / TICK_HZ as f32);
        ticks += 1;
        if let Some(d) = floor.events.iter().find_map(|e| match e {
            Event::GroundPound { drop, .. } => Some(*drop),
            _ => None,
        }) { drop = d; break; }
    }
    (ticks, drop)
}

#[test]
fn ordinary_run_jump_has_no_free_quail_burst() {
    let (before, peak, _) = run_jump(Params::builtin());
    assert!(before > 20.);
    assert!(peak < before * 1.1, "ordinary jump accelerated {before} -> {peak}");
}
#[test]
fn spray_lasts_beyond_five_seconds_and_release_resets_pressure() {
    let (last, _, _, water) = nozzle(Params::builtin(), NozzleKind::Spray);
    assert!((600..800).contains(&last), "last spray tick {last}");
    assert!(water > 0 && water < 10_000);
    let mut sim = sim(Params::builtin());
    let mut floor = Floor::default();
    let held = Pad { r: true, r_analog: 255, ..Pad::default() };
    for _ in 0..840 { sim.advance(&mut floor, &held, 0, 1. / TICK_HZ as f32); }
    sim.advance(&mut floor, &Pad::default(), 0, 1. / TICK_HZ as f32);
    floor.events.clear();
    for _ in 0..12 { sim.advance(&mut floor, &held, 0, 1. / TICK_HZ as f32); }
    assert!(floor.events.iter().any(|e| matches!(e, Event::WaterEmit(_))));
}
#[test]
fn optional_reference_checks_jump_and_all_four_nozzles() {
    let Some(dir) = libsms::assets::find(None) else {
        eprintln!("SKIP reference comparison: no local Sunshine cache"); return;
    };
    let (reference, _) = Params::from_dir(&dir.join("params_mario"));
    let builtin = run_jump(Params::builtin());
    let expected = run_jump(reference.clone());
    eprintln!("run/jump baseline={builtin:?} reference={expected:?}");
    let mut same = (builtin.1 - expected.1).abs() < 1. && (builtin.2 - expected.2).abs() < 5.;
    for kind in [NozzleKind::Spray, NozzleKind::Hover, NozzleKind::Rocket, NozzleKind::Turbo] {
        let actual = nozzle(Params::builtin(), kind);
        let expected = nozzle(reference.clone(), kind);
        eprintln!("{kind:?} baseline={actual:?} reference={expected:?}");
        same &= actual.0.abs_diff(expected.0) < 5 && (actual.1 - expected.1).abs() < 20.
            && (actual.2 - expected.2).abs() < 20. && (actual.3 - expected.3).abs() < 10;
    }
    assert!(same, "code-only baseline diverged from the reference movement/nozzles");
}

/// The belly slide and the ground pound are RoR2 actions too: the fallback must cover the
/// same ground and fall at the same speed as the asset-carrying profile.
#[test]
fn optional_reference_checks_slide_and_pound_actions() {
    let Some(dir) = libsms::assets::find(None) else {
        eprintln!("SKIP reference comparison: no local Sunshine cache"); return;
    };
    let (reference, _) = Params::from_dir(&dir.join("params_mario"));
    let dive_builtin = dive(Params::builtin());
    let dive_reference = dive(reference.clone());
    eprintln!("dive baseline={dive_builtin:?} reference={dive_reference:?}");
    assert!(
        (dive_builtin.0 - dive_reference.0).abs() < 5.
            && (dive_builtin.1 - dive_reference.1).abs() < 5.,
        "the dive/slide covered different ground: {dive_builtin:?} vs {dive_reference:?}"
    );
    let pound_builtin = pound(Params::builtin());
    let pound_reference = pound(reference);
    eprintln!("pound baseline={pound_builtin:?} reference={pound_reference:?}");
    assert!(
        pound_builtin.0.abs_diff(pound_reference.0) <= 2
            && (pound_builtin.1 - pound_reference.1).abs() < 1.,
        "the pound fell differently: {pound_builtin:?} vs {pound_reference:?}"
    );
}
