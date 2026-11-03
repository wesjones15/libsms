# Cohesive spray and camera-steered corkscrews

Exact identity: Sunshine model-water particle **type 0**, not a JPA emitter or
Melee effect model. `sms-main/src/Player/WaterGun.cpp::TNozzleDeform::emit` uses
`NozzleDeform.prm`'s `mType` 0. The flying coverage is **timg/watermask.bti**;
`ModelWaterManager.cpp::calcDrawVtx` stretches its quads,
`drawSilhouette` adds their coverage, and `drawRefracAndSpec` shades the water.
There is no efSync/cue or EfCoData model ID for this system.

The viewer's original screen pass already merges coverage. For hosts using
ordinary transparent materials, `WaterLook` can rasterize the same airborne
type-0 quads and disc mask into a bounded camera-plane grid and emit each
occupied cell once. Close coverage merges; separated drops remain separated.
The surface carries contour alpha and continuous sphere-map UVs from the
combined coverage gradient. Depth comes from the original drops; water farther
apart than a drop's thickness does not accumulate across depth. This is a CPU
host approximation, not Sunshine's full GPU refraction pass. It is **opt-in and
off by default**, including in RoR2: head-on it collapsed the stream into one
flat card, and while moving its grid contour read as a torn sheet, so gameplay
keeps the exact droplets. Ground/wall water, other water types, Yoshi juice,
splashes and simulation stay on their existing paths.

Viewer proof (same airborne particles, side/head-on/moving):

```powershell
cargo run --release -p viewer --example water_stream_views -- assets output/water-views.png
```

The labelled PNGs compare the exact droplets with the merged surface from all
three gameplay views, using tint alone. They do not verify RoR2's shader output.
Do not commit the captures or extracted textures. Source-only unit tests cover
nearby merging, separated drops, crossing streams at different depths, the
smooth contour, and the head-on fallback.

When the opt-in merge is on, the last `sms_view_water_stream_quads` of
`drop_quads` use a white tint texture and their vertex alpha; UVs address the
existing distortion sphere map. `MarioWater` partitions these surfaces and
uses 32-bit mesh indices. Type-0 droplets move into that surface only when a
host opts in; the shipped RoR2 path keeps every droplet.

Both ordinary and Turbo corkscrews share camera steering in `jump_catch`.
This is a host extension to `MarioJump.cpp::jumpCatch`, not Sunshine behavior.
`sms_sim_set_flight_camera` supplies camera yaw and up-positive pitch separately
from FLUDD's crosshair/lock target. At roll start it captures camera pitch; later
changes contribute 35% of their delta, clamped to +/-15 degrees and smoothed at
120 Hz. Yaw chases the camera at `mRotSp` per tick. Total flight speed is retained
while vertical/horizontal components change. Releasing the stick restores the
dive and gravity. Without a host camera, existing stick steering and level flight
remain. The RoR2 driver supplies camera input for the local Mario; the
flight-camera operation is recorded separately from FLUDD aim and replayed by
puppets (network protocol 11).

How the stick holds the roll is `Options::corkscrew_hold_degrees` (host
extension). A whole stick circle still starts it, the game's own
`checkStickRotate` (all four quadrants inside `mStickRotateTime`, 0x18 samples).
Once started, the roll is held by *sweep*: `Mario::corkscrew_turning` adds each
tick's change of the stick's own angle (camera yaw taken back out, so a turning
camera adds nothing) and refreshes the roll each time it has gone that many
degrees the same way round before `CORKSCREW_GRACE` (24 ticks) runs out; turning
back on itself starts the sweep again. 60 is the default: a sixth of a turn a
fifth of a second, so a slow, gentle circle (about one a second) holds the roll
where the old per-tick whole-circle test demanded a fast, full-tilt one. 360 is
the strict reading (a whole circle inside one grace window); 0 holds it while the
stick is merely out of its dead zone.

Tightly cropped generated skill portraits explicitly disable cohesive coverage
using `sms_view_set_water_cohesive`; they keep their approved mask-edge framing.
The generated portrait images are unchanged. Gameplay water uses the unified mesh.

ABI **40** added the stream/flight exports and network protocol **10** the flight-camera
replay. ABI **42** added the flower world's item kind 3 (a buried sambo's stand-in flower);
ABI **43** added `sms_view_set_water_speed`; ABI **44** adds
`SmsTuning::corkscrew_hold_degrees`, growing `SmsTuning` (so it is not 43: a plugin built
against 43's 56-byte struct would read past it). ABI **41** adds
`sms_sim_equip_nozzle` (the radial menu's one-shot nozzle-box
pickup feedback) beside the silent state-sync `sms_sim_select_nozzle`, with no struct
layout change; network protocol **11** adds the equip-operation replay.
The nozzle-sound hotfix adds `sms_sim_switch_nozzle` for a player's quick swap:
`WaterGun.cpp::changeBackup`'s 0x0812 out / 0x0811 back selection cue, without pickup.
Silent state refresh still uses `sms_sim_select_nozzle`; the ABI layouts and network
protocol stay unchanged. Both DLLs must be updated together for the new export.
Install matching `sms.dll` and `MarioTest.dll`. Headless scenarios exercise both
corkscrews turning both ways, relative pitch, its cap, speed and gravity recovery.
The managed/native portrait check verifies the stream partition and quad counts.

RoR2 refraction, camera feel and multiplayer are **not yet played**.
