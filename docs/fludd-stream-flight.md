# Cohesive spray and camera-steered corkscrews

Exact identity: Sunshine model-water particle **type 0**, not a JPA emitter or
Melee effect model. `sms-main/src/Player/WaterGun.cpp::TNozzleDeform::emit` uses
`NozzleDeform.prm`'s `mType` 0. The flying coverage is **timg/watermask.bti**;
`ModelWaterManager.cpp::calcDrawVtx` stretches its quads,
`drawSilhouette` adds their coverage, and `drawRefracAndSpec` shades the water.
There is no efSync/cue or EfCoData model ID for this system.

The viewer's original screen pass already merges coverage. For hosts using
ordinary transparent materials, `WaterLook` now rasterizes the same airborne
type-0 quads and disc mask into a bounded camera-plane grid and emits each
occupied cell once. Close coverage merges; separated drops remain separated.
The surface carries contour alpha and continuous sphere-map UVs from the
combined coverage gradient. Depth comes from the original drops; water farther
apart than a drop's thickness does not accumulate across depth. This is a CPU
host approximation, not Sunshine's full GPU refraction pass. Ground/wall water,
other water types, Yoshi juice, splashes and simulation stay on their existing paths.

Viewer proof (same airborne particles, side view, water frames 8/18/30):

```powershell
cargo run --release -p viewer --example water_stream -- assets output/water-stream.png
```

The labelled PNG compares separate overlays with unified coverage, using tint
alone so segment seams are visible. It does not verify RoR2's shader output.
Do not commit the capture or extracted textures. Source-only unit tests cover
nearby merging, separated drops and crossing streams at different depths.

The last `sms_view_water_stream_quads` of `drop_quads` use a white tint texture
and their vertex alpha; UVs address the existing distortion sphere map. Other
drop quads use the original mask. `MarioWater` partitions these surfaces and
uses 32-bit mesh indices. The stream receives the configured water tint once,
without the legacy per-drop opacity reduction.

Both ordinary and Turbo corkscrews share camera steering in `jump_catch`.
This is a host extension to `MarioJump.cpp::jumpCatch`, not Sunshine behavior.
`sms_sim_set_flight_camera` supplies camera yaw and up-positive pitch separately
from FLUDD's crosshair/lock target. At roll start it captures camera pitch; later
changes contribute 35% of their delta, clamped to +/-15 degrees and smoothed at
120 Hz. Yaw chases the camera at `mRotSp` per tick. Total flight speed is retained
while vertical/horizontal components change. Stick circles still sustain the
roll; releasing restores the dive and gravity. Without a host camera, existing
stick steering and level flight remain. The RoR2 driver supplies camera input
for the local Mario; the flight-camera operation is recorded separately from
FLUDD aim and replayed by puppets (network protocol 10).

Tightly cropped generated skill portraits explicitly disable cohesive coverage
using `sms_view_set_water_cohesive`; they keep their approved mask-edge framing.
The generated portrait images are unchanged. Gameplay water uses the unified mesh.

ABI **40** adds these exports with no struct layout change; network protocol **10**
adds the flight-camera replay operation.
Install matching `sms.dll` and `MarioTest.dll`. Headless scenarios exercise both
corkscrews turning both ways, relative pitch, its cap, speed and gravity recovery.
The managed/native portrait check verifies the stream partition and quad counts.

RoR2 refraction, camera feel and multiplayer are **not yet played**.
