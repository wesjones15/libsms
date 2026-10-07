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
becomes "super" after `mSuperHipAttackCt` (80) ticks.

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
  `SY_SELECT_POMP(_BACK)`.

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
dirt/pollution, water hitting actors.

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
   full press decays to zero after ~6 s. We default to a steady output
   (`Options::spray_pressure_fades = false`). If the game really fades, flip it.
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