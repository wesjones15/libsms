# Goop, goop critters and Bowser Jr. (2026-11-05)

Core modules: `goop.rs` (the field), `dirty.rs` (goop on Mario), `goop_critters.rs` (Swoopin'
Stus, goop Piranhas, their shots), `koopajr.rs` (Shadow Mario / Bowser Jr. as a player).
C ABI: `libsms-ffi/src/goop.rs` (`sms_goop_*`, `sms_sim_set_goop`, `sms_sim_set_koopajr`,
`sms_jr_*`), ABI 46. **Nothing here has been played in game.** The RoR2 survivor is built
(ror2-mario, see "RoR2 survivor" below) but unplayed.

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

## Presentation (libsms-present, 2026-11-05)
Headless tests only; nothing has been drawn in RoR2 or played.
- **Shadow Mario skin** (`Skin::Shadow`, `sms_view_skin` 4 after `sms_view_load_skin` 4;
  `shadow_skin.rs`): `kagemario/default.bmd` on Mario's 29 joints (same rig, so Mario's pose
  drives it) with the paintbrush (`kagemario_brush.bmd`) on `jnt_hand_L` (`TEnemyMario`). The
  game's material warps the *screen* behind him (`gpScreenTexture` into `H_kagemario_dummy`,
  an indirect warp by `indwp4n` read through an env-mapped matrix that `kagemario_scroll.btk`
  scrolls) through the REG0→REG1 ramp, masked by `H_kagemario_main_i8` into REG2's navy. Hosts
  can't read the screen through the view, so every batch shares one texture rebaked each game
  frame (30 Hz, a new `sms_view_texture_version`): the same two stages with the warp read twice
  (the second pushed by the first, both scrolled by the btk) standing in for the screen.
  `ShadowSwirl::params` holds the real inputs for a host shader that can grab its screen.
- **Mario's goop** (`TMario::addDirty`): `H_ma_rak.bti` replaces every `H_ma_rak_dummy` on the body,
  cap, hands and same-rig bodies, and konst 0's alpha is `Mario::dirt_share` in 5 steps
  (`DIRT_LEVELS`; each step is one more bake per material). The goop enemies' and the brush's
  dummies get the mask too, at their own konst alpha.
- **Props** (`sms_view_new_look` 9-14, posed by `sms_view_prop_update`): Bowser Jr., the Stu,
  its brain (rides `sms_view_prop_joint` 2 of the Stu's pose, `TSharedParts`), the goop Piranha,
  its seed/goop ball, and Bowser Jr.'s sign with its draw clip.
- **Goop surface** (`goop_surface.rs`; `sms_goop_chunk_image`, `sms_goop_tile`): each kind's
  layer model (`GoopKind::surface`) baked through its TEV with the density map read as full and
  the matrices left out, so it tiles seamlessly; a chunk's picture lays that tile in world
  space (400 units a repeat) with the density, bilinear across texels and chunks, eased around
  the kind's threshold as alpha. The layer files are per stage on the disc and the extractor
  keeps the first copy of each name (from a test stage), so the looks are: brown chocolate
  (`TestChoco2`) for sink/instakill/electric, the pink-to-yellow paint (`B_RAKenogu_pink`) for the
  fire kinds, glass and poison, see-through slick for oil and safe. The paint swims as on the disc:
  each layer's btk slides its warp (`B_ind_metal`) one repeat every 2000 game frames (67 s), so
  each tile is 12 bakes over that loop, blended by time (`sms_goop_chunk_image_at`; the two slick
  layers don't move).
- **Stamps**: `sms_goop_setup` reads `pollute.bti` and `clean.bti` into the field's stamps (the
  channel that varies, stretched to full strength) and projects Bowser Jr.'s sign from above as
  `stampModel` paints it: the three pens of `stamp_koopa_sign_model1` everywhere the draw clip
  takes them, turned half round, at `mBrushScaleupDuringDrawing` (3) — his graffiti mark. A
  Bowser Jr. sim standing in that world takes it.

- **Screen-grab swirl** (`ShadowSwirl::refraction`, `sms_view_shadow_refraction`): the warp's
  IA offsets at this game frame's scroll, over the body's coordinates, packed as RoR2's
  distortion normals (`[255, -y, 255, x]`, peak 0.8, the gain returned) for a host's
  refraction pass over the body.

## RoR2 survivor (ror2-mario, 2026-11-05; built, not played)
- `KoopaJr.cs`: survivor `KoopaJrTest` / body `KoopaJrTestBody`, a Commando clone with Mario's
  `MarioDriver` and a `KoopaJrBody`. The driver calls `sms_sim_set_koopajr` after
  `sms_sim_new`, routes his buttons in place of FLUDD's (RT held paints as Shadow Mario, RT
  tap a goop ball as Jr, RB tap form, RB hold 0.5 s next kind, d-pad up sign / seed, d-pad down
  hide / appear; keyboard 1 and 2 for the d-pad), hides Mario's model in Jr form or hidden and
  draws look 9 posed by `jr_clip`. HiddenInvincibility while under the goop. Skin: native 4
  (`MarioSkin.Shadow`, `OwnSurvivor`, never in Mario's loadout). Skills: Magic Brush and Change
  Form (descriptive, Commando's icons), Mario's own Ground Pound and Wet Slide defs.
- `KoopaJrGoop.cs`: one world for the process (setup on a worker the first time a Jr is in
  play; `sms_goop_clear` each new stage), handed to every Mario sim. The server steps it with
  the enemies within 80 m of any Jr as targets; `sms_goop_contacts` hurts enemies by kind (a
  host choice; `BowserJr.GoopHurtsEnemies`), critter hits land as Jr's damage, Piranha bursts
  throw coins, sounds go through the player's view, particles through an effects view (look 8).
  Every machine draws the chunks (a quad per goop texel at its height, a 64x64
  `sms_goop_chunk_image_at` picture, nearby chunks redrawn each second to swim) and the Stus
  (with brains), Piranhas and seeds as props.
- `MarioShadowSwirl.cs`: the body drawn again with RoR2's `matDistortion` and the refraction
  map above, re-uploaded each game frame (`BowserJr.SwirlRefraction`, 0 off).
- Multiplayer (`KoopaJrGoopNet.cs`, ABI 47; built, not played): the server's world is the real
  one. A client's world is a mirror (`sms_goop_set_mirror`): its sims stand in it, paint and
  clean it at once, and the field's op log (`GoopField::log`, `GoopOp`) plus the mirror's
  critter ops (shots and seeds taken out of its unstepped critters, stomps and water on
  critter ids) go to the server each step (`sms_goop_take_ops`), which redoes them on its own
  world (`sms_goop_apply_ops`, values range-checked, a client Jr's shots owned by his
  netId-based id). The server sends changed chunks every 0.1 s (16 at a time, a live-texel
  bitmask, a base height and per texel density, kind and u16 height; removals too) and the
  critters within 120 m of a player every 0.1 s (unreliable); the mirror applies chunks with
  `sms_goop_chunk_set` / `_drop`, except ones its own ops touched in the last 0.6 s (those
  wait), and draws `sms_goop_mirror_critters` eased toward each snapshot. Goop sounds and
  bursts go to every machine. Bowser Jr.'s form, hiding, clip and paint kind (`JrStateMsg`)
  go to the other players, whose puppets hide Shadow Mario and draw look 9; the server keeps
  HiddenInvincibility on a client's hidden Jr.
- Not done: critters never target a Mario teammate, critter damage is credited to the first
  Jr, Stu TEV tints, skill icons, a lobby display of the Jr form, and Bowser Jr.'s walk.

## 2026-11-06 pass (built, not played)
- **Shadow Mario's look**: every kagemario material is blend 0 with z write (opaque). TEV stage
  0 lerps REG0 (28,20,93) to REG1 (157,194,255) by the screen-space map, stage 1 lerps by
  `H_kagemario_main` and adds RASC. So the body draws solid with its bake and the
  `matDistortion` refraction pass draws over it (`MarioShadowSwirl.cs`); nothing is see-through.
- **Voice** (`MSound.cpp startMarioVoice`, voice status 2 for `PLAYER_TYPE_SHADOW_MARIO`):
  Mario's lines with no `MA_VO` random alternates, pitch 1.2, volume 0.9, port 11 = 1
  (`VoiceSet::Shadow`, `sms_view_voice_set` 3).
- **Brush attacks** (host moves, `koopajr.rs` `Brush`): RT tap swings `ma_draw` 28 to 72 over
  the upper body and throws a `ShotKind::Paint` blob at frame 44 toward `sms_jr_set_aim`. Held,
  he sprays small blobs while swaying 40 to 52. Each blob splats his paint kind on first touch
  and carries a damage share (hit event `a >> 8`, hundredths). The d-pad sign is set aside.
- **Effects**: the effects view (look 8) also loads `GOOP_EFFECTS` from their own folders
  (`scene/map/pollution`, `enemy/kagemario/jpa`, `enemy/koopajr/jpa`,
  `enemy/gatekeeper/jpa`, else `particle`). `ms_thunder_s` and the two smokes are directional
  and draw nothing from a standing test emitter; not checked in game.
- **Gatekeeper** (`goop_gatekeeper.rs`, `TBiancoGateKeeper`): grows from a pile of at least
  110 texels within 400 units, searched within 2500 of the owner every 60 ticks (at most 1,
  1800-tick cooldown). It runs appear1, then wait3 (swaying and turning toward the target),
  then shoot1, launching at spine frame 0x34. It dives (dive1) after 2400 ticks or when its
  pile is cleaned below half. The launch is an arced Paint blob of the critters' goop (splat
  260, damage share 2), not a Gorogoro. Effects: kemuri, bota, yodare. Sounds: GATEKEEPER
  APPEAR/WAIT/HIDE. It is FFI critter kind 4 (`clip_time` in half-frames),
  `sms_goop_keeper_tune`, look 15 with the goop mask in place of `Q_kepper_dummy_128IA4`. It
  can't be hurt. Its goop flows (`TMultiBtk`: `gene_pakkun_tex0/tex1.btk` at a btk frame a game
  frame, re-baked every other frame, `sms_view_prop_scroll`). Not done: the `BASE` loop sound,
  effects bound to joints (they are emitted at the mouth), and Gorogoro/Stu launches.
- **Goop surface** (`KoopaJrGoop.cs`): each chunk corner is raycast onto the stage's own ground
  (lift 4 cm), so neighbours meet with no grid seams and nothing sinks under the map. The
  chunks use HGStandard cutout (lit, a little gloss). The swim redraw is now every 0.15 s
  within 30 m.
- **Shadow Mario's swirl loop**: `kagemario_scroll.btk` repeats every 200 frames and lands
  where it started (scale 1 to 2 to 1, slide of 3 widths). The bake's second warp read drifted at half
  the slide (1.5 widths a loop) and popped at the wrap; it now drifts a whole number of widths
  a loop. The bake also applies the btk's scale about its centre as J3D does
  (`shadow_skin::tests::the_swirl_loops_with_no_jump`).
- Critters are now placed correctly (ModelView placed them at pos*scale), and goop balls draw
  with the seed model.

## Not done
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
