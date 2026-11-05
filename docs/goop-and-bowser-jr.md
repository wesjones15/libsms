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
- Petey Piranha (`bosspakkun` assets are extracted; no code).
