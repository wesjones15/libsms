//! Headless scenarios: drive Mario on a flat test host and check behaviour against the decomp.
//!
//! If the user's disc data has been extracted to `../extracted/sms` , the
//! tests run with the real `.prm` tuning and animation lengths; otherwise they use the code
//! defaults. Assertions are written to hold either way unless a test says otherwise.

use std::path::PathBuf;

use libsms::{
    AnimTable, Event, FlatHost, Host, Mario, NozzleKind, Pad, Params, Sim, Status, TICK_HZ, Vec2,
    Vec3, WaterEmit,
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
