# Mario statue entrance: viewer proof

The entrance is still a prototype. No RoR2 code has changed and no build has been staged.

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

## Remaining entrance work

Connect the verified viewer assets to Mario's pod, retaining vanilla descent/impact audio, facing the camera toward the M, and timing X activation, Mario's exit and the existing arrival sting. In-game placement, camera framing, collision and timing are not yet played. The current GPU viewer's general J3D interpreter still lacks this indirect material; the interactive example uploads the shared CPU TEV bake onto the portal quad while using the existing GPU J3D renderer for Mario.
