> modern/2026 migration branch: targets RiskOfRain2_DLC4 (Unity 2021.3.45f2). Build/API validation only; in-game validation is pending. Legacy instructions below describe the previous release.

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

The sample viewer plays Mario's animation sounds, FLUDD and impact effects, and background
music from your extracted disc. Press **M** to cycle music (including off).
See [audio integration](docs/audio.md) for the sound crates, host API and verification commands.

## Your own game files

The repository contains no game data. Build the git-ignored `assets/` folder once from your own
disc image (any region; tested with PAL and USA, whose extractions are byte-identical apart from
the language-specific HUD text screens), with no external tools:

```
cargo run --release -p libsms --bin sms-extract -- "path/to/Super Mario Sunshine.iso"
```

That reads a plain ISO/GCM or CISO image, decompresses the archives inside it, and writes the
tuning `.prm` files, the animation clips and the skeleton source into `assets/` in about a second.
A host does the same on first run with `Assets::ensure(iso, dir, log)`, then never touches the
disc again. Everything else (`Assets::discover`, the tests, the viewer) reads from `assets/` and
falls back to the decomp's code defaults when it is missing.

Non-Rust hosts use ABI 18 exports `sms_assets_complete`, `sms_find_disc` and
`sms_extract`. Discovery checks `SMS_DISC` and the supplied directories for Sunshine
ISO/GCM/CISO images. Extraction returns a UTF-8 status/error message; a complete cache
succeeds without the disc. Run extraction on a worker and wait before creating sims or views.
`sms_render_stings` caches RoR2 arrival/death WAVs in `ror2_audio/` using the extracted
sound banks, without opening an output device. It also belongs on the worker.

RVZ, WIA and WBFS images are not supported by the Rust extractor; convert to ISO, or use
`python tools/extract_assets.py <image>`, which delegates to decomp-toolkit's `dtk`.

## Running sample

A 3D obstacle course with a third-person camera: stairs, a wall-kick shaft, a high platform for
hover/rocket, a bridge, and gentle and slippery ramps, with FLUDD water droplets.

```
cargo run -p viewer
```

`cargo run -p viewer -- --iso "path/to/disc.iso"` extracts on first run if `assets/` is missing. It reads `assets/` (a path argument or `SMS_ASSETS` overrides it) and says on screen whether it
found it. With the rig present, a neutral mannequin is posed from the game's real animation clips
the sim is playing; without it, a plain capsule. `--demo` replays a scripted loop. WASD move, Space jump, F dive, Z ground pound, Ctrl crouch,
Shift spray, X swap nozzle, 1-4 pick nozzle, arrows orbit the camera, R respawn.

```
cargo test
cargo run -p libsms --example report
```
