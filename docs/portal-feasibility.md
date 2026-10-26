# Mario statue entrance: viewer proof

The viewer proof is approved and connected to Mario's RoR2 stage-one pod. The adapter is build/boundary checked; in-game testing is pending.

## Identified assets

- `dolpic1.szs:map/map/gate/05_gate01.bmd`, `.btk`, `.brk`: the Bianco Hills M gate. Proved by `sms-main/src/MoveBG/ModelGate.cpp`, `gateMActorNames[0]`, `TModelGate::loadAfter` and `perform`.
- One material, seven TEV stages, six texgens, six texture maps. Its indirect stage reads coordinate/map 4, `P_ms_indwp1_ia`, through the disc's 0.6 * 2^-5 matrix. Its M mask is `P_gate_msk_m2`. It is a model material, not a particle substitute.
- `map/map/map.bmd`: the Pianta sculpture is shape 23, `_king_storn_1`; the pedestal is separate bounded geometry in the same map. Both are retained by `plaza::statue_batches`.
- Portal activation rays: `ms_gatewind_a/a2/a3/b.jpa`, cues 0x131–0x134; spray hits: `ms_gatehit_a/b.jpa`, cues 0x1DD–0x1DE. Source: `ModelGate.cpp::perform/receiveMessage` and `System/Particles.hpp`. The interactive preview draws the activation rays; spray-hit particles are unnecessary for its button-triggered opening.

## Demonstrated

`cargo run --release -p viewer --example portal -- assets ../output/mario-portal`

The headless viewer produces several BTK frames at three BRK activation levels. It preserves the authored distortion and combines it with a replaceable still through the gate's Y/U/V TEV stages. The M mask and still have independent vertical orientation. Idle shows the swirling colours; activation reveals the still.

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

`view::Entrance` shares the approved timeline with the interactive viewer and supplies statue, portal, Mario and particles through the existing model/animated-texture/FX readers. `sms_view_entrance_new/update/bounds` add an opaque entrance view without changing existing C struct layouts. The host's X reflection is compensated in the portal UVs so the M and Plaza still remain oriented correctly. The shared CPU presenter now includes the landing's held-cap/right-hand models.

`ror2-mario/MarioStatuePod.cs` clones Mario's preferred pod (the stock SurvivorPod if unspecified) and retains its animator, animation-event fall/crash sounds, seat and network lifecycle. `MarioPodDescent` inherits the original descent; the landed interaction starts `MarioPodExit`, which keeps Mario seated/protected while native presentation plays. The camera faces the M. X plays the existing once-per-run sting; Mario says the existing jump woohoo when the model starts appearing. After the cap/landing clip, the server ejects him at the vanilla exit point and the ordinary Mario driver takes over. A short visibility bridge avoids a Commando flash or duplicate Mario. The statue stays behind with a pedestal collider bounded from its native geometry.

The custom presentation is gated to stage one. Missing assets, disabled model drawing or an unavailable renderer retain the vanilla pod path. The first-run optional extraction includes the gate, four wind emitters, Plaza map/scene and all five still-shot prop models from the user's own disc; no extracted data is packaged. `portal_assets` lists the exact archive paths for inspection.

`ror2-mario/tests/PortraitCheck` exercises the real DLL's idle/activation/reveal/landing, instance-key uniqueness, animated texture versions, particle expiration, collision clearance, finite geometry and completion. In-game descent alignment, camera framing, sound events and input/network lifecycle are **not yet played**. The GPU viewer's general J3D interpreter still lacks this indirect material; both hosts use the shared CPU TEV bake on the portal quad.

## Revised game profile after the first playtest

The full Sunshine sequence remains in `Entrance::load/update`, the original C entry point and `portal_entrance` viewer. RoR2 now selects `sms_view_entrance_fast_new`: 0.8 seconds of particle/reveal animation, the original rolling BCK at its native 60 fps over a gentler 0.8-second fall, then 0.15 seconds of ordinary `jmped` landing and handoff. The fall duration was adjusted in the live viewer to the user's approved pace. It is host choreography; the archived original still uses the source's fall physics and cap demo. The existing wahoo plays at materialization; Luigi retains his voice pitch and changes only red particle palette entries to green. His HSD retarget receives the appearance BCK's centre scale explicitly so he does not appear early.

The portal returns to swirling colours after handoff. `portal_cache` pre-renders the exact TEV at 128 square on the asset worker: the complete idle BTK loop at 15 fps and the short opening/interior/closing sequence at 30 fps. The ignored cache fingerprint includes all fourteen source files and playback settings. Runtime selects a cached frame and copies pixels into one stable texture; no TEV shader interpretation runs on the game thread. Rigid statue/gate geometry is cached and uses the shared retained-pose flag after its initial upload. Idle and spent views never pose Mario/Luigi. `MarioPerformance.Part.Portal` measures the actual adapter when performance logging is enabled.

Measured with `entrance_bench` on the same machine: the former native idle/post-exit updates averaged about 14.6 ms, with maxima around 31–36 ms. The revised cached idle/post-exit updates averaged about 0.001 ms (maximum 0.004 ms), with zero post-exit vertex uploads. Active native exit updates averaged 0.058 ms, maximum 0.333 ms. These are native CPU timings, not measured RoR2 frame rates. First-time cache rendering runs on the worker; subsequent native view load was about 265 ms.

`MarioStatuePod` forces off all original pod renderer types except the resized descent fire, stops other original particle systems without disabling animator/audio objects, and removes stock pod collision after ejection while retaining the native pedestal collider. It does not rely on `Renderer.enabled`, which can be restored by animation. Vanilla fall/crash audio events remain attached to the original animator.

The original exit sound is the nine BAS cues in `ma_demo_gate_out_appear_get.bas`: `MSD_SE_MA_Y_ROLL9` (0x1963) through `MSD_SE_MA_Y_ROLL` (0x195B), at the file's authored frame keys. `MarioDraw.cpp` loads the BAS beside the BCK; `JAIAnimeSound::playActorAnimSound` is the cue mechanism. The game profile compresses their timing with the particles while retaining authored sample pitch. It does not substitute the unrelated `MSD_SE_MA_WARP_EX` stage-change sound. A device-free test verifies all nine IDs and audible finite PCM.

`portal_fast --capture <folder>` renders the shared native presenter, including the real Luigi HSD mesh, green particle palette, spin, landing and return to colours. The C# boundary test exercises both original and revised profiles, Mario and Luigi, retained geometry, particle expiration, handoff and exact idle-color restoration. The revised RoR2 build still needs a playtest for renderer suppression, audio balance and actual FPS.

## Descent fire and late rider selection

The exact stock fire is Unity's mesh-particle emitter `SurvivorPod/Base/mdlEscapePod/EscapePodArmature/Base/Flames`, ParticleSystem path ID **-470276994286075641** in `ror2-base-survivorpod_assets_all_1195400eaf9e5c72fc5bc5dcfd6ecd25.bundle`. Its renderer is **-4483200324324671225**, rendering `donut5Mesh` (**-6476590250452040025**) with `matEntryBurn` (**-6514494002267390535**). This is a Unity mesh-particle system, so there is no GameCube `efSync` cue. It is not replaced with a JPA emitter. The separate `FireDonut` object is inactive and has no binding in `EscapePodArmature|InitialSpawn`; it is not the active descent emitter.

`tools/export_pod_fire.py <StandaloneWindows64 folder> <ignored output>` exports the exact mesh and four textures locally for `portal_fast --descent-fire <output> --capture <frames>`. The source-labelled captures show multiple animated emitter frames with a 3.5x scale and the luminous core lowered to the pedestal bottom (emitter at native base minimum Y minus 950 cm, accounting for the source renderer's -1.2 Z pivot). The cloud shader in this viewer is a placement approximation; the game retains the original Unity material/shader, rate (8/sec), lifetime (.7–1 second), light budget and descent/crash audio. Only this emitter is excluded from pod suppression. Its hierarchy scaling is enabled explicitly, and emission stops on landing.

The pod can prepare before its passenger/skin arrives. Previously skin selection ran only in `Prepare`, permanently selecting Mario in that case. The adapter now preloads Luigi before the fixed model table is built, reads the master's body loadout, refreshes the seated rider during idle and again at activation, and applies the matching skin/voice pitch. `portal_fast --late-luigi` selects him after idle updates; the boundary check reproduces that ordering and verifies the green palette and actual HSD draw. Final Unity alignment and skin initialization are still pending a playtest.
