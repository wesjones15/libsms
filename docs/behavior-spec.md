# Mario locomotion and FLUDD: behavior spec

Written from the doldecomp/sms decomp (`../sms-main`, GMSP01 build, verified against the PAL
disc). Function names in parentheses are decomp symbols. Numbers marked **(disc)** come from the
`.prm` files in `params.szs` on the user's disc; the constructors' code defaults differ and are
only fallbacks.

Asset pipeline: `libsms::extract` reads the user's disc (plain ISO/CISO), decompresses `.szs` (Yaz0) and
unpacks RARC archives; output is verified byte-identical to decomp-toolkit's. See the README.

Status of each rule: **ported** (implemented and tested), **partial**, **not yet**, or
**unverified** (ported as written but contradicts expectation; see the end).

## 1. Time and units

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

Air control (`doJumping`): `forward *= mJumpSpeedBrake` (0.999); with the stick, `forward +=
mag * cos(d) * mJumpAccelControl (0.008)`, sideways `mag * sin(d) * mJumpSlideControl (0.3)`;
`> 32` decays by 0.2, `< -16` by 0.4.

Landing results from `jumpProcess`: 1 floor, 2 wall within `mClashAngle` (12000) of head-on, 0 air.
A head-on wall hit at speed > `mClashSpeed` bumps; > 16 reflects and starts a wall slide
(`WallSlide`, jump to kick off; slides at `-timer/2` after 20 ticks); else stops dead.

Dive (`JumpCatch`) lands into `Catch` (belly slide), which jumps out (`CatchStop`) or gets up
(`CatchLost`). Ground pound (`HipDrop`): 60 ticks of hover, then falls at `mHipAttackSpeedY` (-80);
becomes "super" after `mSuperHipAttackCt` (80) ticks. Ledge grab and the hanging/climbing states
are **not yet** implemented (option `ledge_grab`, off).

## 6. FLUDD (WaterGun.cpp, NozzleBase/Trigger/Deform, MarioUpper.cpp)

Slots: 0 Spray (`NozzleDeform.prm`), 1 Rocket, 2 Underwater, 3 Yoshi, 4 Hover, 5 Turbo.

- **Tank**: `water` in the current nozzle's units; `mAmountMax` (disc) 10000 Spray/Hover/Diving,
  2100 Rocket/Turbo. Changing nozzle rescales to the same fraction (`changeNozzle`).
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
- Emission is reported as events; water particles, cleaning, enemy damage, and sounds are the
  host's. Emitter position/direction come from the FLUDD skeleton in the game; the sim uses
  analytic stand-ins unless a host overrides them.

Not yet: Spray secondary "bomb" trigger (A + R, `mBomb`), Yoshi juice, the diving helmet nozzle
(swimming is not implemented at all), the nozzle-swap animation timing/deploy delay
(`unk1CEC`), graffiti sinking, dirt/pollution.

## 7. Verified numbers (disc tuning, headless, `cargo run -p libsms --example report`)

| What | Sim |
|------|-----|
| Full run | 891 units/s = 5.6 body heights/s (forward speed 28.9) |
| Standing jump | apex 226 (1.41 heights) at 0.35 s, airtime 0.71 s |
| Triple jump | apex 810 (5.1 heights) |
| Hover | holds ~3.0 s at ~400 units (target 230 + overshoot), costs 7% tank |
| Rocket | 1.48 s charge, apex ~2780 (17 heights), costs 8% tank |
| Turbo | reaches 80 after 0.76 s |

## 8. Unverified (needs a play-test in the real game)

1. **Spray fades to nothing.** `TNozzleBase::movement` (99.8% match) computes output as
   `(pressedLevel - gate) / 65536` where `gate` ramps up at `mTriggerRate` (50/tick), so a held
   full press decays to zero after ~6 s. We default to a steady output
   (`Options::spray_pressure_fades = false`). If the game really fades, flip it.
2. **Tank drain is slow.** Spray costs `emitted * mDecRate` = about 1 unit per tick against a
   10000 tank: ~83 s of continuous spray at 120 Hz. Might be right, might not.
3. **Rocket height.** 17 body heights looks too high versus play memory; the thrust is the
   literal `reaction_y * power` for 5 ticks. The particle manager's `emitRequest` can return fewer
   than `count` when its particle budget is full, which would cut thrust; we assume it never does.
4. Clip names for slip/belly-slide/back-flip are approximations (decomp anim ids map through
   `gMarioAnimeData`, not ported); they only affect timing and the host's skeleton mapping.
