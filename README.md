# libsms

Super Mario Sunshine's Mario locomotion and FLUDD as a game-agnostic Rust library, reimplemented
from the [doldecomp/sms](https://github.com/doldecomp/sms) decomp as a behavioral spec.

The sim talks to the world through a `Host` trait (ground, ceiling and wall probes plus an event
sink), runs at a fixed 120 Hz, and reads its tuning from the user's own disc at runtime. No game
data is in this repository.

```rust
use libsms::*;

let assets = Assets::discover(None).expect("run tools/extract_assets.py first");
let mut sim = Sim::new(assets.params.clone(), assets.anim_table.clone(), Vec3::ZERO, 0);
sim.mario.give_fludd();

struct World; // your game: implement ground/roof/walls/event
# impl Host for World { fn ground(&self, _: Vec3) -> Option<GroundHit> { None } }
let pad = Pad { stick: Vec2::new(0.0, 1.0), ..Pad::default() };
sim.advance(&mut World, &pad, /* camera_yaw */ 0, 1.0 / 60.0);
```

Read `docs/behavior-spec.md` for what is implemented, the tuning numbers, and what still needs
verifying against the real game.

## Your own game files

The repository contains no game data. Build the git-ignored `assets/` folder once from your own
disc image (any region; tested with PAL):

```
python tools/extract_assets.py "path/to/Super Mario Sunshine.iso"
```

It needs [decomp-toolkit](https://github.com/encounter/decomp-toolkit/releases)'s `dtk` (pass
`--dtk`, set `DTK`, put it on PATH, or have a built decomp checkout beside this repo) and writes
the tuning `.prm` files, the animation clips and the skeleton source into `assets/`. Everything
else (`Assets::discover`, the tests, the viewer) reads from there and falls back to the decomp's
code defaults when it is missing.

## Running sample

A 3D obstacle course with a third-person camera: stairs, a wall-kick shaft, a high platform for
hover/rocket, a bridge, and gentle and slippery ramps, with FLUDD water droplets.

```
cargo run -p viewer
```

It reads `assets/` (a path argument or `SMS_ASSETS` overrides it) and says on screen whether it
found it. With the rig present, a neutral mannequin is posed from the game's real animation clips
the sim is playing; without it, a plain capsule. `--demo` replays a scripted loop. WASD move, Space jump, F dive, Z ground pound, Ctrl crouch,
Shift spray, X swap nozzle, 1-4 pick nozzle, arrows orbit the camera, R respawn.

```
cargo test
cargo run -p libsms --example report
```
