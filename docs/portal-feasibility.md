# Mario statue entrance: viewer proof

The entrance is still a prototype. No RoR2 code has changed and no build has been staged.

## Identified assets

- `dolpic1.szs:map/map/gate/05_gate01.bmd`, `.btk`, `.brk`: the Bianco Hills M gate. Proved by `sms-main/src/MoveBG/ModelGate.cpp`, `gateMActorNames[0]`, `TModelGate::loadAfter` and `perform`.
- One material, seven TEV stages, six texgens, six texture maps. Its indirect stage reads coordinate/map 4, `P_ms_indwp1_ia`, through the disc's 0.6 * 2^-5 matrix. Its M mask is `P_gate_msk_m2`. It is a model material, not a particle substitute.
- `map/map/map.bmd`: the Pianta sculpture is shape 23, `_king_storn_1`; the pedestal is separate bounded geometry in the same map. Both are retained by `plaza::statue_batches`.
- Portal activation rays: `ms_gatewind_a/a2/a3/b.jpa`, cues 0x131–0x134; spray hits: `ms_gatehit_a/b.jpa`, cues 0x1DD–0x1DE. Source: `ModelGate.cpp::perform/receiveMessage` and `System/Particles.hpp`. These are identified but not yet included in the preview.

## Demonstrated

`cargo run --release -p viewer --example portal -- assets ../output/mario-portal`

The headless viewer produces several BTK frames at three BRK activation levels. It preserves the authored distortion and combines it with a replaceable still through the gate's Y/U/V TEV stages. The M mask and still have independent vertical orientation. Idle shows the swirling colours; activation reveals the still.

The still is rendered from the user's disc. It includes the scene.bin placements of palms, sewer covers, the clean Shine Gate emblem and DokanGate pipe effects. `TMonumentShine::initMapObj/hitByWater` defines the clean emblem's K0 alpha of zero. The pipe geometry is in the main map; respecting its diffuse colour and zero specular base avoids turning its red paint white.

Outputs under `../output/mario-portal/` are local, extracted-data derivatives and must not be committed or packaged. The frame sheet's columns are BTK frames 0, 30, 60, 90; rows are activation 0, 0.5, 1.

## Remaining entrance work

Draw and verify the original activation rays and Mario's gate-exit animation in the viewer. Then connect the statue to Mario's pod, retaining vanilla descent/impact audio, facing the camera toward the M, and timing X activation, Mario's exit and the existing arrival sting. In-game placement, camera framing, collision and timing are not yet played. The current GPU viewer's general J3D interpreter still lacks this indirect material; this proof uses the shared CPU TEV bake path.
