//! Headless scenarios: drive Mario on a flat test host and check behaviour against the decomp.
//!
//! If the user's disc data has been extracted to `../extracted/sms` , the
//! tests run with the real `.prm` tuning and animation lengths; otherwise they use the code
//! defaults. Assertions are written to hold either way unless a test says otherwise.

use std::path::PathBuf;

use libsms::{
    AnimTable, Event, FlatHost, Host, Mario, NozzleKind, Pad, Params, Sim, Status, TICK_HZ, Vec2,
    Vec3, WaterEmit, POSE_DIVE, POSE_POUND, POSE_SLIDE, POSE_SPIN,
};

/// The git-ignored `assets/` folder built by `tools/extract_assets.py`, if present.
fn extracted() -> Option<PathBuf> {
    libsms::assets::find(None)
}

fn params() -> (Params, bool) {
    match extracted() {
        Some(dir) => {
            let (p, n) = Params::from_dir(&dir.join("params_mario"));
            assert!(n > 10, "only {n} .prm files found");
            (p, true)
        }
        None => (Params::default(), false),
    }
}

/// Tests whose numbers depend on the shipped tuning (nozzle tanks and rates, slip thresholds)
/// are skipped without the user's disc files; run `python tools/extract_assets.py <iso>`.
fn need_disc() -> bool {
    let ok = params().1;
    if !ok {
        eprintln!("skipped: needs assets/ from tools/extract_assets.py");
    }
    !ok
}

fn anims() -> AnimTable {
    extracted()
        .map(|d| AnimTable::from_dir(&d.join("bck")))
        .unwrap_or_default()
}

/// Records events and delegates the world to an inner host.
struct Rig<H: Host> {
    world: H,
    events: Vec<Event>,
}

impl<H: Host> Host for Rig<H> {
    fn ground(&self, p: Vec3) -> Option<libsms::GroundHit> {
        self.world.ground(p)
    }
    fn roof(&self, p: Vec3) -> Option<libsms::RoofHit> {
        self.world.roof(p)
    }
    fn walls(&self, c: Vec3, r: f32, m: usize, out: &mut Vec<libsms::WallHit>) -> Vec3 {
        self.world.walls(c, r, m, out)
    }
    fn event(&mut self, e: Event) {
        self.events.push(e);
    }
    fn takeables(&self, p: Vec3, r: f32, out: &mut Vec<libsms::host::Takeable>) {
        self.world.takeables(p, r, out)
    }
    fn take(&mut self, id: u64) -> bool {
        self.world.take(id)
    }
    fn enemies(&self, p: Vec3, r: f32, out: &mut Vec<libsms::Enemy>) {
        self.world.enemies(p, r, out)
    }
    fn take_enemy(&mut self, id: u64) -> bool {
        self.world.take_enemy(id)
    }
    fn water_level(&self, p: Vec3) -> Option<f32> {
        self.world.water_level(p)
    }
}

struct Scene<H: Host> {
    sim: Sim,
    host: Rig<H>,
    pad: Pad,
}

impl Scene<FlatHost> {
    fn flat() -> Scene<FlatHost> {
        Scene::new(FlatHost::default())
    }
}

impl<H: Host> Scene<H> {
    fn new(world: H) -> Scene<H> {
        let (params, _) = params();
        Scene {
            sim: Sim::new(params, anims(), Vec3::ZERO, 0),
            host: Rig {
                world,
                events: Vec::new(),
            },
            pad: Pad::default(),
        }
    }

    fn mario(&self) -> &Mario {
        &self.sim.mario
    }

    /// Runs `n` ticks (120 Hz) with the current pad.
    fn step(&mut self, n: u32) {
        for _ in 0..n {
            // Camera behind +Z looking toward +Z means camera_yaw = 0x8000 (Mario -> camera
            // points to -Z). Stick-up then walks toward +Z (face yaw 0).
            self.sim
                .advance(&mut self.host, &self.pad, i16::MIN, 1.0 / TICK_HZ as f32);
        }
    }

    fn secs(&mut self, s: f32) {
        self.step((s * TICK_HZ as f32) as u32);
    }

    fn press_a(&mut self) {
        self.pad.a = true;
        self.step(1);
        self.pad.a = false;
    }

    fn run_forward(&mut self) {
        self.pad.stick = Vec2::new(0.0, 1.0);
    }

    fn until(&mut self, max_ticks: u32, f: impl Fn(&Mario) -> bool) -> bool {
        for _ in 0..max_ticks {
            if f(self.mario()) {
                return true;
            }
            self.step(1);
        }
        f(self.mario())
    }

    fn event_count(&self, f: impl Fn(&Event) -> bool) -> usize {
        self.host.events.iter().filter(|e| f(e)).count()
    }
}

#[test]
fn standing_still_stays_still() {
    let mut s = Scene::flat();
    s.secs(2.0);
    assert_eq!(s.mario().status, Status::Wait);
    assert_eq!(s.mario().pos, Vec3::ZERO);
}

#[test]
fn stick_up_runs_away_from_the_camera() {
    let mut s = Scene::flat();
    s.run_forward();
    s.secs(1.0);
    let m = s.mario();
    assert_eq!(m.status, Status::Run);
    assert!(m.pos.z > 100.0, "pos {:?}", m.pos);
    assert!(m.pos.x.abs() < 5.0, "drifted sideways: {:?}", m.pos);
    assert!(m.speed() > 10.0, "speed {}", m.speed());
}

#[test]
fn run_speed_is_capped_by_the_stick() {
    let mut s = Scene::flat();
    s.run_forward();
    s.secs(4.0);
    let m = s.mario();
    // Full stick is at most 32 units/frame of intended speed (run.max_speed).
    assert!(m.speed() <= 32.5, "speed {}", m.speed());
    assert!(m.speed() >= 25.0, "speed {}", m.speed());
}

#[test]
fn releasing_the_stick_stops_mario() {
    let mut s = Scene::flat();
    s.run_forward();
    s.secs(1.0);
    s.pad.stick = Vec2::ZERO;
    s.secs(2.0);
    let m = s.mario();
    assert_eq!(m.status, Status::Wait, "{:?} v={}", m.status, m.speed());
    assert!(m.speed().abs() < 0.01);
}

#[test]
fn reversing_the_stick_turns_around() {
    let mut s = Scene::flat();
    s.run_forward();
    s.secs(1.0);
    s.pad.stick = Vec2::new(0.0, -1.0);
    // Mario skids (Turn/Brake path), then heads the other way.
    s.secs(2.0);
    let m = s.mario();
    assert!(m.pos.z < s.mario().pos.z + 1.0);
    let turned = m.face_yaw.unsigned_abs() > 0x6000;
    assert!(turned, "face {:#x} status {:?}", m.face_yaw, m.status);
}

#[test]
fn standing_jump_height_matches_physics() {
    if need_disc() {
        return;
    }
    let mut s = Scene::flat();
    s.pad.a = true; // holding jump gives the full-height jump
    s.step(1);
    let mut apex = 0.0f32;
    let mut ticks = 0;
    while s.mario().pos.y > 0.0 || ticks < 2 {
        s.step(1);
        apex = apex.max(s.mario().pos.y);
        ticks += 1;
        assert!(ticks < 1000, "never landed");
    }
    // vel.y = 42 and the position advances by vel/4 per tick: apex ~ 42^2/(8*g), g = 1.0 on the
    // disc (0.8 in code defaults), so 220..280 either way.
    assert!((200.0..300.0).contains(&apex), "apex {apex}");
    assert_eq!(
        s.event_count(|e| matches!(e, Event::Jump(libsms::JumpKind::Normal))),
        1
    );
    assert_eq!(s.event_count(|e| matches!(e, Event::Land { .. })), 1);
}

#[test]
fn releasing_jump_early_cuts_the_jump_short() {
    fn apex(hold_ticks: u32) -> f32 {
        let mut s = Scene::flat();
        let mut apex = 0.0f32;
        s.pad.a = true;
        for t in 0..400 {
            if t == hold_ticks {
                s.pad.a = false;
            }
            s.step(1);
            apex = apex.max(s.mario().pos.y);
        }
        apex
    }
    let tap = apex(3);
    let hold = apex(200);
    assert!(tap < hold - 20.0, "tap {tap} vs hold {hold}");
}

#[test]
fn double_and_triple_jump_chain() {
    if need_disc() {
        return;
    }
    let mut s = Scene::flat();
    s.run_forward();
    s.secs(1.0);

    s.press_a();
    s.pad.a = true; // keep holding so the jump isn't cut short
    assert!(
        s.until(600, |m| m.status == Status::JumpSlip),
        "no landing window"
    );
    s.pad.a = false;
    s.step(1);
    s.press_a();
    assert!(
        s.until(5, |m| m.status == Status::SecondJump),
        "got {:?}",
        s.mario().status
    );

    s.pad.a = true;
    assert!(
        s.until(600, |m| m.status == Status::SecJumpSlip),
        "got {:?}",
        s.mario().status
    );
    s.pad.a = false;
    s.step(1);
    s.press_a();
    assert!(
        s.until(5, |m| m.status == Status::UltraJump),
        "got {:?}",
        s.mario().status
    );

    // The triple jump goes higher than the single.
    let mut apex = 0.0f32;
    s.pad.a = true;
    for _ in 0..600 {
        s.step(1);
        apex = apex.max(s.mario().pos.y);
    }
    assert!(apex > 300.0, "triple apex {apex}");
}

#[test]
fn missing_the_chain_window_lands_normally() {
    let mut s = Scene::flat();
    s.run_forward();
    s.secs(1.0);
    s.press_a();
    assert!(s.until(600, |m| m.status == Status::JumpSlip));
    // wait out the window without pressing A
    s.secs(0.5);
    assert_ne!(s.mario().status, Status::SecondJump);
}

/// Runs, jumps holding A, and stops a few ticks before the landing.
fn about_to_land(s: &mut Scene<FlatHost>) {
    s.run_forward();
    s.secs(1.0);
    s.press_a();
    s.pad.a = true;
    assert!(
        s.until(600, |m| m.vel.y < 0.0 && m.pos.y < 25.0),
        "never came down"
    );
    s.pad.a = false;
    s.step(1);
}

#[test]
fn an_early_press_waits_for_the_landing_with_a_buffer() {
    if need_disc() {
        return;
    }
    // The game: a press in the air is lost.
    let mut s = Scene::flat();
    about_to_land(&mut s);
    assert!(s.mario().status.is_jumping());
    s.press_a();
    assert!(!s.until(40, |m| m.status == Status::SecondJump));

    let mut s = Scene::flat();
    s.sim.mario.options.jump_buffer_ticks = 12;
    about_to_land(&mut s);
    s.press_a();
    assert!(
        s.until(40, |m| m.status == Status::SecondJump),
        "got {:?}",
        s.mario().status
    );
}

#[test]
fn chain_grace_keeps_the_double_jump_after_the_window() {
    if need_disc() {
        return;
    }
    let late = |grace: u32| {
        let mut s = Scene::flat();
        s.sim.mario.options.chain_grace_ticks = grace;
        s.run_forward();
        s.secs(1.0);
        s.press_a();
        s.pad.a = true;
        assert!(s.until(600, |m| m.status == Status::JumpSlip));
        s.pad.a = false;
        // Past the 0x10-tick window.
        assert!(s.until(40, |m| m.status != Status::JumpSlip));
        s.step(4);
        s.press_a();
        s.step(2);
        s.mario().status
    };
    assert_eq!(late(0), Status::Jump);
    assert_eq!(late(12), Status::SecondJump);
}

#[test]
fn jump_height_option_scales_the_apex() {
    let apex = |h: f32| {
        let mut s = Scene::flat();
        s.sim.mario.options.jump_height = h;
        s.pad.a = true;
        let mut apex = 0.0f32;
        for _ in 0..600 {
            s.step(1);
            apex = apex.max(s.mario().pos.y);
        }
        apex
    };
    let (one, two) = (apex(1.0), apex(2.0));
    assert!((two / one - 2.0).abs() < 0.15, "{one} -> {two}");
}

#[test]
fn dive_then_belly_slide() {
    let mut s = Scene::flat();
    s.run_forward();
    s.secs(1.0);
    s.press_a();
    s.step(10);
    s.pad.b = true;
    s.step(1);
    s.pad.b = false;
    assert_eq!(s.mario().status, Status::JumpCatch);
    assert!(s.until(600, |m| m.status == Status::Catch));
    // slides to a stop and gets up
    s.pad.stick = Vec2::ZERO;
    assert!(
        s.until(2000, |m| m.status == Status::Wait),
        "stuck in {:?}",
        s.mario().status
    );
}

#[test]
fn spray_can_start_or_continue_through_a_b_dive_and_belly_slide() {
    if need_disc() {
        return;
    }
    for already_spraying in [false, true] {
        let mut s = Scene::flat();
        s.sim.mario.give_fludd();
        s.sim.mario.options.spray_during_dive = true;
        s.sim.mario.options.spray_pressure_fades = false;
        s.run_forward();
        if already_spraying {
            s.pad.r_analog = 255;
        }
        s.secs(1.0);
        s.press_a();
        s.step(10);
        s.pad.b = true;
        s.step(1);
        s.pad.b = false;
        s.pad.r_analog = 255;
        assert_eq!(s.mario().status, Status::JumpCatch);
        s.host.events.clear();
        let water = s.mario().fludd.as_ref().unwrap().water;
        let mut dive_emits = 0;
        let mut slide_emits = 0;
        for _ in 0..240 {
            s.host.events.clear();
            s.step(1);
            let emits = s.event_count(|e| matches!(e, Event::WaterEmit(w) if w.nozzle == NozzleKind::Spray));
            match s.mario().status {
                Status::JumpCatch => dive_emits += emits,
                Status::Catch => slide_emits += emits,
                _ => break,
            }
            assert_eq!(s.mario().anim.clip, "sldct");
            assert!(s.mario().upper_layer().is_none(), "pumping arm replaced the dive/slide pose");
        }
        assert!(dive_emits > 0, "no water during B dive (held: {already_spraying})");
        assert!(slide_emits > 0, "no water during belly slide (held: {already_spraying})");
        assert!(s.mario().fludd.as_ref().unwrap().water < water);
    }
}

/// Host extension `Options::slide_dive_nozzles`: dives with the nozzle's trigger held from the
/// dive on. Returns (emits in the dive, emits in the slide, whether every emit went level, peak
/// distance travelled from the dive on).
fn dive_with(nozzle: NozzleKind, on: bool) -> (u32, u32, bool, f32) {
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    s.sim.mario.options.slide_dive_nozzles = on;
    {
        let p = s.sim.mario.params.clone();
        let f = s.sim.mario.fludd.as_mut().unwrap();
        f.change_nozzle(nozzle, false, &p);
        f.refill(&p);
    }
    s.run_forward();
    s.secs(1.0);
    s.press_a();
    s.step(10);
    s.pad.b = true;
    s.step(1);
    s.pad.b = false;
    s.pad.r_analog = 255;
    let start = s.mario().pos;
    let (mut dive, mut slide, mut level) = (0, 0, true);
    for _ in 0..360 {
        s.host.events.clear();
        s.step(1);
        let (mut emits, mut up) = (0, false);
        for e in s.host.events.iter() {
            if let Event::WaterEmit(w) = e {
                if w.nozzle == nozzle {
                    emits += 1;
                    up |= w.dir.y.abs() > 1e-3;
                }
            }
        }
        level &= !up;
        match s.mario().status {
            Status::JumpCatch => dive += emits,
            Status::Catch => slide += emits,
            _ if dive + slide > 0 => break,
            _ => {}
        }
    }
    let travelled = Vec3::new(s.mario().pos.x - start.x, 0.0, s.mario().pos.z - start.z).length();
    (dive, slide, level, travelled)
}

#[test]
fn the_rocket_and_the_turbo_fire_level_in_a_dive_and_a_slide_and_the_flag_switches_it_back() {
    for nozzle in [NozzleKind::Rocket, NozzleKind::Turbo] {
        let (dive, slide, level, far_on) = dive_with(nozzle, true);
        assert!(dive + slide > 0, "{nozzle:?} never fired in a dive or slide");
        assert!(level, "{nozzle:?} exhaust wasn't level");
        let (dive_off, slide_off, _, far_off) = dive_with(nozzle, false);
        assert_eq!(dive_off + slide_off, 0, "{nozzle:?} fires with the flag off");
        assert!(far_on > far_off, "{nozzle:?} didn't carry him further: on {far_on}, off {far_off}");
    }
}

/// Jumps, then holds R with `nozzle` and a lock-on aim that points level, to the side, for `ticks`. Returns the
/// emits of that nozzle (direction, attack, count), whether he ever rocketed, and the final vertical speed.
fn aimed_burst(nozzle: NozzleKind, master: bool, switch: bool, aim_on: bool, ticks: u32) -> (Vec<(Vec3, i16, u32)>, bool, f32) {
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    s.sim.mario.options.pump_in_long_fall = true;
    {
        let o = &mut s.sim.mario.options;
        o.aim_alt = master;
        o.aim_rocket = switch;
        o.aim_turbo = switch;
        o.aim_hover = switch;
        let p = s.sim.mario.params.clone();
        let f = s.sim.mario.fludd.as_mut().unwrap();
        f.change_nozzle(nozzle, false, &p);
        f.refill(&p);
    }
    s.sim.mario.pos = Vec3::new(0.0, 12000.0, 0.0);
    s.step(5);
    if aim_on {
        s.sim.mario.aim = Some(libsms::Aim { yaw: 0x4000, pitch: 0 });
    }
    s.pad.r_analog = 255;
    let (mut emits, mut rocketed) = (vec![], false);
    for _ in 0..ticks {
        s.host.events.clear();
        s.step(1);
        rocketed |= s.mario().status == Status::Rocket;
        for e in s.host.events.iter() {
            if let Event::WaterEmit(w) = e {
                if w.nozzle == nozzle {
                    emits.push((w.dir, w.attack, w.count));
                }
            }
        }
    }
    (emits, rocketed, s.mario().vel.y)
}

#[test]
fn an_aimed_rocket_fires_one_precision_slug_and_does_not_launch_him() {
    let (shots, rocketed, _) = aimed_burst(NozzleKind::Rocket, true, true, true, 300);
    assert_eq!(shots.len(), 1, "expected one slug, got {}", shots.len());
    let (dir, attack, count) = shots[0];
    assert_eq!(count, 1);
    assert!(dir.x > 0.99, "slug didn't go where aimed: {dir:?}");
    assert!(!rocketed, "the slug launched him");
    // A stronger single hit than the Spray's drop.
    let (plain, plain_rocketed, _) = aimed_burst(NozzleKind::Rocket, false, true, true, 300);
    assert!(plain_rocketed, "master off: the Rocket should launch as usual");
    assert!(plain.len() > 1);
    let spray = libsms::params::Params::default().nozzle(NozzleKind::Spray).attack;
    assert!(attack > spray, "slug {attack} not stronger than a Spray drop {spray}");
    // Each switch off is the movement behavior again.
    let (_, rocketed_off, _) = aimed_burst(NozzleKind::Rocket, true, false, true, 300);
    assert!(rocketed_off, "per-nozzle switch off: it should launch");
    // No target at the discharge: the movement behavior.
    let (_, rocketed_no_target, _) = aimed_burst(NozzleKind::Rocket, true, true, false, 300);
    assert!(rocketed_no_target, "no target: it should launch");
}

#[test]
fn the_aimed_turbo_streams_at_the_target_and_the_master_switch_gates_it() {
    let (aimed, _, _) = aimed_burst(NozzleKind::Turbo, true, true, true, 200);
    let (plain, _, _) = aimed_burst(NozzleKind::Turbo, false, true, true, 200);
    assert!(!aimed.is_empty() && !plain.is_empty());
    assert!(aimed.iter().all(|(d, _, _)| d.x > 0.99), "aimed stream off target");
    assert!(plain.iter().all(|(d, _, _)| d.x < 0.5), "master off but the Turbo aimed");
    let drops = |v: &[(Vec3, i16, u32)]| v.iter().map(|e| e.2).sum::<u32>();
    assert!(drops(&aimed) > drops(&plain), "the aimed stream isn't faster");
}

#[test]
fn an_aimed_hover_points_both_jets_at_the_target_and_keeps_its_lift() {
    let (aimed, _, vy_aimed) = aimed_burst(NozzleKind::Hover, true, true, true, 60);
    let (_, _, vy_plain) = aimed_burst(NozzleKind::Hover, false, true, true, 60);
    assert!(aimed.len() >= 2, "both jets should fire");
    assert!(aimed.iter().all(|(d, _, _)| d.x > 0.99), "jets off target");
    assert!(vy_aimed > 0.0 && (vy_aimed - vy_plain).abs() < 1.0, "lift changed: {vy_aimed} vs {vy_plain}");
}

/// Holds R on the ground (or at height) with `nozzle`; returns every emit's (power, direction) and where he ended.
fn hover_or_spray_from_the_ground(nozzle: NozzleKind, aim: bool) -> (Vec<(f32, Vec3)>, Vec3, f32, bool) {
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    s.sim.mario.options.aim_alt = true;
    {
        let p = s.sim.mario.params.clone();
        let f = s.sim.mario.fludd.as_mut().unwrap();
        f.change_nozzle(nozzle, false, &p);
        f.refill(&p);
    }
    s.step(30);
    let start = s.mario().pos;
    if aim {
        s.sim.mario.aim = Some(libsms::Aim { yaw: 0x4000, pitch: 0 });
    }
    s.pad.r_analog = 255;
    let mut emits = vec![];
    let mut max_vy = 0.0f32;
    let mut stayed = true;
    for _ in 0..90 {
        s.host.events.clear();
        s.step(1);
        max_vy = max_vy.max(s.mario().vel.y);
        stayed &= s.mario().on_ground();
        for e in s.host.events.iter() {
            if let Event::WaterEmit(w) = e {
                if w.nozzle == nozzle {
                    emits.push((w.power, w.dir));
                }
            }
        }
    }
    (emits, s.mario().pos - start, max_vy, stayed)
}

#[test]
fn a_grounded_aimed_hover_is_a_gun_that_reaches_as_far_as_the_spray_and_moves_nothing() {
    let (shots, moved, max_vy, stayed) = hover_or_spray_from_the_ground(NozzleKind::Hover, true);
    assert!(shots.len() >= 2, "both jets should fire, got {}", shots.len());
    assert!(shots.iter().all(|(_, d)| d.x > 0.99), "the jets are off the aim");
    assert!(moved.length() < 1.0 && max_vy <= 0.1 && stayed, "it moved him: {moved:?}, vy {max_vy}");
    let (spray, _, _, _) = hover_or_spray_from_the_ground(NozzleKind::Spray, false);
    let top = |v: &[(f32, Vec3)]| v.iter().map(|e| e.0).fold(0.0f32, f32::max);
    assert!((top(&shots) - top(&spray)).abs() < 0.05 * top(&spray), "power {} vs the Spray's {}", top(&shots), top(&spray));
    // No lock: the plain Hover still lifts him off the ground.
    let (_, _, plain_vy, _) = hover_or_spray_from_the_ground(NozzleKind::Hover, false);
    assert!(plain_vy > 0.5, "the plain Hover no longer lifts: {plain_vy}");
}

/// A Hover held in the air: the lock is taken part way, then let go, and the Hover goes on.
#[test]
fn lb_aims_a_hover_in_flight_and_letting_go_keeps_hovering() {
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    s.sim.mario.options.pump_in_long_fall = true;
    s.sim.mario.options.aim_alt = true;
    {
        let p = s.sim.mario.params.clone();
        let f = s.sim.mario.fludd.as_mut().unwrap();
        f.change_nozzle(NozzleKind::Hover, false, &p);
        f.refill(&p);
    }
    s.sim.mario.pos = Vec3::new(0.0, 12000.0, 0.0);
    s.step(5);
    s.pad.r_analog = 255;
    let burst = |s: &mut Scene<FlatHost>, n: u32| {
        let (mut dirs, mut vy) = (vec![], 0.0f32);
        for _ in 0..n {
            s.host.events.clear();
            s.step(1);
            vy += s.mario().vel.y;
            for e in s.host.events.iter() {
                if let Event::WaterEmit(w) = e {
                    if w.nozzle == NozzleKind::Hover {
                        dirs.push(w.dir);
                    }
                }
            }
        }
        (dirs, vy / n as f32)
    };
    let (plain, vy0) = burst(&mut s, 30);
    assert!(plain.len() >= 2 && plain.iter().all(|d| d.y < -0.5), "the hover isn't straight down: {plain:?}");
    // LB pressed mid-flight: the jets swing to the target, the lift goes on.
    s.sim.mario.aim = Some(libsms::Aim { yaw: 0x4000, pitch: 0 });
    let (aimed, vy1) = burst(&mut s, 30);
    assert!(aimed.len() >= 2 && aimed.iter().all(|d| d.x > 0.99), "mid-flight LB didn't aim the jets");
    assert!(vy1 > 0.0 && (vy1 - vy0).abs() < 1.5, "flight changed with the aim: {vy0} -> {vy1}");
    // LB let go with R still held: the hover goes on, straight down again.
    s.sim.mario.aim = None;
    let (after, vy2) = burst(&mut s, 30);
    assert!(after.len() >= 2 && after.iter().all(|d| d.y < -0.5), "the hover stopped or stayed aimed after LB let go");
    assert!((vy2 - vy0).abs() < 1.5, "lift changed after LB let go: {vy0} -> {vy2}");
    // Lock again: aimed again, with no new press of R.
    s.sim.mario.aim = Some(libsms::Aim { yaw: 0x4000, pitch: 0 });
    let (again, _) = burst(&mut s, 30);
    assert!(again.iter().all(|d| d.x > 0.99) && !again.is_empty(), "a regained lock didn't aim again");
}

#[test]
fn a_lost_target_ends_an_aimed_stream_until_r_is_pressed_again() {
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    s.sim.mario.options.pump_in_long_fall = true;
    s.sim.mario.options.aim_alt = true;
    {
        let p = s.sim.mario.params.clone();
        let f = s.sim.mario.fludd.as_mut().unwrap();
        f.change_nozzle(NozzleKind::Turbo, false, &p);
        f.refill(&p);
    }
    s.sim.mario.pos = Vec3::new(0.0, 12000.0, 0.0);
    s.step(5);
    s.sim.mario.aim = Some(libsms::Aim { yaw: 0x4000, pitch: 0 });
    s.pad.r_analog = 255;
    s.step(1);
    let emits = |s: &mut Scene<FlatHost>, n: u32| {
        let mut total = 0;
        for _ in 0..n {
            s.host.events.clear();
            s.step(1);
            total += s.event_count(|e| matches!(e, Event::WaterEmit(w) if w.nozzle == NozzleKind::Turbo));
        }
        total
    };
    // Run until the stream has just begun (it would otherwise be spent on its own), then break the lock.
    let mut began = false;
    for _ in 0..400 {
        if emits(&mut s, 1) > 0 {
            began = true;
            break;
        }
    }
    assert!(began, "no aimed stream");
    s.sim.mario.aim = None;
    s.step(2);
    assert_eq!(emits(&mut s, 30), 0, "kept firing after the target broke");
    // Still held: still off, even with the target back.
    s.sim.mario.aim = Some(libsms::Aim { yaw: 0x4000, pitch: 0 });
    assert_eq!(emits(&mut s, 30), 0, "restarted without a fresh press");
    s.pad.r_analog = 0;
    s.step(10);
    {
        let p = s.sim.mario.params.clone();
        s.sim.mario.fludd.as_mut().unwrap().refill(&p);
    }
    s.pad.r_analog = 255;
    assert!(emits(&mut s, 200) > 0, "a fresh press didn't start it again");
}

/// Both FLUDD gauges draw `Mario::nozzle_gauge`: for the Turbo it fills with the dash wind-up too, and is
/// only full once the dash is ready.
#[test]
fn the_turbo_gauge_fills_with_the_dash_wind_up() {
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    {
        let p = s.sim.mario.params.clone();
        let f = s.sim.mario.fludd.as_mut().unwrap();
        f.change_nozzle(NozzleKind::Turbo, false, &p);
        f.refill(&p);
    }
    assert!(s.mario().nozzle_gauge().is_none(), "a gauge with nothing built");
    s.pad.r_analog = 255;
    s.step(10);
    let early = s.mario().nozzle_gauge().map_or(0.0, |(n, d)| n / d);
    assert!(early < 0.5, "full too early: {early}");
    s.step(150);
    let (n, d) = s.mario().nozzle_gauge().expect("no gauge");
    assert!((n / d - 1.0).abs() < 1e-3, "not full once wound up: {}", n / d);
}

#[test]
fn spray_can_start_and_stop_during_a_belly_slide() {
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    s.sim.mario.options.spray_during_dive = true;
    s.run_forward();
    s.secs(1.0);
    s.press_a();
    s.step(10);
    s.pad.b = true;
    s.step(1);
    s.pad.b = false;
    assert!(s.until(600, |m| m.status == Status::Catch));
    s.pad.r_analog = 255;
    s.host.events.clear();
    s.step(15);
    assert_eq!(s.mario().status, Status::Catch);
    assert!(s.event_count(|e| matches!(e, Event::WaterEmit(_))) > 0);
    s.pad.r_analog = 0;
    s.host.events.clear();
    s.step(5);
    assert_eq!(s.event_count(|e| matches!(e, Event::WaterEmit(_))), 0);
}

#[test]
fn the_pack_pumps_while_spraying_in_a_dive_or_belly_slide() {
    if need_disc() {
        return;
    }
    // `spray_during_dive` lets the Spray fire where no arm pump clip exists (`sldct`'s
    // `gMarioAnimeData` row is 0xC8): Mario keeps his pose, but FLUDD's pack still pulses —
    // `TWaterGun::calcAnimation` mirrors the pump frame onto `wg_pump`.
    for already_spraying in [false, true] {
        let mut s = Scene::flat();
        s.sim.mario.give_fludd();
        s.sim.mario.options.spray_during_dive = true;
        s.run_forward();
        if already_spraying {
            s.pad.r_analog = 255;
        }
        s.secs(1.0);
        s.press_a();
        s.step(10);
        s.pad.b = true;
        s.step(1);
        s.pad.b = false;
        s.pad.r_analog = 255;
        assert!(s.until(600, |m| m.status == Status::Catch));
        s.step(12); // the handles come out (10 ticks) before water and the pump clip show
        let mut frames: Vec<f32> = Vec::new();
        for _ in 0..40 {
            s.step(1);
            let m = s.mario();
            let look = m.fludd_look().unwrap();
            assert_eq!(look.body_clip, "wg_pump", "pack clip");
            assert!(
                m.upper_layer().is_none(),
                "the dive/slide pose lost its arms"
            );
            frames.push(look.body_frame);
        }
        let mut uniq = frames.clone();
        uniq.sort_by(|a, b| a.partial_cmp(b).unwrap());
        uniq.dedup();
        assert!(
            uniq.len() >= 8,
            "the pack is frozen at {uniq:?} (held: {already_spraying})"
        );
        assert!(*uniq.last().unwrap() > 20.0, "the pump never reaches its end");
    }
}

#[test]
fn the_pack_folds_back_in_after_the_slide_spray_stops() {
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    s.sim.mario.options.spray_during_dive = true;
    s.run_forward();
    s.secs(1.0);
    s.press_a();
    s.step(10);
    s.pad.b = true;
    s.step(1);
    s.pad.b = false;
    s.pad.r_analog = 255;
    assert!(s.until(600, |m| m.status == Status::Catch));
    s.step(20);
    assert_eq!(s.mario().fludd_look().unwrap().body_clip, "wg_pump");
    s.pad.r_analog = 0;
    // `HoldingPump` keeps it out for `mPumpWaitTime`, then `unk1CF8` (360 ticks) passes before
    // the handles fold back over `wg_house` (0.005 a tick).
    s.secs(6.0);
    assert_eq!(s.mario().fludd_look().unwrap().body_clip, "wg_house");
}

#[test]
fn dive_spray_exception_keeps_other_nozzles_and_default_rules_gated() {
    for (enabled, nozzle) in [
        (false, NozzleKind::Spray),
        (true, NozzleKind::Hover),
        (true, NozzleKind::Rocket),
        (true, NozzleKind::Turbo),
    ] {
        let mut s = Scene::flat();
        s.sim.mario.give_fludd();
        s.sim.mario.options.spray_during_dive = enabled;
        let p = s.mario().params.clone();
        s.sim.mario.fludd.as_mut().unwrap().change_nozzle(nozzle, false, &p);
        s.run_forward();
        s.secs(1.0);
        s.press_a();
        s.step(10);
        s.pad.b = true;
        s.step(1);
        s.pad.b = false;
        // Let the dive clip settle before pressing R, as the original animation gate does.
        s.step(2);
        s.pad.r_analog = 255;
        s.host.events.clear();
        s.step(15);
        assert_eq!(s.event_count(|e| matches!(e, Event::WaterEmit(_))), 0, "{nozzle:?}");
    }
}

#[test]
fn a_fresh_b_can_redive_from_both_slide_jump_recoveries() {
    for enabled in [false, true] {
        for (fast, recovery) in [(false, Status::CatchStop), (true, Status::RotateBroadJump)] {
            let mut s = Scene::flat();
            s.sim.mario.options.redive_after_slide = enabled;
            s.run_forward();
            s.secs(1.0);
            s.press_a();
            s.step(10);
            s.pad.b = true;
            s.step(1);
            s.pad.b = false;
            assert!(s.until(600, |m| m.status == Status::Catch));
            // Choose either side of catching's speed threshold for the two A recoveries.
            s.sim.mario.forward_vel = s.mario().params.mario.clash_speed + if fast { 1.0 } else { -1.0 };
            s.press_a();
            assert_eq!(s.mario().status, recovery);
            s.step(3);
            s.pad.b = true;
            s.step(1);
            assert_eq!(s.mario().status, if enabled { Status::JumpCatch } else { recovery });
            s.pad.b = false;
            s.pad.stick = Vec2::ZERO;
            assert!(s.until(600, |m| m.status == if enabled { Status::Catch } else { Status::Wait }));
        }
    }
}

/// The stand-in pose cues follow the native silhouettes: horizontal for a dive and a slide,
/// squashed for a pound, and a wider core for a spin.
#[test]
fn stand_in_pose_cues_follow_dive_slide_pound_and_spin() {
    // A B dive is horizontal; landing runs it into the belly slide, also horizontal.
    let mut s = Scene::flat();
    s.run_forward();
    s.secs(1.0);
    s.press_a();
    s.step(10);
    s.pad.b = true;
    s.step(1);
    s.pad.b = false;
    assert_eq!(s.mario().status, Status::JumpCatch);
    assert_eq!(s.mario().pose() & POSE_DIVE, POSE_DIVE);
    assert_eq!(s.mario().pose() & POSE_SLIDE, 0);
    assert!(s.until(600, |m| m.status == Status::Catch));
    assert_eq!(s.mario().pose() & POSE_SLIDE, POSE_SLIDE);
    assert_eq!(s.mario().pose() & POSE_DIVE, 0);

    // A ground pound is squashed; a stick circle during its fall adds the spin.
    let mut s = Scene::flat();
    s.sim.mario.pos = Vec3::new(0.0, 1500.0, 0.0);
    s.step(2);
    s.pad.l = true;
    s.step(1);
    s.pad.l = false;
    // The pound hovers for 60 ticks before it falls; only the fall is `pounding`.
    s.step(65);
    assert_eq!(s.mario().status, Status::HipDrop);
    assert_eq!(s.mario().pose() & POSE_POUND, POSE_POUND);
    assert_eq!(s.mario().pose() & POSE_SPIN, 0, "a plain pound spun");
    let mut angle = 0.0f32;
    for _ in 0..40 {
        angle += std::f32::consts::TAU / 12.0;
        s.pad.stick = Vec2::new(angle.cos(), angle.sin());
        s.step(1);
    }
    assert_eq!(s.mario().status, Status::HipDrop);
    assert_eq!(s.mario().pose() & POSE_POUND, POSE_POUND);
    assert_eq!(
        s.mario().pose() & POSE_SPIN,
        POSE_SPIN,
        "the pound's circle did not spin"
    );

    // A stick circle in an ordinary jump is a spin jump: a spin without a pound.
    let mut s = Scene::flat();
    s.press_a();
    s.step(10);
    let mut angle = 0.0f32;
    for _ in 0..24 {
        angle += std::f32::consts::TAU / 12.0;
        s.pad.stick = Vec2::new(angle.cos(), angle.sin());
        s.step(1);
        if matches!(
            s.mario().status,
            Status::LeftRotateJump | Status::RightRotateJump
        ) {
            break;
        }
    }
    assert!(
        matches!(
            s.mario().status,
            Status::LeftRotateJump | Status::RightRotateJump
        ),
        "{:?}",
        s.mario().status
    );
    assert_eq!(s.mario().pose() & POSE_SPIN, POSE_SPIN);
    assert_eq!(s.mario().pose() & POSE_POUND, 0);
}

#[test]
fn ground_pound_reaches_the_floor() {
    let mut s = Scene::flat();
    s.pad.a = true;
    s.press_a();
    s.step(40);
    s.pad.l = true;
    s.step(1);
    s.pad.l = false;
    assert_eq!(s.mario().status, Status::HipDrop);
    assert!(s.until(1500, |m| m.status == Status::HipAttackEnd
        || m.status == Status::SlipEnd));
    assert_eq!(s.event_count(|e| matches!(e, Event::GroundPound { .. })), 1);
    assert!(s.mario().pos.y <= 1.0);
}

#[test]
fn wall_stops_a_runner() {
    let mut s = Scene::new(FlatHost {
        wall_x: Some((300.0, -1.0)),
        ..FlatHost::default()
    });
    // Right-handed world: with the camera looking along +Z, stick-left walks toward +X.
    s.pad.stick = Vec2::new(-1.0, 0.0);
    s.sim.mario.face_yaw = 0x4000; // facing +X
    s.secs(3.0);
    let m = s.mario();
    assert!(m.pos.x <= 300.0 - 49.0, "{:?}", m.pos);
    assert!(m.pos.x > 200.0, "never reached the wall: {:?}", m.pos);
}

#[test]
fn walking_off_a_ledge_falls() {
    struct Ledge;
    impl Host for Ledge {
        fn ground(&self, p: Vec3) -> Option<libsms::GroundHit> {
            let h = if p.z < 200.0 { 0.0 } else { -600.0 };
            (p.y >= h).then_some(libsms::GroundHit {
                height: h,
                normal: Vec3::Y,
                flags: libsms::SurfaceFlags::NONE,
            })
        }
    }
    let mut s = Scene::new(Ledge);
    s.run_forward();
    s.secs(3.0);
    let m = s.mario();
    assert!(
        m.pos.y < -100.0 || m.status.is_jumping(),
        "{:?} {:?}",
        m.pos,
        m.status
    );
}

#[test]
fn steep_slope_makes_mario_slip() {
    if need_disc() {
        return;
    }
    struct Slope;
    impl Host for Slope {
        fn ground(&self, p: Vec3) -> Option<libsms::GroundHit> {
            // 70 degree ramp rising toward +Z (steeper than mSlipStart): floor height = z*tan70.
            let n = Vec3::new(0.0, 0.342_020_14, -0.939_692_6);
            let h = p.z * 2.747_477_4;
            (p.y >= h - 0.01).then_some(libsms::GroundHit {
                height: h,
                normal: n,
                flags: libsms::SurfaceFlags::NONE,
            })
        }
    }
    let mut s = Scene::new(Slope);
    s.sim.mario.pos = Vec3::new(0.0, 500.0 * 2.747_477_4, 500.0);
    s.sim.mario.face_yaw = 0; // facing uphill
    s.secs(2.0);
    let slid = s.mario().pos.z < 500.0 - 20.0;
    eprintln!("slope: {:?} {:?}", s.mario().pos, s.mario().status);
    assert!(
        slid,
        "did not slide: {:?} {:?}",
        s.mario().pos,
        s.mario().status
    );
}

// ---- FLUDD ---------------------------------------------------------------------------------

#[test]
fn spraying_drains_the_tank_and_recoils() {
    if need_disc() {
        return;
    }
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    let p = s.mario().params.clone();
    let full = s.mario().fludd.as_ref().unwrap().water;
    assert_eq!(full, p.nozzle(NozzleKind::Spray).amount_max);

    s.pad.r_analog = 200;
    s.secs(1.0);
    let f = s.mario().fludd.as_ref().unwrap();
    assert!(f.water < full, "tank did not drain: {} / {}", f.water, full);
    let emits: Vec<WaterEmit> = s
        .host
        .events
        .iter()
        .filter_map(|e| {
            if let Event::WaterEmit(w) = e {
                Some(*w)
            } else {
                None
            }
        })
        .collect();
    assert!(!emits.is_empty());
    assert!(emits.iter().all(|e| e.nozzle == NozzleKind::Spray));
    // Spray points forward (+Z, facing 0).
    assert!(emits[0].dir.z > 0.5, "{:?}", emits[0].dir);
}

#[test]
fn spray_waits_for_the_handles_then_shoots_with_its_sounds() {
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    s.step(1);
    {
        let look = s.mario().fludd_look().unwrap();
        assert_eq!(look.body_clip, "wg_house");
        assert_eq!(look.nozzle_clip, Some("normal_wg_house"));
    }
    s.pad.r_analog = 200;
    // `unk1CEC` falls 0.1 a tick: ten ticks of handles before any water.
    s.step(5);
    assert_eq!(s.event_count(|e| matches!(e, Event::WaterEmit(_))), 0);
    assert!(s.mario().fludd.as_ref().unwrap().housed > 0.0);
    s.step(20);
    let f = s.mario().fludd.as_ref().unwrap();
    assert_eq!(f.housed, 0.0);
    assert!(s.event_count(|e| matches!(e, Event::WaterEmit(_))) > 0);
    let look = s.mario().fludd_look().unwrap();
    assert_eq!(look.body_clip, "wg_pump");
    assert!(
        matches!(
            look.nozzle_clip,
            Some("normal_wg_shoot_start" | "normal_wg_shooting")
        ),
        "{look:?}"
    );
    let (upper, _) = s.mario().upper_layer().expect("pumping arm");
    assert_eq!(upper, "pump");
    let sounds: Vec<&str> = s
        .host
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Sound(c) => Some(c.name),
            _ => None,
        })
        .collect();
    assert!(
        sounds.contains(&"MSD_SE_PO_NORMAL_NOZZLE_IMI"),
        "{sounds:?}"
    );
    assert!(sounds.contains(&"MSD_SE_PO_WATER_HI"), "{sounds:?}");
    // Water leaves a reported muzzle, turned with Mario.
    s.sim
        .mario
        .set_emitters(NozzleKind::Spray, &[(Vec3::new(0.0, 100.0, 50.0), Vec3::Z)]);
    s.host.events.clear();
    s.step(4);
    let w = s
        .host
        .events
        .iter()
        .find_map(|e| match e {
            Event::WaterEmit(w) => Some(*w),
            _ => None,
        })
        .unwrap();
    let m = s.mario();
    assert!(
        (w.pos - (m.pos + Vec3::new(0.0, 100.0, 50.0))).length() < 1.0,
        "{w:?}"
    );
    // Release: the handles stay out a while, then fold back.
    s.pad.r_analog = 0;
    s.secs(2.0);
    assert_eq!(s.mario().fludd.as_ref().unwrap().housed, 0.0);
    s.secs(6.0);
    assert_eq!(s.mario().fludd.as_ref().unwrap().housed, 1.0);
    assert!(s.mario().upper_layer().is_none());
}

#[test]
fn pumping_an_empty_tank_makes_the_pump_sounds() {
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    s.sim.mario.fludd.as_mut().unwrap().water = 0;
    s.pad.r_analog = 255;
    s.secs(1.0);
    assert!(s.event_count(|e| matches!(e, Event::Sound(c) if c.name == "MSD_SE_PO_POMPING1")) > 0);
    assert!(s.event_count(|e| matches!(e, Event::Sound(c) if c.name == "MSD_SE_PO_POMPING2")) > 0);
}

#[test]
fn x_swap_plays_the_nozzle_clips() {
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    s.pad.x = true;
    s.step(1);
    s.pad.x = false;
    s.step(2);
    let look = s.mario().fludd_look().unwrap();
    assert_eq!(look.nozzle, NozzleKind::Spray);
    assert_eq!(look.nozzle_clip, Some("house_wg_change_start"));
    assert!(s.until(60, |m| m.fludd.as_ref().unwrap().nozzle
        == NozzleKind::Hover));
    s.step(1);
    let look = s.mario().fludd_look().unwrap();
    assert_eq!(look.nozzle_clip, Some("hover_wg_change_end"));
    assert!(
        s.event_count(|e| matches!(e, Event::Sound(c) if c.name == "MSD_SE_SY_SELECT_POMP_BACK"))
            == 1
    );
}

#[test]
fn an_empty_tank_does_not_spray() {
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    s.sim.mario.fludd.as_mut().unwrap().water = 0;
    s.pad.r_analog = 255;
    s.secs(0.5);
    assert_eq!(s.event_count(|e| matches!(e, Event::WaterEmit(_))), 0);
}

#[test]
fn pressing_r_with_a_dry_tank_puffs_empty_air() {
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    s.sim.mario.fludd.as_mut().unwrap().water = 0;
    s.pad.r = true;
    s.pad.r_analog = 255;
    s.secs(0.5);
    assert_eq!(
        s.event_count(|e| matches!(e, Event::Sound(c) if c.name == "MSD_SE_PO_ACTION_ON_EMPTY")),
        1
    );
}

fn tank(s: &Scene<impl Host>) -> (i32, i32) {
    let f = s.mario().fludd.as_ref().unwrap();
    (f.water, f.max_water(&s.mario().params))
}

#[test]
fn r_in_shallow_water_sucks_the_tank_full_then_sprays() {
    let mut s = Scene::new(FlatHost {
        water: Some(30.0),
        ..FlatHost::default()
    });
    s.sim.mario.give_fludd();
    s.sim.mario.fludd.as_mut().unwrap().water = 0;
    s.step(1);
    assert!(s.mario().in_water());
    s.pad.r_analog = 255;
    s.secs(0.5);
    let (water, max) = tank(&s);
    assert!(water > 0, "nothing sucked up");
    assert!(
        s.event_count(|e| matches!(e, Event::Sound(c) if c.name == "MSD_SE_PO_SUCK_WATER_B")) > 0
    );
    // Filling, nothing comes out.
    if water < max {
        assert_eq!(s.event_count(|e| matches!(e, Event::WaterEmit(_))), 0);
    }
    assert!(s.until(120 * 20, |m| {
        let f = m.fludd.as_ref().unwrap();
        f.water == f.max_water(&m.params)
    }));
    s.secs(0.5);
    assert_eq!(
        s.event_count(|e| matches!(e, Event::Sound(c) if c.name == "MSD_SE_SY_PO_WATER_FULL")),
        1
    );
    // Full, it sprays, and stays full.
    assert!(s.event_count(|e| matches!(e, Event::WaterEmit(_))) > 0);
    let (water, max) = tank(&s);
    assert_eq!(water, max);
}

#[test]
fn out_of_the_water_r_does_not_refill() {
    let mut s = Scene::new(FlatHost {
        water: Some(-10.0),
        ..FlatHost::default()
    });
    s.sim.mario.give_fludd();
    s.sim.mario.fludd.as_mut().unwrap().water = 0;
    s.pad.r_analog = 255;
    s.secs(0.5);
    assert!(!s.mario().in_water());
    assert_eq!(tank(&s).0, 0);
}

#[test]
fn unlimited_water_never_runs_dry() {
    let mut s = Scene::flat();
    s.sim.mario.options.unlimited_water = true;
    s.sim.mario.give_fludd();
    s.pad.r_analog = 255;
    s.secs(2.0);
    assert!(s.event_count(|e| matches!(e, Event::WaterEmit(_))) > 0);
    let (water, max) = tank(&s);
    assert_eq!(water, max);
    assert_eq!(
        s.event_count(|e| matches!(e, Event::Sound(c) if c.name == "MSD_SE_SY_PO_WATER_FULL")),
        0
    );
}

#[test]
fn a_host_refill_fills_the_tank_and_chimes() {
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    s.sim.mario.fludd.as_mut().unwrap().water = 0;
    s.step(1);
    s.sim.mario.refill_water();
    s.step(1);
    let (water, max) = tank(&s);
    assert_eq!(water, max);
    assert_eq!(
        s.event_count(|e| matches!(e, Event::Sound(c) if c.name == "MSD_SE_SY_PO_WATER_FULL")),
        1
    );
}

#[test]
fn hover_nozzle_lifts_and_holds_then_runs_out() {
    if need_disc() {
        return;
    }
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    {
        let p = s.sim.mario.params.clone();
        let f = s.sim.mario.fludd.as_mut().unwrap();
        f.change_nozzle(NozzleKind::Hover, false, &p);
        f.refill(&p);
    }
    s.press_a();
    s.step(20);
    s.pad.r_analog = 255;
    let hover_height = s.mario().params.water_gun.hover_height;
    let mut max_y = 0.0f32;
    let mut hovered_ticks = 0;
    for _ in 0..(5 * TICK_HZ) {
        s.step(1);
        max_y = max_y.max(s.mario().pos.y);
        if s.mario().status == Status::Rocket {
            hovered_ticks += 1;
        }
    }
    assert!(
        hovered_ticks > TICK_HZ,
        "only hovered {hovered_ticks} ticks"
    );
    assert!(
        max_y > 0.5 * hover_height,
        "max_y {max_y}, hover height {hover_height}"
    );
    // After the trigger dies (~3 s) Mario drops: not still up there at 5 s
    s.pad.r_analog = 0;
    s.secs(3.0);
    assert!(s.mario().pos.y < 5.0, "still airborne: {}", s.mario().pos.y);
}

#[test]
fn rocket_nozzle_needs_charge_then_launches() {
    if need_disc() {
        return;
    }
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    {
        let p = s.sim.mario.params.clone();
        let f = s.sim.mario.fludd.as_mut().unwrap();
        f.change_nozzle(NozzleKind::Rocket, false, &p);
        f.refill(&p);
    }
    s.pad.r = true; // crouch click so the stance stays up
    s.pad.r_analog = 255;
    // Not launched immediately...
    s.step(10);
    assert!(s.mario().pos.y < 1.0);
    // ...but after the charge time.
    s.secs(3.0);
    let max_y = {
        let mut m = 0.0f32;
        for _ in 0..(4 * TICK_HZ) {
            s.step(1);
            m = m.max(s.mario().pos.y);
        }
        m
    };
    let fired = s.event_count(|e| matches!(e, Event::TriggerFired(NozzleKind::Rocket)));
    assert!(fired >= 1, "rocket never fired");
    assert!(max_y > 400.0, "rocket apex {max_y}");
}

#[test]
fn a_stick_circle_spins_him_out_of_the_rockets_thrust_and_the_held_spray_does_not_pull_him_back() {
    if need_disc() {
        return;
    }
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    {
        let p = s.sim.mario.params.clone();
        let f = s.sim.mario.fludd.as_mut().unwrap();
        f.change_nozzle(NozzleKind::Rocket, false, &p);
        f.refill(&p);
    }
    s.press_a();
    s.step(6);
    s.pad.r = true;
    s.pad.r_analog = 255;
    // The Rocket takes its charge time in the air too.
    s.until(300, |m| m.status == Status::Rocket);
    assert_eq!(s.mario().status, Status::Rocket, "the rocket never started");
    for i in 0..24 {
        let a = i as f32 * std::f32::consts::TAU / 12.0;
        s.pad.stick = Vec2::new(a.cos(), a.sin());
        s.step(1);
        if matches!(s.mario().status, Status::LeftRotateJump | Status::RightRotateJump) {
            break;
        }
    }
    assert!(matches!(s.mario().status, Status::LeftRotateJump | Status::RightRotateJump), "{:?}", s.mario().status);
    s.pad.stick = Vec2::ZERO;
    for _ in 0..6 {
        s.step(1);
        assert!(
            matches!(s.mario().status, Status::LeftRotateJump | Status::RightRotateJump),
            "the held spray pulled him back into the rocket: {:?}",
            s.mario().status
        );
    }
    // And not once the spin is over either: the rocket is spent until he is back on the ground.
    for i in 0..400 {
        s.step(1);
        if s.mario().on_ground() {
            break;
        }
        assert_ne!(s.mario().status, Status::Rocket, "a second rocket in the same jump (tick {i})");
    }
}

fn fall_impact(s: &Scene<FlatHost>) -> (f32, bool, bool, bool) {
    s.host
        .events
        .iter()
        .rev()
        .find_map(|e| match e {
            Event::FallImpact {
                drop,
                diving,
                ground_pound,
                cushioned,
                ..
            } => Some((*drop, *diving, *ground_pound, *cushioned)),
            _ => None,
        })
        .expect("no floor impact")
}

#[test]
fn fall_impact_measures_default_host_rocket_below_safe_height() {
    if need_disc() {
        return;
    }
    let mut s = Scene::flat();
    s.sim.mario.options.jump_height = 1.6;
    s.sim.mario.options.rocket_thrust = 2.0f32.powf(1.0 / 1.9);
    s.sim.mario.give_fludd();
    let p = s.mario().params.clone();
    let f = s.sim.mario.fludd.as_mut().unwrap();
    f.change_nozzle(NozzleKind::Rocket, false, &p);
    f.refill(&p);
    s.pad.r = true;
    s.pad.r_analog = 255;
    let mut apex = 0.0f32;
    for _ in 0..3000 {
        s.step(1);
        apex = apex.max(s.mario().pos.y);
        if apex > 3000.0 && s.mario().vel.y < 0.0 {
            s.pad.r = false;
            s.pad.r_analog = 0;
        }
        if apex > 100.0 && s.mario().pos.y <= 0.0 {
            break;
        }
    }
    let impact = fall_impact(&s);
    println!(
        "default rocket apex {} heights, impact {impact:?}",
        apex / 160.0
    );
    assert!(apex > 3000.0 && apex < 40.0 * 160.0, "apex {apex}");
    assert!((impact.0 - apex).abs() < 1.0, "{impact:?}, apex {apex}");
    assert!(!impact.1 && !impact.2);
}

#[test]
fn fall_impact_keeps_height_across_a_late_dive() {
    let mut s = Scene::flat();
    s.sim.mario.teleport(Vec3::Y * 16000.0);
    s.step(250);
    assert!(s.mario().pos.y < 15000.0 && s.mario().pos.y > 1000.0);
    s.pad.b = true;
    s.step(1);
    s.pad.b = false;
    assert_eq!(s.mario().status, Status::JumpCatch);
    s.step(1400);
    let impact = fall_impact(&s);
    assert!((impact.0 - 16000.0).abs() < 1.0, "{impact:?}");
    assert!(impact.1 && !impact.2 && !impact.3, "{impact:?}");
}

#[test]
fn fall_impact_marks_ground_pounds_and_water_landings_safe() {
    let mut pound = Scene::flat();
    pound_from(&mut pound, 16000.0);
    let impact = fall_impact(&pound);
    assert!(impact.2, "{impact:?}");
    assert!(impact.0 > 15000.0, "{impact:?}");

    let mut water = Scene::new(FlatHost {
        water: Some(5.0),
        ..FlatHost::default()
    });
    water.sim.mario.teleport(Vec3::Y * 16000.0);
    water.step(1400);
    assert!(fall_impact(&water).3);
}

#[test]
fn fall_impact_teleport_discards_the_previous_peak() {
    let mut s = Scene::flat();
    s.sim.mario.teleport(Vec3::Y * 16000.0);
    s.step(100);
    s.sim.mario.teleport(Vec3::Y * 100.0);
    s.step(200);
    let impact = fall_impact(&s);
    assert!((impact.0 - 100.0).abs() < 1.0, "{impact:?}");
    assert!(impact.3, "short fall should be cushioned: {impact:?}");
}

#[test]
fn x_swaps_nozzles_and_rescales_the_tank() {
    if need_disc() {
        return;
    }
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    let p = s.mario().params.clone();
    s.pad.x = true;
    s.step(1);
    s.pad.x = false;
    s.secs(1.0);
    let f = s.mario().fludd.as_ref().unwrap();
    assert_eq!(f.nozzle, NozzleKind::Hover);
    assert!((f.level(&p) - 1.0).abs() < 0.01);
}

#[test]
fn disc_parameters_are_used_when_available() {
    let (p, from_disc) = params();
    if !from_disc {
        eprintln!("skipped: no extracted disc data");
        return;
    }
    // The shipped values differ from the code defaults (MarioInit.cpp): gravity 1.0 vs 0.8,
    // dash rotation 60 vs 100, hover height 230 vs 160.
    assert_eq!(p.jump.gravity, 1.0);
    assert_eq!(p.run.dash_rot_sp, 60);
    assert_eq!(p.water_gun.hover_height, 230.0);
    assert_eq!(p.nozzle(NozzleKind::Spray).amount_max, 10_000);
    assert_eq!(p.nozzle(NozzleKind::Rocket).amount_max, 2_100);
    assert!(anims().len() > 100);
}

// ---- skeleton and clips --------------------------------------------------------------------

fn skeleton() -> Option<(libsms::Skeleton, libsms::ClipLibrary)> {
    let dir = extracted()?;
    let bmd = std::fs::read(dir.join("bmd/ma_mdl1.bmd")).ok()?;
    Some((
        libsms::Skeleton::from_bmd(&bmd)?,
        libsms::ClipLibrary::from_dir(&dir.join("bck")),
    ))
}

#[test]
fn rig_and_clips_load_from_the_disc() {
    let Some((rig, clips)) = skeleton() else {
        eprintln!("skipped: no extracted rig (run tools/extract_assets.py)");
        return;
    };
    assert_eq!(rig.len(), 29);
    assert_eq!(rig.names[1], "center");
    assert_eq!(rig.parents[0], None);
    assert_eq!(
        rig.parents[rig.find("jnt_head").unwrap()],
        rig.find("jnt_chest")
    );
    assert!(clips.len() > 150, "{} clips", clips.len());
    let wait = clips.get("wait").expect("wait clip");
    assert_eq!(wait.joint_count(), 29);

    // Standing idle: head well above the pelvis, feet near the floor, everything finite.
    let pose = rig.pose(wait, 0.0);
    let y = |name: &str| pose[rig.find(name).unwrap()].w_axis.y;
    assert!(pose.iter().all(|m| m.is_finite()));
    assert!(
        (40.0..80.0).contains(&y("center")),
        "pelvis {}",
        y("center")
    );
    assert!(
        y("jnt_head") > y("center") + 30.0,
        "head {} pelvis {}",
        y("jnt_head"),
        y("center")
    );
    assert!(
        y("jnt_foot_R") < y("center") - 30.0,
        "foot {}",
        y("jnt_foot_R")
    );
}

#[test]
fn run_clip_moves_the_legs() {
    let Some((rig, clips)) = skeleton() else {
        return;
    };
    let run = clips.get("run2").expect("run2");
    let foot = rig.find("jnt_foot_R").unwrap();
    let ys: Vec<f32> = (0..20)
        .map(|i| rig.pose(run, i as f32 * run.frames / 20.0)[foot].w_axis.y)
        .collect();
    let (lo, hi) = ys
        .iter()
        .fold((f32::MAX, f32::MIN), |(l, h), &v| (l.min(v), h.max(v)));
    assert!(hi - lo > 8.0, "foot barely moves: {lo}..{hi}");
}

// ---- Ledges --------------------------------------------------------------------------------

/// Floor at 0, and a block from z = 100 onwards whose top is `top` high (its face looks -Z).
struct LedgeHost {
    top: f32,
}

impl Host for LedgeHost {
    fn ground(&self, p: Vec3) -> Option<libsms::GroundHit> {
        let h = if p.z >= 100.0 && p.y >= self.top - 1.0 {
            self.top
        } else {
            0.0
        };
        Some(libsms::GroundHit {
            height: h,
            normal: Vec3::Y,
            flags: libsms::SurfaceFlags::NONE,
        })
    }
    fn walls(&self, c: Vec3, r: f32, _m: usize, out: &mut Vec<libsms::WallHit>) -> Vec3 {
        if c.y < 0.0 || c.y >= self.top || c.z <= 100.0 - r || c.z >= 100.0 + r {
            return c;
        }
        out.push(libsms::WallHit {
            normal: Vec3::new(0.0, 0.0, -1.0),
            flags: libsms::SurfaceFlags::NONE,
        });
        Vec3::new(c.x, c.y, 100.0 - r)
    }
}

fn ledge_scene(top: f32, z: f32, y: f32) -> Scene<LedgeHost> {
    let mut s = Scene::new(LedgeHost { top });
    s.sim.mario.pos = Vec3::new(0.0, y, z);
    s.sim.mario.prev_pos = s.sim.mario.pos;
    s
}

#[test]
fn a_jump_against_a_ledge_catches_it_and_climbs_up() {
    if need_disc() {
        return;
    }
    let mut s = ledge_scene(300.0, 40.0, 0.0);
    s.step(2);
    s.pad.stick = Vec2::new(0.0, 0.5);
    s.pad.a = true;
    let caught = s.until(240, |m| m.status == Status::Hanging);
    s.pad.a = false;
    assert!(
        caught,
        "never caught: {:?} at {:?}",
        s.mario().status,
        s.mario().pos
    );
    let m = s.mario();
    assert_eq!(m.pos.y, 300.0);
    assert!((m.pos.z - 110.0).abs() < 1.0, "{:?}", m.pos);
    assert_eq!(m.anim.clip, "hang");
    // Hold toward the wall: after 40 ticks he climbs onto the ledge.
    s.pad.stick = Vec2::new(0.0, 1.0);
    assert!(s.until(120, |m| m.status == Status::Ascend));
    assert!(s.until(400, |m| m.status != Status::Ascend));
    assert!(s.mario().pos.y >= 299.0, "{:?}", s.mario().pos);
}

#[test]
fn b_lets_go_of_a_ledge() {
    if need_disc() {
        return;
    }
    let mut s = ledge_scene(300.0, 40.0, 0.0);
    s.step(2);
    s.pad.stick = Vec2::new(0.0, 0.5);
    s.pad.a = true;
    assert!(s.until(240, |m| m.status == Status::Hanging));
    s.pad.a = false;
    s.pad.stick = Vec2::ZERO;
    s.step(10);
    s.pad.b = true;
    s.step(1);
    s.pad.b = false;
    assert_ne!(s.mario().status, Status::Hanging);
    assert!(s.until(240, |m| m.pos.y == 0.0));
}

#[test]
fn walking_slowly_off_an_edge_lowers_mario_to_hang() {
    if need_disc() {
        return;
    }
    // On the block, walking slowly toward its edge (-Z).
    let mut s = ledge_scene(300.0, 200.0, 300.0);
    s.step(2);
    s.pad.stick = Vec2::new(0.0, -0.25);
    assert!(
        s.until(600, |m| m.status == Status::Descend),
        "{:?} at {:?}",
        s.mario().status,
        s.mario().pos
    );
    // He turned to face the wall.
    assert!(s.mario().face_yaw.abs() < 0x100, "{}", s.mario().face_yaw);
    s.pad.stick = Vec2::ZERO;
    assert!(s.until(300, |m| m.status == Status::Hanging));
}

// ---- Carrying ------------------------------------------------------------------------------

/// A flat floor with one fruit that Mario can take; it follows his hand while held.
struct FruitHost {
    fruit: Vec3,
    held: bool,
}

impl Host for FruitHost {
    fn ground(&self, _p: Vec3) -> Option<libsms::GroundHit> {
        Some(libsms::GroundHit {
            height: 0.0,
            normal: Vec3::Y,
            flags: libsms::SurfaceFlags::NONE,
        })
    }
    fn takeables(&self, pos: Vec3, radius: f32, out: &mut Vec<libsms::host::Takeable>) {
        if !self.held && (self.fruit - pos).length() < radius + 40.0 {
            out.push(libsms::host::Takeable {
                id: 7,
                pos: self.fruit,
                radius: 40.0,
                fruit: None,
            });
        }
    }
    fn take(&mut self, id: u64) -> bool {
        self.held = id == 7;
        self.held
    }
}

fn press_b<H: Host>(s: &mut Scene<H>) {
    s.pad.b = true;
    s.step(1);
    s.pad.b = false;
}

#[test]
fn b_with_nothing_in_reach_grabs_at_the_air() {
    let mut s = Scene::flat();
    s.step(2);
    press_b(&mut s);
    assert_eq!(s.mario().status, Status::TakePose);
    assert_eq!(s.mario().anim.clip, "get_fail");
    assert!(s.until(300, |m| m.status == Status::Wait));
    assert!(s.mario().held.is_none());
}

#[test]
fn mario_picks_up_fruit_carries_puts_and_throws_it() {
    let mut s = Scene::new(FruitHost {
        fruit: Vec3::new(0.0, 0.0, 60.0),
        held: false,
    });
    s.step(2);
    // Hold B: the hand closes on the fruit at frame 11 of `raise`.
    s.pad.b = true;
    s.step(1);
    assert_eq!(s.mario().status, Status::Take);
    s.step(1);
    assert_eq!(s.mario().anim.clip, "raise");
    s.pad.b = false;
    assert!(s.until(300, |m| m.status == Status::Wait));
    assert_eq!(s.mario().held, Some(7));
    assert_eq!(s.mario().upper, libsms::UpperState::HoldingObject);
    assert_eq!(s.mario().upper_layer().map(|u| u.0), Some("carry_p"));

    // B at a standstill puts it down.
    s.step(5);
    press_b(&mut s);
    assert_eq!(s.mario().status, Status::Putting);
    assert!(s.until(300, |m| m.held.is_none()));
    assert!(s.event_count(|e| matches!(e, Event::Put { id: 7, .. })) == 1);
    assert!(s.until(300, |m| m.status == Status::Wait));

    // Take it again, run, and B throws.
    s.host.world.held = false;
    s.host.world.fruit = s.mario().pos + Vec3::new(0.0, 0.0, 60.0);
    s.pad.b = true;
    s.step(1);
    s.pad.b = false;
    assert!(s.until(300, |m| m.held.is_some() && m.status == Status::Wait));
    s.run_forward();
    assert!(s.until(240, |m| m.forward_vel > 16.0));
    press_b(&mut s);
    assert_eq!(s.mario().status, Status::Pitching);
    assert!(s.until(120, |m| m.held.is_none()));
    assert!(s.event_count(|e| matches!(e, Event::Throw { id: 7, .. })) == 1);
}

/// Low gravity (a host's moon): the same jump goes higher and comes down slower; 1 is the game's own.
#[test]
fn a_gravity_scale_makes_jumps_higher_and_falls_slower() {
    let jump = |scale: f32| -> (f32, usize) {
        let mut s: Scene<FlatHost> = Scene::flat();
        s.sim.mario.set_gravity_scale(scale);
        s.step(2);
        s.press_a();
        let y0 = s.mario().pos.y;
        let (mut apex, mut ticks) = (y0, 0);
        for _ in 0..3000 {
            s.step(1);
            apex = apex.max(s.mario().pos.y);
            ticks += 1;
            if s.mario().on_ground() && ticks > 10 {
                break;
            }
        }
        (apex - y0, ticks)
    };
    let (normal_h, normal_t) = jump(1.0);
    let (moon_h, moon_t) = jump(1.0 / 6.0);
    assert!(moon_h > normal_h * 3.0, "moon jump {moon_h}, normal {normal_h}");
    assert!(moon_t > normal_t * 2, "moon airtime {moon_t} ticks, normal {normal_t}");
    // Back to 1 restores the game's own jump exactly.
    let (again_h, again_t) = jump(1.0);
    assert_eq!((again_h, again_t), (normal_h, normal_t));
}

/// The stick nudges a ground pound's fall sideways, only a little; with no stick it falls straight.
#[test]
fn the_stick_slightly_aims_a_ground_pound_in_its_fall() {
    let drift = |stick: Vec2| -> f32 {
        let mut s: Scene<FlatHost> = Scene::flat();
        s.sim.mario.pos = Vec3::new(0.0, 3000.0, 0.0);
        s.step(2);
        s.pad.l = true;
        s.step(1);
        s.pad.l = false;
        s.step(70); // past the wind-up, into the fall
        s.pad.stick = stick;
        let start = s.mario().pos;
        for _ in 0..3000 {
            if s.mario().status != Status::HipDrop {
                break;
            }
            s.step(1);
        }
        let end = s.mario().pos;
        ((end.x - start.x).powi(2) + (end.z - start.z).powi(2)).sqrt()
    };
    let (still, aimed) = (drift(Vec2::ZERO), drift(Vec2::Y));
    assert!(still < 1.0, "a pound with no stick drifted {still}");
    assert!(aimed > 20.0, "the stick didn't aim the pound ({aimed})");
    assert!(aimed < 1500.0, "the aim isn't slight ({aimed})");
}

/// A fire hit during a ground pound (wind-up or fall) doesn't cut it short or start the burn reaction.
#[test]
fn a_fire_hit_does_not_cancel_a_ground_pound() {
    for fall in [false, true] {
        let mut s: Scene<FlatHost> = Scene::flat();
        s.sim.mario.pos = Vec3::new(0.0, 3000.0, 0.0);
        s.step(2);
        s.pad.l = true;
        s.step(1);
        s.pad.l = false;
        if fall {
            s.step(100);
        }
        assert_eq!(s.mario().status, Status::HipDrop);
        s.sim.mario.damage(Vec3::new(50.0, 0.0, 0.0), libsms::DamageKind::Fire);
        s.sim.mario.knock_down(Vec3::new(50.0, 0.0, 0.0), libsms::DamageKind::Fire);
        s.step(1);
        assert_eq!(s.mario().status, Status::HipDrop, "the burn cancelled the pound (fall: {fall})");
    }
}

/// B in mid-air with something in hand throws it (no dive), and he stays in his jump.
#[test]
fn b_in_mid_air_throws_what_he_carries_instead_of_diving() {
    let mut s = Scene::new(FruitHost {
        fruit: Vec3::new(0.0, 0.0, 60.0),
        held: false,
    });
    s.step(2);
    s.pad.b = true;
    s.step(1);
    s.pad.b = false;
    assert!(s.until(300, |m| m.held.is_some() && m.status == Status::Wait));
    s.run_forward();
    assert!(s.until(240, |m| m.forward_vel > 16.0));
    s.press_a();
    assert!(s.until(60, |m| !m.on_ground() && m.vel.y > 0.0));
    s.step(5);
    press_b(&mut s);
    s.step(1);
    assert_eq!(s.event_count(|e| matches!(e, Event::Throw { id: 7, .. })), 1, "nothing was thrown");
    assert!(s.mario().held.is_none());
    assert_ne!(s.mario().status, Status::JumpCatch, "he dived instead");
    assert_ne!(s.mario().upper, libsms::UpperState::HoldingObject);
}

/// A stick circle during the pound's fall spins it (the landing reports it); without one it doesn't.
#[test]
fn a_stick_circle_during_the_pound_spins_it() {
    let run = |circle: bool| -> bool {
        let mut s: Scene<FlatHost> = Scene::flat();
        s.sim.mario.pos = Vec3::new(0.0, 1500.0, 0.0);
        s.step(2);
        s.pad.l = true;
        s.step(1);
        s.pad.l = false;
        assert_eq!(s.mario().status, Status::HipDrop);
        let mut angle = 0.0f32;
        for i in 0..3000 {
            if circle && i < 40 {
                angle += std::f32::consts::TAU / 12.0;
                s.pad.stick = Vec2::new(angle.cos(), angle.sin());
            } else {
                s.pad.stick = Vec2::ZERO;
            }
            if s.mario().status == Status::HipAttackEnd {
                break;
            }
            s.step(1);
        }
        let swirls = s.event_count(|e| matches!(e, Event::Particle(c) if c.name == "ms_m_blur2"));
        let spun = s.host.events.iter().any(|e| matches!(e, Event::GroundPound { spin: true, .. }));
        // The spin jump's swirl surrounds a spinning pound, and only that.
        assert_eq!(swirls > 8, spun, "swirl particles: {swirls}, spun: {spun}");
        spun
    };
    assert!(run(true), "the circle didn't spin the pound");
    assert!(!run(false), "a plain pound spun");
}

#[test]
fn what_he_carries_can_be_taken_away_and_his_hands_come_down() {
    let mut s = Scene::new(FruitHost {
        fruit: Vec3::new(0.0, 0.0, 60.0),
        held: false,
    });
    s.step(2);
    s.pad.b = true;
    s.step(1);
    s.pad.b = false;
    assert!(s.until(300, |m| m.held == Some(7) && m.status == Status::Wait));
    assert_eq!(s.mario().upper, libsms::UpperState::HoldingObject);
    // A teammate he held broke free: the host lets him go, and nothing is thrown.
    s.sim.mario.let_go();
    assert!(s.mario().held.is_none());
    assert_ne!(s.mario().upper, libsms::UpperState::HoldingObject);
    s.step(10);
    assert_eq!(s.event_count(|e| matches!(e, Event::Throw { .. })), 0);
    assert_eq!(s.event_count(|e| matches!(e, Event::Put { .. })), 0);
}

#[test]
fn a_mario_held_by_another_flails_in_the_fire_hop_without_fire_or_voice() {
    let mut s = Scene::flat();
    s.step(30);
    let at = s.mario().pos;
    s.sim.mario.set_carried(true);
    assert!(s.mario().is_carried());
    s.step(240);
    assert!(s.mario().is_carried());
    assert_eq!(s.mario().anim.clip, "firejmp");
    assert!((s.mario().pos - at).length() < 0.01, "pinned where the host put him");
    assert!(!s.mario().on_fire());
    // No yell, no flames; and hits do nothing while he is held.
    let loud = |e: &Event| match e {
        Event::Sound(c) => c.name.contains("DAMAGE_FIRE"),
        Event::Particle(_) => false,
        _ => false,
    };
    assert_eq!(s.event_count(loud), 0);
    assert!(!s.sim.mario.damage(at, libsms::DamageKind::EnemyCommon));
    assert!(s.mario().is_carried());
    // Set down: he falls and lands on his feet.
    s.sim.mario.set_carried(false);
    assert!(!s.mario().is_carried());
    assert!(s.until(600, |m| m.status == Status::Wait || m.status == Status::Landing || m.status == Status::JumpEnd));
}

// ---- Yoshi ---------------------------------------------------------------------------------

/// A flat floor with fruit lying about, and enemies (position, edible, caught); taken things
/// stay out of reach until they are eaten.
struct YoshiHost {
    fruits: Vec<(Vec3, libsms::Fruit, bool)>,
    enemies: Vec<(Vec3, bool, bool)>,
}

impl Host for YoshiHost {
    fn ground(&self, _p: Vec3) -> Option<libsms::GroundHit> {
        Some(libsms::GroundHit {
            height: 0.0,
            normal: Vec3::Y,
            flags: libsms::SurfaceFlags::NONE,
        })
    }
    fn takeables(&self, pos: Vec3, radius: f32, out: &mut Vec<libsms::host::Takeable>) {
        for (i, (p, kind, taken)) in self.fruits.iter().enumerate() {
            if !taken && (*p - pos).length() < radius + 40.0 {
                out.push(libsms::host::Takeable {
                    id: i as u64,
                    pos: *p,
                    radius: 40.0,
                    fruit: Some(*kind),
                });
            }
        }
    }
    fn take(&mut self, id: u64) -> bool {
        match self.fruits.get_mut(id as usize) {
            Some(f) if !f.2 => {
                f.2 = true;
                true
            }
            _ => false,
        }
    }
    fn enemies(&self, pos: Vec3, radius: f32, out: &mut Vec<libsms::Enemy>) {
        for (i, (p, _, caught)) in self.enemies.iter().enumerate() {
            if !caught && (*p - pos).length() < radius + 60.0 {
                out.push(libsms::Enemy {
                    id: i as u64,
                    pos: *p,
                    radius: 60.0,
                    height: 100.0,
                });
            }
        }
    }
    fn take_enemy(&mut self, id: u64) -> bool {
        match self.enemies.get_mut(id as usize) {
            Some(e) if e.1 && !e.2 => {
                e.2 = true;
                true
            }
            _ => false,
        }
    }
}

/// A scene with every clip length from the disc (Yoshi's and the egg's too).
fn yoshi_scene(fruits: Vec<(Vec3, libsms::Fruit)>) -> Scene<YoshiHost> {
    let mut s = Scene::new(YoshiHost {
        fruits: fruits.into_iter().map(|(p, k)| (p, k, false)).collect(),
        enemies: Vec::new(),
    });
    if let Some(dir) = extracted() {
        s.sim.mario.anims = libsms::Assets::load(&dir).anim_table;
    }
    s.sim.mario.give_fludd();
    s
}

/// Puts Yoshi under a falling Mario so he lands on him.
fn mount<H: Host>(s: &mut Scene<H>, fruit: Option<libsms::Fruit>) {
    s.step(2);
    s.press_a();
    assert!(s.until(240, |m| m.vel.y < 0.0 && m.pos.y < 60.0));
    let p = s.mario().pos;
    s.sim.mario.spawn_yoshi(Vec3::new(p.x, 0.0, p.z), 0, fruit);
    assert!(s.until(10, |m| m.on_yoshi()));
    assert!(s.until(240, |m| m.status == Status::Wait));
}

fn sound_count<H: Host>(s: &Scene<H>, name: &str) -> usize {
    s.event_count(|e| matches!(e, Event::Sound(c) if c.name == name))
}

#[test]
fn the_egg_hatches_yoshi_for_the_fruit_it_wants_and_comes_back() {
    let mut s = yoshi_scene(vec![(Vec3::new(0.0, 0.0, 60.0), libsms::Fruit::Papaya)]);
    let egg = s.sim.mario.add_egg(Vec3::new(0.0, 0.0, 600.0), i16::MIN);
    s.sim.mario.eggs[egg].wants = libsms::Fruit::Papaya;
    s.step(2);
    // Pick the papaya up and carry it to the egg.
    s.pad.b = true;
    s.step(1);
    s.pad.b = false;
    assert!(s.until(300, |m| m.held.is_some() && m.status == Status::Wait));
    assert_eq!(s.mario().held_fruit, Some(libsms::Fruit::Papaya));
    s.run_forward();
    assert!(s.until(240, |m| m.eggs[0].state == libsms::EggState::Right));
    s.pad.stick = Vec2::ZERO;
    assert_eq!(sound_count(&s, "MSD_SE_SY_COLLECT_PRETTY"), 1);
    // The egg breaks and Yoshi hatches, eating the papaya out of Mario's hands.
    assert!(s.until(600, |m| m.yoshi.state == libsms::YoshiState::Hatching));
    assert!(s.mario().held.is_none());
    assert_eq!(s.event_count(|e| matches!(e, Event::Eaten { id: 0 })), 1);
    assert_eq!(s.mario().yoshi.color, 1, "papaya juice is orange");
    assert!(s.until(600, |m| m.yoshi.state == libsms::YoshiState::Unmounted));
    assert!(s.until(600, |m| m.eggs[0].state == libsms::EggState::Gone));
    assert!(s.mario().yoshi_look().is_some());

    // Out of juice, Yoshi goes and the egg is back with a new wish.
    s.sim.mario.yoshi.juice = 30;
    assert!(s.until(120, |m| m.yoshi.state == libsms::YoshiState::Egg));
    assert_eq!(s.mario().eggs[0].state, libsms::EggState::Waiting);
    assert!(s.mario().yoshi_look().is_none());
}

#[test]
fn the_egg_shakes_its_head_at_the_wrong_fruit() {
    let mut s = yoshi_scene(vec![(Vec3::new(0.0, 0.0, 60.0), libsms::Fruit::Durian)]);
    let egg = s.sim.mario.add_egg(Vec3::new(0.0, 0.0, 600.0), i16::MIN);
    s.sim.mario.eggs[egg].wants = libsms::Fruit::Banana;
    s.step(2);
    s.pad.b = true;
    s.step(1);
    s.pad.b = false;
    assert!(s.until(300, |m| m.held.is_some() && m.status == Status::Wait));
    s.run_forward();
    assert!(s.until(600, |m| m.eggs[0].state == libsms::EggState::Wrong));
    s.pad.stick = Vec2::ZERO;
    assert_eq!(sound_count(&s, "MSD_SE_SY_NOT_COLLECT_YOSHI"), 1);
    assert!(!s.mario().yoshi.is_hatched());
    assert!(s.mario().held.is_some());
}

#[test]
fn landing_on_yoshi_rides_him_and_x_gets_off() {
    let mut s = yoshi_scene(vec![]);
    mount(&mut s, None);
    let m = s.mario();
    assert_eq!(m.fludd.as_ref().unwrap().nozzle, NozzleKind::Yoshi);
    assert_eq!(m.yoshi.pos, m.pos);
    assert_eq!(sound_count(&s, "MSD_SE_YV_DELICIOUS"), 1);
    // Mario's clip follows Yoshi's.
    assert!(
        s.mario().anim.clip.starts_with("yo_"),
        "{}",
        s.mario().anim.clip
    );
    // B is the tongue, not a dive or a grab.
    s.pad.b = true;
    s.step(1);
    s.pad.b = false;
    assert_ne!(s.mario().status, Status::TakePose);
    assert_ne!(
        s.mario().yoshi.tongue.state,
        libsms::tongue::TongueState::Idle
    );

    s.step(120);
    s.pad.x = true;
    s.step(1);
    s.pad.x = false;
    assert!(!s.mario().on_yoshi());
    assert_eq!(s.mario().status, Status::BackJump);
    assert_eq!(s.mario().yoshi.state, libsms::YoshiState::Unmounted);
    assert_eq!(s.mario().fludd.as_ref().unwrap().nozzle, NozzleKind::Spray);
    // Standing next to him, Mario is kept out of his body.
    assert!(s.until(600, |m| m.status == Status::Wait));
    let d = s.mario().pos - s.mario().yoshi.pos;
    assert!(Vec2::new(d.x, d.z).length() >= 100.0, "{d:?}");
}

#[test]
fn yoshi_runs_faster_and_flutters() {
    if need_disc() {
        return;
    }
    let top = |ride: bool| {
        let mut s = yoshi_scene(vec![]);
        if ride {
            mount(&mut s, None);
        }
        s.run_forward();
        s.secs(3.0);
        s.mario().forward_vel
    };
    let (walk, ride) = (top(false), top(true));
    assert!(ride > walk * 1.1, "riding {ride} vs {walk}");

    // Holding A past the top of a jump flutters: longer in the air.
    let airtime = |hold: bool| {
        let mut s = yoshi_scene(vec![]);
        mount(&mut s, None);
        s.pad.a = true;
        let mut n = 0;
        while (n == 0 || s.mario().status.is_jumping()) && n < 2000 {
            if s.mario().vel.y < 0.0 {
                s.pad.a = hold;
            }
            s.step(1);
            n += 1;
        }
        n
    };
    let (plain, held) = (airtime(false), airtime(true));
    assert!(
        held > plain + 60,
        "held {held} vs released at the top {plain}"
    );
}

#[test]
fn yoshi_skin_flutters_without_riding_and_lands_ready_again() {
    if need_disc() {
        return;
    }
    // The skin starts it with a fresh A press anywhere in the air, once per airtime.
    let mut s = yoshi_scene(vec![]);
    s.sim.mario.options.yoshi_skin = true;
    s.pad.a = true;
    s.step(30);
    s.pad.a = false;
    s.step(20);
    assert!(!s.mario().fluttering());
    s.pad.a = true;
    let mut fluttered = false;
    for _ in 0..60 {
        s.step(1);
        fluttered |= s.mario().fluttering();
    }
    assert!(fluttered, "a fresh A press in the air should flutter the skin");
    // Let go: the flutter ends, and the landing arms it again.
    s.pad.a = false;
    s.secs(2.0);
    assert!(!s.mario().fluttering());
    s.pad.a = true;
    s.step(30);
    s.pad.a = false;
    s.step(20);
    s.pad.a = true;
    let mut again = false;
    for _ in 0..60 {
        s.step(1);
        again |= s.mario().fluttering();
    }
    assert!(again, "the next jump flutters again after landing");
}

#[test]
fn yoshi_tongues_fruit_and_spits_its_juice() {
    let mut s = yoshi_scene(vec![(Vec3::new(0.0, 0.0, 3000.0), libsms::Fruit::Banana)]);
    mount(&mut s, None);
    // No juice before he has eaten.
    s.pad.r_analog = 255;
    s.step(30);
    s.pad.r_analog = 0;
    assert_eq!(
        s.event_count(|e| matches!(e, Event::WaterEmit(w) if w.nozzle == NozzleKind::Yoshi)),
        0
    );
    let p = s.mario().pos;
    s.host.world.fruits[0].0 = p + Vec3::new(0.0, 0.0, 250.0);
    s.pad.b = true;
    s.step(1);
    s.pad.b = false;
    assert!(
        s.until(240, |m| m.yoshi.color != 0),
        "{:?}",
        s.mario().yoshi.tongue
    );
    assert_eq!(s.event_count(|e| matches!(e, Event::Eaten { id: 0 })), 1);
    assert_eq!(s.mario().yoshi.color, 3, "banana juice is pink");
    assert_eq!(s.mario().juice_color(), 3);

    s.step(30);
    s.pad.r_analog = 255;
    s.step(30);
    let emits: Vec<WaterEmit> = s
        .host
        .events
        .iter()
        .filter_map(|e| match e {
            Event::WaterEmit(w) if w.nozzle == NozzleKind::Yoshi => Some(*w),
            _ => None,
        })
        .collect();
    assert!(!emits.is_empty());
    if !need_disc() {
        assert_eq!(emits[0].particle_type, 12, "WTP12_YoshiMouth");
    }
    // The head eats while spitting.
    assert_eq!(
        s.mario().yoshi.eat.map(|e| e.0),
        Some(libsms::yoshi::clip::EAT)
    );
}

#[test]
fn running_out_of_juice_throws_mario_off() {
    let mut s = yoshi_scene(vec![]);
    mount(&mut s, Some(libsms::Fruit::Pine));
    s.sim.mario.yoshi.juice = 20;
    assert!(s.until(60, |m| !m.on_yoshi()));
    assert_eq!(s.mario().status, Status::YoshiThrown);
    assert!(s.until(60, |m| m.yoshi.state == libsms::YoshiState::Egg));
}

#[test]
fn yoshi_tongues_and_eats_an_enemy_but_not_a_boo() {
    let mut s = yoshi_scene(vec![]);
    mount(&mut s, Some(libsms::Fruit::Papaya));
    let p = s.mario().pos;
    // A small enemy ahead, and something that won't be taken (`isEatenByYosshi` false).
    s.host.world.enemies = vec![(p + Vec3::new(0.0, 0.0, 300.0), true, false)];
    s.pad.b = true;
    s.step(1);
    s.pad.b = false;
    assert!(
        s.until(240, |m| m.yoshi.tongue.state
            == libsms::tongue::TongueState::Idle),
        "{:?}",
        s.mario().yoshi.tongue
    );
    assert_eq!(
        s.event_count(|e| matches!(e, Event::EnemyEaten { id: 0 })),
        1
    );
    // Enemies don't change the juice.
    assert_eq!(s.mario().yoshi.color, 1);

    s.host.world.enemies = vec![(p + Vec3::new(0.0, 0.0, 300.0), false, false)];
    s.step(30);
    s.pad.b = true;
    s.step(1);
    s.pad.b = false;
    assert!(s.until(240, |m| m.yoshi.tongue.state
        == libsms::tongue::TongueState::Idle));
    assert_eq!(s.event_count(|e| matches!(e, Event::EnemyEaten { .. })), 1);
    assert!(!s.host.world.enemies[0].2);

    // Alone, he only looks for fruit.
    s.pad.x = true;
    s.step(1);
    s.pad.x = false;
    assert!(s.until(600, |m| m.status == Status::Wait));
    let y = s.mario().yoshi.pos;
    s.host.world.enemies = vec![(y + Vec3::new(0.0, 0.0, 150.0), true, false)];
    s.step(600);
    assert_eq!(s.event_count(|e| matches!(e, Event::EnemyEaten { .. })), 1);
}

#[test]
fn a_hit_knocks_mario_back_costs_health_and_he_flashes() {
    let mut s = Scene::flat();
    s.step(2);
    let max = s.mario().params.mario.hp_max;
    assert_eq!(s.mario().health, max);
    // An enemy straight ahead (+Z): he falls on his back, sliding away from it.
    assert!(
        s.sim
            .mario
            .damage(Vec3::new(0.0, 0.0, 100.0), libsms::DamageKind::EnemyCommon)
    );
    assert_eq!(s.mario().health, max - 1);
    assert!(s.mario().is_invincible());
    assert!(
        matches!(
            s.mario().status,
            Status::ShortBackDown | Status::SafeBackDown | Status::BackDown
        ),
        "{:?}",
        s.mario().status
    );
    // While flashing, another hit does nothing.
    assert!(
        !s.sim
            .mario
            .damage(Vec3::new(0.0, 0.0, 100.0), libsms::DamageKind::EnemyCommon)
    );
    s.step(30);
    assert!(s.mario().pos.z < -10.0, "{:?}", s.mario().pos);
    assert_eq!(
        s.event_count(|e| matches!(e, Event::Damaged { damage: 1, .. })),
        1
    );
    assert!(s.until(600, |m| m.status == Status::Wait));
    assert!(s.until(600, |m| !m.is_invincible()));
    // From behind he falls forward, away from it.
    let behind = s.mario().pos - Vec3::new(0.0, 0.0, 100.0);
    assert!(s.sim.mario.damage(behind, libsms::DamageKind::EnemyCommon));
    assert!(
        matches!(
            s.mario().status,
            Status::ShortForeDown | Status::SafeForeDown | Status::ForeDown
        ),
        "{:?}",
        s.mario().status
    );
    assert_eq!(s.mario().health, max - 2);
}

#[test]
fn the_strollin_stu_only_trips_mario() {
    if need_disc() {
        return;
    }
    let mut s = Scene::flat();
    s.step(2);
    // `DmgHamukuri.prm`: down type 0, a stumble.
    assert!(
        s.sim
            .mario
            .damage(Vec3::new(0.0, 0.0, 100.0), libsms::DamageKind::Hamukuri)
    );
    assert_eq!(s.mario().status, Status::SafeBackDown);
    assert_eq!(sound_count(&s, "MSD_SE_MV03_DAMAGE_LITLE_01"), 0);
    s.step(1);
    assert_eq!(sound_count(&s, "MSD_SE_MV03_DAMAGE_LITLE_01"), 1);
}

#[test]
fn coins_heal_up_to_full() {
    let mut s = Scene::flat();
    s.sim.mario.health = 5;
    s.sim.mario.get_coin(libsms::Coin::Gold);
    assert_eq!(s.mario().health, 6);
    s.sim.mario.get_coin(libsms::Coin::Red);
    assert_eq!(s.mario().health, 8);
    s.sim.mario.get_coin(libsms::Coin::Blue);
    assert_eq!(s.mario().health, 8);
    assert_eq!(s.mario().coins, 1);
    s.step(1);
    assert_eq!(sound_count(&s, "MSD_SE_SY_COIN"), 1);
    // The chime only while it heals.
    assert_eq!(sound_count(&s, "MSD_SE_SY_HP_RECOVER"), 2);
}

#[test]
fn on_yoshi_a_hit_throws_mario_off_and_costs_nothing() {
    let mut s = yoshi_scene(vec![]);
    mount(&mut s, Some(libsms::Fruit::Papaya));
    let max = s.mario().health;
    assert!(
        s.sim
            .mario
            .damage(Vec3::new(0.0, 0.0, 100.0), libsms::DamageKind::EnemyCommon)
    );
    assert!(!s.mario().on_yoshi());
    assert_eq!(s.mario().status, Status::YoshiThrown);
    assert_eq!(s.mario().health, max);
    s.step(1);
    assert_eq!(sound_count(&s, "MSD_SE_YV_DAMAGE"), 1);
    assert_eq!(
        s.event_count(|e| matches!(e, Event::Damaged { yoshi: true, .. })),
        1
    );
}

#[test]
fn out_of_health_mario_lies_down_for_good() {
    let mut s = Scene::flat();
    s.step(2);
    for _ in 0..s.mario().params.mario.hp_max {
        s.sim.mario.invincible = 0;
        s.sim
            .mario
            .damage(Vec3::new(0.0, 0.0, 100.0), libsms::DamageKind::EnemyCommon);
    }
    assert_eq!(s.mario().health, 0);
    assert!(s.mario().is_down());
    s.step(600);
    assert!(s.mario().is_down());
    assert_eq!(s.event_count(|e| matches!(e, Event::Down)), 1);
    assert_eq!(sound_count(&s, "MSD_SE_MV08B_DOWN_REACT_01"), 1);
    s.sim.mario.invincible = 0;
    assert!(
        !s.sim
            .mario
            .damage(Vec3::ZERO, libsms::DamageKind::EnemyCommon)
    );
}

fn is_down_status(m: &Mario) -> bool {
    use Status::*;
    matches!(
        m.status,
        BackDown
            | ForeDown
            | ShortBackDown
            | ShortForeDown
            | SafeBackDown
            | SafeForeDown
            | JumpShortBackDown
            | JumpShortForeDown
            | JumpForeDown
            | JumpBackDown
    )
}

#[test]
fn a_host_knock_down_leaves_the_sim_health_alone() {
    let mut s = Scene::flat();
    s.step(2);
    let max = s.mario().params.mario.hp_max;
    assert!(
        s.sim
            .mario
            .knock_down(Vec3::new(0.0, 0.0, 100.0), libsms::DamageKind::EnemyCommon)
    );
    assert_eq!(s.mario().health, max);
    s.step(1);
    assert!(is_down_status(s.mario()), "{:?}", s.mario().status);
    assert_eq!(sound_count(&s, "MSD_SE_MV01_DAMAGE_BIG_01"), 1);
    // He can't be knocked again until the guard runs out, and it is short.
    assert!(
        !s.sim
            .mario
            .knock_down(Vec3::new(0.0, 0.0, 100.0), libsms::DamageKind::EnemyCommon)
    );
    assert!(s.mario().invincible <= 120);
}

#[test]
fn a_hit_does_not_take_him_out_of_a_ground_pound() {
    let mut s = Scene::flat();
    s.pad.a = true;
    s.press_a();
    s.step(40);
    s.pad.l = true;
    s.step(1);
    s.pad.l = false;
    assert_eq!(s.mario().status, Status::HipDrop);
    assert!(
        s.sim
            .mario
            .knock_down(Vec3::new(0.0, 0.0, 100.0), libsms::DamageKind::EnemyCommon)
    );
    // The hit lands (flash, voice) but he carries on with the pound (host extension).
    assert_eq!(s.mario().status, Status::HipDrop);
}

#[test]
fn a_hit_does_not_take_him_out_of_the_landing_of_a_ground_pound() {
    let mut s = Scene::flat();
    s.pad.a = true;
    s.press_a();
    s.step(40);
    s.pad.l = true;
    s.step(1);
    s.pad.l = false;
    assert_eq!(s.mario().status, Status::HipDrop);
    // Fall until he lands and the pound's get-up begins.
    assert!(s.until(600, |m| m.status == Status::HipAttackEnd));
    assert_eq!(s.mario().pounding(), 3);
    assert!(
        s.sim
            .mario
            .knock_down(Vec3::new(0.0, 0.0, 100.0), libsms::DamageKind::EnemyCommon)
    );
    // The hit lands (flash, voice) but the get-up is not cut short (host extension).
    assert_eq!(s.mario().status, Status::HipAttackEnd);
}

#[test]
fn a_dive_is_not_knocked_out_of() {
    let mut s = Scene::flat();
    s.pad.a = true;
    s.press_a();
    s.step(40);
    s.pad.b = true;
    s.step(1);
    s.pad.b = false;
    assert_eq!(s.mario().status, Status::JumpCatch);
    s.sim
        .mario
        .knock_down(Vec3::new(0.0, 0.0, 100.0), libsms::DamageKind::EnemyCommon);
    assert_eq!(s.mario().status, Status::JumpCatch);
}

#[test]
fn knockdown_cancel_gets_him_up_once_the_slide_is_spent() {
    if need_disc() {
        return;
    }
    let stood_after = |cancel: bool, stick: bool| {
        let mut s = Scene::flat();
        s.sim.mario.options.knockdown_cancel = cancel;
        s.step(2);
        assert!(
            s.sim
                .mario
                .knock_down(Vec3::new(0.0, 0.0, 100.0), libsms::DamageKind::EnemyCommon)
        );
        if stick {
            s.pad.stick = Vec2::new(0.0, 1.0);
        }
        let mut ticks = 0;
        while is_down_status(s.mario()) && ticks < 2000 {
            s.step(1);
            ticks += 1;
        }
        ticks
    };
    let whole = stood_after(false, true);
    let early = stood_after(true, true);
    let idle = stood_after(true, false);
    println!("whole clip {whole}, cancelled {early}, cancel but no input {idle}");
    assert!(early + 30 < whole, "{early} {whole}");
    assert!(
        idle >= whole - 2,
        "no input waits the whole clip: {idle} {whole}"
    );
}

/// A floor and some enemies that stay put: (feet, radius, height).
struct Enemies(Vec<(Vec3, f32, f32)>);

impl Host for Enemies {
    fn ground(&self, p: Vec3) -> Option<libsms::GroundHit> {
        FlatHost::default().ground(p)
    }
    fn enemies(&self, _pos: Vec3, _radius: f32, out: &mut Vec<libsms::Enemy>) {
        for (i, &(pos, radius, height)) in self.0.iter().enumerate() {
            out.push(libsms::Enemy {
                id: i as u64,
                pos,
                radius,
                height,
            });
        }
    }
}

fn enemy_hits<H: Host>(s: &Scene<H>, kind: libsms::EnemyHitKind) -> usize {
    s.event_count(|e| matches!(e, Event::EnemyHit { id: 0, kind: k } if *k == kind))
}

#[test]
fn landing_on_an_enemy_stomps_it_higher_each_time() {
    let mut s = Scene::new(Enemies(vec![(Vec3::ZERO, 60.0, 100.0)]));
    s.pad.a = true;
    s.step(1);
    let mut apexes = Vec::new();
    for n in 1..=3 {
        assert!(
            s.until(600, |m| m.status == Status::Trample && m.vel.y > 0.0),
            "stomp {n}: {:?}",
            s.mario().status
        );
        assert_eq!(enemy_hits(&s, libsms::EnemyHitKind::Trample), n);
        let mut apex = 0.0f32;
        while s.mario().vel.y > 0.0 {
            s.step(1);
            apex = apex.max(s.mario().pos.y);
        }
        apexes.push(apex);
    }
    assert!(apexes[0] < apexes[1] && apexes[1] < apexes[2], "{apexes:?}");
}

/// How far the first three stomps on an enemy carry Mario above where each bounce began.
fn stomp_rises(bounce: f32) -> Vec<f32> {
    let mut s = Scene::new(Enemies(vec![(Vec3::ZERO, 60.0, 100.0)]));
    s.sim.mario.options.stomp_bounce = bounce;
    s.pad.a = true;
    s.step(1);
    (0..3)
        .map(|_| {
            assert!(s.until(600, |m| m.status == Status::Trample && m.vel.y > 0.0));
            let from = s.mario().pos.y;
            let mut apex = from;
            while s.mario().vel.y > 0.0 {
                s.step(1);
                apex = apex.max(s.mario().pos.y);
            }
            apex - from
        })
        .collect()
}

#[test]
fn stomp_bounce_option_bounces_higher() {
    let (game, bouncy) = (stomp_rises(1.0), stomp_rises(2.5));
    println!("stomp rises (units): game {game:?}, x2.5 {bouncy:?}");
    for (g, b) in game.iter().zip(&bouncy) {
        let ratio = b / g;
        assert!((2.2..2.8).contains(&ratio), "{game:?} {bouncy:?}");
    }
}

#[test]
fn a_ground_pound_onto_an_enemy_hits_it_once() {
    let mut s = Scene::new(Enemies(vec![(Vec3::ZERO, 60.0, 100.0)]));
    // A held: a full jump, well above the enemy before pounding.
    s.pad.a = true;
    s.step(40);
    s.pad.l = true;
    s.step(1);
    s.pad.l = false;
    s.pad.a = false;
    assert_eq!(s.mario().status, Status::HipDrop);
    assert!(s.until(1500, |m| m.status == Status::HipAttackEnd
        || m.status == Status::SlipEnd));
    assert_eq!(enemy_hits(&s, libsms::EnemyHitKind::HipDrop), 1);
    assert_eq!(enemy_hits(&s, libsms::EnemyHitKind::Trample), 0);
}

#[test]
fn enemy_push_keeps_mario_out_of_enemies() {
    let reach = |push: bool| {
        let mut s = Scene::new(Enemies(vec![(Vec3::new(0.0, 0.0, 300.0), 60.0, 100.0)]));
        s.sim.mario.options.enemy_push = push;
        s.run_forward();
        s.secs(2.0);
        s.mario().pos.z
    };
    let (blocked, through) = (reach(true), reach(false));
    // Out of the enemy's cylinder: its radius plus Mario's (50) short of it.
    assert!(blocked < 300.0 - 100.0, "pushed: {blocked}");
    assert!(through > 400.0, "not pushed: {through}");
}

#[test]
fn cycling_nozzles_steps_through_all_four() {
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    s.sim.mario.options.cycle_nozzles = true;
    let mut seen = vec![s.mario().fludd.as_ref().unwrap().nozzle];
    for _ in 0..4 {
        s.pad.x = true;
        s.step(1);
        s.pad.x = false;
        s.secs(1.5);
        let f = s.mario().fludd.as_ref().unwrap();
        assert!(!f.is_switching(), "still switching");
        seen.push(f.nozzle);
    }
    use NozzleKind::*;
    assert_eq!(seen, vec![Spray, Hover, Rocket, Turbo, Spray]);
}

#[test]
fn move_speed_option_covers_more_ground() {
    let run = |k: f32| {
        let mut s = Scene::flat();
        s.sim.mario.options.move_speed = k;
        s.run_forward();
        s.secs(2.0);
        s.mario().pos.z
    };
    let (one, fast) = (run(1.0), run(1.5));
    assert!((fast / one - 1.5).abs() < 0.05, "{one} -> {fast}");
}

/// Pounds from `height` above the floor; returns the landing's `(super_, drop)` and whether
/// Mario fell in flames ([`Mario::pounding`] 2) on the way.
fn pound_from(s: &mut Scene<FlatHost>, height: f32) -> (bool, f32, bool) {
    s.sim.mario.pos = Vec3::new(0.0, height, 0.0);
    s.step(2);
    s.pad.l = true;
    s.step(1);
    s.pad.l = false;
    assert_eq!(s.mario().status, Status::HipDrop);
    let mut flames = false;
    for _ in 0..3000 {
        flames |= s.mario().pounding() == 2;
        if s.mario().status == Status::HipAttackEnd {
            break;
        }
        s.step(1);
    }
    assert_eq!(s.mario().status, Status::HipAttackEnd);
    let (super_, drop) = s
        .host
        .events
        .iter()
        .rev()
        .find_map(|e| match e {
            Event::GroundPound { super_, drop, .. } => Some((*super_, *drop)),
            _ => None,
        })
        .expect("no ground pound event");
    (super_, drop, flames)
}

#[test]
fn a_long_ground_pound_turns_super_and_reports_its_drop() {
    let (low, high) = (
        pound_from(&mut Scene::flat(), 300.0),
        pound_from(&mut Scene::flat(), 3000.0),
    );
    println!("pound from 300: {low:?}, from 3000: {high:?}");
    assert!(!low.0 && !low.2, "{low:?}");
    assert!(high.0 && high.2, "{high:?}");
    // The wind-up lifts Mario a little before the fall.
    assert!((300.0..450.0).contains(&low.1), "{low:?}");
    assert!((3000.0..3150.0).contains(&high.1), "{high:?}");
}

/// Apex above the floor of the jump out of a pound from `height`.
fn pound_jump_apex(height: f32, share: f32) -> Option<f32> {
    let mut s = Scene::flat();
    s.sim.mario.options.pound_jump = share;
    pound_from(&mut s, height);
    s.press_a();
    if s.mario().status != Status::UltraJump {
        return None;
    }
    let mut apex = 0.0f32;
    while s.mario().vel.y > 0.0 || s.mario().pos.y > apex {
        apex = apex.max(s.mario().pos.y);
        s.step(1);
    }
    Some(apex)
}

#[test]
fn pound_jump_climbs_back_a_share_of_the_fall() {
    assert_eq!(pound_jump_apex(3000.0, 0.0), None, "the game ignores A");
    let high = pound_jump_apex(3000.0, 0.75).unwrap();
    let low = pound_jump_apex(300.0, 0.75).unwrap();
    println!("pound jump apex: from 3000 {high:.0}, from 300 {low:.0}");
    assert!((0.7 * 3000.0..0.85 * 3000.0).contains(&high), "{high}");
    // Never lower than a triple jump (about 800).
    assert!(low > 700.0, "{low}");
}

/// How A meets the first stomp.
#[derive(Debug, Clone, Copy)]
enum StompA {
    /// Held since the jump: no fresh press (the game's full bounce).
    Held,
    /// Let go before, never pressed again (the game cuts the bounce short).
    Never,
    /// Pressed just before landing on the enemy, and held.
    Early,
    /// Let go before, pressed this many ticks into the bounce, and held.
    Late(u32),
}

/// The first stomp's rise above where the bounce began.
fn stomp_rise_with_a(a: StompA) -> f32 {
    let mut s = Scene::new(Enemies(vec![(Vec3::ZERO, 60.0, 100.0)]));
    s.sim.mario.options.stomp_jump = 2.0;
    s.pad.a = true;
    s.step(1);
    s.pad.a = matches!(a, StompA::Held);
    for _ in 0..600 {
        let m = s.mario();
        if m.status == Status::Trample {
            break;
        }
        // About 8 ticks from the enemy's top (100).
        if matches!(a, StompA::Early) && m.vel.y < 0.0 && m.pos.y - 100.0 < -m.vel.y * 0.25 * 8.0 {
            s.pad.a = true;
        }
        s.step(1);
    }
    assert!(s.mario().status == Status::Trample && s.mario().vel.y > 0.0);
    let from = s.mario().pos.y;
    let mut apex = from;
    let mut t = 0;
    while s.mario().vel.y > 0.0 {
        if let StompA::Late(n) = a
            && t >= n
        {
            s.pad.a = true;
        }
        s.step(1);
        t += 1;
        apex = apex.max(s.mario().pos.y);
    }
    apex - from
}

#[test]
fn a_well_timed_a_on_a_stomp_bounces_higher() {
    let rise = |a| (a, stomp_rise_with_a(a));
    let [held, never, early, late, too_late] = [
        StompA::Held,
        StompA::Never,
        StompA::Early,
        StompA::Late(3),
        StompA::Late(40),
    ]
    .map(rise);
    println!("stomp rises: {held:?} {never:?} {early:?} {late:?} {too_late:?}");
    let base = held.1;
    assert!(never.1 < base, "letting go cuts the bounce: {never:?}");
    assert!(early.1 > 1.7 * base, "{early:?} vs {base}");
    assert!(late.1 > 1.7 * base, "{late:?} vs {base}");
    assert!(too_late.1 < 1.2 * base, "{too_late:?} vs {base}");
}

/// How far a belly slide carries Mario from where it starts.
fn belly_slide_length(wet: bool) -> f32 {
    let mut s = Scene::flat();
    s.run_forward();
    s.secs(1.0);
    s.press_a();
    s.step(10);
    s.pad.b = true;
    s.step(1);
    s.pad.b = false;
    s.pad.stick = Vec2::ZERO;
    assert!(s.until(600, |m| m.status == Status::Catch));
    let from = s.mario().pos;
    s.sim.mario.water_underfoot = wet;
    assert!(s.until(4000, |m| m.status != Status::Catch));
    (s.mario().pos - from).length()
}

#[test]
fn a_belly_slide_over_fludd_water_glides() {
    let (dry, wet) = (belly_slide_length(false), belly_slide_length(true));
    println!("belly slide: dry {dry:.0}, wet {wet:.0}");
    assert!(wet > 1.6 * dry, "{dry} {wet}");
}

#[test]
fn a_spin_jump_calls_out_and_whooshes() {
    let mut s = Scene::flat();
    // Two quick turns of the stick, then A.
    for i in 0..24 {
        let a = i as f32 * std::f32::consts::TAU / 12.0;
        s.pad.stick = Vec2::new(a.cos(), a.sin());
        s.step(1);
    }
    s.press_a();
    assert!(
        matches!(
            s.mario().status,
            Status::LeftRotateJump | Status::RightRotateJump
        ),
        "{:?}",
        s.mario().status
    );
    s.step(20);
    let cue = |name: &str| s.event_count(|e| matches!(e, Event::Sound(c) if c.name == name));
    assert_eq!(cue("MSD_SE_MV24_JUMP_SPECIAL_01"), 1);
    assert_eq!(cue("MSD_SE_MA_ROLL_JUMP"), 1);
}

#[test]
fn a_spin_jumps_spray_ignores_the_hosts_aim_and_is_tagged() {
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    // The host's aim (the crosshair), a quarter turn off to the side of where he faces.
    let aim_yaw = s.sim.mario.model_yaw.wrapping_add(0x4000);
    s.sim.mario.aim = Some(libsms::Aim { yaw: aim_yaw, pitch: 0 });
    let aim_heading = aim_yaw as f32 * std::f32::consts::PI / 32768.0;
    let off = |a: f32, b: f32| (a - b + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    let waters = |s: &Scene<FlatHost>| -> Vec<libsms::WaterEmit> {
        s.host
            .events
            .iter()
            .filter_map(|e| if let Event::WaterEmit(w) = e { Some(*w) } else { None })
            .collect()
    };
    // Spraying standing: it goes where the aim says, and isn't a spin's.
    s.pad.r_analog = 200;
    s.step(30);
    let flat = waters(&s);
    assert!(!flat.is_empty() && flat.iter().all(|w| !w.spin));
    assert!(
        flat.iter().all(|w| off(w.dir.x.atan2(w.dir.z), aim_heading).abs() < 0.4),
        "the standing spray didn't go where the host aimed"
    );
    // The same spray in a spin jump: tagged, and fanning out all round as he turns.
    s.host.events.clear();
    s.pad.r_analog = 0;
    s.step(2);
    for i in 0..24 {
        let a = i as f32 * std::f32::consts::TAU / 12.0;
        s.pad.stick = Vec2::new(a.cos(), a.sin());
        s.step(1);
    }
    s.press_a();
    assert!(matches!(s.mario().status, Status::LeftRotateJump | Status::RightRotateJump));
    s.host.events.clear();
    s.pad.stick = Vec2::ZERO;
    s.pad.r_analog = 200;
    s.step(40);
    let spun = waters(&s);
    assert!(!spun.is_empty());
    assert!(spun.iter().all(|w| w.spin), "a spin jump's water wasn't tagged");
    // Not the host's aim: in a spin the nozzle goes with him (the view's posed muzzle turns with the
    // spin; see libsms-present's test).
    assert!(
        spun.iter().all(|w| off(w.dir.x.atan2(w.dir.z), aim_heading).abs() > 0.8),
        "a spin jump's spray was bent to the host's aim"
    );
}

#[test]
fn a_ground_spin_sprays_all_round_whatever_the_hosts_aim() {
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    let aim_yaw = s.sim.mario.model_yaw.wrapping_add(0x4000);
    s.sim.mario.aim = Some(libsms::Aim { yaw: aim_yaw, pitch: 0 });
    // Spraying in the ground spin (a stick circle starts it from a turn or a stop; here he is put in it).
    s.pad.r_analog = 200;
    s.step(10);
    s.sim.mario.status = Status::RotateL;
    s.host.events.clear();
    s.step(20);
    assert!(matches!(s.mario().status, Status::RotateL), "{:?}", s.mario().status);
    let spun: Vec<libsms::WaterEmit> = s
        .host
        .events
        .iter()
        .filter_map(|e| if let Event::WaterEmit(w) = e { Some(*w) } else { None })
        .collect();
    assert!(!spun.is_empty(), "the ground spin didn't spray");
    assert!(spun.iter().all(|w| w.spin), "a ground spin's water wasn't tagged");
    // Where it flies is the view's posed muzzle turning with the spin, not the host's aim: see
    // libsms-present's `a_ground_spins_water_turns_round_in_the_world_whatever_the_host_aims`.
}

#[test]
fn the_ground_spin_has_its_water_shot_and_swirl_effects() {
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    s.pad.r_analog = 200;
    s.step(10);
    s.sim.mario.status = Status::RotateL;
    s.host.events.clear();
    s.step(8);
    let named = |name: &str| s.event_count(|e| matches!(e, Event::Particle(c) if c.name == name));
    // `rotating`: emitRotateShootEffect (spinshot a and b) and emitBlurSpinJump, every tick.
    assert!(named("ms_m_spinshot_a") >= 8, "{}", named("ms_m_spinshot_a"));
    assert!(named("ms_m_spinshot_b") >= 8);
    assert!(named("ms_m_blur2") >= 8 && named("ms_m_blur2sp") >= 8);
}

#[test]
fn a_wet_belly_slide_holds_the_slide_pose() {
    if need_disc() {
        return;
    }
    let mut s = Scene::flat();
    s.run_forward();
    s.secs(1.0);
    s.press_a();
    s.step(10);
    s.pad.b = true;
    s.step(1);
    s.pad.b = false;
    s.pad.stick = Vec2::ZERO;
    assert!(s.until(600, |m| m.status == Status::Catch));
    s.sim.mario.water_underfoot = true;
    // `catching` stops the dive clip at frame 50 for the whole slide, instead of letting it
    // run on (and loop the dive's spin).
    let mut ticks = 0;
    while s.mario().status == Status::Catch && ticks < 4000 {
        // (The tick's own playback steps it half a frame on before the next clamp.)
        assert!(
            s.mario().anim.frame <= 50.5,
            "frame {}",
            s.mario().anim.frame
        );
        s.step(1);
        ticks += 1;
    }
    assert!(ticks > 120, "slid {ticks} ticks");
}

#[test]
fn a_long_fall_cries_out_once() {
    struct Cliff;
    impl Host for Cliff {
        fn ground(&self, p: Vec3) -> Option<libsms::GroundHit> {
            let h = if p.z < 200.0 { 0.0 } else { -100_000.0 };
            (p.y >= h).then_some(libsms::GroundHit {
                height: h,
                normal: Vec3::Y,
                flags: libsms::SurfaceFlags::NONE,
            })
        }
    }
    let mut s = Scene::new(Cliff);
    s.run_forward();
    assert!(s.until(600, |m| m.status == Status::Landing));
    let cry = |s: &Scene<Cliff>| {
        s.event_count(|e| matches!(e, Event::Sound(c) if c.name == "MSD_SE_MV10B_CRY_JUMP_01"))
    };
    // `landing`: two seconds into the fall, the cry and the flailing pose.
    s.step(200);
    assert_eq!(cry(&s), 0);
    s.step(100);
    assert_eq!(cry(&s), 1);
    assert_eq!(s.mario().anim.clip, "thrown");
    s.step(200);
    assert_eq!(cry(&s), 1);
}

#[test]
fn a_long_fall_can_still_dive_out() {
    struct Cliff;
    impl Host for Cliff {
        fn ground(&self, p: Vec3) -> Option<libsms::GroundHit> {
            let h = if p.z < 200.0 { 0.0 } else { -100_000.0 };
            (p.y >= h).then_some(libsms::GroundHit {
                height: h,
                normal: Vec3::Y,
                flags: libsms::SurfaceFlags::NONE,
            })
        }
    }
    let mut s = Scene::new(Cliff);
    s.run_forward();
    assert!(s.until(600, |m| m.status == Status::Landing));
    s.step(300);
    assert_eq!(s.mario().anim.clip, "thrown");
    s.pad.b = true;
    s.step(1);
    s.pad.b = false;
    assert_eq!(
        s.mario().status,
        Status::JumpCatch,
        "B did not take him out of the long fall"
    );
    s.step(10);
    assert_eq!(s.mario().status, Status::JumpCatch, "the dive ended in mid-air");
}

#[test]
fn a_long_fall_can_still_hover_out() {
    if need_disc() {
        return;
    }
    struct Cliff;
    impl Host for Cliff {
        fn ground(&self, p: Vec3) -> Option<libsms::GroundHit> {
            let h = if p.z < 200.0 { 0.0 } else { -100_000.0 };
            (p.y >= h).then_some(libsms::GroundHit {
                height: h,
                normal: Vec3::Y,
                flags: libsms::SurfaceFlags::NONE,
            })
        }
    }
    let mut s = Scene::new(Cliff);
    // RoR2's host option: the game refuses the pump in the `thrown` flail (its table entry is
    // `isPumpOK` false); RoR2's long drops must still allow a Hover/Rocket out.
    s.sim.mario.options.pump_in_long_fall = true;
    s.sim.mario.give_fludd();
    {
        let p = s.sim.mario.params.clone();
        let f = s.sim.mario.fludd.as_mut().unwrap();
        f.change_nozzle(NozzleKind::Hover, false, &p);
        f.refill(&p);
    }
    s.run_forward();
    assert!(s.until(600, |m| m.status == Status::Landing));
    s.step(300);
    assert_eq!(s.mario().anim.clip, "thrown");
    s.pad.r = true;
    s.pad.r_analog = 255;
    s.step(2);
    assert_eq!(
        s.mario().status,
        Status::Rocket,
        "the Hover did not take him out of the long fall"
    );
}

#[test]
fn short_hop_option_keeps_a_tapped_jump_high() {
    fn apex(short_hop: f32, hold_ticks: u32) -> f32 {
        let mut s = Scene::flat();
        s.sim.mario.options.short_hop = short_hop;
        let mut apex = 0.0f32;
        s.pad.a = true;
        for t in 0..400 {
            if t == hold_ticks {
                s.pad.a = false;
            }
            s.step(1);
            apex = apex.max(s.mario().pos.y);
        }
        apex
    }
    let full = apex(0.0, 200);
    let (game, raised) = (apex(0.0, 3), apex(0.5, 3));
    println!("tap: game {game:.0}, short_hop 0.5 {raised:.0}, full {full:.0}");
    assert!(raised >= 0.5 * full - 1.0, "{raised} vs {full}");
    assert!(raised > game + 20.0, "{raised} vs {game}");
    // Held, it changes nothing.
    assert!((apex(0.5, 200) - full).abs() < 1.0);
}

// ---- host limits (limits.rs): none of this is the game's ------------------------------------

fn emits(s: &Scene<FlatHost>) -> u32 {
    s.host
        .events
        .iter()
        .map(|e| match e {
            Event::WaterEmit(w) => w.count,
            _ => 0,
        })
        .sum()
}

/// Mario in the air with FLUDD's Spray, well clear of the ground.
fn airborne_with_fludd(s: &mut Scene<FlatHost>) {
    s.sim.mario.give_fludd();
    s.pad.a = true;
    s.press_a();
    s.pad.a = true;
    s.step(10);
    assert!(s.mario().status.is_jumping());
}

fn tap_r(s: &mut Scene<FlatHost>) {
    s.pad.r_analog = 200;
    s.step(3);
    s.pad.r_analog = 0;
    s.step(1);
}

fn air_uses(s: &Scene<FlatHost>, extra: bool) -> usize {
    s.event_count(|e| matches!(e, Event::AirNozzle { extra: x } if *x == extra))
}

#[test]
fn limited_air_uses_spend_the_free_one_then_extras_then_refuse() {
    let mut s = Scene::flat();
    s.sim.mario.limits.air_uses = true;
    s.sim.mario.limits.air_extra = 1;
    airborne_with_fludd(&mut s);
    // The Hover: the Spray's starts aren't counted (see the next test).
    let params = s.sim.mario.params.clone();
    s.sim.mario.fludd.as_mut().unwrap().change_nozzle(libsms::params::NozzleKind::Hover, false, &params);
    tap_r(&mut s);
    assert_eq!((air_uses(&s, false), air_uses(&s, true)), (1, 0));
    tap_r(&mut s);
    assert_eq!((air_uses(&s, false), air_uses(&s, true)), (1, 1));
    assert_eq!(s.mario().limits.air_extra, 0);
    // None left: R does nothing.
    let before = emits(&s);
    s.pad.r_analog = 200;
    s.step(40);
    assert_eq!(emits(&s), before, "sprayed without a use");
    assert_eq!(air_uses(&s, false) + air_uses(&s, true), 2);
    // Let go and land, and the free use is back.
    s.pad.r_analog = 0;
    s.pad.a = false;
    assert!(s.until(600, |m| !m.status.is_jumping()));
    s.step(5);
    s.press_a();
    s.step(10);
    tap_r(&mut s);
    assert_eq!(air_uses(&s, false), 2);
}

#[test]
fn a_granted_extra_use_starts_another_hover_in_the_same_jump() {
    if need_disc() {
        return;
    }
    // The game allows one Rocket or Hover start per jump; each use the host grants is another.
    let mut s = Scene::flat();
    s.sim.mario.limits.air_uses = true;
    s.sim.mario.limits.air_extra = 1;
    airborne_with_fludd(&mut s);
    let params = s.sim.mario.params.clone();
    s.sim.mario.fludd.as_mut().unwrap().change_nozzle(libsms::params::NozzleKind::Hover, false, &params);
    s.pad.r_analog = 200;
    s.step(3);
    assert_eq!(s.mario().status, Status::Rocket, "the free use did not start the hover");
    // Let go; it ends, and he falls.
    s.pad.r_analog = 0;
    assert!(s.until(120, |m| m.status != Status::Rocket));
    // The extra use: another hover, which the same jump's first one would have blocked.
    s.pad.r_analog = 200;
    s.step(6);
    assert_eq!(s.mario().status, Status::Rocket, "the extra use did not start a second hover: uses {:?}, extra left {}, airborne {}", (air_uses(&s, false), air_uses(&s, true)), s.mario().limits.air_extra, s.mario().status.is_jumping());
    assert_eq!((air_uses(&s, false), air_uses(&s, true)), (1, 1));
}

#[test]
fn rocket_after_rocket_in_one_jump_needs_and_spends_a_granted_use() {
    if need_disc() {
        return;
    }
    // From the ground: the first Rocket is the jump's free use; each Rocket after it in the air is one of the extras.
    let run = |extra: u32| {
        let mut s = Scene::flat();
        s.sim.mario.limits.air_uses = true;
        s.sim.mario.limits.air_extra = extra;
        s.sim.mario.give_fludd();
        let params = s.sim.mario.params.clone();
        s.sim.mario.fludd.as_mut().unwrap().change_nozzle(libsms::params::NozzleKind::Rocket, false, &params);
        let mut launches = 0;
        for _ in 0..3 {
            // Hold R until the Rocket launches (or give up), then let go and come off it.
            s.pad.r_analog = 255;
            let launched = s.until(300, |m| m.status == Status::Rocket);
            s.pad.r_analog = 0;
            if !launched {
                break;
            }
            launches += 1;
            s.step(30);
            assert!(s.until(600, |m| m.status != Status::Rocket));
            s.step(2);
            if !s.mario().status.is_jumping() && s.mario().status != Status::RocketLanding {
                break; // down already
            }
        }
        launches
    };
    assert_eq!(run(0), 1, "a second Rocket with no extra use");
    assert_eq!(run(1), 2, "the extra use did not start a second Rocket");
    assert_eq!(run(2), 3, "two extras did not start the third Rocket");
}

#[test]
fn a_rocket_from_the_ground_is_the_jumps_free_use() {
    if need_disc() {
        return;
    }
    let mut s = Scene::flat();
    s.sim.mario.limits.air_uses = true;
    s.sim.mario.limits.air_extra = 0;
    s.sim.mario.give_fludd();
    let params = s.sim.mario.params.clone();
    s.sim.mario.fludd.as_mut().unwrap().change_nozzle(libsms::params::NozzleKind::Hover, false, &params);
    // Hover started on the ground: the jump's one start.
    s.pad.r_analog = 200;
    s.step(6);
    assert_eq!(s.mario().status, Status::Rocket);
    s.pad.r_analog = 0;
    assert!(s.until(120, |m| m.status != Status::Rocket));
    // Another in the air has no use to spend (no extra on hand): refused.
    s.pad.r_analog = 200;
    s.step(6);
    assert_ne!(s.mario().status, Status::Rocket, "a second start with no extra use");
    assert_eq!(air_uses(&s, true), 0);
}

#[test]
fn the_sprays_air_starts_are_free_but_other_nozzles_are_counted() {
    let mut s = Scene::flat();
    s.sim.mario.limits.air_uses = true;
    s.sim.mario.limits.air_extra = 0;
    airborne_with_fludd(&mut s);
    // The Spray as often as he likes in one jump: nothing is spent, and it always fires.
    for _ in 0..3 {
        let before = emits(&s);
        s.pad.r_analog = 200;
        s.step(15);
        assert!(emits(&s) > before, "the Spray was refused");
        s.pad.r_analog = 0;
        s.step(4);
    }
    assert_eq!(s.event_count(|e| matches!(e, Event::AirNozzle { .. })), 0);
    // Another nozzle in the same jump still takes the free use, and then none.
    let params = s.sim.mario.params.clone();
    s.sim.mario.fludd.as_mut().unwrap().change_nozzle(libsms::params::NozzleKind::Hover, false, &params);
    tap_r(&mut s);
    assert_eq!(air_uses(&s, false), 1);
    tap_r(&mut s);
    assert_eq!(air_uses(&s, false), 1, "a second Hover start had no use to spend");
}

#[test]
fn air_uses_are_free_on_the_ground_and_unlimited_by_default() {
    let mut s = Scene::flat();
    airborne_with_fludd(&mut s);
    for _ in 0..3 {
        tap_r(&mut s);
    }
    assert_eq!(s.event_count(|e| matches!(e, Event::AirNozzle { .. })), 0);
    let mut s = Scene::flat();
    s.sim.mario.limits.air_uses = true;
    s.sim.mario.give_fludd();
    s.pad.r_analog = 200;
    s.step(20);
    assert_eq!(s.event_count(|e| matches!(e, Event::AirNozzle { .. })), 0);
}

#[test]
fn air_jumps_climb_the_chain() {
    let mut s = Scene::flat();
    s.sim.mario.limits.air_jumps = 3;
    s.run_forward();
    s.secs(1.0);
    s.press_a();
    assert_eq!(s.mario().status, Status::Jump);
    s.step(20);
    s.press_a();
    assert_eq!(s.mario().status, Status::SecondJump);
    s.step(20);
    s.press_a();
    assert_eq!(s.mario().status, Status::UltraJump);
    let y = s.mario().pos.y;
    s.step(20);
    s.press_a();
    assert_eq!(s.mario().status, Status::UltraJump);
    assert!(s.mario().vel.y > 0.0 && s.mario().pos.y > y);
    // All three spent.
    s.step(20);
    s.press_a();
    assert_eq!(s.event_count(|e| matches!(e, Event::AirJump)), 3);
    // Back on landing.
    s.pad.stick = Vec2::ZERO;
    assert!(s.until(2000, |m| !m.status.is_jumping()));
    s.step(30);
    s.press_a();
    s.step(20);
    s.press_a();
    assert_eq!(s.event_count(|e| matches!(e, Event::AirJump)), 4);
}

#[test]
fn an_air_jump_cancels_a_dive_in_flight() {
    let dive = |air_jumps: u32| {
        let mut s = Scene::flat();
        s.sim.mario.limits.air_jumps = air_jumps;
        s.run_forward();
        s.secs(1.0);
        s.press_a();
        s.step(30);
        s.pad.b = true;
        s.step(1);
        s.pad.b = false;
        assert_eq!(s.mario().status, Status::JumpCatch);
        s.step(10);
        let (y, vy) = (s.mario().pos.y, s.mario().vel.y);
        s.press_a();
        (s.mario().status, y, vy, s.mario().vel.y, s)
    };
    let (status, _, falling, vy, s) = dive(1);
    assert_eq!(status, Status::Jump);
    assert!(
        vy > 0.0 && vy > falling,
        "rises out of the dive: {falling} -> {vy}"
    );
    assert_eq!(s.event_count(|e| matches!(e, Event::AirJump)), 1);
    // None on hand: the dive goes on.
    let (status, ..) = dive(0);
    assert_eq!(status, Status::JumpCatch);
}

#[test]
fn no_air_jumps_by_default() {
    let mut s = Scene::flat();
    s.press_a();
    s.step(20);
    s.press_a();
    assert_eq!(s.mario().status, Status::Jump);
    assert_eq!(s.event_count(|e| matches!(e, Event::AirJump)), 0);
}

#[test]
fn spray_rate_fires_more_drops_for_the_same_water() {
    if need_disc() {
        return;
    }
    let run = |rate: f32| {
        let mut s = Scene::flat();
        s.sim.mario.limits.spray_rate = rate;
        s.sim.mario.give_fludd();
        let full = s.mario().fludd.as_ref().unwrap().water;
        s.pad.r_analog = 200;
        s.secs(2.0);
        (emits(&s), full - s.mario().fludd.as_ref().unwrap().water)
    };
    let (n1, w1) = run(1.0);
    let (n2, w2) = run(2.0);
    println!("spray: rate 1 {n1} drops {w1} water, rate 2 {n2} drops {w2} water");
    assert!(n2 as f32 > 1.8 * n1 as f32, "{n1} {n2}");
    assert!(
        (w2 - w1).abs() as f32 <= 0.05 * w1 as f32 + 2.0,
        "{w1} {w2}"
    );
}

#[test]
fn spray_stops_at_its_limit_until_r_is_let_go() {
    let mut s = Scene::flat();
    s.sim.mario.limits.spray_ticks = TICK_HZ;
    s.sim.mario.give_fludd();
    s.pad.r_analog = 200;
    s.secs(1.2);
    let at_limit = emits(&s);
    assert!(at_limit > 0);
    s.secs(1.0);
    assert_eq!(emits(&s), at_limit, "kept spraying past the limit");
    s.pad.r_analog = 0;
    s.step(2);
    s.pad.r_analog = 200;
    s.secs(0.5);
    assert!(emits(&s) > at_limit);
}

fn hover_ticks(hover_time: f32) -> u32 {
    let mut s = Scene::flat();
    s.sim.mario.limits.hover_time = hover_time;
    s.sim.mario.give_fludd();
    {
        let p = s.sim.mario.params.clone();
        let f = s.sim.mario.fludd.as_mut().unwrap();
        f.change_nozzle(NozzleKind::Hover, false, &p);
        f.refill(&p);
    }
    s.press_a();
    s.step(20);
    s.pad.r_analog = 255;
    let mut ticks = 0;
    for _ in 0..(10 * TICK_HZ) {
        s.step(1);
        if s.mario().status == Status::Rocket {
            ticks += 1;
        }
    }
    ticks
}

#[test]
fn hover_time_stretches_the_hover() {
    if need_disc() {
        return;
    }
    let (one, two) = (hover_ticks(1.0), hover_ticks(2.0));
    println!("hover: {one} ticks, x2 {two}");
    assert!(two as f32 > 1.6 * one as f32, "{one} {two}");
}

fn slide_length(limits: libsms::Limits, wet: bool) -> (f32, usize) {
    let mut s = Scene::flat();
    s.sim.mario.limits = limits;
    s.run_forward();
    s.secs(1.0);
    s.press_a();
    s.step(10);
    s.pad.b = true;
    s.step(1);
    s.pad.b = false;
    s.pad.stick = Vec2::ZERO;
    assert!(s.until(600, |m| m.status == Status::Catch));
    let from = s.mario().pos;
    s.sim.mario.water_underfoot = wet;
    assert!(s.until(4000, |m| m.status != Status::Catch));
    (
        (s.mario().pos - from).length(),
        s.event_count(|e| matches!(e, Event::WetSlide)),
    )
}

#[test]
fn a_wet_slide_charge_makes_any_slide_glide_once() {
    if need_disc() {
        return;
    }
    use libsms::{Limits, WetSlide};
    let charged = |ready, anywhere, max_ticks| Limits {
        wet_slide: Some(WetSlide {
            ready,
            anywhere,
            max_ticks,
            water_free: false,
            glide: false,
        }),
        ..Limits::default()
    };
    let dry = slide_length(Limits::default(), false).0;
    let game_wet = slide_length(Limits::default(), true).0;
    let (anywhere, spent) = slide_length(charged(true, true, 0), false);
    let (no_charge, none) = slide_length(charged(false, true, 0), true);
    let (needs_water, _) = slide_length(charged(true, false, 0), false);
    let (capped, _) = slide_length(charged(true, true, 60), false);
    println!(
        "slides: dry {dry:.0}, game wet {game_wet:.0}, charged {anywhere:.0}, no charge on water \
         {no_charge:.0}, charged dry needing water {needs_water:.0}, capped {capped:.0}"
    );
    assert_eq!((spent, none), (1, 0));
    assert!(anywhere > 1.6 * dry, "{dry} {anywhere}");
    assert!((no_charge - dry).abs() < 0.05 * dry, "{dry} {no_charge}");
    assert!(
        (needs_water - dry).abs() < 0.05 * dry,
        "{dry} {needs_water}"
    );
    assert!(
        capped > dry && capped < anywhere,
        "{dry} {capped} {anywhere}"
    );
}

#[test]
fn timed_wet_slides_use_actual_surface_wetness_and_go_on_while_it_stays_wet() {
    if need_disc() {
        return;
    }
    struct Floor(bool);
    impl Host for Floor {
        fn ground(&self, pos: Vec3) -> Option<libsms::GroundHit> {
            let mut g = FlatHost::default().ground(pos)?;
            if self.0 {
                g.flags = g.flags.with(libsms::SurfaceFlags::WET);
            }
            Some(g)
        }
    }
    for wet in [false, true] {
        let mut s = Scene::new(Floor(wet));
        s.sim.mario.limits.wet_slide = Some(libsms::WetSlide::timed(60, true));
        s.run_forward();
        s.step(120);
        s.press_a();
        s.step(10);
        s.pad.b = true;
        s.step(1);
        s.pad.b = false;
        assert!(s.until(600, |m| m.status == Status::Catch));
        s.step(1);
        assert_eq!(s.mario().wet_sliding(), wet);
        if wet {
            // Over wet terrain the time never runs down: well past 60 ticks he is still gliding, wet.
            for tick in 0..150 {
                s.sim.mario.limits.wet_slide = Some(libsms::WetSlide::timed(60, true));
                s.step(1);
                if s.mario().status != Status::Catch {
                    assert!(tick > 60, "the slide ended at {tick}, inside the cap");
                    break;
                }
                assert!(s.mario().wet_sliding(), "a slide over wet terrain turned plain at {tick}");
            }
        } else {
            // Dry ground: it never turned wet in the first place (the assert above).
        }
    }
}

#[test]
fn a_pound_reports_its_start() {
    let mut s = Scene::flat();
    s.pad.a = true;
    s.press_a();
    s.step(40);
    s.pad.l = true;
    s.step(1);
    s.pad.l = false;
    assert_eq!(s.event_count(|e| matches!(e, Event::PoundStart)), 1);
    assert!(s.until(1500, |m| !m.status.is_jumping()));
    assert_eq!(s.event_count(|e| matches!(e, Event::PoundStart)), 1);
}

#[test]
fn a_wet_mario_can_start_a_wet_slide_on_dry_ground_until_he_dries() {
    for (soaked, dry_first) in [(false, false), (true, false), (true, true)] {
        // A genuinely dry floor: no stage water, WET flag, or FLUDD ground patches.
        let mut s = Scene::flat();
        s.sim.mario.limits.wet_slide = Some(libsms::WetSlide::timed(60, true));
        s.run_forward();
        s.step(120);
        if soaked {
            s.sim.mario.wet();
            if dry_first { s.step(120); }
        }
        s.press_a();
        s.step(10);
        s.pad.b = true;
        s.step(1);
        s.pad.b = false;
        assert!(s.until(600, |m| m.status == Status::Catch));
        s.step(1);
        assert_eq!(s.mario().wet_sliding(), soaked && !dry_first,
            "dry-ground slide: soaked={soaked}, dried before takeoff={dry_first}");
    }
}

#[test]
fn a_launch_throws_mario_and_he_lands() {
    let mut s = Scene::flat();
    s.sim.mario.face_yaw = 0;
    // Up and back (-Z, behind him): he flies backwards, still facing +Z.
    s.sim.mario.launch(Vec3::new(0.0, 60.0, -30.0));
    assert_eq!(s.mario().status, Status::ForceJump);
    assert!(s.mario().forward_vel < 0.0);
    let mut apex = 0.0f32;
    for _ in 0..600 {
        s.step(1);
        apex = apex.max(s.mario().pos.y);
        if !s.mario().status.is_jumping() {
            break;
        }
    }
    assert!(apex > 300.0, "apex {apex}");
    assert!(s.mario().pos.z < -100.0, "{:?}", s.mario().pos);
    assert!(!s.mario().status.is_jumping());
}

#[test]
fn host_jump_pad_preserves_ballistics_without_sunshine_speed_caps() {
    let mut s = Scene::flat();
    // 160 mVel exceeds Sunshine's horizontal cap. 0.5 gravity is deliberately
    // different from both the shipped tuning and code defaults.
    let velocity = Vec3::new(160.0, 100.0, 0.0);
    let gravity = 0.5;
    s.sim.mario.launch_pad(velocity, gravity);
    // The stick steers nothing in a pad flight (a move breaks it, see the tests below).
    s.pad.stick = Vec2::new(-1.0, 0.0);
    for tick in 1..=300 {
        s.step(1);
        let n = tick as f32;
        let expected = Vec3::new(
            velocity.x * 0.25 * n,
            0.25 * (velocity.y * n - 0.5 * gravity * n * n),
            0.0,
        );
        assert!(
            (s.mario().pos - expected).length() < 0.1,
            "tick {tick}: {:?}, expected {expected}",
            s.mario().pos
        );
    }
    assert!(s.until(150, |m| !m.status.is_jumping()));
    s.pad = Pad::default();
    s.run_forward();
    s.step(120);
    assert!(
        s.mario().vel.z > 0.0,
        "normal control returns after landing"
    );
}

/// How far (flat) a dive from a pad flight carries him before he lands, the pad throwing him with
/// `velocity`; B is pressed `after` ticks into the flight.
fn pad_dive_distance(velocity: Vec3, after: u32) -> (f32, Status) {
    let mut s = Scene::flat();
    s.sim.mario.launch_pad(velocity, 0.5);
    s.step(after);
    let from = s.mario().pos;
    press_b(&mut s);
    let status = s.mario().status;
    assert!(s.until(900, |m| !m.status.is_jumping()));
    let d = s.mario().pos - from;
    (Vec3::new(d.x, 0.0, d.z).length(), status)
}

#[test]
fn a_dive_breaks_a_jump_pad_flight_and_keeps_its_sideways_speed() {
    // The same height, thrown straight up or across: the dive out of the second goes further by
    // the pad's own momentum.
    let (up, status) = pad_dive_distance(Vec3::new(0.0, 100.0, 0.0), 30);
    assert_eq!(status, Status::JumpCatch);
    let (across, status) = pad_dive_distance(Vec3::new(60.0, 100.0, 0.0), 30);
    assert_eq!(status, Status::JumpCatch);
    assert!(across > up + 150.0, "straight up {up}, across {across}");
}

#[test]
fn a_move_waits_for_the_pad_to_throw_him_before_it_breaks_the_flight() {
    let mut s = Scene::flat();
    s.sim.mario.launch_pad(Vec3::new(60.0, 100.0, 0.0), 0.5);
    // B pressed in the launch's first moments doesn't break the flight...
    press_b(&mut s);
    s.step(4);
    assert_eq!(s.mario().status, Status::ForceJump);
    // ...but a press once it has got going does.
    s.step(14);
    press_b(&mut s);
    assert_eq!(s.mario().status, Status::JumpCatch);
}

#[test]
fn the_rocket_and_the_turbo_break_a_pad_flight() {
    if need_disc() {
        return;
    }
    let rocket = |nozzle: libsms::params::NozzleKind| {
        let mut s = Scene::flat();
        s.sim.mario.give_fludd();
        let params = s.sim.mario.params.clone();
        s.sim.mario.fludd.as_mut().unwrap().change_nozzle(nozzle, false, &params);
        s.sim.mario.fludd.as_mut().unwrap().refill(&params);
        s.sim.mario.launch_pad(Vec3::new(60.0, 100.0, 0.0), 0.5);
        s.step(30);
        // Steering does nothing in the flight; it does once a move has broken it.
        s.pad.stick = Vec2::new(0.0, 1.0);
        s.pad.r = true;
        s.pad.r_analog = 255;
        // The Rocket takes its charge time in the air too; the Turbo goes at once.
        if nozzle == libsms::params::NozzleKind::Rocket {
            s.until(300, |m| m.status == Status::Rocket);
        } else {
            s.step(10);
        }
        (s.mario().status, s.mario().vel.z)
    };
    let (status, _) = rocket(libsms::params::NozzleKind::Rocket);
    assert_eq!(status, Status::Rocket);
    let (status, sideways) = rocket(libsms::params::NozzleKind::Turbo);
    assert_eq!(status, Status::ForceJump);
    assert!(sideways.abs() > 0.5, "the Turbo left the flight unsteered: {sideways}");
}

#[test]
fn a_ground_pound_breaks_a_pad_flight_too() {
    let mut s = Scene::flat();
    s.sim.mario.launch_pad(Vec3::new(60.0, 100.0, 0.0), 0.5);
    s.step(30);
    s.pad.l = true;
    s.pad.l_analog = 255;
    s.pad.b = true;
    s.step(2);
    assert_eq!(s.mario().status, Status::HipDrop);
}

#[test]
fn a_jump_boost_carries_further_until_landing() {
    let run = |boost: f32| {
        let mut s = Scene::flat();
        s.run_forward();
        s.secs(1.0);
        s.press_a();
        s.sim.mario.boost_jump(boost);
        let from = s.mario().pos.z;
        assert!(s.until(600, |m| !m.status.is_jumping()));
        let air = s.mario().pos.z - from;
        // Gone once he's down.
        let z = s.mario().pos.z;
        s.pad.stick = Vec2::ZERO;
        s.step(1);
        (air, s.mario().pos.z - z)
    };
    let (plain, _) = run(1.0);
    let (boosted, after) = run(2.0);
    assert!(boosted > 1.8 * plain, "{plain} {boosted}");
    assert!(after < 40.0, "{after}");
}

#[test]
fn add_water_tops_up_the_tank() {
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    let p = s.sim.mario.params.clone();
    s.sim.mario.fludd.as_mut().unwrap().water = 0;
    s.sim.mario.add_water(0.25);
    let f = s.mario().fludd.as_ref().unwrap();
    let max = f.max_water(&p);
    assert!((f.water - max / 4).abs() <= 1, "{} of {max}", f.water);
    s.sim.mario.add_water(5.0);
    assert_eq!(s.mario().fludd.as_ref().unwrap().water, max);
}

// ---- light-press bursts, glides, the L-button camera, knock-outs ---------------------------

/// Spray presses at raw R `raw` for `hold` ticks: (ticks that emitted, farthest landing from
/// Mario), the water flown through libsms's own particle manager.
fn spray_burst(raw: u8, hold: u32, limits: libsms::Limits) -> (u32, f32) {
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    s.sim.mario.limits = limits;
    s.secs(0.5);
    let (params, _) = params();
    let mut w = libsms::water::WaterManager::new(&params);
    let (mut emitting, mut far) = (0, 0.0f32);
    for t in 0..(hold + 600) {
        s.pad.r_analog = if t < hold { raw } else { 0 };
        s.host.events.clear();
        s.step(1);
        let mut any = false;
        for e in &s.host.events {
            if let Event::WaterEmit(we) = e {
                w.emit(we);
                any = true;
            }
        }
        emitting += any as u32;
        w.step(&s.host.world, s.mario().pos);
        for p in w.particles.iter_mut() {
            if p.state == libsms::water::State::Ground && p.life > 0.0 && p.size > 0.0 {
                far = far.max(Vec2::new(p.pos.x, p.pos.z).length());
                p.life = 0.0;
            }
        }
    }
    (emitting, far)
}

#[test]
fn a_light_press_is_a_short_burst_and_a_held_one_fades() {
    if need_disc() {
        return;
    }
    let none = libsms::Limits::default();
    let light = spray_burst(15, 1200, none);
    let full = spray_burst(255, 1200, none);
    let stretched = spray_burst(
        255,
        2400,
        libsms::Limits {
            spray_fade: 2.0,
            ..none
        },
    );
    println!("light {light:?}, full {full:?}, fade x2 {stretched:?}");
    // A light press: a quarter second of water that lands a few metres out.
    assert!(light.0 < 60, "{light:?}");
    assert!(light.1 < 0.4 * full.1, "{light:?} {full:?}");
    // A held full press stops by itself, after about 5 s.
    assert!(full.0 > 480 && full.0 < 800, "{full:?}");
    assert!(
        stretched.0 as f32 > 1.6 * full.0 as f32,
        "{stretched:?} {full:?}"
    );
}

/// Whether the Spray's drops at trigger `raw` say they're a light press's burst.
fn burst_drops(raw: u8) -> (u32, u32) {
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    s.secs(0.5);
    let (mut burst, mut plain) = (0, 0);
    for _ in 0..120 {
        s.pad.r_analog = raw;
        s.host.events.clear();
        s.step(1);
        for e in &s.host.events {
            if let Event::WaterEmit(we) = e {
                if we.burst {
                    burst += we.count;
                } else {
                    plain += we.count;
                }
            }
        }
    }
    (burst, plain)
}

#[test]
fn a_light_press_says_its_drops_are_a_burst() {
    if need_disc() {
        return;
    }
    let light = burst_drops(15);
    let full = burst_drops(255);
    println!("light {light:?} full {full:?}");
    assert!(light.0 > 0 && light.1 == 0, "{light:?}");
    assert!(full.0 == 0 && full.1 > 0, "{full:?}");
}

fn charged(water_free: bool, glide: bool) -> libsms::Limits {
    libsms::Limits {
        wet_slide: Some(libsms::WetSlide {
            ready: true,
            anywhere: true,
            max_ticks: 180,
            water_free,
            glide,
        }),
        ..libsms::Limits::default()
    }
}

#[test]
fn a_slide_over_fludd_water_is_wet_for_free_and_a_glide_carries_on() {
    let (dry, _) = slide_length(libsms::Limits::default(), false);
    let (on_water, spent) = slide_length(charged(true, false), true);
    let (charged_plain, _) = slide_length(charged(false, false), false);
    let (glide, _) = slide_length(charged(false, true), false);
    println!(
        "dry {dry:.0}, on water {on_water:.0} ({spent} spent), charged {charged_plain:.0}, glide {glide:.0}"
    );
    assert_eq!(spent, 0, "water's free");
    assert!(on_water > 1.6 * dry);
    assert!(glide > 3.0 * charged_plain, "{charged_plain} {glide}");
}

#[test]
fn a_host_aim_without_l_turns_a_standing_mario_and_pitches_the_nozzle() {
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    s.sim.mario.face_yaw = 0;
    s.sim.mario.aim = Some(libsms::Aim {
        yaw: 0x4000,
        pitch: 0x1000,
    });
    s.step(60);
    let m = s.mario();
    assert_ne!(m.status, Status::Squat, "no L, no crouch");
    assert!(
        (m.face_yaw as i32 - 0x4000).abs() < 0x100,
        "{:#x}",
        m.face_yaw
    );
    // The neck's angle counts the other way (negative lifts the barrel), a little more for its droop.
    let gun = m.fludd.as_ref().unwrap().gun_angle();
    assert!(
        (-0x1000 as f32 - 2000.0..-0x1000 as f32).contains(&gun),
        "{gun}"
    );
}

#[test]
fn a_host_aim_sends_the_water_its_way_even_while_mario_runs_elsewhere() {
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    s.sim.mario.aim = Some(libsms::Aim {
        yaw: 0x4000,
        pitch: 0,
    });
    // Running forward (+Z of his start facing) while the aim is a quarter turn off.
    s.pad.stick = Vec2::new(0.0, 1.0);
    s.pad.r_analog = 140;
    let mut dir = None;
    for _ in 0..120 {
        s.host.events.clear();
        s.step(1);
        for e in &s.host.events {
            if let Event::WaterEmit(we) = e {
                dir.get_or_insert(we.dir);
            }
        }
    }
    let d = dir.expect("sprayed");
    // Yaw 0x4000 faces +X (forward is (sin, cos)).
    assert!(d.x > 0.7 && d.z.abs() < 0.5, "{d:?}");
}

#[test]
fn a_host_aim_pitches_the_water_as_it_points() {
    let rise = |pitch: i16| {
        let mut s = Scene::flat();
        s.sim.mario.give_fludd();
        s.sim.mario.aim = Some(libsms::Aim { yaw: 0, pitch });
        s.pad.r_analog = 140;
        let mut dir = None;
        for _ in 0..120 {
            s.host.events.clear();
            s.step(1);
            for e in &s.host.events {
                if let Event::WaterEmit(we) = e {
                    dir.get_or_insert(we.dir);
                }
            }
        }
        dir.expect("sprayed")
    };
    // A quarter of the way up (45 degrees) and down, and level: exactly where it points.
    let (up, down, level) = (rise(0x2000), rise(-0x2000), rise(0));
    println!("up {up:?} down {down:?} level {level:?}");
    assert!((up.y - 0.707).abs() < 0.02 && up.z > 0.6, "{up:?}");
    assert!((down.y + 0.707).abs() < 0.02 && down.z > 0.6, "{down:?}");
    assert!(level.y.abs() < 0.02 && level.z > 0.99, "{level:?}");
}

#[test]
fn a_knock_out_lays_mario_down() {
    let mut s = Scene::flat();
    s.sim.mario.knock_out();
    s.step(240);
    assert_eq!(s.mario().status, Status::DownLoser);
    assert_eq!(s.event_count(|e| matches!(e, Event::Down)), 1);
}

#[test]
fn a_shine_touched_on_the_ground_starts_the_pose_at_once() {
    let mut s = Scene::flat();
    s.sim.mario.health = 3;
    assert!(s.sim.mario.get_shine(Vec3::new(0.0, 50.0, 0.0), 0x2000));
    s.step(2);
    let m = s.mario();
    assert_eq!(m.status, Status::WinDemo);
    assert!(m.win_demo_posing());
    assert_eq!(m.face_yaw, 0x2000);
    assert_eq!(m.health, m.params.mario.hp_max, "a Shine refills his health");
    assert_eq!(s.event_count(|e| matches!(e, Event::ShineTouch { .. })), 1);
    assert_eq!(s.event_count(|e| matches!(e, Event::ShineDemo { .. })), 1);
    assert_eq!(sound_count(&s, "MSD_SE_SY_GET_SHINE"), 1);
    assert_eq!(m.anim.clip, "demo_shine_get");
}

#[test]
fn a_shine_touched_in_the_air_lets_him_land_before_the_demo() {
    let mut s = Scene::flat();
    s.press_a();
    assert!(s.until(600, |m| m.pos.y > 40.0 && m.vel.y < 5.0), "near the top of the jump");
    assert!(s.sim.mario.get_shine(s.sim.mario.pos, 0));
    s.step(1);
    assert!(s.mario().is_win_demo());
    assert!(!s.mario().win_demo_posing(), "still falling");
    assert_eq!(s.event_count(|e| matches!(e, Event::ShineDemo { .. })), 0);
    assert!(s.until(600, |m| m.win_demo_posing()));
    assert!(s.mario().pos.y.abs() < 1.0, "landed: {:?}", s.mario().pos);
    assert!(s.mario().speed().abs() < 0.01);
    assert_eq!(s.event_count(|e| matches!(e, Event::ShineDemo { .. })), 1);
}

#[test]
fn the_shine_pose_holds_still_and_is_not_interrupted() {
    let mut s = Scene::flat();
    s.sim.mario.get_shine(Vec3::ZERO, 0);
    s.pad.stick = Vec2::new(0.0, 1.0);
    s.pad.a = true;
    s.secs(2.0);
    let m = s.mario();
    assert_eq!(m.status, Status::WinDemo, "the sticks do nothing");
    assert!(m.pos.length() < 1.0, "{:?}", m.pos);
    let hp = m.health;
    assert!(!s.sim.mario.damage(Vec3::new(100.0, 0.0, 0.0), libsms::DamageKind::EnemyCommon));
    assert_eq!(s.mario().health, hp);
    // A second touch (the Shine is gone) changes nothing.
    assert!(!s.sim.mario.get_shine(Vec3::ZERO, 0x4000));
    assert_eq!(s.event_count(|e| matches!(e, Event::ShineDemo { .. })), 1);
}

#[test]
fn ending_the_shine_demo_lets_him_stand_again() {
    let mut s = Scene::flat();
    s.sim.mario.get_shine(Vec3::ZERO, 0);
    s.secs(1.0);
    s.sim.mario.end_win_demo();
    s.pad = Pad::default();
    s.secs(1.0);
    assert_eq!(s.mario().status, Status::Wait);
    s.run_forward();
    s.secs(1.0);
    assert_eq!(s.mario().status, Status::Run);
}

#[test]
fn a_mario_who_is_down_cannot_get_a_shine() {
    let mut s = Scene::flat();
    s.sim.mario.knock_out();
    assert!(!s.sim.mario.get_shine(Vec3::ZERO, 0));
    assert_eq!(s.mario().status, Status::DownLoser);
}

fn dive_slide(shallow_wets: bool, water: Option<f32>) -> f32 {
    let mut s = Scene::flat();
    s.sim.mario.options.shallow_water_wets = shallow_wets;
    s.host.world.water = water;
    s.run_forward();
    s.secs(1.0);
    s.press_a();
    s.step(10);
    s.pad.b = true;
    s.step(1);
    s.pad.b = false;
    s.pad.stick = Vec2::ZERO;
    assert!(s.until(600, |m| m.status == Status::Catch));
    let from = s.mario().pos;
    assert!(s.until(4000, |m| m.status != Status::Catch));
    (s.mario().pos - from).length()
}

#[test]
fn shallow_water_wets_a_slide_only_when_the_host_asks() {
    if need_disc() {
        return;
    }
    let dry = dive_slide(false, None);
    let off = dive_slide(false, Some(15.0));
    let on = dive_slide(true, Some(15.0));
    println!("dry {dry:.0}, in water {off:.0}, in water and wet {on:.0}");
    assert!(
        off < 0.5 * dry,
        "water alone drags, as in the game: {dry} {off}"
    );
    assert!(on > 1.6 * dry, "{dry} {on}");
}

#[test]
fn the_slide_spray_is_as_big_as_the_host_says() {
    let size = |scale: f32| {
        let mut s = Scene::flat();
        s.sim.mario.options.slide_fx_scale = scale;
        s.sim.mario.water_underfoot = true;
        s.run_forward();
        s.secs(1.0);
        s.press_a();
        s.step(10);
        s.pad.b = true;
        s.step(1);
        s.pad.b = false;
        s.pad.stick = Vec2::ZERO;
        s.sim.mario.water_underfoot = true;
        s.step(240);
        s.host
            .events
            .iter()
            .filter_map(|e| match e {
                Event::Particle(c) if c.name == "ms_m_watslide_a" => Some(c.scale),
                _ => None,
            })
            .fold(0.0f32, f32::max)
    };
    assert!((size(1.0) - 1.0).abs() < 1e-4);
    assert!((size(1.5) - 1.5).abs() < 1e-4);
}

/// Fires `nozzle` from a standing start (a jump first, for the Hover) for a few seconds and returns
/// every event it raised.
fn nozzle_events(nozzle: NozzleKind) -> Vec<Event> {
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    {
        let p = s.sim.mario.params.clone();
        let f = s.sim.mario.fludd.as_mut().unwrap();
        f.change_nozzle(nozzle, false, &p);
        f.refill(&p);
    }
    if nozzle == NozzleKind::Hover {
        s.press_a();
        s.step(20);
    }
    s.pad.r = true;
    s.pad.r_analog = 255;
    s.secs(6.0);
    s.host.events.clone()
}

fn cues(events: &[Event], name: &str) -> Vec<libsms::FxAt> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Particle(c) if c.name == name => Some(c.at),
            _ => None,
        })
        .collect()
}

fn voices(events: &[Event], name: &str) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, Event::Sound(c) if c.name == name))
        .count()
}

#[test]
fn the_rocket_launches_with_its_voice_and_burst_and_exhaust() {
    if need_disc() {
        return;
    }
    let e = nozzle_events(NozzleKind::Rocket);
    assert!(voices(&e, "MSD_SE_MV24_JUMP_SPECIAL_01") >= 1, "no launch voice");
    for burst in ["ms_m_rocket_d", "ms_m_rocket_c", "ms_hipdrop_a"] {
        let at = cues(&e, burst);
        assert_eq!(at.len(), 1, "{burst} should burst once at the launch: {at:?}");
        assert!(matches!(at[0], libsms::FxAt::Once { .. }), "{burst} is bound");
    }
    for exhaust in ["ms_m_rocket_a", "ms_m_rocket_b", "ms_m_rocket_b2"] {
        let at = cues(&e, exhaust);
        assert_eq!(at, vec![libsms::FxAt::MuzzleFollow(0)], "{exhaust} should start once, bound to the muzzle");
    }
    assert!(!cues(&e, "ms_nozzlesmoke").is_empty(), "no nozzle smoke while it fires");
}

#[test]
fn the_turbo_and_the_hover_start_with_their_own_voices_and_no_rocket_effects() {
    if need_disc() {
        return;
    }
    // The Turbo's start voice is the Rocket status's too, which the Turbo never enters (its
    // `mRocketType` isn't 1), so only the Hover has one.
    for (nozzle, voice, muzzles) in [
        (NozzleKind::Turbo, "", 1),
        (NozzleKind::Hover, "MSD_SE_MV21_JUMP_SMALL_01", 2),
    ] {
        let e = nozzle_events(nozzle);
        assert!(voice.is_empty() || voices(&e, voice) >= 1, "{nozzle:?} has no start voice {voice}");
        assert!(cues(&e, "ms_m_rocket_c").is_empty() && cues(&e, "ms_m_rocket_a").is_empty(), "{nozzle:?} got the Rocket's effects");
        let smoke = cues(&e, "ms_nozzlesmoke");
        for m in 0..muzzles {
            assert!(smoke.contains(&libsms::FxAt::Muzzle(m)), "{nozzle:?} has no smoke on muzzle {m}");
        }
        assert!(!smoke.contains(&libsms::FxAt::Muzzle(muzzles)), "{nozzle:?} smokes a muzzle it hasn't");
    }
}

#[test]
fn the_spray_smokes_its_muzzle_only_while_water_leaves() {
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    s.secs(0.5);
    assert!(cues(&s.host.events, "ms_nozzlesmoke").is_empty(), "smoke with no spray");
    s.pad.r_analog = 200;
    s.secs(1.0);
    assert!(cues(&s.host.events, "ms_nozzlesmoke").len() > 20, "spraying makes no smoke");
    s.host.events.clear();
    s.pad.r_analog = 0;
    s.secs(1.0);
    assert!(cues(&s.host.events, "ms_nozzlesmoke").len() < 8, "smoke carries on after the spray stops");
}

/// The Turbo's dash (`checkController`, `TMarioEffect::startDashEffect`): once it is charged Mario
/// starts `MSD_SE_PO_SNIPER_TRIGGER` on himself, then `MSD_SE_PO_SPREAD` every tick of the dash.
#[test]
fn the_turbo_dash_has_its_start_sound_and_its_spread_sound_every_tick() {
    if need_disc() {
        return;
    }
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    {
        let p = s.sim.mario.params.clone();
        let f = s.sim.mario.fludd.as_mut().unwrap();
        f.change_nozzle(NozzleKind::Turbo, false, &p);
        f.refill(&p);
    }
    s.pad.stick = Vec2::new(0.0, 1.0);
    s.pad.r_analog = 255;
    s.secs(6.0);
    let dash_starts = voices(&s.host.events, "MSD_SE_PO_SNIPER_TRIGGER");
    let spreads = voices(&s.host.events, "MSD_SE_PO_SPREAD");
    assert!(s.sim.mario.dashing(), "he never dashed: {:?}", s.sim.mario.status);
    assert!(dash_starts >= 2, "the fire sound and the dash start sound should both play: {dash_starts}");
    assert!(spreads > 60, "the dash has no spread sound: {spreads}");
}

/// A Turbo dash jump (`broadJumping`) keeps its dash flag until he lands, and takes no dive. Once the
/// Turbo is let go in the air the dash is over, and B dives like out of any other nozzle's jump.
#[test]
fn letting_go_of_the_turbo_in_a_dash_jump_ends_the_dash_and_allows_a_dive() {
    if need_disc() {
        return;
    }
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    {
        let p = s.sim.mario.params.clone();
        let f = s.sim.mario.fludd.as_mut().unwrap();
        f.change_nozzle(NozzleKind::Turbo, false, &p);
        f.refill(&p);
    }
    s.pad.stick = Vec2::new(0.0, 1.0);
    s.pad.r_analog = 255;
    s.secs(6.0);
    assert!(s.sim.mario.dashing(), "he never dashed: {:?}", s.sim.mario.status);
    s.pad.a = true;
    s.step(30);
    assert_eq!(s.sim.mario.status, Status::BroadJump, "the dash didn't jump");
    assert!(s.sim.mario.dashing());
    // Held, he can't dive.
    s.pad.b = true;
    s.step(2);
    s.pad.b = false;
    s.step(2);
    assert_eq!(s.sim.mario.status, Status::BroadJump, "dived with the Turbo held");
    // Let go: the dash ends, and B dives.
    s.pad.r_analog = 0;
    s.step(4);
    assert!(!s.sim.mario.dashing(), "the dash outlives the trigger in the air");
    assert_eq!(s.sim.mario.status, Status::BroadJump, "landed too soon for the test");
    s.pad.b = true;
    s.step(2);
    assert_eq!(s.sim.mario.status, Status::JumpCatch, "he can't dive after letting go of the Turbo");
}

/// A full stick circle out of a Turbo dash jump is the corkscrew dive: head first, forward, rolling about
/// its own length and then upright in the plain dive (host extension).
#[test]
fn a_stick_circle_in_a_turbo_dash_jump_is_a_corkscrew_dive() {
    if need_disc() {
        return;
    }
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    {
        let p = s.sim.mario.params.clone();
        let f = s.sim.mario.fludd.as_mut().unwrap();
        f.change_nozzle(NozzleKind::Turbo, false, &p);
        f.refill(&p);
    }
    s.pad.stick = Vec2::new(0.0, 1.0);
    s.pad.r_analog = 255;
    s.secs(6.0);
    s.pad.a = true;
    s.step(30);
    assert_eq!(s.sim.mario.status, Status::BroadJump, "the dash didn't jump");
    let before = s.sim.mario.forward_vel;
    for i in 0..24 {
        let a = i as f32 * std::f32::consts::TAU / 12.0;
        s.pad.stick = Vec2::new(a.cos(), a.sin());
        s.step(1);
        if s.sim.mario.status == Status::JumpCatch {
            break;
        }
    }
    assert_eq!(s.sim.mario.status, Status::JumpCatch, "the circle didn't start the dive");
    // On top of the Turbo's own top speed (32), not instead of it, and held while it spins.
    assert!(s.sim.mario.forward_vel > 80.0, "no push forward: {} (was {before})", s.sim.mario.forward_vel);
    // It goes on at the same height for as long as the stick keeps circling, past the old 6 s cap (720 ticks).
    let y0 = s.sim.mario.pos.y;
    let (mut rolled, mut slowest) = (false, f32::MAX);
    let mut angle = 0.0f32;
    for i in 0..1000 {
        angle += std::f32::consts::TAU / 12.0;
        s.pad.stick = Vec2::new(angle.cos(), angle.sin());
        s.step(1);
        assert_eq!(s.sim.mario.status, Status::JumpCatch, "it ended after {i} ticks of circling");
        assert_eq!(s.sim.mario.model_pitch, 0x4000, "it stopped being the corkscrew after {i} ticks of circling");
        assert!((s.sim.mario.pos.y - y0).abs() < 8.0, "it left its height: {} -> {}", y0, s.sim.mario.pos.y);
        slowest = slowest.min(s.sim.mario.forward_vel);
        rolled |= s.sim.mario.model_roll != 0 && s.sim.mario.anim.clip == libsms::anim::clip::SPIN;
    }
    assert!(rolled, "it never rolled");
    assert!(slowest > 80.0, "it slowed to {slowest} while spinning");
    // Let go of the stick: the dive follows.
    s.pad.stick = Vec2::ZERO;
    let mut ended = None;
    for i in 0..120 {
        s.step(1);
        if s.sim.mario.model_pitch != 0x4000 {
            ended = Some(i);
            break;
        }
    }
    let ended = ended.expect("it never ended once the stick stopped");
    assert!(ended < 80, "it took {ended} ticks to dive after the stick stopped");
    if s.sim.mario.status == Status::JumpCatch {
        assert_eq!(s.sim.mario.model_roll, 0, "it should end upright in the plain dive");
        assert_eq!(s.sim.mario.model_pitch, 0);
    }
    // The dive lands as a wet belly slide.
    for _ in 0..600 {
        if s.sim.mario.status != Status::JumpCatch {
            break;
        }
        s.step(1);
    }
    s.step(2);
    assert_eq!(s.sim.mario.status, Status::Catch, "it didn't land in the slide");
    assert!(s.sim.mario.wet_sliding(), "the slide after a corkscrew isn't wet");
}

/// A stick circle in any jump with the Turbo held is the corkscrew, never a plain spin jump with the
/// Turbo's water flying round him.
#[test]
fn a_stick_circle_in_a_plain_jump_with_the_turbo_held_is_the_corkscrew() {
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    {
        let p = s.sim.mario.params.clone();
        let f = s.sim.mario.fludd.as_mut().unwrap();
        f.change_nozzle(NozzleKind::Turbo, false, &p);
        f.refill(&p);
    }
    s.press_a();
    s.step(10);
    s.pad.r_analog = 255;
    s.step(10);
    for i in 0..24 {
        let a = i as f32 * std::f32::consts::TAU / 12.0;
        s.pad.stick = Vec2::new(a.cos(), a.sin());
        s.step(1);
        assert!(
            !matches!(s.sim.mario.status, Status::LeftRotateJump | Status::RightRotateJump),
            "a plain spin jump with the Turbo held"
        );
        if s.sim.mario.status == Status::JumpCatch {
            break;
        }
    }
    assert_eq!(s.sim.mario.status, Status::JumpCatch, "the circle didn't start the corkscrew");
    s.step(1);
    assert_eq!(s.sim.mario.model_pitch, 0x4000, "it isn't the corkscrew");
}

/// Host extension `Options::turbo_air_thrust`: a Turbo held in a standing jump pushes him forward
/// once its wind-up is full; with the flag off (the game's) it adds nothing.
fn turbo_air_speed(thrust: bool) -> f32 {
    let mut s = Scene::flat();
    s.sim.mario.options.turbo_air_thrust = thrust;
    s.sim.mario.give_fludd();
    {
        let p = s.sim.mario.params.clone();
        let f = s.sim.mario.fludd.as_mut().unwrap();
        f.change_nozzle(NozzleKind::Turbo, false, &p);
        f.refill(&p);
    }
    s.press_a();
    s.step(5);
    s.pad.r_analog = 255;
    // Peak speed while his feet are off the ground.
    let mut peak = 0.0f32;
    let mut aloft = 0;
    for _ in 0..60 {
        s.step(1);
        if s.sim.mario.pos.y > 1.0 {
            aloft += 1;
            peak = peak.max(s.sim.mario.forward_vel);
        }
    }
    assert!(aloft > 10, "never left the ground");
    peak
}

#[test]
fn the_held_turbo_thrusts_in_the_air_and_the_flag_switches_it_back() {
    let off = turbo_air_speed(false);
    let on = turbo_air_speed(true);
    assert!(on > off + 5.0, "no air thrust: on {on}, off {off}");
}

/// The stick turns Mario in the rocket's thrust and in the climb after it, not only pushes him along.
#[test]
fn the_stick_turns_him_after_the_rocket() {
    if need_disc() {
        return;
    }
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    {
        let p = s.sim.mario.params.clone();
        let f = s.sim.mario.fludd.as_mut().unwrap();
        f.change_nozzle(NozzleKind::Rocket, false, &p);
        f.refill(&p);
    }
    s.press_a();
    s.step(6);
    s.pad.r = true;
    s.pad.r_analog = 255;
    s.until(300, |m| m.status == Status::Rocket);
    let facing = s.sim.mario.face_yaw;
    // A quarter turn off from where he faces.
    s.pad.stick = Vec2::new(1.0, 0.0);
    s.step(20);
    assert_ne!(s.sim.mario.face_yaw, facing, "the stick didn't turn him in the rocket's flight");
}


/// A Turbo dash jump a full stick circle into the corkscrew, over a flat floor with `enemies`.
fn corkscrew_scene(enemies: Vec<(Vec3, f32, f32)>) -> Scene<Enemies> {
    let mut s = Scene::new(Enemies(enemies));
    s.sim.mario.give_fludd();
    {
        let p = s.sim.mario.params.clone();
        let f = s.sim.mario.fludd.as_mut().unwrap();
        f.change_nozzle(NozzleKind::Turbo, false, &p);
        f.refill(&p);
    }
    s.pad.stick = Vec2::new(0.0, 1.0);
    s.pad.r_analog = 255;
    s.secs(6.0);
    s.pad.a = true;
    s.step(30);
    for i in 0..24 {
        let a = i as f32 * std::f32::consts::TAU / 12.0;
        s.pad.stick = Vec2::new(a.cos(), a.sin());
        s.step(1);
        if s.sim.mario.status == Status::JumpCatch {
            break;
        }
    }
    s
}

/// Once the corkscrew has started, holding it only asks the stick to keep *turning*
/// (`Options::corkscrew_hold_degrees`, a host extension): a slow, half-tilt circle holds the
/// roll, where the game's whole-circle test would want a fast, full sweep every fifth of a
/// second.
#[test]
fn a_slow_gentle_circle_holds_the_corkscrew() {
    if need_disc() {
        return;
    }
    let mut s = corkscrew_scene(vec![]);
    assert_eq!(s.sim.mario.status, Status::JumpCatch, "the circle didn't start the dive");
    // Half the stick, 90 degrees every 12 ticks: a full circle takes 0.4 s (48 ticks), twice
    // the 0x18-sample window Sunshine's circle test looks at.
    let mut angle = std::f32::consts::TAU / 12.0 * 12.0; // where the fast start left it
    for i in 0..600 {
        angle += std::f32::consts::TAU / 48.0;
        s.pad.stick = Vec2::new(angle.cos(), angle.sin()) * 0.5;
        s.step(1);
        if s.sim.mario.status != Status::JumpCatch {
            panic!("the slow circle dropped the corkscrew after {i} ticks");
        }
        assert_eq!(s.sim.mario.model_pitch, 0x4000, "it stopped rolling after {i} ticks");
    }
    // Centering the stick ends it, as before.
    s.pad.stick = Vec2::ZERO;
    s.step(40);
    assert_ne!(s.sim.mario.model_pitch, 0x4000, "it kept rolling with no stick");
}

/// `CorkscrewHold = 360` is the strict reading: the stick must circle the whole way in one
/// grace window, so the same slow circle drops the roll.
#[test]
fn a_whole_circle_rule_drops_a_slow_corkscrew() {
    if need_disc() {
        return;
    }
    let mut s = corkscrew_scene(vec![]);
    s.sim.mario.options.corkscrew_hold_degrees = 360.0;
    assert_eq!(s.sim.mario.status, Status::JumpCatch);
    let mut angle = std::f32::consts::TAU;
    let mut ended = None;
    for i in 0..200 {
        angle += std::f32::consts::TAU / 48.0;
        s.pad.stick = Vec2::new(angle.cos(), angle.sin()) * 0.5;
        s.step(1);
        if s.sim.mario.model_pitch != 0x4000 {
            ended = Some(i);
            break;
        }
    }
    let ended = ended.expect("a slow circle held a whole-circle rule");
    assert!(ended < 60, "it took {ended} ticks to drop");
}

/// Switching to the Spray in a corkscrew turns the water with the roll: the spin's water, round the line of
/// flight and down, whatever the host aims at.
#[test]
fn the_spray_in_a_corkscrew_turns_with_the_roll_as_spin_water() {
    if need_disc() {
        return;
    }
    let mut s = corkscrew_scene(vec![]);
    assert_eq!(s.sim.mario.status, Status::JumpCatch);
    {
        let p = s.sim.mario.params.clone();
        let f = s.sim.mario.fludd.as_mut().unwrap();
        f.change_nozzle(NozzleKind::Spray, false, &p);
        f.refill(&p);
    }
    // The host's crosshair points straight up and forward: not where the water goes.
    s.sim.mario.aim = Some(libsms::Aim { yaw: s.sim.mario.face_yaw, pitch: 0x2000 });
    s.pad.r_analog = 255;
    let mut angle = 0.0f32;
    for _ in 0..80 {
        angle += std::f32::consts::TAU / 12.0;
        s.pad.stick = Vec2::new(angle.cos(), angle.sin());
        s.step(1);
        assert_eq!(s.sim.mario.status, Status::JumpCatch, "the corkscrew ended");
    }
    let drops: Vec<_> = s
        .host
        .events
        .iter()
        .filter_map(|e| match e {
            Event::WaterEmit(w) if w.nozzle == NozzleKind::Spray => Some((w.dir, w.spin)),
            _ => None,
        })
        .collect();
    assert!(drops.len() > 20, "the Spray put out {} bursts", drops.len());
    assert!(drops.iter().all(|&(_, spin)| spin), "some water wasn't the spin's");
    let (low, high) = drops.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &(d, _)| (lo.min(d.y), hi.max(d.y)));
    assert!(low < -0.9 && high > 0.9, "the water didn't go round: y from {low} to {high}");
    let sideways = drops.iter().any(|&(d, _)| d.x.abs() + d.z.abs() > 0.9);
    assert!(sideways, "the water never went out to the side");
}

/// A corkscrew that has spun for one second burns, and its one hit is the super blow, which bonks
/// Mario back over.
#[test]
fn a_corkscrew_spun_for_one_second_burns_and_its_hit_bonks_him() {
    if need_disc() {
        return;
    }
    let mut s = corkscrew_scene(vec![]);
    assert_eq!(s.sim.mario.status, Status::JumpCatch);
    let start = s.sim.mario.pos;
    let mut angle = 0.0f32;
    let mut spin = |s: &mut Scene<Enemies>| {
        angle += std::f32::consts::TAU / 12.0;
        s.pad.stick = Vec2::new(angle.cos(), angle.sin());
        s.step(1);
    };
    let mut flaming_at = None;
    for i in 0..700 {
        spin(&mut s);
        if i < 100 {
            assert!(!s.sim.mario.corkscrew_flaming(), "burning after only {i} ticks");
        }
        if s.sim.mario.corkscrew_flaming() {
            flaming_at = Some((i, s.sim.mario.pos));
            break;
        }
    }
    let (ticks, at) = flaming_at.expect("it never caught fire");
    assert!((110..=130).contains(&ticks), "it caught fire after {ticks} ticks (1 s is 120)");
    let ahead = Vec3::new(at.x - start.x, 0.0, at.z - start.z).normalize();
    // The same flight again, with an enemy in its path a little past where it catches fire.
    let enemy = at + ahead * 1500.0;
    let mut s = corkscrew_scene(vec![(enemy, 120.0, 400.0)]);
    let mut angle = 0.0f32;
    for _ in 0..800 {
        angle += std::f32::consts::TAU / 12.0;
        s.pad.stick = Vec2::new(angle.cos(), angle.sin());
        s.step(1);
        if s.sim.mario.status != Status::JumpCatch {
            break;
        }
    }
    assert_eq!(enemy_hits(&s, libsms::EnemyHitKind::SuperCorkscrew), 1, "no super blow: {:?}", s.sim.mario.status);
    assert_eq!(enemy_hits(&s, libsms::EnemyHitKind::Corkscrew), 0);
    assert_eq!(s.sim.mario.status, Status::JumpShortBackDown, "he wasn't bonked");
    assert!(s.sim.mario.forward_vel < 0.0, "he wasn't thrown back: {}", s.sim.mario.forward_vel);
}

/// Runs a Rocket jump, with a stick circle at the start of the climb if `spin`, over `enemies`.
fn rocket_climb(spin: bool, enemies: Vec<(Vec3, f32, f32)>) -> (Scene<Enemies>, f32, bool) {
    let mut s = Scene::new(Enemies(enemies));
    s.sim.mario.give_fludd();
    {
        let p = s.sim.mario.params.clone();
        let f = s.sim.mario.fludd.as_mut().unwrap();
        f.change_nozzle(NozzleKind::Rocket, false, &p);
        f.refill(&p);
    }
    s.press_a();
    s.step(6);
    s.pad.r = true;
    s.pad.r_analog = 255;
    s.until(300, |m| m.status == Status::Rocket);
    let y0 = s.mario().pos.y;
    let (mut apex, mut flamed) = (y0, false);
    let mut angle = 0.0f32;
    for i in 0..900 {
        if spin && i < 24 {
            angle += std::f32::consts::TAU / 12.0;
            s.pad.stick = Vec2::new(angle.cos(), angle.sin());
        } else {
            s.pad.stick = Vec2::ZERO;
        }
        s.step(1);
        apex = apex.max(s.mario().pos.y);
        flamed |= s.mario().flames() == 2;
        if i > 60 && s.mario().on_ground() {
            break;
        }
    }
    (s, apex - y0, flamed)
}

/// A stick circle in the Rocket's climb doubles the climb, burns while it rises, calls out once, and hits
/// what is above him.
#[test]
fn a_spin_out_of_the_rockets_climb_doubles_it_and_hits_what_is_above() {
    if need_disc() {
        return;
    }
    let call = |s: &Scene<Enemies>| {
        s.event_count(|e| matches!(e, Event::Sound(c) if c.name == "MSD_SE_MV24_JUMP_SPECIAL_01"))
    };
    let (s, plain, flamed) = rocket_climb(false, vec![]);
    assert!(!flamed);
    let launch_calls = call(&s);
    let (s, spun, flamed) = rocket_climb(true, vec![]);
    assert!(spun > plain * 1.8, "the spin climbed {spun}, a plain rocket {plain}");
    assert!(spun < plain * 2.4, "the spin climbed {spun}, more than double a plain rocket's {plain}");
    assert!(flamed, "it never burned while rising");
    assert_eq!(call(&s), launch_calls + 1, "the spin should call out once, besides the rocket's launch call");
    // An enemy overhead.
    let (s, _, _) = rocket_climb(true, vec![(Vec3::new(0.0, 2500.0, 0.0), 400.0, 300.0)]);
    assert_eq!(enemy_hits(&s, libsms::EnemyHitKind::RocketSpin), 1, "the enemy above wasn't hit");
    // A huge one around the whole climb (a boss he can sit inside): still one hit, not one per knock-back.
    let (s, _, _) = rocket_climb(true, vec![(Vec3::new(0.0, 100.0, 0.0), 3000.0, 6000.0)]);
    assert!(
        enemy_hits(&s, libsms::EnemyHitKind::RocketSpin) <= 1,
        "a spin inside a big enemy hit it more than once"
    );
}

/// A burning corkscrew lands in a slide that keeps its speed and its fire for two seconds, then is a plain wet slide.
#[test]
fn a_burning_corkscrew_lands_in_a_blazing_slide_for_two_seconds() {
    if need_disc() {
        return;
    }
    let mut s = corkscrew_scene(vec![]);
    let mut angle = 0.0f32;
    // Spin four seconds and a bit, then let go and land.
    for i in 0..1500 {
        if i < 500 {
            angle += std::f32::consts::TAU / 12.0;
            s.pad.stick = Vec2::new(angle.cos(), angle.sin());
        } else {
            s.pad.stick = Vec2::ZERO;
        }
        s.step(1);
        if s.sim.mario.status == Status::Catch {
            break;
        }
    }
    assert_eq!(s.sim.mario.status, Status::Catch, "it never landed in the slide");
    s.step(2);
    assert!(s.sim.mario.wet_sliding());
    assert_eq!(s.sim.mario.flames(), 1, "the slide isn't burning");
    s.step(200);
    assert_eq!(s.sim.mario.status, Status::Catch);
    assert_eq!(s.sim.mario.flames(), 1, "the fire went out early");
    assert!(s.sim.mario.forward_vel > 60.0, "the slide slowed to {}", s.sim.mario.forward_vel);
    s.step(60);
    assert_eq!(s.sim.mario.flames(), 0, "the fire never went out");
    assert!(s.sim.mario.wet_sliding() || s.sim.mario.status != Status::Catch);
}

/// The blazing slide hits like the corkscrew, not like a plain slide.
#[test]
fn the_blazing_slide_hits_as_a_corkscrew() {
    if need_disc() {
        return;
    }
    let mut s = corkscrew_scene(vec![]);
    let (mut angle, mut landed) = (0.0f32, None);
    for i in 0..1500 {
        if i < 500 {
            angle += std::f32::consts::TAU / 12.0;
            s.pad.stick = Vec2::new(angle.cos(), angle.sin());
        } else {
            s.pad.stick = Vec2::ZERO;
        }
        s.step(1);
        if s.sim.mario.status == Status::Catch {
            landed = Some(s.sim.mario.pos);
            break;
        }
    }
    let at = landed.expect("it never landed");
    // Replay with an enemy a little way ahead on the ground.
    let start = corkscrew_scene(vec![]).sim.mario.pos;
    let dir = Vec3::new(at.x - start.x, 0.0, at.z - start.z).normalize();
    let mut s = corkscrew_scene(vec![(at + dir * 600.0, 120.0, 300.0)]);
    let mut angle = 0.0f32;
    for i in 0..1500 {
        if i < 500 {
            angle += std::f32::consts::TAU / 12.0;
            s.pad.stick = Vec2::new(angle.cos(), angle.sin());
        } else {
            s.pad.stick = Vec2::ZERO;
        }
        s.step(1);
        if i > 500 && s.sim.mario.status != Status::Catch && s.sim.mario.status != Status::JumpCatch {
            break;
        }
    }
    assert_eq!(enemy_hits(&s, libsms::EnemyHitKind::Corkscrew), 1, "the slide didn't hit as a corkscrew");
    assert_eq!(enemy_hits(&s, libsms::EnemyHitKind::Punch), 0);
}

/// A full stick circle in a plain dive is a corkscrew at the dive's own speed, once per dive, for as long as the stick circles.
#[test]
fn a_stick_circle_in_a_plain_dive_is_a_normal_speed_corkscrew() {
    if need_disc() {
        return;
    }
    let mut s = Scene::flat();
    s.pad.stick = Vec2::new(0.0, 1.0);
    s.pad.a = true;
    s.step(30);
    s.pad.a = false;
    s.pad.b = true;
    s.step(2);
    s.pad.b = false;
    assert_eq!(s.sim.mario.status, Status::JumpCatch, "no dive");
    let y0 = s.sim.mario.pos.y;
    let mut angle = 0.0f32;
    // Past the Turbo corkscrew's 6 s cap (720 ticks).
    for i in 0..1000 {
        angle += std::f32::consts::TAU / 12.0;
        s.pad.stick = Vec2::new(angle.cos(), angle.sin());
        s.step(1);
        if i > 30 {
            assert_eq!(s.sim.mario.model_pitch, 0x4000, "not corkscrewing at {i}");
            assert!((s.sim.mario.pos.y - y0).abs() < 40.0, "left its height");
            assert!(s.sim.mario.forward_vel < 60.0, "faster than a dive: {}", s.sim.mario.forward_vel);
        }
    }
    // Let go: the plain dive, and a fresh circle doesn't start another corkscrew.
    s.pad.stick = Vec2::ZERO;
    s.step(60);
    assert_ne!(s.sim.mario.model_pitch, 0x4000, "it never ended");
    for _ in 0..30 {
        angle += std::f32::consts::TAU / 12.0;
        s.pad.stick = Vec2::new(angle.cos(), angle.sin());
        s.step(1);
        assert_ne!(s.sim.mario.model_pitch, 0x4000, "a second corkscrew in one dive");
        if s.sim.mario.status != Status::JumpCatch { break; }
    }
}

#[test]
fn both_corkscrews_follow_camera_yaw_and_only_relative_pitch() {
    if need_disc() {
        return;
    }
    fn steer<H:Host>(s:&mut Scene<H>) {
        // A tilted starting camera must still launch level. Raise well above terrain
        // to measure the downward steering without ending the roll on the floor.
        s.sim.mario.pos.y=2000.0;
        let initial_yaw=s.sim.mario.face_yaw;
        s.sim.mario.flight_camera=Some(libsms::Aim {yaw:initial_yaw,pitch:6000});
        let mut angle=0.0f32;
        let mut run=|s:&mut Scene<H>,ticks| {
            for _ in 0..ticks {
                angle+=std::f32::consts::TAU/12.0;
                s.pad.stick=Vec2::new(angle.cos(),angle.sin());s.step(1);
                assert_eq!(s.sim.mario.status,Status::JumpCatch);
            }
        };
        run(s,32);
        assert!(s.sim.mario.vel.y.abs()<0.001,"initial camera tilt changed level flight");
        let speed=s.sim.mario.vel.length();
        let right=initial_yaw.wrapping_add(6000);
        s.sim.mario.flight_camera=Some(libsms::Aim {yaw:right,pitch:12000});
        run(s,60);
        assert_eq!(s.sim.mario.face_yaw,right,"camera right did not steer");
        assert!(s.sim.mario.vel.y>0.0,"looking up did not climb");
        let max_vertical=speed*15.0_f32.to_radians().sin();
        assert!(s.sim.mario.vel.y<=max_vertical+0.01,"pitch was not bounded");
        assert!((s.sim.mario.vel.length()-speed).abs()<0.1,"steering changed flight speed");
        let left=initial_yaw.wrapping_sub(6000);
        s.sim.mario.flight_camera=Some(libsms::Aim {yaw:left,pitch:-12000});
        run(s,80);
        assert_eq!(s.sim.mario.face_yaw,left,"camera left did not steer");
        // By now (past 120 ticks) the corkscrew is flaming, which pitches up to 55 degrees.
        let flaming_vertical=speed*55.0_f32.to_radians().sin();
        assert!(s.sim.mario.vel.y<0.0 && s.sim.mario.vel.y>=-flaming_vertical-0.01,"downward pitch was not bounded");
        assert!(s.sim.mario.vel.y < -max_vertical,"a flaming corkscrew didn't pitch past the plain one's limit");
        s.sim.mario.flight_camera=Some(libsms::Aim {yaw:left,pitch:6000});
        run(s,100);
        assert!(s.sim.mario.vel.y.abs()<0.01,"returning to starting angle did not level flight");
        s.pad.stick=Vec2::ZERO;s.step(80);
        assert_eq!(s.sim.mario.model_pitch,0,"roll failed to end after releasing the stick");
        assert!(s.sim.mario.vel.y<0.0,"gravity did not resume");
    }
    steer(&mut corkscrew_scene(vec![]));
    let mut plain=Scene::flat();
    plain.pad.stick=Vec2::Y;plain.pad.a=true;plain.step(30);
    plain.pad.a=false;plain.pad.b=true;plain.step(2);plain.pad.b=false;
    steer(&mut plain);
}

#[test]
fn quick_nozzle_switch_plays_the_source_selection_cue_once_without_pickup() {
    let mut s = Scene::flat();
    s.sim.mario.give_fludd();
    for alternate in [2, 3, 4] {
        for (slot, name, other) in [
            (alternate, "MSD_SE_SY_SELECT_POMP_BACK", "MSD_SE_SY_SELECT_POMP"),
            (1, "MSD_SE_SY_SELECT_POMP", "MSD_SE_SY_SELECT_POMP_BACK"),
        ] {
            s.host.events.clear();
            assert!(s.sim.mario.switch_nozzle(slot));
            s.step(24);
            assert_eq!(sound_count(&s, name), 1, "wrong quick-switch cue for slot {slot}");
            assert_eq!(sound_count(&s, other), 0);
            assert_eq!(sound_count(&s, "MSD_SE_SY_GET_NOZZLE"), 0);
            assert_eq!(sound_count(&s, "MSD_SE_MA_SHINING_EFF_CALM2"), 0);
            assert_eq!(particle_count(&s, "ms_itemget1_a"), 0);
            s.host.events.clear();
            assert!(s.sim.mario.switch_nozzle(slot));
            s.step(1);
            assert_eq!(sound_count(&s, name), 0, "unchanged nozzle replayed selection");
        }
    }
    s.host.events.clear();
    assert!(!s.sim.mario.switch_nozzle(99));
    assert!(s.sim.mario.switch_nozzle(0));
    assert!(s.sim.mario.switch_nozzle(2));
    assert!(s.sim.mario.select_nozzle(1)); // Fitted-state refresh stays silent.
    s.step(1);
    for name in ["MSD_SE_SY_SELECT_POMP", "MSD_SE_SY_SELECT_POMP_BACK", "MSD_SE_SY_GET_NOZZLE"] {
        assert_eq!(sound_count(&s, name), 0, "sync/enable/disable played {name}");
    }
}

#[test]
fn only_a_radial_equip_plays_the_nozzle_pickup_sparkle_and_sounds_once() {
    let mut s=Scene::flat();s.sim.mario.give_fludd();
    // Fitted-state synchronization is silent; quick swaps are tested separately.
    assert!(s.sim.mario.select_nozzle(2));s.step(1);
    assert_eq!(s.sim.mario.fludd.as_ref().unwrap().nozzle,NozzleKind::Hover);
    assert_eq!(s.event_count(|e| matches!(e,Event::Particle(c) if c.name=="ms_itemget1_a")),0,"quick swap played pickup");
    // The radial menu equips a different alternate: one sparkle and both sounds.
    s.host.events.clear();
    assert!(s.sim.mario.equip_nozzle(3));s.step(1);
    assert_eq!(s.sim.mario.fludd.as_ref().unwrap().nozzle,NozzleKind::Rocket);
    assert_eq!(sound_count(&s, "MSD_SE_SY_SELECT_POMP"), 0);
    assert_eq!(sound_count(&s, "MSD_SE_SY_SELECT_POMP_BACK"), 0);
    assert_eq!(s.event_count(|e| matches!(e,Event::Particle(c) if c.name=="ms_itemget1_a" && c.at==libsms::FxAt::JointFollow(1))),1);
    for name in ["MSD_SE_SY_GET_NOZZLE","MSD_SE_MA_SHINING_EFF_CALM2"] {
        assert_eq!(s.event_count(|e| matches!(e,Event::Sound(c) if c.name==name)),1);
    }
    s.host.events.clear();assert!(s.sim.mario.equip_nozzle(3));s.step(1);
    assert_eq!(s.event_count(|e| matches!(e,Event::Particle(c) if c.name=="ms_itemget1_a")),0,"unchanged nozzle retriggered pickup");
    s.host.events.clear();assert!(s.sim.mario.equip_nozzle(1));s.step(1);
    assert_eq!(s.event_count(|e| matches!(e,Event::Particle(c) if c.name=="ms_itemget1_a")),0,"returning to Spray triggered pickup");
    // Equipping from Spray stays a real radial choice and still gets its pickup.
    s.host.events.clear();assert!(s.sim.mario.equip_nozzle(4));s.step(1);
    assert_eq!(s.sim.mario.fludd.as_ref().unwrap().nozzle,NozzleKind::Turbo);
    assert_eq!(s.event_count(|e| matches!(e,Event::Particle(c) if c.name=="ms_itemget1_a")),1,"radial equip from Spray lost its pickup");
}

fn particle_count<H: Host>(s: &Scene<H>, name: &str) -> usize {
    s.event_count(|e| matches!(e, Event::Particle(c) if c.name == name))
}

#[test]
fn a_fire_hit_throws_mario_up_burning_and_he_lies_down_after_two_bounces() {
    if need_disc() {
        return;
    }
    let mut s = Scene::flat();
    s.step(2);
    let max = s.mario().health;
    assert!(s.sim.mario.damage(Vec3::new(0.0, 0.0, 100.0), libsms::DamageKind::Fire));
    // `receiveMessage`: FIRE_DOWN, the fire sound and `ms_firedmg_a` after `damageExec`.
    assert_eq!(s.mario().status, Status::FireDown);
    assert!(s.mario().on_fire());
    assert!(s.mario().health < max);
    // `mFireDownForce` throws him up; the yell is on the second tick.
    s.step(1);
    assert_eq!(sound_count(&s, "MSD_SE_MA_DAMAGE_FIRE"), 1);
    assert_eq!(particle_count(&s, "ms_firedmg_a"), 1);
    assert!(s.mario().vel.y > 0.0, "he wasn't thrown up: {}", s.mario().vel.y);
    s.step(2);
    assert_eq!(sound_count(&s, "MSD_SE_MV05_DAMAGE_FIRE_01"), 1);
    // Two bounces, then the game lies him down (`FIRE_JUMP_END`) and he gets up.
    let mut bounces = 0;
    let mut prev_up = false;
    let mut lay = false;
    for _ in 0..2400 {
        s.step(1);
        let up = s.mario().vel.y > 0.0 && s.mario().status == Status::FireDown;
        if up && !prev_up {
            bounces += 1;
        }
        prev_up = up;
        if s.mario().status == Status::FireJumpEnd {
            lay = true;
            break;
        }
    }
    assert!(lay, "he never lay down: {:?}", s.mario().status);
    assert_eq!(bounces, 3, "the throw and two bounces");
    assert_eq!(sound_count(&s, "MSD_SE_MV07_DAMAGE_REACT_01"), 1);
    assert!(s.until(600, |m| m.status == Status::Wait), "he never got up");
}

#[test]
fn a_host_fire_keeps_the_vanilla_burn_hop_without_a_panic_run() {
    if need_disc() {
        return;
    }
    let mut s = Scene::flat();
    s.sim.mario.options.host_fire_reaction = true;
    s.step(2);
    let max = s.mario().health;
    assert!(s.sim.mario.ignite());
    assert_eq!(s.mario().health, max, "the host keeps the health");
    assert_eq!(s.mario().status, Status::FireDown);
    assert_eq!(s.mario().params.jump.fire_down_force, 50.0, "owned-disc burn-hop tuning");
    assert_eq!(s.mario().vel.y, 50.0, "setStatusToJumping uses mFireDownForce directly");
    let mut peak = 0.0f32;
    let mut bounces = 0;
    for _ in 0..600 {
        let before = s.mario().vel.y;
        s.step(1);
        peak = peak.max(s.mario().pos.y);
        assert_ne!(s.mario().status, Status::FireDash, "vanilla never enters the panic run");
        if s.mario().status == Status::FireDown && before < 0.0 && s.mario().vel.y > 0.0 {
            bounces += 1;
            assert!((s.mario().vel.y + before * 0.4).abs() < 1.0, "bounce must retain 40% of downward speed");
        }
        if !s.mario().on_fire() { break; }
        assert_eq!(s.mario().anim.clip, "firejmp");
    }
    assert_eq!(bounces, 2);
    assert!((315.0..=325.0).contains(&peak), "burn-hop apex {peak}");
    assert_eq!(s.mario().status, Status::FireJumpEnd);
    assert_eq!(sound_count(&s, "MSD_SE_MV05_DAMAGE_FIRE_02"), 1, "first reaction uses voice 02 once");
    assert_eq!(sound_count(&s, "MSD_SE_MV05_DAMAGE_FIRE_01"), 0, "do not combine the voices");
    assert!(!s.mario().on_fire());
    // Repeated attempts during the five-second escape window must not chain a new run.
    for elapsed in 0..599 {
        assert!(!s.sim.mario.ignite(), "fire trap retriggered after {elapsed} grace ticks");
        s.step(1);
    }
    s.step(1);
    assert!(s.sim.mario.ignite());
    s.step(3);
    assert_eq!(sound_count(&s, "MSD_SE_MV05_DAMAGE_FIRE_01"), 1, "next reaction alternates to 01");
    assert_eq!(sound_count(&s, "MSD_SE_MV05_DAMAGE_FIRE_02"), 1, "one voice per reaction");
}

#[test]
fn burning_mario_cannot_be_lit_twice_or_while_down() {
    if need_disc() {
        return;
    }
    let mut s = Scene::flat();
    s.step(2);
    assert!(s.sim.mario.ignite());
    s.step(5);
    assert!(!s.sim.mario.ignite(), "already burning");
    s.sim.mario.knock_out();
    assert!(!s.sim.mario.ignite(), "down for good");
}

#[test]
fn body_water_extinguishes_every_fire_phase_and_keeps_the_escape_grace() {
    for phase in [Status::FireDown, Status::FireDash, Status::FireJump, Status::FireLand] {
        let mut s = Scene::flat();
        s.sim.mario.options.host_fire_reaction = true;
        s.step(2);
        assert!(s.sim.mario.ignite());
        s.sim.mario.status = phase;
        let health = s.mario().health;
        s.sim.mario.wet();
        assert!(!s.mario().on_fire(), "{phase:?} was not extinguished");
        assert!(s.mario().is_wet());
        assert_eq!(s.mario().health, health);
        s.step(120);
        assert!(!s.mario().is_wet(), "vanilla runoff timer lasts 120 ticks");
        assert!(!s.sim.mario.ignite(), "quenching must retain five seconds of escape grace");
        s.step(480);
        assert!(s.sim.mario.ignite());
    }
}

#[test]
fn stage_water_quenches_the_initial_fire_bounce_and_continuously_renews_wetness() {
    let mut s = Scene::flat();
    s.sim.mario.options.host_fire_reaction = true;
    s.step(2);
    assert!(s.sim.mario.ignite());
    s.host.world.water = Some(10000.0);
    s.step(1);
    assert!(!s.mario().on_fire());
    for _ in 0..800 {
        s.step(1);
        assert!(s.mario().is_wet());
        assert!(!s.sim.mario.ignite());
    }
    s.host.world.water = None;
    // Escape grace expired in the pool, but vanilla's 120-tick timer begins on exit.
    s.step(119);
    assert!(!s.sim.mario.ignite());
    s.step(1);
    assert!(s.sim.mario.ignite());
}

#[test]
fn vanilla_runoff_requests_real_droplets_every_eight_ticks_after_water_exit() {
    let mut s = Scene::new(FlatHost { water: Some(10.0), ..Default::default() });
    s.step(2);
    s.host.events.clear();
    s.host.world.water = None;
    for elapsed in 1..=120 {
        s.step(1);
        let count = s.host.events.iter().filter(|e| matches!(e, Event::BodyDrip { .. })).count();
        assert_eq!(count, elapsed / 8, "checkWet cadence at tick {elapsed}");
    }
    assert!(!s.mario().is_wet());
    s.step(30);
    assert_eq!(s.host.events.iter().filter(|e| matches!(e, Event::BodyDrip { .. })).count(), 15);
}

#[test]
fn spray_keeps_up_with_a_boosted_jump() {
    // Host extension: the part of his travel above 1x (a Wax Quail) is carried by the water in
    // full; vanilla's eighth stays the rest.
    let inherit = |boost: f32| {
        let mut s = Scene::flat();
        s.sim.mario.give_fludd();
        s.run_forward();
        s.secs(1.0);
        s.press_a();
        s.sim.mario.boost_jump(boost);
        s.host.events.clear();
        s.pad.r_analog = 200;
        s.step(20);
        assert!(s.mario().status.is_jumping(), "still in the air");
        let w = s.host.events.iter().filter_map(|e| match e {
            Event::WaterEmit(w) => Some(*w),
            _ => None,
        }).last().expect("water left the nozzle");
        w.inherit.z
    };
    let (plain, boosted) = (inherit(1.0), inherit(3.0));
    assert!(plain > 0.0, "{plain}");
    assert!(boosted > 8.0 * plain, "{plain} {boosted}");
}

/// Apex of the jump out of a pound from 1500, with or without a stick circle in the fall.
/// Pounds from 8000 up; returns (fastest fall speed while plain, fastest while flaming, height the flame began).
fn pound_fall(circle: bool, fast: bool) -> (f32, f32, f32) {
    let mut s: Scene<FlatHost> = Scene::flat();
    s.sim.mario.options.spin_pound_fast = fast;
    s.sim.mario.pos = Vec3::new(0.0, 8000.0, 0.0);
    s.step(2);
    s.pad.l = true;
    s.step(1);
    s.pad.l = false;
    let (mut plain, mut flame, mut flame_at) = (0.0f32, 0.0f32, f32::NAN);
    let mut angle = 0.0f32;
    for i in 0..3000 {
        if circle && i < 40 {
            angle += std::f32::consts::TAU / 12.0;
            s.pad.stick = Vec2::new(angle.cos(), angle.sin());
        } else {
            s.pad.stick = Vec2::ZERO;
        }
        if s.mario().status == Status::HipAttackEnd {
            break;
        }
        s.step(1);
        match s.mario().pounding() {
            1 => plain = plain.max(-s.mario().vel.y),
            2 => {
                if flame_at.is_nan() {
                    flame_at = s.mario().pos.y;
                }
                flame = flame.max(-s.mario().vel.y);
            }
            _ => {}
        }
    }
    (plain, flame, flame_at)
}

/// Host extension `Options::spin_pound_fast`: a spun pound falls faster, flaming or not, the flame
/// starts at the same height, and the flag off is the game's fall.
#[test]
fn a_spinning_pound_falls_faster_and_still_flames_at_the_same_height() {
    let (plain, flame, flame_at) = pound_fall(false, true);
    let (spin_plain, spin_flame, spin_at) = pound_fall(true, true);
    let (off_plain, off_flame, off_at) = pound_fall(true, false);
    assert!(spin_plain > flame, "regular spin {spin_plain} isn't faster than a plain flaming pound {flame}");
    assert!(spin_flame > flame, "flaming spin {spin_flame} isn't faster than a flaming pound {flame}");
    assert!(spin_plain > plain);
    assert!(flame_at.is_finite() && spin_at.is_finite(), "no flame: {flame_at} {spin_at}");
    assert!((spin_at - flame_at).abs() < 40.0, "flame height moved: {flame_at} -> {spin_at}");
    assert_eq!((off_plain, off_flame), (plain, flame), "flag off changed the fall");
    assert_eq!(off_at, flame_at);
}

fn spin_pound_jump_apex(circle: bool, spin_mult: f32) -> f32 {
    let mut s: Scene<FlatHost> = Scene::flat();
    s.sim.mario.options.pound_jump = 0.75;
    s.sim.mario.options.spin_pound_jump = spin_mult;
    s.sim.mario.pos = Vec3::new(0.0, 1500.0, 0.0);
    s.step(2);
    s.pad.l = true;
    s.step(1);
    s.pad.l = false;
    let mut angle = 0.0f32;
    for i in 0..3000 {
        if circle && i < 40 {
            angle += std::f32::consts::TAU / 12.0;
            s.pad.stick = Vec2::new(angle.cos(), angle.sin());
        } else {
            s.pad.stick = Vec2::ZERO;
        }
        if s.mario().status == Status::HipAttackEnd {
            break;
        }
        s.step(1);
    }
    s.pad.stick = Vec2::ZERO;
    s.press_a();
    assert_eq!(s.mario().status, Status::UltraJump);
    let mut apex = 0.0f32;
    while s.mario().vel.y > 0.0 || s.mario().pos.y > apex {
        apex = apex.max(s.mario().pos.y);
        s.step(1);
    }
    apex
}

#[test]
fn a_spinning_pound_jumps_much_higher_than_a_plain_one() {
    let plain = spin_pound_jump_apex(false, 2.5);
    let spun = spin_pound_jump_apex(true, 2.5);
    let spun_off = spin_pound_jump_apex(true, 1.0);
    println!("pound jump apex: plain {plain:.0}, spun {spun:.0}, spun at x1 {spun_off:.0}");
    assert!(spun > plain * 2.0, "{spun} vs {plain}");
    assert!((spun_off - plain).abs() < plain * 0.1, "{spun_off} vs {plain}");
}



/// The Rocket's exhaust effects, authored to blow downward, are turned to blow rearward along the heading
/// behind him when the shot is level (a dive or slide); with the flag off there is none (the game has no
/// Rocket in a dive).
#[test]
fn the_level_rockets_exhaust_is_turned_rearward() {
    for on in [true, false] {
        let mut s = Scene::flat();
        s.sim.mario.give_fludd();
        s.sim.mario.options.slide_dive_nozzles = on;
        {
            let p = s.sim.mario.params.clone();
            let f = s.sim.mario.fludd.as_mut().unwrap();
            f.change_nozzle(NozzleKind::Rocket, false, &p);
            f.refill(&p);
        }
        s.run_forward();
        s.secs(1.0);
        s.press_a();
        s.step(10);
        s.pad.b = true;
        s.step(1);
        s.pad.b = false;
        s.pad.r_analog = 255;
        let mut cues = vec![];
        for _ in 0..360 {
            s.host.events.clear();
            s.step(1);
            for e in s.host.events.iter() {
                if let Event::Particle(c) = e {
                    if c.name.starts_with("ms_m_rocket_") {
                        cues.push((c.name, c.at, s.mario().face_yaw));
                    }
                }
            }
            if !matches!(s.mario().status, Status::JumpCatch | Status::Catch) && !cues.is_empty() {
                break;
            }
        }
        if !on {
            assert!(cues.is_empty(), "a Rocket exhaust with the flag off");
            continue;
        }
        let names: std::collections::BTreeSet<_> = cues.iter().map(|c| c.0).collect();
        assert_eq!(names.len(), 3, "exhaust effects a, b and b2 expected, got {names:?}");
        for (name, at, face) in cues {
            match at {
                libsms::FxAt::MuzzleFollowAlong(0, yaw) => {
                    assert_eq!(yaw, face.wrapping_add(i16::MIN), "{name} not turned behind him")
                }
                other => panic!("{name} started at {other:?}, not turned rearward"),
            }
        }
    }
}



/// A flat run, then a 30 degree embankment up to a plateau, along +Z.
struct Embankment;
impl Embankment {
    const START: f32 = 2600.0;
    const TOP: f32 = 3400.0;
    const SLOPE: f32 = 0.5774;
}
impl Host for Embankment {
    fn ground(&self, p: Vec3) -> Option<libsms::GroundHit> {
        let (h, normal) = if p.z < Self::START {
            (0.0, Vec3::Y)
        } else if p.z < Self::TOP {
            ((p.z - Self::START) * Self::SLOPE, Vec3::new(0.0, 0.866, -0.5))
        } else {
            ((Self::TOP - Self::START) * Self::SLOPE, Vec3::Y)
        };
        (p.y >= h).then_some(libsms::GroundHit { height: h, normal, flags: libsms::SurfaceFlags::NONE })
    }
}

/// Slides on Turbo up the embankment: returns the most ticks in a row with his feet clear of the ground,
/// the highest he got above it, and the fastest he went on the way.
fn sled_over_the_embankment(on: bool) -> (u32, f32, f32) {
    let mut s = Scene::new(Embankment);
    s.sim.mario.give_fludd();
    s.sim.mario.options.slide_dive_nozzles = on;
    {
        let p = s.sim.mario.params.clone();
        let f = s.sim.mario.fludd.as_mut().unwrap();
        f.change_nozzle(NozzleKind::Turbo, false, &p);
        f.refill(&p);
    }
    s.sim.mario.pos = Vec3::new(0.0, 0.0, 1200.0);
    s.run_forward();
    s.secs(0.5);
    s.press_a();
    s.step(10);
    s.pad.b = true;
    s.step(1);
    s.pad.b = false;
    s.pad.r_analog = 255;
    s.pad.stick = Vec2::ZERO;
    let (mut run, mut best, mut high, mut fastest) = (0u32, 0u32, 0.0f32, 0.0f32);
    for _ in 0..900 {
        s.step(1);
        let m = s.mario();
        fastest = fastest.max(m.forward_vel);
        let ground = Embankment.ground(m.pos + Vec3::Y * 30.0).map_or(0.0, |g| g.height);
        let clear = m.pos.y - ground;
        if clear > 3.0 && m.pos.z > Embankment::START {
            run += 1;
            best = best.max(run);
            high = high.max(clear);
        } else {
            run = 0;
        }
        if m.pos.z > Embankment::TOP + 800.0 {
            break;
        }
    }
    (best, high, fastest)
}

#[test]
fn a_turbo_sled_flies_off_the_crest_of_an_embankment() {
    let (air, high, fast) = sled_over_the_embankment(true);
    assert!(air >= 20 && high > 40.0, "no airtime off the crest: {air} ticks, {high:.0} up (fastest {fast:.0})");
    // Flag off: the game's slide takes the slope, nothing like that.
    let (air_off, high_off, _) = sled_over_the_embankment(false);
    assert!(high_off < 40.0, "flag off still flew: {air_off} ticks, {high_off:.0} up");
}

/// A level Rocket shot in a belly slide is a rocket sled: well past the dive's speed and the Turbo's, and it holds it
/// for a good while, not one short surge that friction eats.
#[test]
fn a_rocket_slide_is_fast_and_stays_fast() {
    let run = |on: bool| {
        let mut s = Scene::flat();
        s.sim.mario.give_fludd();
        s.sim.mario.options.slide_dive_nozzles = on;
        {
            let p = s.sim.mario.params.clone();
            let f = s.sim.mario.fludd.as_mut().unwrap();
            f.change_nozzle(NozzleKind::Rocket, false, &p);
            f.refill(&p);
        }
        s.run_forward();
        s.secs(1.0);
        s.press_a();
        s.step(10);
        s.pad.b = true;
        s.step(1);
        s.pad.b = false;
        s.pad.r_analog = 255;
        s.pad.stick = Vec2::ZERO;
        let start = s.mario().pos;
        let (mut fired_at, mut peak, mut fast_ticks) = (None, 0.0f32, 0u32);
        for i in 0..600 {
            s.host.events.clear();
            s.step(1);
            if fired_at.is_none() && s.event_count(|e| matches!(e, Event::TriggerFired(_))) > 0 {
                fired_at = Some(i);
            }
            if fired_at.is_some() {
                peak = peak.max(s.mario().forward_vel);
                if s.mario().forward_vel > 100.0 {
                    fast_ticks += 1;
                }
            }
            if !matches!(s.mario().status, Status::JumpCatch | Status::Catch) && fired_at.is_some() {
                break;
            }
        }
        let m = s.mario();
        (peak, fast_ticks, Vec3::new(m.pos.x - start.x, 0.0, m.pos.z - start.z).length())
    };
    let (peak, fast, far) = run(true);
    let (_, _, far_off) = run(false);
    assert!(peak >= 130.0, "peak speed only {peak}");
    assert!(fast >= 60, "fast for only {fast} ticks");
    assert!(far > far_off * 1.8, "went {far}, only {:.1}x the plain slide ({far_off})", far / far_off);
}

/// The Rocket's head follows the lock through the charge-up (before it fires), so it is already on target when the
/// shot goes; with the master switch off nothing aims.
#[test]
fn the_rocket_aims_during_the_charge_up_as_well_as_the_shot() {
    for master in [true, false] {
        let mut s = Scene::flat();
        s.sim.mario.give_fludd();
        s.sim.mario.options.pump_in_long_fall = true;
        s.sim.mario.options.aim_alt = master;
        {
            let p = s.sim.mario.params.clone();
            let f = s.sim.mario.fludd.as_mut().unwrap();
            f.change_nozzle(NozzleKind::Rocket, false, &p);
            f.refill(&p);
        }
        s.sim.mario.pos = Vec3::new(0.0, 12000.0, 0.0);
        s.step(5);
        s.sim.mario.aim = Some(libsms::Aim { yaw: 0x4000, pitch: 0 });
        s.pad.r_analog = 255;
        s.step(30);
        let charging = s.mario().fludd.as_ref().unwrap().trigger_state();
        assert_eq!(charging, libsms::fludd::TriggerState::Inactive, "it already fired");
        let (_, dir) = s.mario().water_emitter(0).expect("no emitter");
        let look = s.mario().fludd_look().expect("no look");
        if master {
            assert!(dir.x > 0.99, "the head isn't on the target while charging: {dir:?}");
            assert!(look.aimed, "the look isn't aimed while charging");
        } else {
            assert!(dir.x < 0.5 && !look.aimed, "aimed with the master switch off: {dir:?}");
        }
    }
}

/// The stand-in's new cues: lying on the back or the front after a knock-down, and on fire.
#[test]
fn the_stand_in_is_told_when_he_lies_down_and_when_he_burns() {
    let mut s = Scene::flat();
    for (status, want) in [
        (Status::BackDown, libsms::POSE_DOWN),
        (Status::ShortBackDown, libsms::POSE_DOWN),
        (Status::DownLoser, libsms::POSE_DOWN),
        (Status::ForeDown, libsms::POSE_FACEDOWN),
        (Status::SafeBackDown, 0),
        (Status::Run, 0),
    ] {
        s.sim.mario.change_status(status, 0, true);
        let pose = s.mario().pose();
        assert_eq!(pose & (libsms::POSE_DOWN | libsms::POSE_FACEDOWN), want, "{status:?}");
        assert_eq!(pose & libsms::POSE_FIRE, 0, "{status:?} burning");
    }
    // A super ground pound burns.
    let mut s = Scene::flat();
    s.sim.mario.pos = Vec3::new(0.0, 8000.0, 0.0);
    s.step(2);
    s.pad.l = true;
    s.step(1);
    s.pad.l = false;
    let mut burned = false;
    for _ in 0..1500 {
        s.step(1);
        burned |= s.mario().pose() & libsms::POSE_FIRE != 0;
        if s.mario().status == Status::HipAttackEnd {
            break;
        }
    }
    assert!(burned, "no fire cue in a super pound");
}
