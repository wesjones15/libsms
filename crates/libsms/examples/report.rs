//! Prints headline numbers from the sim, to eyeball against the real game.
//! Run: `cargo run -p libsms --example report` (uses `assets/` if present).

use libsms::{
    AnimTable, Event, FlatHost, Host, NozzleKind, Pad, Params, Sim, Status, TICK_HZ, Vec2, Vec3,
};

struct Log(Vec<Event>);
impl Host for Log {
    fn ground(&self, p: Vec3) -> Option<libsms::GroundHit> {
        FlatHost::default().ground(p)
    }
    fn event(&mut self, e: Event) {
        self.0.push(e);
    }
}

fn sim() -> Sim {
    let (params, anims) = match libsms::Assets::discover(None) {
        Some(a) => (a.params, a.anim_table),
        None => {
            eprintln!("(no assets/ folder: using code defaults; run tools/extract_assets.py)");
            (Params::default(), AnimTable::default())
        }
    };
    Sim::new(params, anims, Vec3::ZERO, 0)
}

fn step(s: &mut Sim, h: &mut Log, pad: &Pad, n: u32) {
    for _ in 0..n {
        s.advance(h, pad, i16::MIN, 1.0 / TICK_HZ as f32);
    }
}

fn main() {
    let hz = TICK_HZ as f32;

    // Run speed
    let (mut s, mut h) = (sim(), Log(vec![]));
    let pad = Pad {
        stick: Vec2::new(0.0, 1.0),
        ..Pad::default()
    };
    step(&mut s, &mut h, &pad, 2 * TICK_HZ);
    let z0 = s.mario.pos.z;
    step(&mut s, &mut h, &pad, TICK_HZ);
    println!(
        "full run: {:.0} units/s (forward_vel {:.1}), Mario is 160 tall -> {:.1} heights/s",
        s.mario.pos.z - z0,
        s.mario.speed(),
        (s.mario.pos.z - z0) / 160.0
    );

    // Standing jump
    let (mut s, mut h) = (sim(), Log(vec![]));
    let pad = Pad {
        a: true,
        ..Pad::default()
    };
    let (mut apex, mut t, mut apex_t) = (0.0f32, 0u32, 0u32);
    loop {
        step(&mut s, &mut h, &pad, 1);
        t += 1;
        if s.mario.pos.y > apex {
            apex = s.mario.pos.y;
            apex_t = t;
        }
        if s.mario.pos.y <= 0.0 && t > 5 {
            break;
        }
    }
    println!(
        "standing jump: apex {apex:.0} units ({:.2} heights) at {:.2}s, airtime {:.2}s",
        apex / 160.0,
        apex_t as f32 / hz,
        t as f32 / hz
    );

    // Triple jump
    let (mut s, mut h) = (sim(), Log(vec![]));
    let run = Pad {
        stick: Vec2::new(0.0, 1.0),
        ..Pad::default()
    };
    step(&mut s, &mut h, &run, TICK_HZ);
    let mut names = vec![];
    for want in [Status::JumpSlip, Status::SecJumpSlip] {
        let hold = Pad { a: true, ..run };
        step(&mut s, &mut h, &hold, 1);
        for _ in 0..2000 {
            if s.mario.status == want {
                break;
            }
            step(&mut s, &mut h, &hold, 1);
        }
        names.push(format!("{:?}", s.mario.status));
        step(&mut s, &mut h, &run, 1);
    }
    let hold = Pad { a: true, ..run };
    step(&mut s, &mut h, &hold, 1);
    let mut apex = 0.0f32;
    for _ in 0..800 {
        step(&mut s, &mut h, &hold, 1);
        apex = apex.max(s.mario.pos.y);
    }
    println!(
        "triple jump chain: {names:?}, final apex {apex:.0} units ({:.2} heights)",
        apex / 160.0
    );

    // Spray over time: does it fade?
    let (mut s, mut h) = (sim(), Log(vec![]));
    s.mario.give_fludd();
    let spray = Pad {
        r_analog: 255,
        ..Pad::default()
    };
    print!("spray output by second (full R): ");
    for sec in 0..10 {
        step(&mut s, &mut h, &spray, TICK_HZ);
        let f = s.mario.fludd.as_ref().unwrap();
        print!(
            "{sec}s amt={:.2} tank={:.0}% | ",
            f.spray_amount(),
            f.level(&s.mario.params) * 100.0
        );
    }
    println!();

    // Spray reach: standing, full R, aimed level; where the water lands.
    for (reach, speed) in [(1.0f32, 1.0), (1.5, 1.0), (2.0, 1.0), (2.0, 1.2)] {
        let (mut s, mut h) = (sim(), Log(vec![]));
        s.mario.give_fludd();
        let mut water = libsms::water::WaterManager::new(&s.mario.params);
        water.reach = reach;
        water.spray_speed = speed;
        let (mut far, mut ticks, mut emitted, mut attack) = (0.0f32, 0u32, 0u32, 0i32);
        for _ in 0..3 * TICK_HZ {
            step(&mut s, &mut h, &spray, 1);
            for e in h.0.drain(..) {
                if let Event::WaterEmit(w) = e {
                    emitted += w.count;
                    attack += w.attack as i32 * w.count as i32;
                    water.emit(&w);
                }
            }
            ticks += 1;
            if ticks % 4 == 0 {
                water.step(&FlatHost::default(), s.mario.pos);
                for e in water.events.drain(..) {
                    if let libsms::water::WaterEvent::SplashSound { pos, .. } = e {
                        far = far.max(Vec2::new(pos.x, pos.z).length());
                    }
                }
            }
        }
        println!(
            "spray reach x{reach}, flight x{speed}: lands up to {far:.0} units ({:.1} heights); {:.0} particles/s, attack {:.1} each",
            far / 160.0,
            emitted as f32 / 3.0,
            attack as f32 / emitted.max(1) as f32
        );
    }

    // Hover
    let (mut s, mut h) = (sim(), Log(vec![]));
    s.mario.give_fludd();
    let p = s.mario.params.clone();
    s.mario
        .fludd
        .as_mut()
        .unwrap()
        .change_nozzle(NozzleKind::Hover, false, &p);
    s.mario.fludd.as_mut().unwrap().refill(&p);
    step(
        &mut s,
        &mut h,
        &Pad {
            a: true,
            ..Pad::default()
        },
        30,
    );
    let hov = Pad {
        r_analog: 255,
        ..Pad::default()
    };
    let (mut max_y, mut first_hover, mut last_hover) = (0.0f32, None, 0u32);
    for t in 0..(8 * TICK_HZ) {
        step(&mut s, &mut h, &hov, 1);
        max_y = max_y.max(s.mario.pos.y);
        if s.mario.status == Status::Rocket {
            first_hover.get_or_insert(t);
            last_hover = t;
        }
    }
    println!(
        "hover: height {max_y:.0} (target {:.0}), hovering {:.1}s, tank after {:.0}%",
        p.water_gun.hover_height,
        (last_hover - first_hover.unwrap_or(0)) as f32 / hz,
        s.mario.fludd.as_ref().unwrap().level(&s.mario.params) * 100.0
    );

    // Rocket, at the game's lift and with Options::rocket_thrust.
    for thrust in [1.0f32, 1.25, 1.5, 2.0] {
        let (mut s, mut h) = (sim(), Log(vec![]));
        s.mario.options.rocket_thrust = thrust;
        s.mario.give_fludd();
        let p = s.mario.params.clone();
        s.mario
            .fludd
            .as_mut()
            .unwrap()
            .change_nozzle(NozzleKind::Rocket, false, &p);
        s.mario.fludd.as_mut().unwrap().refill(&p);
        let rock = Pad {
            r: true,
            r_analog: 255,
            ..Pad::default()
        };
        let (mut max_y, mut fired_at) = (0.0f32, None);
        for t in 0..(8 * TICK_HZ) {
            step(&mut s, &mut h, &rock, 1);
            max_y = max_y.max(s.mario.pos.y);
            if fired_at.is_none() && h.0.iter().any(|e| matches!(e, Event::TriggerFired(_))) {
                fired_at = Some(t);
            }
        }
        println!(
            "rocket x{thrust}: charge {:.2}s, apex {max_y:.0} units ({:.1} heights), tank {:.0}%",
            fired_at.unwrap_or(0) as f32 / hz,
            max_y / 160.0,
            s.mario.fludd.as_ref().unwrap().level(&s.mario.params) * 100.0
        );
    }

    // Turbo
    let (mut s, mut h) = (sim(), Log(vec![]));
    s.mario.give_fludd();
    let p = s.mario.params.clone();
    s.mario
        .fludd
        .as_mut()
        .unwrap()
        .change_nozzle(NozzleKind::Turbo, false, &p);
    s.mario.fludd.as_mut().unwrap().refill(&p);
    let turbo = Pad {
        r_analog: 255,
        stick: Vec2::new(0.0, 1.0),
        ..Pad::default()
    };
    let mut dash_at = None;
    let mut top = 0.0f32;
    for t in 0..(6 * TICK_HZ) {
        step(&mut s, &mut h, &turbo, 1);
        top = top.max(s.mario.speed());
        if dash_at.is_none() && s.mario.speed() > p.mario.dash_max - 1.0 {
            dash_at = Some(t);
        }
    }
    println!(
        "turbo: top speed {top:.0} (dash max {:.0}), reached after {:?} ticks",
        p.mario.dash_max, dash_at
    );
}
