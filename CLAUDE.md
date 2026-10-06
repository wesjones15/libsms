# libsms

Goal: reimplement Super Mario Sunshine's Mario locomotion and FLUDD as a game-agnostic Rust
library, so any host (Elden Ring first, like libpikmin) can drive the Sunshine character.
Sibling of `../libpikmin`; same shape and rules.

## Layout
This folder is the repo root. Sibling folders are read-only references:
- `../sms-main/`: doldecomp/sms decomp, GMSP01 (PAL). READ-ONLY spec. Built against the user's
  PAL disc (`orig/GMSP01/*.iso`, main.dol SHA-1 OK). Game code is 85% fuzzy-matched, player character code is
  96-100% (`src/Player/`, `src/NPC/`).
- `assets/` (inside this repo, git-ignored): the user's disc files, built by
  `cargo run -p libsms --bin sms-extract -- <iso>` (pure Rust: ISO/CISO, Yaz0, RARC; `libsms::extract`),
  or `python tools/extract_assets.py <iso>` for RVZ/WIA/WBFS (needs dtk). `params_mario/` =
  `params.szs:mario/*.prm` (tuning); the rest of `assets/` is all of `mario.szs` at its own paths
  (`bck/` clips, `bmd/` models incl. the `ma_mdl1.bmd` rig, `bas/` clip sound keys, `btp/` face
  textures, `watergun2/` FLUDD); `AudioRes/` is the disc's sound folder plus `msound.aaf`
  (`nintendo.szs:audi/`). Never commit it. Hosts call `Assets::discover_or_extract`, which finds a
  complete folder or extracts from the disc (`SMS_DISC`, or an image in the given folders) on first
  run, and redoes an older partial one. The older copy at `../extracted/sms` is not used.
- `../libpikmin/`: the pattern to follow (Host trait, fixed step, events, disc-parsed params).

Workspace:
- `crates/libsms/`: the sim. No Elden Ring or libsm64 types.
- `crates/libsms-assets/`: readers for drawing (J3D `.bmd` models incl. MAT3/TEV materials, GX
  textures, `.btp` patterns, skinning, which hand models each clip uses). Like libpikmin-assets.
- `crates/viewer/`: sample host. Draws the disc's Mario (body, hands, cap, FLUDD + nozzle) with a
  GLSL TEV interpreter on raw miniquad pipelines (`j3d.rs`). `SMS_SHOT=<png>` (+`SMS_SHOT_FRAME`,
  `SMS_CAM=yaw,pitch,dist`, `SMS_HIDE=body,cap,hands,fludd,nozzle`) takes a screenshot and quits.
  miniquad disables the depth *test* for pipelines without depth writes; `j3d.rs` masks writes
  with raw GL instead.
- `crates/libsms-ffi/`: C ABI stub (cbindgen header later).
- `docs/behavior-spec.md`: the spec, written from the decomp. Update it with the code.

## Rules
- Reimplement from the decomp as a behavioral spec. Do not copy code verbatim. Cite decomp
  files/functions in comments.
- The core talks to the world only through `Host` (ground/roof/wall probes, event sink).
- **The sim runs at 120 Hz** (`TMarDirector::direct`: 5-unit ticks, 600/vsync units per frame),
  not the 30 fps the game renders at. Constants are per tick; position advances `0.25 * vel`.
- Decomp units (about 1 cm; player character is 160 tall). Unit and axis conversion lives in host adapters.
- **Tuning values come from the disc, not the code defaults.** `Params::from_dir` reads the `.prm`
  files; `Default` is the code default and differs (gravity 0.8 vs 1.0, run max 45 vs 80,...).
- No Nintendo data in git: no discs, no extracted `.prm`/`.bck`/`.bmd`. Everything comes from
  `assets/`, which only the extractor fills, from the user's own disc. Hosts call `Assets::ensure`
  on first run, then ignore the disc.
- Tests that need the shipped tuning skip themselves when `assets/` is missing; the rest must pass
  on a clean checkout.
- Offline only. Never suggest running modded Elden Ring online.
- WaterGun.cpp is NonMatching in the decomp. Where a literal port contradicts how the game
  plays, make it an `Options` flag and list it under "Unverified" in the spec.

## Workflow
- Core is tested headless: `cargo test`. Scenario tests use `assets/` when present.
- `cargo run -p libsms --example report` (uses `assets/`) prints headline numbers (run speed, jump heights,
  hover/rocket) to compare against the real game.
- Toolchain: rustup stable (MSVC); `%USERPROFILE%\.cargo\bin` may not be on PATH in a shell.
- Python 3.12 is at `%LOCALAPPDATA%\Programs\Python\Python312` (bare `python` is the Store stub).
- Rebuild the decomp: `cd ../sms-main; python configure.py --version GMSP01; ninja`.

## Status
Ported and tested: disc-file loading (`prm`, `bck`, rig `JNT1`/`INF1`), input, collision contract, ground movement, slipping, the jump family and the
double/triple chain, dive/belly slide, ground pound, wall slide/kick, bump knock-downs, FLUDD tank,
Spray/Hover/Rocket/Turbo, crouch, nozzle swap. Not yet: swimming, ledge grab/hanging, bars, wires,
poles, Yoshi, carrying objects, the Spray "bomb", water-particle simulation. See the spec.
