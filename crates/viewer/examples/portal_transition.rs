//! Exact 05_gate01 BMD/BTK/BRK material transition, MoveBG/ModelGate.cpp::perform.
//! No JPA cue or effect-model ID: this is the gate material's authored colour animation.
//! cargo run --release -p viewer --example portal_transition -- <assets> <output-dir>
use std::path::PathBuf;
use libsms_present::{portal::Portal, portal_cache as timing};
#[path = "support/png.rs"]
mod png;

pub fn main() {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().expect("assets"));
    let out = PathBuf::from(args.next().expect("output folder"));
    std::fs::create_dir_all(&out).unwrap();
    let mut portal = Portal::load(&dir).unwrap();
    portal.set_interior(&libsms_present::plaza::plaza_still(&dir, 512).unwrap());
    println!("05_gate01 BRK: open {}s; control {}s; return {}..{}s", timing::OPEN, timing::COMPLETE, timing::RETURN_START, timing::CLOSE);
    let count = ((timing::CLOSE + 0.5) * 30.).ceil() as usize;
    for frame in 0..=count {
        let time = frame as f32 / 30.;
        let activation = timing::activation(time);
        let pixels = timing::render_frame(&mut portal, time * 60., activation).unwrap();
        std::fs::write(out.join(format!("frame-{frame:03}.png")), png::rgba(256, 256, &pixels)).unwrap();
        if frame % 30 == 0 { println!("{time:.2}s: BRK activation {activation:.3}"); }
    }
    std::fs::write(out.join("SOURCE.txt"),
        "Exact asset: dolpic1.szs:map/map/gate/05_gate01.bmd + 05_gate01.btk + 05_gate01.brk. Material transition, not a particle emitter or effect model; no numeric JPA/efSync ID. Source: sms-main/src/MoveBG/ModelGate.cpp::loadAfter (unkD4=.1, unkD8=.02) and perform (unkD0 * BRK end frame). Original indirect distortion, M mask and Y/U/V TEV stages. Host adaptation: reveal over 0.8 seconds, hold the revealed Plaza until two seconds after control returns, then use the source's BRK closing rate. Existing disc-rendered Plaza still is unchanged. Frames at 30 fps.\n").unwrap();
}
