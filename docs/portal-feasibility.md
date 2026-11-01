# Mario statue entrance: viewer proof

The viewer proof is approved and connected to Mario's RoR2 stage-one pod. The adapter is build/boundary checked; in-game testing is pending.

## Identified assets

- `dolpic1.szs:map/map/gate/05_gate01.bmd`, `.btk`, `.brk`: the Bianco Hills M gate. Proved by `sms-main/src/MoveBG/ModelGate.cpp`, `gateMActorNames[0]`, `TModelGate::loadAfter` and `perform`.
- One material, seven TEV stages, six texgens, six texture maps. Its indirect stage reads coordinate/map 4, `P_ms_indwp1_ia`, through the disc's 0.6 * 2^-5 matrix. Its M mask is `P_gate_msk_m2`. It is a model material, not a particle substitute.
- `map/map/map.bmd`: the Pianta sculpture is shape 23, `_king_storn_1`; the pedestal is separate bounded geometry (shape 3, `_m_white_wall0`) in the same map. Both are retained by `plaza::statue_batches`. The same bounding box also catches the map's baked statue shadow (shape 9, `_m00kage_1`, a flat mesh 3 cm above the plaza floor) and one plaza floor triangle (shape 14, `_m00yuka`); those are decals, not the pedestal, so `statue_batches` drops ground-level horizontal triangles. They drew as a dark plate and a floor chip hanging off the base, and their spread made the RoR2 pedestal collider 10.3 x 11 m instead of the pedestal's own 8 x 8 m. `cargo run --release -p libsms-present --example statue -- assets ../output/statue` prints the per-batch bounds the collider uses and renders the selection.
- Portal activation rays: `ms_gatewind_a/a2/a3/b.jpa`, cues 0x131–0x134; spray hits: `ms_gatehit_a/b.jpa`, cues 0x1DD–0x1DE. Source: `ModelGate.cpp::perform/receiveMessage` and `System/Particles.hpp`. The interactive preview draws the activation rays; spray-hit particles are unnecessary for its button-triggered opening.

## Smooth stage-one reveal and return (modern)

The exact transition is the `05_gate01.bmd` material driven by `05_gate01.brk`,
with `05_gate01.btk` continuing the texture distortion. It is not an effect
model or particle emitter and has no numeric JPA/efSync cue. `ModelGate.cpp`
`loadAfter` sets `unkD4 = 0.1`, `unkD8 = 0.02`; `perform` applies
`unkD0 * BRK.end` as the colour-animation frame. The button-triggered host keeps
the authored curve, slows its reveal to 0.8 s, and retains the source closing
rate at 30 Hz (roughly 1.67 s).

The former host cache compressed these to 0.18 s and 0.14 s, which read as image
swaps. Cache revision `SMSPOD05` / `entrance-v5-*` keeps the authored BRK ramp and
holds the Plaza for two seconds after the normal 1.75-second control handoff.
The return starts at 3.75 s and completes near 5.42 s. This extra hold is a host
presentation choice. Mario's reveal, spin, landing, control handoff, cleanup and
the portal's hidden/offscreen sampling rules retain their previous timing.

The cache hashes all transition timings and source files, so an older fast-fade
cache cannot be reused. It retains 512-to-256 antialiasing, the original indirect
map, M mask and existing disc-rendered Plaza still. Idle and return sample the
same elapsed texture-animation clock when the return finishes.

Viewer proof: `cargo run --release -p viewer --example portal_transition --
<assets> <output>`. When the graphics dependency build is unavailable, the
identical source under `crates/viewer/examples/` runs through the device-free
entry point: `cargo run --release -p libsms-present --example portal_transition
-- <assets> <output>`. It renders the same antialiased cache frames and writes
source/identity metadata. Local labeled frames and animation are under
`../output/mario-portal/smooth-modern/` (disc derivatives; never committed).

Native boundary checks verify the Plaza remains visible after control returns,
the return includes intermediate BRK frames, eventual idle pixels match direct
sampling, early slope handoff does not freeze the return, and Mario/Luigi's rider
geometry and effects retire on schedule. In-game verification remains manual.

## Earlier viewer demonstrations

`cargo run --release -p viewer --example portal -- assets ../output/mario-portal`

The headless viewer produces several BTK frames at three BRK activation levels. It preserves the authored distortion and combines it with a replaceable still through the gate's Y/U/V TEV stages. The M mask and still have independent orientation. Idle shows the swirling colours; activation reveals the still.

The still is rendered from the user's disc. It includes the scene.bin placements of palms, sewer covers, the clean Shine Gate emblem and DokanGate pipe effects. `TMonumentShine::initMapObj/hitByWater` defines the clean emblem's K0 alpha of zero. The pipe geometry is in the main map; respecting its diffuse colour and zero specular base avoids turning its red paint white.

Outputs under `../output/mario-portal/` are local, extracted-data derivatives and must not be committed or packaged. The frame sheet's columns are BTK frames 0, 30, 60, 90; rows are activation 0, 0.5, 1.

## Interactive statue entrance

`cargo run --release -p viewer --example portal_entrance -- assets`

X replays activation immediately, F frames the full statue, C frames the portal, arrow keys orbit, Space pauses, and Escape closes. The default demonstration loops. `--capture <folder>` saves a deterministic 30 fps sequence.

The full statue and pedestal carry the exact gate. The gate bake retains the original material's fractional alpha and applies its GX source-alpha/zero-destination blend to preserve the dark edge, then exposes ordinary coverage alpha to hosts. Its baked row convention is explicitly matched when placed on the authored quad, keeping both the M and the Plaza image upright.

`Player/MarioAutodemo.cpp::TMario::warpOut`, successful-return branch (`mStatusArg & 0xff == 2`), selects `demo_gate_out_appear_get`, `demo_gate_out_rolling_get` and `demo_gate_out_get2`. The first clip contains the transition: central-joint scale remains about 0.00001 through frame 59, reaches about 0.305 at frame 71, and reaches about 0.997 at frame 89. The ordinary 180-tick delay at 120 Hz gives 1.5 seconds for appearance. The preview uses the Bianco return point from `dolpic1`'s `MarioPositionObj` (relative to the gate: 0,250,308.53), then the zero-speed vertical fall from `MarioPhysics.cpp::jumpProcess`, with disc gravity. The former invented forward jump arc has been removed.

`Player/MarioParticle.cpp::warpOutEffect` case 0 supplies `ms_m_gout_gate.jpa`, cue 0x40. It starts with appearance, before the rolling/jump phase. The shared particle engine preserves the wind emitters' authored delays (0, 25, 50 and 0 particle frames). The viewer also follows `MarioCap.cpp::TMarioCap::perform` and `MarioDraw.cpp` for the landing's `ma_cap3` and `ma_hnd4r` changes, so Mario can hold and replace his cap.

The colored Mario particles are the exact **particle emitter 0x40**, not a model effect. Its FLD1 fields are Newton (3), magnet (2), and vortex (4). `libs/JSystem/src/JParticle/JPAField.cpp` proves their attraction, radial falloff, tangent direction and lifetime fades. Supporting those fields in the shared particle renderer restores the curling stream and convergence. Its static randomized BSP1 palette is baked into a single atlas with source-alpha blending, preserving each colored ball's occlusion instead of adding overlapping layers into white.

`System/EmitterViewObj.cpp::TMarioParticleManager::perform` runs `SMSGetAnmFrameRate()` particle calculations per display frame; `System/Application.cpp` defines that rate as 60/display Hz. The entrance emitter therefore uses the shared scheduler's existing 2x playback control to match the BCK's 60 Hz clock. Its 30-frame emission plus 60-frame maximum particle lifetime ends at the 89-frame appearance clip's completion. The legacy shared scheduler and unrelated effects retain their existing timing. The reference sequence's particles disappear as Mario finishes forming, before he rolls and lands. A disc-backed boundary test checks convergence, alpha blending and expiration at this handoff.

Local reference `https://i.redd.it/m8q9ktlfj2we1.gif` shows the idle M's rippling surface and dark edge. It contains no Mario exit sequence.

## RoR2 adapter

`view::Entrance` shares the approved timeline with the interactive viewer and supplies statue, portal, Mario and particles through the existing model/animated-texture/FX readers. `sms_view_entrance_new/update/bounds` add an opaque entrance view without changing existing C struct layouts. The quad retains the established interior UV mapping; the fixed M mask has its own horizontal correction for the host's front view. The shared CPU presenter now includes the landing's held-cap/right-hand models.

`ror2-mario/MarioStatuePod.cs` clones Mario's preferred pod (the stock SurvivorPod if unspecified) and retains its animator, animation-event fall/crash sounds, seat and network lifecycle. `MarioPodDescent` inherits the original descent; the landed interaction starts `MarioPodExit`, which keeps Mario seated/protected while native presentation plays. The camera faces the M. X plays the existing once-per-run sting; Mario says the existing jump woohoo when the model starts appearing. After the cap/landing clip, the server ejects him at the vanilla exit point and the ordinary Mario driver takes over. A short visibility bridge avoids a Commando flash or duplicate Mario. The statue stays behind with a pedestal collider bounded from its native geometry.

The custom presentation is gated to stage one. Missing assets, disabled model drawing or an unavailable renderer retain the vanilla pod path. The first-run optional extraction includes the gate, four wind emitters, Plaza map/scene and all five still-shot prop models from the user's own disc; no extracted data is packaged. `portal_assets` lists the exact archive paths for inspection.

`ror2-mario/tests/PortraitCheck` exercises the real DLL's idle/activation/reveal/landing, instance-key uniqueness, animated texture versions, particle expiration, collision clearance, finite geometry and completion. In-game descent alignment, camera framing, sound events and input/network lifecycle are **not yet played**. The GPU viewer's general J3D interpreter still lacks this indirect material; both hosts use the shared CPU TEV bake on the portal quad.

## Revised game profile after the first playtest

The full Sunshine sequence remains in `Entrance::load/update`, the original C entry point and `portal_entrance` viewer. RoR2 now selects `sms_view_entrance_fast_new`: 0.8 seconds of particle/reveal animation, the original rolling BCK at its native 60 fps over a gentler 0.8-second fall, then 0.15 seconds of ordinary `jmped` landing and handoff. The fall duration was adjusted in the live viewer to the user's approved pace. It is host choreography; the archived original still uses the source's fall physics and cap demo. The existing wahoo plays at materialization; Luigi retains his voice pitch and changes only red particle palette entries to green. His HSD retarget receives the appearance BCK's centre scale explicitly so he does not appear early.

The portal returns to swirling colours after handoff. `portal_cache` pre-renders the exact TEV on the asset worker: the complete idle BTK loop at 15 fps and the short opening/interior/closing sequence at 30 fps. Revision 4 renders at 512 square and stores 256-square RGBA with antialiased coverage (the earlier cache was 128 square). The ignored cache fingerprint includes all fourteen source files and playback settings. Runtime selects a cached frame and copies pixels into one stable texture; no TEV shader interpretation runs on the game thread. Rigid statue/gate geometry is cached and uses the shared retained-pose flag after its initial upload. Idle and spent views never pose Mario/Luigi. `MarioPerformance.Part.Portal` measures the actual adapter when performance logging is enabled.

Measured with `entrance_bench` on the same machine: the former native idle/post-exit updates averaged about 14.6 ms, with maxima around 31–36 ms. The revised cached idle/post-exit updates averaged about 0.001 ms (maximum 0.004 ms), with zero post-exit vertex uploads. Active native exit updates averaged 0.058 ms, maximum 0.333 ms. These are native CPU timings, not measured RoR2 frame rates. First-time cache rendering runs on the worker; subsequent native view load was about 265 ms.

`MarioStatuePod` forces off all original pod renderer types except the resized descent fire, stops other original particle systems without disabling animator/audio objects, and removes stock pod collision after ejection while retaining the native pedestal collider. It does not rely on `Renderer.enabled`, which can be restored by animation. Vanilla fall/crash audio events remain attached to the original animator.

The original exit sound is the nine BAS cues in `ma_demo_gate_out_appear_get.bas`: `MSD_SE_MA_Y_ROLL9` (0x1963) through `MSD_SE_MA_Y_ROLL` (0x195B), at the file's authored frame keys. `MarioDraw.cpp` loads the BAS beside the BCK; `JAIAnimeSound::playActorAnimSound` is the cue mechanism. The game profile compresses their timing with the particles while retaining authored sample pitch. It does not substitute the unrelated `MSD_SE_MA_WARP_EX` stage-change sound. A device-free test verifies all nine IDs and audible finite PCM.

`portal_fast --capture <folder>` renders the shared native presenter, including the real Luigi HSD mesh, green particle palette, spin, landing and return to colours. The C# boundary test exercises both original and revised profiles, Mario and Luigi, retained geometry, particle expiration, handoff and exact idle-color restoration. The revised RoR2 build still needs a playtest for renderer suppression, audio balance and actual FPS.

## Descent fire and late rider selection

The exact stock fire is Unity's mesh-particle emitter `SurvivorPod/Base/mdlEscapePod/EscapePodArmature/Base/Flames`, ParticleSystem path ID **-470276994286075641** in `ror2-base-survivorpod_assets_all_1195400eaf9e5c72fc5bc5dcfd6ecd25.bundle`. Its renderer is **-4483200324324671225**, rendering `donut5Mesh` (**-6476590250452040025**) with `matEntryBurn` (**-6514494002267390535**). This is a Unity mesh-particle system, so there is no GameCube `efSync` cue. It is not replaced with a JPA emitter. The separate `FireDonut` object is inactive and has no binding in `EscapePodArmature|InitialSpawn`; it is not the active descent emitter.

`tools/export_pod_fire.py <StandaloneWindows64 folder> <ignored output>` exports the exact mesh and four textures locally for `portal_fast --descent-fire <output> --capture <frames>`. The source-labelled captures show multiple animated emitter frames with a 3.5x scale. This viewer approximates the cloud shader and particle pivot; its visible fire height is not proof of Unity placement. The user's in-game screenshot showed that the former -950 cm emitter correction detached the effect far below the statue. That correction is removed: the emitter is centered at the actual native pedestal minimum Y. The game retains the original Unity material/shader, rate (8/sec), lifetime (.7–1 second), light budget and descent/crash audio. Only this emitter is excluded from pod suppression. Its hierarchy scaling is enabled explicitly, and emission stops on landing. Fire startup is confirmed in the user's log; the corrected height still needs a playtest.

The pod can prepare before its passenger/skin arrives. Previously skin selection ran only in `Prepare`, permanently selecting Mario in that case. The adapter now preloads Luigi before the fixed model table is built, reads the master's body loadout, refreshes the seated rider during idle and again at activation, and applies the matching skin/voice pitch. `portal_fast --late-luigi` selects him after idle updates; the boundary check reproduces that ordering and verifies the green palette and actual HSD draw. Final Unity alignment and skin initialization are still pending a playtest.

## In-game M orientation correction

The user's in-game screenshot confirmed that the M was horizontally backwards. `Portal::load` now negates only texture-matrix slot 5's X scale (`P_gate_msk_m2`, fixed mask); Y/U/V interior slots 0–2 stay unchanged. The gate remains the exact `05_gate01` model/material identified above, not a replacement effect. `portal_fast` no longer undoes native U in the viewer, which had hidden the host mismatch; the archived full-sequence viewer uses the same quad UVs. Captures in local `output/mario-portal/m-facing-corrected/` show idle and activated frames. A disc-backed regression verifies the front-view silhouette without the independent animated warp and checks that the interior matrices stay unchanged. The existing distortion test still passes. Cache revision `SMSPOD03` / `entrance-v3-*` forces a refresh so old mirrored frames cannot be reused. This correction is viewer/test checked; the new DLL has not yet been played.

## Edge quality and attached fire revision

The mirror changes a texture matrix, not extracted image pixels. Caches contain raw RGBA, not lossy compression. In-game feedback identified rough edges at 128 square: `SMSPOD04` / `entrance-v4-*` now renders the authored TEV at 512 square on the worker and resolves 2x2 samples to 256-square straight-alpha RGBA. Fractional coverage is retained at the contour while the interior stays opaque. The game profile uses the shared translucent shader at full opacity so it does not discard that coverage as a hard cutout; the archived original profile is unchanged. Local `m-smooth-256/` captures show idle and activated frames. Rust and C# boundary regressions verify fractional edge alpha, opaque interior, resolution and blend classification. The cache is about 89.5 MiB; native warm idle/spent updates averaged 0.003 ms (maxima 0.019/0.017 ms), active updates 0.058 ms (maximum 0.348 ms), and spent geometry uploads remain zero. These measurements exclude Unity/GPU time.

The user's next screenshot showed the fire still below the base with zero emitter offset. Following their placement and height feedback, its emitter is raised to native pedestal minimum Y +550 cm; horizontal scale stays 3.5x while the authored local Z (vertical) scale is reduced to 1.75x. `ParticleSystemSimulationSpace.Local` is explicitly selected so existing particles follow the falling statue's transform; the original space is restored on cleanup and reported in the startup log. The emitter is the same identified Flames system and retains its authored material, rate, lifetime and light budget. Local `fire-wide-short-plus550/` captures show multiple mesh-particle frames at this scale. The viewer already draws its fire in the statue's current frame, but its pivot approximation remains unsuitable for proving Unity height. The new host offset and attachment need an in-game check.
