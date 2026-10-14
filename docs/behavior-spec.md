# Mario locomotion and FLUDD: behavior spec

Written from the doldecomp/sms decomp (`../sms-main`, GMSP01 build, verified against the PAL
disc). Function names in parentheses are decomp symbols. Numbers marked **(disc)** come from the
`.prm` files in `params.szs` on the user's disc; the constructors' code defaults differ and are
only fallbacks.

Asset pipeline: `libsms::extract` reads the user's disc (plain ISO/CISO), decompresses `.szs` (Yaz0) and
unpacks RARC archives; output is verified byte-identical to decomp-toolkit's. See the README.

Status of each rule: **ported** (implemented and tested), **partial**, **not yet**, or
**unverified** (ported as written but contradicts expectation; see the end).

Host fall damage: `jumpProcess` emits `FallImpact` at floor contact before changing
status, with the continuous airborne peak minus impact Y, position, and dive/pound/
cushioned flags. Dive transitions retain that peak; host teleports reset it. Water,
Yoshi and slow landings are cushioned (see `MarioJump.cpp::askStrongGroundTouch`).
This event leaves Sunshine health alone. RoR2 chooses its own height thresholds:
40 Mario heights safe (default rocket measured at 34.7), dive safe through 80, then
half damage; ground pounds are always safe. Presentation omits super-pound landing
stars (`ms_m_sphipd_hit_d`) while keeping smoke/ring.

Host extension: `Mario::launch_pad` uses the host's complete ballistic velocity and
gravity until collision. Unlike ordinary `launch` knockback, it suspends air steering,
Sunshine speed caps, short-hop cuts and terminal-speed clamps. Velocity remains in
units per 30 Hz frame; gravity is velocity lost per 120 Hz tick. RoR2 supplies its
world gravity, so pads reach their intended target instead of a shortened Sunshine arc.

## 1. Time and units

Host presentation: ground-pound landing effects stay at their impact position through a
rebound. This includes both `ms_hipdrop_*` and `ms_m_sphipd_hit_*` from
`TMario::hipAttacking` (MarioJump.cpp). They exclude the host's current Mario draw offset;
the separate `ms_m_sphipd_a..d` flames remain attached during the fall.

Host preference: every smoke or dust cloud (the `kemuball` billboards: `ms_mariwalk1_a`,
`ms_jump_ed_b`, `ms_m_slipsmoke`, `ms_mariwallsl1`, `ms_wallkick_a`, and the pound's
`ms_hipdrop_b`, `ms_m_sphipd_hit_b`; `libsms_present::view::DUST_CLOUDS`) keeps each puff's
birth height. The other effects (debris, shock rings, marks) are untouched. Its horizontal spread, size, colour and lifetime still
advance; its authored upward velocity/fields do not lift the dust. Both use smoke speed.

- **The simulation runs at 120 Hz in both regions.** `TMarDirector::direct`
  (System/MarDirectorDirect.cpp) turns each rendered frame into `600 / vsync` time units
  (`vsync` = `SMSGetVSyncTimesPerSec()` = 30 NTSC, 25 PAL) and burns them in 5-unit ticks:
  4 ticks per frame at 30 fps, 4.8 at 25 fps. The game renders at 30/25 fps but every Mario
  constant is per 120 Hz tick. (ported: `clock.rs`)
- Velocity is stored in "units per 30 Hz frame"; each tick moves by a quarter of it
  (`jumpProcess`: `next = pos + 0.25 * vel`; `walkProcess`: `vel / 4`). Gravity is subtracted
  once per tick. Terminal fall speed -75, horizontal air speed cap `mJumpingMax` (80, disc).
- 1 unit is about 1 cm. Mario: 160 tall, wall radius 50, steps up 100, head clearance 160.
- Angles are `s16` (0x4000 = 90 degrees). Facing `a` points along `(sin a, 0, cos a)`.
  `matan(z, x)` = `atan2(x, z)`. The world is right-handed: with the camera looking along +Z,
  stick-right moves toward -X.
- Animation clips are 60 fps authored; `setAnimation(clip, rate)` plays at `rate * 0.5`
  frames per tick. Many transitions fire on `isLast1AnimeFrame` (frame >= length - 1), so the
  sim runs clip timing from the BCK lengths on the disc. (`anim.rs`)

## 2. Input

`TMario::checkController` (MarioMove.cpp). (ported: `input.rs`)

- Stick (`mCompSPos`, the SDK's raw/54 clamped to the unit disc) x 128, truncated to `s16`.
  Dead zone: ignore |v| <= 7, subtract 6 outside. `dist = |stick| * mLengthMult^mLengthMultTimes`
  (disc 0.78^2); if `dist > 64`, rescale the stick to 64. `intendedMag = 32 * (dist/64)^2`.
  `intendedYaw = matan(-V, H) + cameraYaw`; neutral = current facing.
- Triggers: raw 0..255 -> level 0..1 (`getLRLevel`): 0 below `mAnalogLRToZeroVal`, soft ramp to
  `mAnalogLRMiddleLevel` at `...ToMiddleVal`, linear to 1 at `...ToMaxVal` (disc 10/60/150, 0.5).
- `mInput` bits, named in `Input`: STICK 0x1, JUMP (A pressed) 0x2, AIRBORNE 0x4 (pos.y > floor +
  100), SLIP 0x8 (slope steeper than `mSlipStart`), IDLE 0x20 (no stick, no jump), A-held 0x80,
  R-pressed 0x100, R-or-L held 0x200 (FLUDD only), B-pressed 0x2000, B-held 0x4000, DIVE 0x8000
  (B pressed, or L pressed while in a jumping status). Bit 0x10 is never set anywhere
  (vestigial; handlers that test it are dead code).
- Button roles: A jump, B dive, L ground-pound (in the air) and crouch (with FLUDD), R trigger =
  spray pressure plus a digital click that crouches, X swaps nozzle.
- Spin detection (`checkStickRotate`, `makeHistory`): keeps the last `mStickRotateTime - 1`
  (disc 39) intended yaws while the stick is held; a spin is when all four quadrants appear;
  direction from whether the yaw values mostly increase. (The decomp leaves its quadrant flags
  uninitialised; we start them false.)

## 3. World contract (Host)

Three probes, matching `TMap` (Map/Map.hpp). (ported: `host.rs`)

- `checkGround(x, y, z)`: highest floor at or below `y`, with normal and flags. `None` = void
  (treated as an ILLEGAL plane at -200000).
- `checkRoof(x, y, z)`: lowest ceiling above `y`.
- `isTouchedWallsAndMoveXZ(center, radius, max)`: pushes a vertical cylinder out of walls; nearest
  wall first. Mario probes at heights 30 and 60 (walking) and 30 and 150 (airborne), and
  `jumpWallHeight` (disc 500) for wall kicks.
- Surface flags the logic branches on: ILLEGAL, MARIO_THROUGH, FORCE_SLIP, SLIDER, SLIP_45,
  NO_SLIP, WET, WATER_SURFACE, FENCE, NO_WALL_JUMP, NO_LEDGE_GRAB, POUND_THROUGH.
- Pass-through floors are skipped by re-probing 1 unit below (`checkGroundPlane`).
- Enemies (`Host::enemies`): standing cylinders (feet, `mDamageRadius`, `mDamageHeight`) near a
  point; the sim tests overlap itself with `checkDistance` (section 7c).
- Shallow water (`Host::water_level`): a pool's surface at a point (`TPoolManager::getWaterLevel`).
  `thinkWaterSurface` (after `stateMachineUpper`) puts Mario in shallow water while it's above his
  feet. Only the pool half is ported: no water-surface planes, no swimming (deep water is wading).

Per tick (`checkCurrentPlane`): probe the floor at `y + 25` and ceiling at `y + 80`; set slope angle
`matan(n.z, n.x)`; AIRBORNE if `y > floor + 100`; SLIP if `isSlipStart`.

## 4. Ground movement (MarioRun.cpp, MarioWait.cpp, MarioPhysics.cpp)

`walkProcess`: move by `vel/4 * normal.y`, then `checkGroundAtWalking`: push out of walls (radius
25 at 30, 50 at 60), probe floor at `y + 30`. Result 2 (blocked) if the floor is illegal or the
head would hit the ceiling; 0 (fell) if the new `y` is more than 100 above the floor; otherwise
snap to the floor and return 1 (glancing wall, 30 to 75 degrees off) or 3 (head-on, reported as
2). (ported)

`doRunning` (disc `Run.prm`): target speed `min(intendedMag, mMaxSpeed 32)`.
- speed <= 0: `+= mVelMinusBrake (1.1)`; speed <= target: `+= mAddBase (1.1) - speed * mAddVelDiv
  (0.0233)`; above target on flat ground (`normal.y >= 0.95`): `-= mDecBrake (5)`.
- Turn rate toward the stick: `0.03125 * speed * (max - min) + min` with `mRunningRotSp` min/max
  1500/1500 (disc); 60 (`mDashRotSp`) while turbo-dashing; `mPumpingRotSp` 300/600 while the FLUDD
  stance is up.
- Slope: `slopeProcess` adds `mSlopeAcceleUp/Down` (per surface kind) scaled by the slope's
  horizontal normal, caps at `mRunningMax` (disc 80).

Statuses (all ported unless noted): `Wait`, `Run`, `Turn` (stick > ~100 degrees off while faster
than `mTurnNeedSp`), `TurnEnd`, `Brake`, `BrakeEnd`, `WalkEnd`, `Squat`, `SquatStandup`, the `*End`
wait states, `RotateL/R` (spray spin), `Slip` -> `SlipFore`/`SlipBack`, `Catch` (belly slide),
`CatchLost`, `ShortBackDown`/`ShortForeDown`/`BackDown`/`ForeDown`/`CatchDown` (knock-downs).

Transitions worth knowing:
- Wait -> Run when `intendedMag > mStartToWalkLevel` (disc 1). Run -> WalkEnd on IDLE (-> Brake if
  running > 240 ticks at >= 16 on a >= 80 degree slope... see `running()`). Run -> Turn.
- Wall bump while running faster than `mClashSpeed` (disc 40): knocked back (`JumpShortBackDown`).
  Pressing jump into a wall that reaches `jumpWallHeight`: wall kick (`WallJump`, vel.y 52, speed 50).
- Slipping: `slideProcess`/`doSliding` steer a slide velocity downhill with the surface's
  `SlipParam*.prm`; stop below `mSlideStopNormal`. Slip set chosen by `isForceSlip`
  (`normal.y < mForceSlipAngle` 0.4 disc, or FORCE_SLIP), SLIDER, SLIP_45, WET (ground vs slope by
  `normal.y > 0.99`), else Normal.

## 5. Jumps (MarioJump.cpp, MarioMove.cpp `setStatusToJumping`)

Take-off (`mVel.y = forwardVel * k + force`, then `forwardVel *= m`), values from `Jump.prm` (disc):

| Jump        | k            | force          | forward multiplier |
|-------------|--------------|----------------|--------------------|
| Normal      | 0.25         | 42             | 0.8                |
| Second      | 0.25 (`mSecJumpSpeedMult`) | 52 | 0.8 (`mSecJumpXZMult`) |
| Triple (Ultra) | 0.15      | 75             | 1.0                |
| Spin        | 0.25         | 70 (`mRotateJumpForceY`) | 0.8       |
| Back flip   | 0            | 70             | forward = -18      |
| U-turn      | 0            | 70             | forward = 8, face stick |
| Wall        | 0            | 62             | forward = 24       |
| Dive        | -            | -              | `min(15 + speed, 48)` (speed first set to `mJumpJumpCatchSp` 80) |

Gravity 1.0 per tick (disc `Jump.prm`; `newmario.prm` also has a 0.97 but the game only loads `Mario.prm`), 0.35 when spin-jumping and falling.
**Variable height** (`isFallCancel`): in Jump, Second, Wall, Miss, RotateBroad and SlipFall, letting
go of A while rising faster than 20 multiplies `vel.y` by 0.75 each tick.

**The jump chain has no timers of its own.** Landing puts Mario in a `*Slip` status (e.g.
`JumpSlip`) for a window of `0x10` ticks (0x04 after a triple, 0x18 broad); pressing A inside it
starts the next jump: JumpSlip -> Second (needs speed >= `mSecJumpEnableSp`, disc 0), SecJumpSlip ->
Triple (speed >= `mTriJumpEnableSp`, disc 25), anything else -> a plain Jump. Missing the window
runs the `*End` clip then `Wait`. (ported: `air.rs`, `ground.rs jump_slip*`)

Host tuning (not the game; all off by default, `Options`): `jump_height` multiplies take-off
`vel.y` by its square root (every jump but dives and forced ones), `jump_buffer_ticks` keeps an A
press that did nothing in the air and fires it on the first grounded tick, and
`chain_grace_ticks` keeps the Second/Triple on offer for that long after a window closes. For hosts
whose worlds are built bigger than Delfino and whose players aren't on a GameCube pad.
`move_speed` multiplies the ground covered per tick in `walkProcess` and `jumpProcess` (speeds,
and so the run animation, stay the game's), for a host's speed buffs. `enemy_push` and
`cycle_nozzles`: see sections 7c and 6. The Spray's reach is the water manager's (`reach`). `stomp_bounce`
multiplies a stomp's (`Trample`) take-off `vel.y` by its square root, on top of `jump_height`.
`rocket_thrust` multiplies the Rocket nozzle's vertical recoil (`mReactionY`); the apex grows
about as its 1.9th power (`examples/report.rs`: x1.25 flies 1.52 times as high, x1.5 2.16, x2
3.76), and `sms_sim_set_tuning` takes a height multiplier and converts. `pound_jump`: A during
`HipAttackEnd` (which the game ignores) takes off as the triple jump (`UltraJump`, never cut short)
at `max(triple, sqrt(8 g h))`, `h` being that share of the pound's fall: the apex of a take-off at
`v`, moving `v/4` a tick under `g` a tick, is about `v^2/8g`. `stomp_jump`: an A press within 15
ticks of a stomp, before it or after, makes its take-off `vel.y` that factor's square root as
fast; a late press sets the boosted speed less the gravity since (so letting go of A before
pressing, which cuts the bounce short as the game does, costs nothing). `short_hop`: `isFallCancel`
only cuts once the jump has risen that share of its full apex (`v^2/8g` from take-off), so a tap
rises further (disc tuning: a 3-tick tap is 89 of 226 in the game, 145 at 0.4, 160 at 0.5).

Air control (`doJumping`): `forward *= mJumpSpeedBrake` (0.999); with the stick, `forward +=
mag * cos(d) * mJumpAccelControl (0.008)`, sideways `mag * sin(d) * mJumpSlideControl (0.3)`;
`> 32` decays by 0.2, `< -16` by 0.4.

Landing results from `jumpProcess`: 1 floor, 2 wall within `mClashAngle` (12000) of head-on, 0 air.
A head-on wall hit at speed > `mClashSpeed` bumps; > 16 reflects and starts a wall slide
(`WallSlide`, jump to kick off; slides at `-timer/2` after 20 ticks); else stops dead.

Dive (`JumpCatch`) lands into `Catch` (belly slide), which jumps out (`CatchStop`) or gets up
(`CatchLost`); the slide holds the dive clip at frame 50, its belly pose, and plays `MA_SLIP`
every tick (`catching`). Ground pound (`HipDrop`): 60 ticks of hover, then falls at `mHipAttackSpeedY` (-80);
becomes "super" after `mSuperHipAttackCt` (80) ticks, falling at `mSuperHipAttackSpeedY` in flames
(`emitBlurHipDropSuper`, `MS_M_SPHIPD_A..D`; `Mario::pounding` 2 for hosts to draw), and lands
with the super hit (`HIT_MESSAGE_SUPER_HIP_DROP`, `MS_M_SPHIPD_HIT_*`). `Event::GroundPound`
carries the drop from the top of the hover (`unk104`) to the floor.

**Long falls** (`landing`, MarioJump.cpp; the `Landing` status is a plain fall, e.g. off a ledge):
after 240 ticks falling (2 s) Mario flails (`ANIM_THROWN`) and cries out (`MV10B_CRY_JUMP`; the
game requests it every tick from then, libsms once). Jumps falling as far don't.

**Particle effects** (`libsms_present::particles`, from the disc's `particle.szs` `.jpa` files):
`JPABaseEmitter` and its particles run once per game frame (4 ticks): emission over the volume
(`createParticle`), gravity and air fields, air resistance and weight, the `ESP1` scale, alpha and
spin over life and the `BSP1` colour tables, drawn as `JPADrawExec*` builds billboards,
directional quads (including the two perpendicular planes of directional cross) and
(crossed) stripes. A persistent request per frame keeps an emitter making
particles; a frame without lets it go (`TMarioParticleManager::perform`,
`becomeInvalidEmitter`). TEV (`JPADrawSetupTev`): stage 0's colour is `d + a(1 - c) + b c` over
the texture colour and the prm/env registers, its alpha texture alpha times prm alpha; additive
effects are split into basis textures and vertex colours that sum to it.
`View::events` observes every sim tick, retaining one-shot and held `Event::Particle` cues
between renders; the FFI does this even with sound disabled.
Dust on starting, skidding, landing, wall sliding and wall kicking, bump stars, the pound's
smear and landing, spin-jump swirls, belly-slide smoke/spray and stomp bursts load from the disc.
One-shots keep their initial position and yaw and expire using their JPA's duration;
held effects follow the posed center, joint or feet and release when requests stop.
`setGlobalScale` shrinks motion and drawn sizes together (light landings use 0.7).
The super pound's flames (`MS_M_SPHIPD_A..D`) bind to the `center` joint as drawn.
Smoke has a host presentation override: emitted puffs stay anchored in world space,
with smooth motion between particle frames and optional smoke-only playback speed.
See [smoke presentation](smoke-presentation.md) for the host API and RoR2 defaults.
Not ported: children
(`SSP1`), `KFA1` key frames, indirect textures, the other field kinds.

**Wet belly slide** (`checkCurrentPlane` → `askHitWaterParticleOnGround`, `catching`,
`doSliding`). Touching ground (`isTouchGround4cm`) within 300 (summed over the axes) of a FLUDD
particle stuck to the ground sets `MARIO_FLAG_RECENTLY_LEFT_WATER` (misnamed); `catching` then
sets state 1, and on normal ground `doSliding` uses `mWasOnWaterSlip` (0.97) for the belly
slide's friction instead of the normal slip's (`mInWaterSlip` 0.9 in water): about twice as far.
Ground impacts leave persistent, drying wet marks in `WaterManager`, independent of whether
an airborne droplet sticks or splashes away. Their base radius uses the disc's
`mCleanSize * 10` (`TModelWaterManager::splashGround`); light bursts use twice that radius
as a host-style approximation of a broad approach patch (not verified against vanilla).
Marks last the particle type's `mAlive`, capped at 256 water frames, and repeated spray
refreshes nearby coverage. Presentation appends their surface quads after drops and splashes,
over the disc's soft water mask, with opacity fading over the last 60 water frames.
RoR2 draws them as a subtle black alpha overlay (`Look.GroundWetness`, default 0.12);
they darken the terrain colour beneath them without changing its material. A sampled 3x3 surface grid follows slopes and rejects ledges.
`on_ground_near` tests the same circular footprint and height that presentation draws,
so an invisible droplet 300 units away can no longer make dry ground count as wet.
The host feeds `Mario::water_underfoot` from `WaterManager::on_ground_near` each step. The game's
spray off the slide (`frontSlipEffect`, `MS_M_WATSLIDE_A..C`) binds all three disc effects
to the animated center joint. `MS_M_WATSLIDE_C` uses `JPADrawExecDirectionalCross` (and its
rotating variant): both perpendicular textured planes draw with the same pivot, scale and
particle frame, restoring the side spray. RoR2 uses `WetSlide::timed` semantics: wet ground
is required at activation, no stock is consumed, and every wet slide has a duration cap
(including on shallow water). Leaving the wet patch does not cancel an active glide;
remaining on wet ground cannot restart an expired cap until a new slide. The Special
slot has zero recharge; cooldown items extend duration and each added Special stock
(Lysate Cell) adds one baseline duration before that multiplier. Host `slide_fx_scale` scales all three effects
together. The old stand-in splash sprites (`WaterManager::slide_splashes`) are optional.

**Spin jump sounds** (`soundMovement`, MarioSound.cpp, every tick off Yoshi): entering a spin jump
by any route (take-off or `considerJumpRotate` mid-air) starts `MV24_JUMP_SPECIAL` once (on top of
the take-off's `MV23`), and its 10th tick `MA_ROLL_JUMP`. A stomp starts `MA_KICK_ENEMY`
(`trampleExec`).

**Ledges** (MarioSpecial.cpp `hanging`/`ascend`/`descend`/`hangJumping`, MarioPhysics.cpp
`hangonCheck`/`checkDescent`; ported: `hang.rs`). A jump step that touches a wall at waist height
(+30) but not at head height (+150) calls `hangonCheck`: Mario must be falling and not moving away,
and the floor 60 past the wall must be more than 100 above his feet, with 160 of headroom, and not
force-slip or `NoLedgeGrab`. His position moves onto that ledge top (10 in from the face) facing
the wall; the `hang` clip lowers the body. While `Hanging`: B or losing the floor lets go
(`startHangLanding`: back 60 and down 100, `WallSlide`); A, or a floor within 100 below the hands,
hops up (`hgjmp`); after 40 ticks the stick toward the wall (within 0x400) climbs (`hgup`),
away (beyond 0x71C7) drops off, and sideways shimmies `mMoveSp * mag` along the face, turning
round corners whose normals differ by less than `mHangWallMovableAngle` (0.3), with
`hmov_l/r` at `distance * mAnmRate`. After `mRapidTime` (2000, disc) ticks the shimmy stops and
the hang clip runs at `mAnmRapid` (8); at `mLimitTime` (2400) he falls. Walking slower than
`mDescentSp` (10) off an edge with a drop over 160 turns him round and lowers him to hang
(`hgdwn`).

**Carrying** (MarioAction.cpp, MarioCollision.cpp `isTakeSituation`; ported: `carry.rs`).
B at a standstill or a slow walk reaches out (`TakePose`, `get_fail`). While B is held, touching
a takeable object within `mHoldRadius` (80, disc) plus its radius and 60 degrees of facing starts
`Take`: `raise`, the hand closes at frame 11 (`HIT_MESSAGE_TAKE`, `Host::take`), then the upper
layer switches to `carry_p` (`UPPER_STATE_HOLDING_OBJECT`, no pumping). B again puts it down
(`put`, released at frame 20) or, faster than 16, throws it (`throw`, frame 4, `mThrowPower` 2.8
disc). Objects position themselves from the `Put`/`Throw` events (`TMapObjGeneral::put/thrown`).
Not yet: throwing from the air (`JUMP_THROW`), barrels and other holders' special cases.

## 6. FLUDD (WaterGun.cpp, NozzleBase/Trigger/Deform, MarioUpper.cpp)

Slots: 0 Spray (`NozzleDeform.prm`), 1 Rocket, 2 Underwater, 3 Yoshi, 4 Hover, 5 Turbo.

- **Tank**: `water` in the current nozzle's units; `mAmountMax` (disc) 10000 Spray/Hover/Diving,
  2100 Rocket/Turbo. Changing nozzle rescales to the same fraction (`changeNozzle`).
- **Refilling** (`gunExec`, MarioMove.cpp; `TWaterGun::suck`): in shallow water, R draws water in
  at `pressure * mSuckRate` a tick (`PO_SUCK_WATER_B` while it isn't full) instead of spraying;
  only a full tank sprays there, and stays full (`resetWaterToFull`). Out of water, pumping
  sprays. R pressed (`mInput & 0x100`: FLUDD on, not dashing) with the tank at 0 plays
  `PO_ACTION_ON_EMPTY` (`soundMovement`). The HUD chimes `SY_PO_WATER_FULL` as the tank reaches
  full (`updateWaterGaugeFill`). `Mario::refill_water` is a host's own refill;
  `Options::unlimited_water` (host tuning, not the game) keeps the tank full.
- **Upper-body state**: R pressure > 0 enters `Pumping`; releasing holds the stance for
  `mPumpWaitTime` (5) ticks. Firing only happens in `Pumping`.
- **Trigger nozzles** (Rocket, Hover, Turbo; `TNozzleTrigger::movement`): while pumping,
  `fill += 150 * rLevel` each tick, `fill -= mInsidePressureDec`, clamp to `mInsidePressureMax`; on
  reaching max the nozzle goes `Active` for `mTriggerTime` ticks, then `Dead` until R is released.
  Disc: Hover max 1, dec 0 -> active immediately, 360 ticks (**3.0 s**); Rocket max 8000, dec 105 ->
  **1.5 s** charge, active 5 ticks; Turbo max 4500, dec 100 -> 0.75 s.
- **Firing** (`emit`): per active tick, `count = pressure*(num-numMin)+numMin` particles, power
  `pressure*(emitPow - emitPowMin) + emitPowMin`, tank `-= count * mDecRate`. Recoil:
  `forward += reaction * -(dir.forward) * power`, `vel.y += -dir.y * power * mReactionY`
  (Rocket: 0.7 x power 50 = +35 per active tick, which is the rocket launch).
- **Hover** (`rocketing`): `vel.y = (target - y) * mAccelRate (0.02)` toward `y0 + mHHoverHeight
  (230)`, `forward *= mBrake (0.985)`; the stick tilts the two nozzles and steers
  (`mRotSp` 200). **Rocket**: the recoil does the work, no height target.
- **Turbo** (`checkController`): while pumping with R, `dashSpeed += mDashAcc` toward 32 and feeds
  `intendedMag`; once the trigger is active and `mDashStartTime` passes, `fludd_emitting` makes
  Run use speed `mDashMax` (80, disc) and A becomes `BroadJump`.
- **Spray** (`TNozzleDeform`): output `0..1` from trigger pressure x `mEmitPowScale` (10), clamped.
  Aim: pitch eases toward `-mLAngleBase` (0), or with the stick while crouched
  (`+= stickY * mRButtonMult`), clamped to [-13000, 8000] at chase 0.018.
- **Crouch/aim**: hold R or L (not Rocket/Turbo nozzles) -> `Squat`; R-click + A while holding
  Spray -> back flip.
- Emission is reported as `WaterEmit` events. The game takes each emitter's position and
  direction (its X axis) from the posed nozzle's `null_G_muzzle` joints (`getEmitPosDirSpeed`);
  hosts report them with `Mario::set_emitters` (model space), else analytic stand-ins are used.
  Particles inherit Mario's `vel.xz / 8`.
- **Drawing state** (ported: `fludd_anim.rs`): the pumping arm is a second clip over `jnt_chest`
  and below (`M3UModelMario` slot 1): `gMarioAnimeData`'s pump clip for the body clip (`pump`,
  `hgpmp` hanging, `fepmp` fences, `swpmp` swimming), restarted at rate 1 on a change and
  ping-ponged while pumping; with an empty tank it runs at `raw R * mPumpAnmSpeed` (0.015, disc)
  with `PO_POMPING1/2` at frames 8/13. FLUDD's handles (`unk1CEC`): out by 0.1 a tick when pumping
  starts; **no water leaves until they are fully out**; folded back by 0.005 a tick after 360
  idle ticks (`wg_house`). The body clip follows the pump frame (`wg_pump`...). Each nozzle runs
  `TNozzleBase/Trigger/Deform::animation` over its clips, numbered in the disc archive's
  directory order: shoot start/shooting/shoot end while firing, swap clips driven by the swap
  progress, and the Spray nozzle's own housed pose. Joint callbacks aim it: `NozzleCtrl` (gun
  angle on the neck when negative), `WaterGunDivingCtrlL/R` (hover tilts), `RotateCtrl`
  (Turbo propeller). The nozzle models' `H_watergun_main_dummy` texture is FLUDD's texture 1.
- **Sounds** are `Event::Sound` at the decomp's call sites: Spray `PO_NORMAL_NOZZLE_IMI` (pitch
  by trigger pressure) and `PO_WATER_HI` (pitch by output; JALModSe curves), Turbo
  `PO_WATER_HI`, Hover/Underwater `PO_HOVER`, trigger charge `SY_NEWP_AIR_TAME`, Rocket/Turbo
  firing `PO_ROCKET_TRIGGER`/`PO_SNIPER_TRIGGER`, refilling `PO_SUCK_WATER_B`, X
  `SY_SELECT_POMP(_BACK)`. As in the game, Turbo's `PO_WATER_HI` gets the nozzle's output
  (`unk378`), which `TNozzleTrigger::movement` never sets, so its pitch curve gates it silent.

**Water gauge** (`TGCConsole2`, GCConsole2.cpp; ported: `libsms_present::hud`). The layout is
`game_6/scrn/standard_1.blo` (J2DScreen `blo1`), the `' w_0'` pane at (493, 355) of the 600x480
screen, its pictures in `game_6/timg/`. `drawWaterBack`: the splash `w_ba` in blue (TEV C0/C1
0x0000FF, alpha 0x78 x texture alpha); while a trigger nozzle charges (`isPressureOn`: its
`unk388 > 0`) the part below `y1 + (1 - p / pMax) * height` is 0xFF3F3F at alpha 0xC8, and at a
full charge `getPressureFlashColor` steps it to yellow and back over 25 frames. The screen draws
the glass `w_t0`. `drawWater`: two layers, `w_t3` in 0x00B4F0 then `w_t2` in 0x64DCFF, each at
`(1 - mask) * texture` alpha, the mask the same picture shifted by `fill * (1 - h)` of its height
(h 0.13 and 0.161; tops 10 and 7 below the glass) so the water's edge keeps the face's slant; the
top face `w_t1` at `glass.y + (int)(47 * (1 - fill) - 0.5) + 1` with alpha 0xFF full, 0 empty,
0x50 between; then `w_tx` ("WATER") and the fitted nozzle's icon `nz01..04` (Spray, Hover,
Rocket, Turbo). Not ported: the X-button hint, the slide in/out and the icon's swap animation,
Yoshi's juice gauge.

**Water particles** (`TModelWaterManager`, ModelWaterManager.cpp; ported: `water.rs`, stepped by
the host once per 30 Hz frame). 256 slots. `makeEmit` adds the trembles to direction, power and
size. Each frame: horizontal speed capped at the particle's size; flying ones fall by their
`WTPnn_*.prm` type's `mGravity`; on the ground a particle splashes (sound, 50% splash sprite),
shrinks by `mMagnify` and either sticks (50%) or vanishes; stuck ones run downhill with friction
0.93 wet / 0.7 dry, dry by 2 / 1 a frame from at most 256, and merge with neighbours closer than
2.5 x their sizes; walls likewise (80% splash, stick 50%, slide down 3 / 1 a frame). Water
surfaces swallow them. Particles over 5000 from Mario vanish. Drawing: `calcDrawVtx` quads
stretched along the screen velocity by `mExtension * 4`, summed into alpha and shaded as one body.

Not yet: Spray secondary "bomb" trigger (A + R, `mBomb`), the diving helmet nozzle
(swimming is not implemented at all), the nozzle mist particle (`0x10D`), graffiti sinking,
dirt/pollution, the point-blank spray message `hitNormal` sends to whatever Mario touches.

**Water hitting enemies** (`TObjHitCheck::checkWater`, ObjHitCheck.cpp; top of
`TModelWaterManager::move`): each frame every flying particle's attack cylinder
(`mStaticHitActor`: radius 50, height 80, standing at the particle) is tested against the
actors' damage cylinders (`checkDistance`); the first one it reaches gets
`HIT_MESSAGE_SPRAYED_BY_WATER` and, if it takes it, the particle splashes (`splashSound`, 30%
splash, `unk5D88[8]`) and is spent. The enemy's reaction is its own: small enemies lose
`getParticleAttack` hit points (at least 1, `TSmallEnemy::decHpByWater`); many are only pushed
or stunned. Here every enemy takes it, and `WaterEvent::Hit { id, pos, attack }` hands the
reaction to the host (ror2-mario turns it into damage, by the player's request, not the game).

**Host tuning, not the game**: `WaterManager::reach` multiplies the Spray nozzle's particle speed
and its horizontal cap (`size`); the landing distance grows about linearly (full R, standing:
960 units at 1, 1480 at 1.5, 2040 at 2). `Options::cycle_nozzles` makes X step Spray, Hover,
Rocket, Turbo, Spray: from Spray the backpack swings out to Hover as usual; from a secondary it
swings back and, at the halfway point where the game fits Spray, fits the next secondary and
swings out again; from Turbo it swings back to Spray.

## 7. Yoshi (Yoshi.cpp, Tongue.cpp, Item.cpp `TEggYoshi`, the `onYoshi()` branches)

Ported: `yoshi.rs`, `tongue.rs`, `egg.rs`. Assets: `yoshi.szs` (`assets/yoshi/`), the egg and
its bubble from the stage archives (`scene/mapobj/eggyoshi*`), the tongue from `mario.szs`.

**Egg** (`TEggYoshi`). It wants one fruit, shown in its bubble (`decideRandomLoveFruit`: banana,
papaya, pine or coconut at random; `eggyoshi_fukidashi.btp` frames 1/3/5/7/9 for banana,
durian, papaya, pine, coconut, 11 for right and 12 for wrong). Fruit Mario carries, or loose
fruit, inside its hit cylinder (radius 300, height 100) is `touchFruit`: the wanted kind plays
`eggyoshi_true` (`SY_COLLECT_PRETTY`) and turns the egg toward the fruit; any other kind, when
the current clip is about to end, plays `eggyoshi_false` (`SY_NOT_COLLECT_YOSHI`). When `true`
ends, `eggyoshi_broken` plays and Yoshi hatches (`appearFromEgg`): `born` clip, facing the
egg's way, eating that fruit out of Mario's hands. The shell vanishes when its clip ends. When
Yoshi is gone the egg comes back with a new wish (`startFruit`).

**Yoshi alone** (`STATE_UNMOUNTED`). Falls under Mario's `mGravity` (applied to his own
speed each tick, without Mario's quarter step) and stands on the floor; water, death or void
floors make him disappear. Every 120 to 240 ticks he looks for fruit within 300 of his mouth,
turns toward it by a tenth of the (unwrapped) angle each tick and tongues it (`doSearch`).

**Mounting.** Mario falling onto him from above (within 100 above his feet, closer than 180,
not carrying, not just thrown off or back-flipping) lands in the saddle (`checkCollision`):
`ride` (`YV_DELICIOUS`), FLUDD's nozzle becomes the Yoshi nozzle (FLUDD is switched on if
Mario had none). Otherwise Mario is pushed out of an 80 cylinder (`keepDistance`).

**Riding.** Yoshi's position and facing are Mario's. `thinkAnimation` picks his clip from
Mario's status (run with the walk blended in below 30, slide pose, jump, fall, flutter, hip
drop, wait) and Mario's clip is the matching `yo_*` (`setAnimation`'s jump table; `yo_run`
while spitting). `Yoshi.prm`: run target x `mRunYoshiMult` (1.2 disc), turn x `mRotYoshiMult`
(2.0), jump speed x `mJumpYoshiMult` (1.2); `SlipParamYoshi` friction on plain ground; wall
radius 80, and his head (`mHeadFront` 120 ahead, `mHeadRadius` 10) is pushed out of walls.
No dive, pump, grab, ledge hang or descent; running or jumping into a wall just stops; no hard
landings. B is the tongue, R spits juice, X hops off backwards (`BACK_JUMP`, `mGetOffYoshiY`
60 disc, +100 up, speed -8; FLUDD back to Spray with the Hover second and a full tank).

**Flutter** (`thinkHoldOut`). Each jump re-arms it; falling faster than 5 with A held pushes
up 1.2 a tick (against gravity 1.0) for up to 120 ticks, until A is let go (`YV_FUNBARI`).
Steering while fluttering uses `mHoldOutAccCtrlF/B` and `mHoldOutSldCtrl`.

**Juice.** Eating sets his colour (`doEat`: papaya and pine orange, durian purple, banana and
coconut pink; `YO_TONGUE_GOKKUN`) and refills `unk8` = 21300 ticks. It counts down always;
in the last 600 ticks he blinks and the colour is gone already (`entry`), at 0 he throws Mario
(`getOffYoshi(true)`: the 0x89C jump with `thrown`, `YV_DAMAGE`) and dies; the egg returns.
Riding onto a water-surface floor drowns him (`water_die`). The Yoshi nozzle is
`TNozzleDeform` with `NozzleYoshiMouth.prm` (particle type 12), from his `null_tongue` joint,
needs a colour, never empties (`mCurrentWater = mAmountMax` each frame), and doesn't wait for
FLUDD's handles. Water is drawn in the juice colour (`gModelWaterManagerWaterColor`).

**Tongue** (`TYoshiTongue`). `emitTongue` throws the tip from the mouth along the joint's X
axis at 10 a tick plus half Mario's velocity, running up to 10 steps at once until it is 160
out. Flying, it homes on the nearest fruit or enemy ahead (within 60 degrees, reach 1000;
enemies only when Mario shoots it, `findTarget`'s `allowExtra`) at 5% of the gap a tick and
sends it `HIT_MESSAGE_TAKE` within 200 (`YV_PERON`). Fruit always lets itself be taken; an
enemy only if it is edible (`TSmallEnemy::isEatenByYosshi`: small enemies yes, Boos and
Blooper kin no; `Host::take_enemy`), otherwise the tongue keeps flying. It gives up after 60
ticks, behind the mouth or in a wall. Reeling in keeps 70% of the length a tick and shrinks
the catch to 80%; within 80 it is swallowed (`HIT_MESSAGE_UNKB`: `Event::Eaten` for fruit,
`Event::EnemyEaten` for an enemy, which dies). Enemies don't change the juice. Alone he
only goes for fruit. Yoshi's head plays `eat` / `eat_end` over `jnt_18` while the tongue is
out or juice flows (`thinkUpper`).

The egg's wish is always the random one; `decideRandomLoveFruit`'s per-stage picks (a
pine in one stage, a durian in another, a banana for the shadow-Mario egg) are left out on
purpose, as is carrying the egg (`TEggYoshi` takes `HIT_MESSAGE_TAKE`).

Not yet: coins and the other non-enemy extras on the tongue, sidewalk/pivot clips (no L/R
strafing), `THROWN_DOWN` (the sim lands the throw in `BackDown`), swimming.

Drawing: Yoshi's, Mario's and FLUDD's models chain their joints Softimage-style (`INF1`
flags 1, `J3DMtxCalcSoftimage`): scales multiply down the hierarchy and are applied to each
joint's own matrix, not passed through the chain. The riding clips scale `jnt_neck_2` by 1.26
along X and `chn_head` by 1/1.26; chained the Basic way that stretches his head forward.
Joint callbacks (`YoshiHeadCtrl`, `NozzleCtrl`) turn the joint's children only.
`libsms::calc_joints` does all three modes.

## 7b. Damage, health and coins (MarioCollision.cpp, MarioReceiveMsg.cpp, Item.cpp)

Mario starts with `mHPMax` (8 on disc) health. A host reports an enemy's touch with
`Mario::damage(from, DamageKind)`; `receiveMessage` picks one of the `/Mario/Dmg*.prm`
sets by the attacker's actor type (`DmgEnemyCommon` for most, `DmgHamukuri` for the
Strollin' Stu, ...), and `damageExec` applies it:

- While invincible, at 0 health, or down, nothing happens.
- On Yoshi, no health is lost: Mario is thrown off (`getOffYoshi(true)`), Yoshi plays
  `yoshi_damage` with `yoshi_damage_tx` eyes and cries `YV_DAMAGE`; no invincibility starts.
- Otherwise Mario turns to the attacker and gets at least `mMinSpeed`. Attacker within 90
  degrees of where he faced: he keeps facing it and slides backwards (`*_BACK_DOWN`);
  behind him: he turns away and slides forward (`*_FORE_DOWN`). The status comes from
  `mDownType` + airborne + direction: type 0 `SAFE_*_DOWN` (a stumble, `sfbdn`/`sffdn`), 1
  `SHORT_*_DOWN`, 2 `*_DOWN`, 3 none; in the air `JUMP_SHORT_*_DOWN` / `JUMP_*_DOWN`. He drops
  what he carries. `mInvincibleTime` ticks of invincibility start (300), the health drops by
  `mDamage`, FLUDD loses `mDamageLoss` per `mWaterEmit`, and he cries by size: `MV01` for
  more than 1, `MV03` for a stumble, else `MV02`.
- While invincible the body, cap and hands get a pink fog (`addDamageFog`: colour (255, 0,
  128), linear from 700 in front of Mario to 500 behind, both swinging 300 with
  `sin(moveTicks * 0x888)`, a quarter-second pulse). The countdown is `thinkParams`, one a tick.
- At 0 health (`loserExec`): `HP_DOWN`, Yoshi runs off, and `DOWN_LOSER` (`loserDown`: the
  `die` clip, `MV08A_DOWN`, then `MV08B_DOWN_REACT`). `Event::Down` hands over to the host.

Disc tables: EnemyCommon 1 damage, type 1, speed 30; Hamukuri 1, type 0, speed 30; Hinokuri
3, type 1, 2 water bursts; Killer 3, type 1. `mInvincibleTime` is 300 unless a file says
otherwise.

Coins (`Mario::get_coin`): gold heals 1 (`getCoin`) and counts, red and blue heal 2; healing
stops at `mHPMax` and chimes `SY_HP_RECOVER` only when it healed (`incHP`). The coin's own
sound comes with it (`SY_COIN`, `SY_RED_COIN_GET`, `SY_BLUE_COIN_GET`).

Not ported: the run flinch for type 3 (`setUpperDamageRun`), the water the hit sprays out,
camera shake and rumble (hosts can use `Event::Damaged`), dirt, air underwater, trampling
enemies, lives and the 100-coin Shine.

## 7c. Mario against enemies (MarioCheckCol.cpp `checkCollision`/`hitNormal`, MarioCollision.cpp `trampleExec`, MarioPhysics.cpp `keepDistance`)

(ported: `collide.rs`.) Mario's attack cylinder is `mTrampleRadius` (100) wide and
`mAttackHeight` (50) tall from his feet; each tick the actors whose damage cylinders it reaches
(`checkDistance`: vertical ranges overlap and XZ distance under the radii's sum) go in his
collision list, five at most. For enemies (`hitNormal`):

- Jumping, falling (`vel.y < 0`) and above the enemy's feet: in `HipDrop`, send
  `HIP_DROP` (the ground pound carries on through); otherwise `trampleExec` (not in a dive):
  send `TRAMPLE` and bounce. A long jump turns into a back flip; else status `TRAMPLE` with clip
  `step1`, or `step2` after `step1`, or `step3` (restarted) after `step2`/`step3`, and
  `setStatusToJumping(TRAMPLE)`: `vel.y = 0.25 * forward + mTramplePowStep1/2/3` (30/40/50 by
  clip), `forward *= 0.8`, the small/mid/large jump voice. In the air `trample` is
  `jumpingCommonEvents` then `jumpingBasic(SEC_JUMP_SLIP, clip, 3)`; it allows the short hop.
- Belly sliding (`CATCH`) or diving (`JUMP_CATCH`): send `PUNCH` (then `TRAMPLE`).

Each message is `Event::EnemyHit { id, kind }`, once per enemy per status (the game's enemies
react to the first and die or flinch; a host's may outlast it). The game keeps Mario out of only
some actors (`keepDistance(actor, 0)`: Kumokun, the Manta's children, ...); the rest he passes
into and they hurt him. `Options::enemy_push` (host tuning) keeps him out of every enemy he isn't
stomping: pushed to the radii's sum at most 50 a tick, or knocked down (`JUMP_SHORT_BACK_DOWN`)
when running into it faster than `mClashSpeed`.

Not yet: `SUPER_HIP_DROP` from below (`MARIO_FLAG_UNK200`), pick-up enemies, the fence and
roof punches, NPCs.

## 7d. Host limits and host forces (`limits.rs`, `Mario::launch`/`add_water`; not the game)

None of this is Sunshine's. It exists for hosts that bill some of Mario's moves as abilities
with charges and cooldowns (Risk of Rain 2's skills and items). `Limits::default()` is the game:
nothing limited, nothing stretched, and every scenario that doesn't set limits plays as before.

- `air_uses`: each start of a nozzle while airborne (R pressed, or another nozzle fitted while
  R stays down) spends a use; one is free per time off the ground, then `air_extra` (handed in
  by the host and counted down). With none left R reads as up until it's let go. A use lasts
  while R stays down; starting on the ground is free. Reported as `AirNozzle { extra }`.
- `air_jumps`: A in the air does the next jump of the chain as if timed perfectly on a landing
  (from a fall a plain jump, then the double, then triples), unless a stick spin makes it a
  spin jump or a stomp's late window wants it. Back on landing. Reported as `AirJump`.
- `spray_rate`: the Spray's drops a tick times this, each taking that much less water (a
  fractional carry keeps the total exact). `spray_ticks`: the Spray stops after that long until
  R is let go.
- `hover_time`: the Hover's `mTriggerTime` times this. `turbo_speed`: ground covered while the
  Turbo dashes times this, on top of `Options::move_speed`.
- `spray_fade`: a Spray press lasts this many times as long before it fades (the pressure
  gate climbs `mTriggerRate / spray_fade` a tick; see section 9).
- `wet_slide`: a belly slide turns wet (`mWasOnWaterSlip` friction, as over FLUDD's water) with
  a charge (`ready`), over FLUDD's water or (`anywhere`) on any ground; it glides for
  `max_ticks` then turns plain. Without a charge it stays plain, unless `water_free`: then a
  slide over FLUDD's water glides as in the game, free and uncapped. `glide`: a wet slide keeps
  its speed as if the stick were held forward (`doSliding`'s stick term at full) unless the
  stick pulls back. `WetSlide` reports a charge spent.
- `PoundStart`: a ground pound's wind-up began (for hosts that charge for pounds).
- `Mario::launch(vel)`: the host's world throws Mario (jump pads, knockback): `mVel = vel`,
  facing along its horizontal part (or away from it, flying backwards, when that's mostly behind
  him), status `FORCE_JUMP` until he lands. `Mario::boost_jump(k)`: horizontal travel in the air
  times `k` until landing (a sprint-jump item). `Mario::add_water(share)`: tops up the tank.
- `Mario::aim` (the game's, from the host's camera): `calcGunAngle` points the nozzle at the
  L-button camera's pitch while it's on. Not the game's: a crouching Mario turns to its
  heading (half the way a tick), since the host's player steers that camera; the stick then
  side-steps (L) as in the game, and no longer turns him (R).
- `Mario::knock_out()`: the host's health ran out; `loserExec` as at 0 health.

## 8. Verified numbers (disc tuning, headless, `cargo run -p libsms --example report`)

| What | Sim |
|------|-----|
| Full run | 879 units/s = 5.5 body heights/s (forward speed about 26; `doRunning` subtracts `Yoshi.prm`'s `mDecBrake` too, riding or not) |
| Standing jump | apex 226 (1.41 heights) at 0.35 s, airtime 0.71 s |
| Triple jump | apex 810 (5.1 heights) |
| Hover | holds ~3.0 s at ~400 units (target 230 + overshoot), costs 7% tank |
| Rocket | 1.48 s charge, apex ~2780 (17 heights), costs 8% tank |
| Turbo | reaches 80 after 0.76 s |

## 9. Unverified (needs a play-test in the real game)

The viewer now routes animation and game-event sounds through `libsms-audio`; see
[audio integration](audio.md) for source references, timing and fidelity limits.

1. **Spray fades to nothing.** `TNozzleBase::movement` (99.8% match) computes output as
   `(pressedLevel - gate) / 65536` where `gate` ramps up at `mTriggerRate` (50/tick), so a held
   full press decays to zero after ~6 s, and a light press is a short burst that lands just
   ahead (raw R 15: 0.23 s of water landing 0.6-2.4 m out; raw 25: 0.8 s; 40 and up: full
   reach). Play memory of light-press bursts backs it, so this is now the default
   (`Options::spray_pressure_fades = true`); a host can still ask for a steady output.
2. **Tank drain is slow.** Spray costs `emitted * mDecRate` = about 1 unit per tick against a
   10000 tank: ~83 s of continuous spray at 120 Hz. Might be right, might not.
3. **Rocket height.** 17 body heights looks too high versus play memory; the thrust is the
   literal `reaction_y * power` for 5 ticks. The particle manager's `emitRequest` can return fewer
   than `count` when its particle budget is full, which would cut thrust; we assume it never does.
4. Clip names for slip/belly-slide/back-flip are approximations (decomp anim ids map through
   `gMarioAnimeData`, not ported); they only affect timing and the host's skeleton mapping.
5. **Nozzle clip speed.** `MActor` clips are set to `SMSGetAnmFrameRate()` = 2 frames per
   update and update in `calcAnm`, which Mario runs every 120 Hz tick; read literally that is
   four times Mario's own clip speed. The sim plays them at Mario's 60 fps instead
   (`fludd_anim::NOZZLE_RATE`).
6. **Spray particle size.** `TNozzleDeform::emit` (NonMatching) computes the size ramp as
   `(min - a) / (max - a)`, negative inside the range; the sim uses the plain
   `(a - min) / (max - min)`.
7. **Grab from a standstill.** `waitingCommonEvents` starts `TAKE_POSE` on `mInput & 0x10000`,
   which nothing in the decomp's `checkController` sets; the sim treats it as a B press.
8. **Water timing.** `TModelWaterManager::move` runs on `CUE_MOVE`; the host steps it once per
   30 Hz frame, consistent with its per-frame velocities and Mario's `vel` convention.
9. **The tongue while it flies out.** `TYoshiTongue::calcAnim` sets the shape-hide flag while
   `EXTENDING` and only lays out the arc in the other states, so read literally the tongue is
   invisible on the way out (up to 60 ticks without a target). The viewer draws it whenever it
   is out.
10. **Yoshi's clip rates.** His clips update every tick at 0.5 frames, the mounted run at
   `0.1 + 0.045 * forward speed` frames a tick (`unkA0/A4`, about 1.8 at full speed), and the
   eating head at 1 frame a tick, all as the decomp sets them.
11. **Drowning while riding.** The sim has no swimming; riding onto a floor flagged
   `WATER_SURFACE` stands in for `thinkWaterSurface`'s deep-water check.
## RoR2 yellow coin drops

Enemy deaths credited to a Mario body leave one yellow coin (host rule).
`Look::Coin` / `sms_view_new_look(..., 2)` expose the original
`scene/mapobj/coin.bmd` bind pose and gold material through the normal model readers.
Hosts instance and rotate it without running Mario animation on the coin view.
Pickup uses `MSD_SE_SY_COIN` (`TCoin::taken`, MoveBG/Item.cpp) and both
`ms_coinget_a` / `ms_coinget_b` effects (`TMario::emitGetCoinEffect`, MarioParticle.cpp).
`sms_view_coin_get` plays these at the collection point; the host owns pickup
eligibility and healing. The core get_coin path now emits both effects too.
RoR2 heals one life-meter segment (fullCombinedHealth / 8) through its normal
Heal method, capped by available health and subject to healing-item rules.
Coins fall and settle on world geometry, spin at two degrees per sim tick
(TItemManager::perform), and use the coin/Mario pickup cylinders.
The host caps loose coins at 128, expires them after 60 seconds, and clears them
on scene changes and run exit. Nintendo files stay in the ignored assets folder.
