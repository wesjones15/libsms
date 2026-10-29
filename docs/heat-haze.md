# Sunshine heat haze

Exact source: `sms-main/src/Map/Shimmer.cpp::TShimmer::load`, `loadAfter`,
and `perform`. This is an indirect scene-refraction model, with no particle
emitter or efSync cue. The typed `Shimmer` record in `scene/dolpic1.szs`'s
`map/scene.bin` names **ShimmerLow**. Its files are
`mapobj/shimmerlow.bmd` and `.btk`; material `_mat1_2` uses texture 0,
`B_kagero_s` (64x64 I8), as its indirect map. Texture 1, `indirectdummy`, is
replaced by the scene's `TScreenTexture`.

The original sheet is at camera-space Z -2425. It has a 2x5 texture scale and
a 600-frame looping BTK scroll. Its indirect matrix is read from MAT3, with
STU = alpha/blue/green and bias mask 7. The source screen texture is half the
640x448 render target (`MarioUtil/ScreenUtil.cpp::TScreenTexture::load` and
`System/Resolution.cpp`), so indirect texel offsets convert against 320x224.
No invented noise or sine-wave animation is substituted.

`libsms-present::heat_haze::HeatHaze` reads these files from the player's own
disc cache. The optional extractor adds `heat_haze/shimmerlow.bmd` and `.btk`.
The native bridge is ABI 32: `sms_heat_new/free/texture/frame`. Frame output
is eight floats: source depth, normal-map gain, and two three-float UV rows.
Short buffers are rejected without partial writes.

The RoR2 adapter shares MarioWater's existing scene-refraction material, with
its own magnitude independent of `Look.WaterRefraction`. It uploads the map
once as **linear** vector data. An asset-derived normal gain lifts the largest
vector to length 0.8 to survive the host shader's near-flat-normal discard;
the magnitude compensates by the reciprocal gain. A camera-facing
sheet at the source depth (24.25 m for a 1.6 m Mario) uses scene depth to leave
nearby geometry clear, and is submitted only to its owner's scene camera.
HUD, portrait and other-player cameras are excluded. `Look.HeatHaze=false`
stops submission immediately. Every Mario skin shares this path.

Host adaptations: fit the sheet to the current camera FOV/aspect, play BTK
frames at the presenter's 60 frames/s, and enable it for Mario's presentation
throughout RoR2 stages. Sunshine's cap/shadow/pool/map-number gating has no
direct counterpart in RoR2 and is not asserted as reproduced. The backend is
RoR2's refraction shader; final GPU appearance still requires a playtest.

Headless source proof (no desktop window):

```powershell
cargo run --offline --release -p viewer --example heat_haze -- <Sunshine.iso>
```

It audits the typed source record, extracts only the identified assets, and
writes `target/heat-haze-shimmerlow.png` with a source-label companion file.
Top row: disabled. Bottom: 0/0.25/0.5 seconds. The foreground block stays
sharp while the background grid refracts. Captures and source assets remain
local and uncommitted.

Checks cover source texture identity/depth/scroll, periodicity, the host's
normal-discard threshold, and the C ABI's buffer bounds. Mario's host checks
also cover presentation ownership through body loss, spectator cameras,
Remote Operation drone control, respawn, user replacement, and run cleanup.
