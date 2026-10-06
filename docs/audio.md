# Sound data and playback

`libsms-assets::sound` reads the user's extracted `AudioRes/` banks, waves, sequence archive,
sound table and names, plus AFC streams and `.bas` animation sound tables.
`SoundData::load` needs the extracted assets directory; wave samples are decoded lazily.
No game data is embedded in either crate.

`libsms-audio::SoundSystem` plays effects through `se.scom`, BGM sequences and decoded streams,
and renders interleaved stereo PCM at the host's output sample rate.
Call `frame()` at 30 Hz, and `render()` whenever the output device needs samples.
`play`, `play_se`, `mario_voice`, `play_bgm` and `play_stream` are the game-facing entry points.
Keep `SoundHandle`s to move, adjust, stop or refresh sounds.
`refresh` renews a held sound without restarting its sequence; stale handles return false.
Initial sound commands remain queued until the SE sequence has created its child tracks.

## Animation cues

`TMario::initModel` in `Player/MarioDraw.cpp` loads `bas/ma_<stem>.bas` beside
`bck/ma_<stem>.bck`; `setAnimation` selects the corresponding sound table.
`MAnmSound::animeLoop` in `MSound/MAnmSound.cpp` and `TMario::soundProcess` in
`Player/MarioSound.cpp` supply the frame-controller time and speed.
The viewer uses the sim's clip stem and passes `anim.rate * 0.5` as that speed.

`animation::AnimationPlayer` follows `JAIAnimeSound`'s frame crossings, direction filters,
exact-loop selection (flag 8), eight active slots, held windows, end-frame stop (flag 16),
and stop-on-animation-change (flag 4).
Sound ids without bits `0xC00` need refreshing while their window is active.
The sound table's AnimeSpeed switch enables continued speed-based pitch and volume changes.
Mario voice-category keys use `SoundSystem::mario_voice` and its variant selection.
Call `reset` when selecting a new clip, and `stop` when removing or respawning the actor.
Sample at least once per animation wrap; skipping an entire loop cannot be recovered from
two wrapped frame values. The viewer observes every 120 Hz simulation tick.

## Sample viewer

`crates/viewer/src/sound.rs` keeps device setup separate from scene/event routing.
CPAL's callback renders the sound system behind a mutex, supports f32/i16/u16 output,
downmixes stereo for mono devices and leaves extra surround channels silent.
The listener's view matrix and all emitter positions use the original centimeter-like units,
not the renderer's meters. The camera is posted each display frame.

The sim reports the decomp's own `startSoundActor` call sites as `Event::Sound` (FLUDD's
water, triggers, pumping, nozzle selection; ledge and carry voices). Held ids (no `0x800` bit)
arrive every tick; the viewer keeps one handle per name and refreshes it, like the game's
once-a-frame request (`MSSeGate_Continuous`). `startSoundActorWithInfo` values go through
`SoundSystem::play_with_info`, which applies `modse`'s gate and pitch/volume curves
(`JALSeModPitFunk`/`VolFunk`, registered in `MSoundSE`). Names starting `MSD_SE_MV` go to
`mario_voice`. Water particle events add `WT_GND` splashes, `WT_INTO_WATER` and `PO_WT_DRY_UP`.

Takeoff effects, running and animation-driven landings use the `.bas` keys.
Jump voices are separate: `TMario::setStatusToJumping` in `MarioMove.cpp` selects small,
medium or large calls for each jump kind, and an action call for dives.
Explicit sim events add these voices, hard landings, ground-pound impact, common hard-wall
impact, FLUDD nozzle selection, Rocket/Turbo triggers and held Spray/Hover/Turbo water sounds.
Sources are `MarioSound.cpp`, `MarioJump.cpp`, `WaterGun.cpp` and `MSound::getWallSound`.
The viewer starts `MSD_BGM_DOLPIC`; M cycles named BGM sequences and then off.
Respawn clears the animation cursor and stops the tracked animation/water sounds.

## Verification

Run `cargo test --workspace --offline` for parser, timing, sim and headless output checks.
Disc-backed tests skip when assets are absent.
The asset tests parse all 123 extracted `.bas` files and validate ids against the sound table;
122 have Mario's `ma_` prefix. `wg_noz_para_open.bas` contains id `0x803`, which has no name.
Viewer tests render jump cues and isolated jump-event voices to nonzero finite PCM, verify held-water handle reuse and
release, and check music selection/off without opening a device.
For a real output-device check (plays a short sound), run:

```
cargo test -p viewer --offline output_device_smoke -- --ignored --nocapture
```

## Remaining fidelity work

- Sound balance, sequencing and effects still need comparison by ear against the game.
- The sim's existing approximate clip choices also affect which animation sounds are selected.
- The sample world's wall event lacks impact speed/material, so it uses the common hard hit.
- Surface-dependent footsteps, water/pollution variants, FLUDD charge/pressure modulation,
  environmental effects and unimplemented character states are not fully routed.
- The existing sim clock treats mirror clips as ordinary loops; the sound player cannot restore
  missing reverse motion. A host supplying a reverse clock can use reverse-direction cues.
