# Goop, goop critters and Bowser Jr. (2026-11-05)

Core modules: `goop.rs` (the field), `dirty.rs` (goop on Mario), `goop_critters.rs` (Swoopin'
Stus, goop Piranhas, their shots), `koopajr.rs` (Shadow Mario / Bowser Jr. as a player).
C ABI: `libsms-ffi/src/goop.rs` (`sms_goop_*`, `sms_sim_set_goop`, `sms_sim_set_koopajr`,
`sms_jr_*`), ABI 46. Headless tests only. **Nothing here has been played in game, and there is
no RoR2 survivor yet.**

## Decomp sources
- Pollution: Map/PollutionManager.cpp, PollutionLayer.cpp, PollutionAction.cpp
  (`action`, `fire`, `electric`, `glassWall`, `cleaned`), PollutionLayer.hpp `POLLUTION_TYPE_*`.
- Mario: Player/MarioMove.cpp `checkGraffito`, `checkGraffitoFire/Slip/Elec`, `checkSink`,
  `thinkDirty`, `isForceSlip`; MarioRun.cpp `oilRun/oilSlip/oilSlope`; MarioAutodemo.cpp
  `electricDamage`, `sinkLoser`; MarioDraw.cpp `addDirty`, the sink translation.
- Critters: Enemy/namekuri.cpp (`TNameKuri`, launcher), pakkun.cpp (`TPakkun`, `TPakkunSeed`).
- Shadow Mario: Enemy/enemyMario.cpp (`emHide`, `emAppear`, `emDrawStamp`, `emDisappear`,
  `emJumping`, water counter). Bowser Jr.: Enemy/limitkoopajr.cpp.

## Goop kinds (`GoopKind`)
| kind | value | threshold | what Mario gets |
|---|---|---|---|
| Sink | 0 | 30 | sinks, floor hit every `mSinkDmgTime`, slowed, jumps cut; `SINK_LOSER` if out of health |
| Fire | 1 | 128 | `DmgGraffitoFire` hit and `FIRE_DOWN` within `mFireHeight` |
| Slip | 2 | 30 | `OIL_RUN`/`OIL_SLIP`/`OIL_SLOPE` coasting, spreads oil, floor hit per fog cycle |
| GlassWall | 3 | 30 | put back where he was |
| Electric | 4 | 30 | `ELECTRIC_DAMAGE` after `mGraffitoNoDmgTime` |
| Instakill | 5 | 30 | `SINK_LOSER` at once, plunging |
| Safe | 6 | 30 | dirt only |
| Scorch (UNK7) | 7 | 200 | as Fire |
| Poison (host) | 16 | 30 | `DmgGraffitoPoison` stumble every `mSinkDmgTime` (the game has no check) |
| Lava (host) | 17 | 128 | as Fire with `DmgGraffitoLava` (`checkGraffitoLava` is empty) |

Ambient effects near players (`GoopField::ambient`): fire kinds `ms_newfire_b/_a` at 1.5x +
`MSD_SE_EF_FIRE` every 21 ticks within 1600; electric `ms_thunder_s` + `MSD_SE_EF_ELEC` every 16
within 1000; the rest `ms_raku_indawa` ripples every 16 within 2000. Cleaning dense goop:
`ms_raku_kie` and `MSD_SE_ERASE_SCRAWL`, one call in six.

## Host choices (not the game)
- One sparse world field of 32-unit texels in 16x16 chunks; each texel keeps density, kind,
  height and paint time.
- **Lifetime**: 90 s after last painted, thinning over the last 5 s (`lifetime`, `fade`; 0 =
  forever, the game's). Swept one chunk slot in 30 per tick.
- **Cap**: 48k texels and 768 chunks; past either the oldest goop is cleared first.
- Mario's oil takeover only from standing/running statuses (the game does it from any status on
  the ground; that would eat take-offs here).
- Critters target Marios only within `mario_range` (400), enemies within `mSLSearchLength`.
- Reconstructed: the Stu's leap (`TNerveNameKuriJumpAttack` isn't decompiled; vertical speed
  solved to reach the target), the Piranha's swell (`unk1B8` is never written; here it grows with
  water taken, `burst_water`), the goop shot's splat scale (`unk158`), mouth offsets, the sign's
  size, the brush interval, Bowser Jr.'s shot arc and his clip choice (the disc has no Jr walk).
- FLUDD cleaning: `WaterManager::cleans` records each ground splash (`mCleanSize * 10`) and wall
  splash (`mCleanSize * 32`); the FFI cleans the goop world there.

## Disc values used (PAL)
dirty.prm, graffito.prm (FireHeight 5, FireInvincibleTime 50, SinkTime 240, SinkDmgTime 90,
SinkDmgDepth 0.6), dmggraffitofire/elec/poison/lava.prm, dmgnamekuri.prm, namekuri.prm,
pakkun.prm. Code defaults stand in where a file is absent.

## Not done
- Presentation: the real swirling Shadow Mario skin (`H_kagemario_main_i8`, the screen-texture
  dummy, the `indwp4n` warp scrolled by `kagemario_scroll.btk`), Mario's dirt material
  (`H_ma_rak.bti` at konst alpha = `dirt_share`), the goop surface (the `pollution0N` layer
  models), Bowser Jr./Stu/Piranha/seed models in the view, the stamp masks from
  `pollute.bti`/`clean.bti` and the `stamp_koopa_sign` silhouette (discs stand in).
- The RoR2 survivor (plugin, skills, skin, multiplayer).
- Layer spreading (`FLAG_CAN_SPREAD`), footprints, `OIL_PULLING`, rumble.
- Petey: no C ABI yet, no model in the view, no `.bas` playback wired (events name the clips).

## Petey Piranha, the sidekick (`petey.rs`, 2026-11-05)
`TBossPakkun` (Enemy/bosspakkun.cpp, `bosspakkun.prm`) as a core: `Petey::from_dir(assets, pos)`,
then `Petey::step(host, goop, owner, targets)` each tick after Bowser Jr. Headless only, no FFI
yet, never played.
- The nerve chain as `TSpine` runs it (stack, `pushNerve`, `reset`+`setNext`). Wait → head swing
  (target within `mSLSwingLength` and 60°, thrown sideways and up at 100) or pivot
  (`mSLPivotSpeed`/`Aware`) → vomit (`pollut_start` at `mSLVomitAnmRate`/2 speed, mouth open at
  frames 25-165, then the goop pours out and the wind blows), goop ball (`SMSCalcJumpVelocityXZ`
  at `mSLPollBallSpeed`, 0.1 gravity, stamp 300 x `mSLPollBallStampScale`) or tornado (circles
  600+ out from a centre heading for the target at `mSLTornadoSpeed`, 721 ticks, throws up at 100).
- Water into the open mouth from within `mSLDamageAngle` fills the mark (limit 250 on the disc)
  → swallow, then tumble with the navel up for `mSLTumbleTime` (1000) → `stomp()` costs one of 3
  HP → stomp react, get up; 0 HP → `bosspaku_end`, `Defeated`. `trample()` = standing on the
  navel. `belly()`/`belly_scale(4|36)` and `head_yaw` for the view.
- Flying: take off (5 a tick), fly to the owner, touch down; `flying_form` hovers instead (the
  `Fly` weak point: `water_hit`/`knock` drops him, `fall_*`), shooting balls at tick 168.
- Events (`PeteyEvent`): clips with blend ticks, `MSD_SE_BS_BSPAKU_*` sounds, `ms_bopa_*` effects
  on joint indices (`held` = bound emitters), hits, splats, shakes, the water spout, coins.

Host choices: the owner leash (1500), follow distance (500), fly height (800), attack range
(2500), Marios only within 400, the attack pick (tornado one time in `mSLTornadoProp`, the ball
past the vomit's reach, else the vomit), sleep after 7200 idle ticks, `respawn`. Reconstructed:
joint positions (offsets unless the host sets `joints`), the vomit footprint (discs marching to
1800 out), the ball stamp's base radius, fall gravity, fly speed as a per-tick speed with a
catch-up multiplier, the tornado fade (60), one hit per target per vomit/swing, the ball
checking the ground from tick 0 (the second fight's rule). A tornado never touches a target
standing still at its goal (it circles 600 out), as in the game.
