# libsms

Super Mario Sunshine's Mario locomotion and FLUDD as a game-agnostic Rust library, reimplemented
from the [doldecomp/sms](https://github.com/doldecomp/sms) decomp as a behavioral spec.

The sim talks to the world through a `Host` trait (ground, ceiling and wall probes plus an event
sink), runs at a fixed 120 Hz, and reads its tuning from the user's own disc at runtime. No game
data is in this repository.

```rust
use libsms::*;

let (params, _) = Params::from_dir("extracted/sms/params_mario".as_ref());
let anims = AnimTable::from_dir("extracted/sms/bck".as_ref());
let mut sim = Sim::new(params, anims, Vec3::ZERO, 0);
sim.mario.give_fludd();

struct World; // your game: implement ground/roof/walls/event
# impl Host for World { fn ground(&self, _: Vec3) -> Option<GroundHit> { None } }
let pad = Pad { stick: Vec2::new(0.0, 1.0), ..Pad::default() };
sim.advance(&mut World, &pad, /* camera_yaw */ 0, 1.0 / 60.0);
```

Read `docs/behavior-spec.md` for what is implemented, the tuning numbers, and what still needs
verifying against the real game.

## Running sample

A 3D obstacle course with a third-person camera: stairs, a wall-kick shaft, a high platform for
hover/rocket, a bridge, and gentle and slippery ramps, with FLUDD water droplets.

```
cargo run -p viewer
```

It looks for your extracted disc data in `extracted/sms` (or a path argument / `SMS_DATA`) and
says on screen whether it found it. WASD move, Space jump, F dive, Z ground pound, Ctrl crouch,
Shift spray, X swap nozzle, 1-4 pick nozzle, arrows orbit the camera, R respawn.

```
cargo test
cargo run -p libsms --example report
```
