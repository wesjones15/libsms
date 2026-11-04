# libsms

Goal: reimplement Super Mario Sunshine's Mario locomotion and FLUDD as a game-agnostic Rust
library, so any host can drive the Sunshine character. First host: Risk of Rain 2 (2023 build),
in `../ror2-mario/` (BepInEx plugin over `libsms-ffi`).
Sibling of `../libpikmin`; same shape and rules.

## Layout
This folder is the repo root. Sibling folders are read-only references:
- `../libcommon/`: shared Rust texture wrapping/sampling; a sibling Cargo path dependency compiled into sms.dll.
- `../sms-main/`: doldecomp/sms decomp, GMSP01 (PAL). READ-ONLY spec. Built against the user's
  PAL disc (`orig/GMSP01/*.iso`, main.dol SHA-1 OK). Game code is 85% fuzzy-matched, player character code is
  96-100% (`src/Player/`, `src/NPC/`).
- `assets/` (inside this repo, git-ignored): the user's disc files, built by
  `cargo run -p libsms --bin sms-extract -- <iso>` (pure Rust: ISO/CISO, Yaz0, RARC; `libsms::extract`),
  or `python tools/extract_assets.py <iso>` for RVZ/WIA/WBFS (needs dtk). `params_mario/` =
  `params.szs:mario/*.prm` (tuning); the rest of `assets/` is all of `mario.szs` at its own paths
  (`bck/` clips, `bmd/` models incl. the `ma_mdl1.bmd` rig, `bas/` clip sound keys, `btp/` face
  textures, `watergun2/` FLUDD); `AudioRes/` is the disc's sound folder plus `msound.aaf`
  (`nintendo.szs:audi/`); `npc/monte*/` are the Pianta models, clips and accessories gathered
  from every stage archive, `params_npc/` the NPC tuning (`params.szs:npc/`), `scene/mapobj/fruit*`
  the fruit models and `scene/mapobj/eggyoshi*` Yoshi's egg (stage archives), `yoshi/` Yoshi
  (`yoshi.szs`), `enemy/hamukuri*` the Strollin' Stu and `scene/mapobj/coin*` the coins (stage
  archives), `params_enemy/` the enemies' tuning (`params.szs:enemy/`), `particle/` the
  common particle effects (`particle.szs`, `.jpa`) and `game_6/` the HUD's layout and pictures
  (`game_6_en.szs` on PAL, `game_6.szs` elsewhere; USA verified 2026-10-13: same 1486 files bar the HUD text screens). Never commit it. Hosts call `Assets::discover_or_extract`, which finds a
  complete folder or extracts from the disc (`SMS_DISC`, or an image in the given folders) on first
  run, and redoes an older partial one. The older copy at `../extracted/sms` is not used.
- `../libpikmin/`: the pattern to follow (Host trait, fixed step, events, disc-parsed params).

Workspace:
- `crates/libsms/`: the sim. No Elden Ring or libsm64 types.
- `crates/libsms-assets/`: readers for drawing (J3D `.bmd` models incl. MAT3/TEV materials, GX
  textures, `.btp` patterns, `.blo` screens, skinning, which hand models each clip uses). Like libpikmin-assets.
- `crates/viewer/`: sample host. Draws the disc's Mario (body, hands, cap, FLUDD + nozzle) with a
  GLSL TEV interpreter on raw miniquad pipelines (`j3d.rs`). `SMS_SHOT=<png>` (+`SMS_SHOT_FRAME`, or `SMS_SHOT_TIME=<seconds of sim time>`, which a rocket launch needs; `SMS_NOZZLE=<1-4>`,
  `SMS_CAM=yaw,pitch,dist`, `SMS_HIDE=body,cap,hands,fludd,nozzle`) takes a screenshot and quits;
  `SMS_R=<0-255>` holds the trigger, `SMS_HANG=1` starts hanging from a ledge, `SMS_HOLD=1`
  starts carrying fruit, `SMS_YOSHI=1` rides Yoshi (`=2`: he stands by), `SMS_B=<frame>` presses
  B once, `SMS_HIT=<frame>` hurts Mario then (K in play), `SMS_FOCUS=yoshi` points the camera at Yoshi, `SMS_FLOWER=<frame>` draws
  the sambo flower (all three clips), the tower and the head for reference captures (the flower with its
  `flower_orange.bmt` material table unless `SMS_FLOWER_NOTABLE=1`), `SMS_PROF=1` prints where each second's
  frame time went (sim, scene, water). No GL state queries per frame: `glGetIntegerv` waits for
  the GPU (it cost 5 ms a frame while spraying). FLUDD and the nozzle pose from `Mario::fludd_look`, and the drawn
  muzzles go back through `Mario::set_emitters`. It draws the JPA particle effects (the nozzle mist, smoke, bursts, exhaust) through `libsms-present`'s `View` (`fx.rs`, macroquad quads; refraction batches and the water-boost cone model aren't drawn); the `nozzle` example renders them headlessly to a PNG, and the nozzle tests in `view.rs` check them (set `SMS_ASSETS` or they skip). Water is `libsms::water` drawn like the game: coverage into the
  window's alpha against the scene's depth, then a refracting screen pass (`water.rs`); fruit is `props.rs`; a
  walking Strollin' Stu for Yoshi to eat is `stu.rs` (a stand-in, no enemy AI; walking into
  it hurts Mario); coins are `coin.rs`; the damage flash is `Look::fog`;
  Yoshi, his tongue and the egg are `yoshi_model.rs` (`Mario::yoshi_look`, `Egg::look`; the posed
  mouth goes back via `Mario::set_yoshi_mouth`).
  miniquad disables the depth *test* for pipelines without depth writes; `j3d.rs` masks writes
  with raw GL instead.
- `crates/libsms-present/`: presentation for hosts that draw Mario themselves: `view` (the
  disc's body, hands, cap, FLUDD and nozzles posed from the sim like the viewer's
  `mario_model.rs`, CPU-skinned into one vertex buffer in Mario's model space; `set_vr_arms`
  adds each forearm and glove cut from the body by skin weight, keyed `VR_ARM_KEY` + side and
  posed in its own arm space for a VR host's controllers, `examples/vr_arms.rs` in the viewer
  captures them), `tev` (each
  material's TEV run on the CPU per texel and baked into one RGBA texture, at full diffuse
  light with no specular: Mario's materials light themselves in TEV with a toon ramp read at
  the lit colour (`GX_TG_SRTG`) plus a highlight; env-only materials bake over the env map's
  space), `particles` (the disc's `.jpa` effects run as JParticle runs them and handed out as
  quads: Mario's movement dust, stars, smears, slide spray, stomp bursts and super pound flames;
  hosts pass each tick's events to `View::events`, which the FFI does even without sound),
  `screenfx` (the losing screens: the "TOO BAD!" letters of a miss and the GAME OVER wipe, from
  `ConsoleStr.cpp` processMiss and `hx_wiper.c` Hx_GameOver, as sprites and a per-pixel overlay for a host to
  draw; their pictures are the optional `assets/wipe/` and the HUD's `big_tx_*`, `sms_view_screenfx_*`;
  `--example screenfx` draws them), `coins` (the yellow and blue coin counters and Mario's lives banner, icon, "x" and digit pictures as the HUD lays them out, up to
  seven digits; `sms_view_coins_*`; `--example coins` draws them), `hud` (FLUDD's water gauge drawn as `TGCConsole2`
  draws it into one RGBA image), `life` (Mario's life meter, the Shine Sprite sun: a hub and eight
  rays in two layers, lit by health share, a cyan shield after them; `LifeMeter::load(dir, 10, false)`
  fills the empty arc on top with two more rays for a sun alone; `sms_view_life_*`; `--example life`
  draws it, `--example blo` in libsms-assets lists a screen's panes) and `audio` (the viewer's old `sound.rs`: the game's
  sounds on cpal, music optional). `Look::Shine` (`sms_view_new_look(.., 3)`) is the Shine Sprite as a prop: `shine.bmd` posed by
  `shine_float` / `shine_demo_shine_get` (`sms_view_shine_update`), its glow (`ms_shine_*.jpa`, in the `sms_view_fx_*` readers) and `democam` (the Shine Get
  camera path, `sms_view_camera_shot`); the "SHINE!" letters are `screenfx` kind 2. Their files are `OPTIONAL_EXTRACTS` (`sms_extract_extras`).
  `cargo run -p libsms-present --example bakes -- <dir>` writes every bake to PNG; `--example hud
  -- out.png` draws the water gauge at a few levels.
- `crates/libsms-ffi/`: C ABI (`sms.dll`): `SmsHost` callbacks (ground/roof/walls/events/enemies),
  `sms_sim_new/step/state/teleport/free`, and `sms_view_*` (models, bakes, per-frame draws;
  its structs match libpikmin's `pik_view_*` so `../ror2-common/` serves both). Pads may carry
  a world-space move direction instead of a stick. The sim handle also runs FLUDD's water
  (`libsms::water`, through the same callbacks; its enemy hits come back as `WaterHit` events)
  and takes host tuning (`sms_sim_set_tuning`: jump height, A buffer, chain grace, move speed,
  spray reach, solid enemies, nozzle cycling, stomp bounce, rocket height, pound jump, stomp jump,
  short hop, unlimited water; `Options`), host limits (`sms_sim_set_limits`, `libsms::Limits` in
  `limits.rs`: mid-air FLUDD uses, mid-air jumps, spray rate and limit, hover time, Turbo speed,
  charged wet slides; all off by default, so the core stays the game's), `sms_sim_launch` (jump
  pads, knockback), `sms_sim_boost_jump`, `sms_sim_add_water`, `sms_sim_refill` and an optional `water` callback
  (shallow water's surface, `Host::water_level`). `SmsState::pounding` says when to draw the super pound's flames; a `GroundPound`
  event's value is the drop. Each step the FFI tells Mario about water on the ground for the
  wet belly slide; the view builds the water's quads
  (`libsms_present::water`, `sms_view_water_*`), the particles' (`sms_view_fx_*`) and the
  water gauge's image (`sms_view_hud_*`). Bump `SMS_ABI_VERSION` on any layout change
  and keep `sms_struct_sizes`' test in step; `../ror2-mario/MarioTest/Native.cs` mirrors it.
- `crates/libsms-audio/`: device-independent PCM engine (`Engine`), JAI game layer
  (`SoundSystem`) and `.bas` cue playback (`animation::AnimationPlayer`).
  `libsms-assets::sound` loads banks, waves, sequences, streams, names and animation tables.
  See `docs/audio.md` for the host contract, tests and remaining fidelity gaps.
- Viewer audio (`libsms_present::audio`) uses CPAL, samples animation/events after each
  120 Hz tick with `Sim::advance_observed`, and runs JAI frames at 30 Hz.
  It starts Delfino Plaza music; **M** cycles extracted BGM tracks, including off.
  Missing sound data or an unavailable output device leaves a silent, usable viewer.
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
double/triple chain, dive/belly slide, ground pound, wall slide/kick, bump knock-downs, FLUDD tank
(refilled from shallow water; the HUD gauge),
Spray/Hover/Rocket/Turbo, crouch, nozzle swap, FLUDD/nozzle/pumping-arm animation and sound cues,
water particles, ledge grab/hanging/shimmy/climb, grabbing/carrying/putting/throwing,
Yoshi's egg, riding, flutter, tongue and juice, damage, health and coins, stomping and
pounding enemies, water hitting enemies, catching fire (`FIRE_DOWN` bounce, `FIRE_DASH` panic run, `Mario::ignite`; the decomp never enters the run, so `Options::fire_panic_run` is the host way in), the Shine Get pose (`shine.rs`: `Mario::get_shine`, the `WinDemo` status, `Event::ShineTouch` / `ShineDemo`). Not yet: swimming, bars, wires, poles, fences,
the Spray "bomb". See the spec.
