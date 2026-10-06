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
