# Pikmin 2 treasures (prep for RoR2 Ship Repair)

Everything a host needs to place Pikmin 2's treasures: extraction from the user's own disc, a typed config
table, baked models with their carry wobble, names, the sound effects and jingles, and the particle
effects they use. No Nintendo data is in git; every file below lands in the ignored `assets/p2/treasure/`
cache. Nothing here is wired into a game yet, and nothing has been played.

Spec read: `PelletList::Mgr` (`gamePelletList.cpp`), `PelletConfig` (`pelletConfig.cpp`), `Pellet`
(`pelletMgr.cpp`), `PelletOtakara` (`pelletOtakara.cpp`), `ItemTreasure` (`itemTreasure.cpp`), `onyonMgr.cpp`
(delivery), `TItemZukan` (`zukan2D.cpp`, names), `SoundID.h`, `ParticleID.h` and `efx/TOtakara.h`.

## Extraction

`libsms::extract::p2_treasure::extract(iso, cache, log)` (marker `p2treasure.ok`, version 2, independent of
Louie's `p2.ok`). `cargo run -p libsms --example p2_treasure_extract -- <Pikmin 2 iso> assets/p2`. Region is
taken from the disc id (`user/Abe/Pellet/{us,pal,jpn}/`); 813 files, 6.4 MB for the USA disc.

| Cache path (under `treasure/`) | From the disc | What |
|---|---|---|
| `config/otakara_config.txt` | `user/Abe/Pellet/<r>/` (same text as inside `pelletlist_<r>.szs`) | 188 treasures |
| `config/item_config.txt` | same | 13 upgrades (Brute Knuckles ... The Key) |
| `config/carcass_config.txt` | same | 51 enemy corpses; text only, no models extracted |
| `models/<archive>/<bmd>`, `carry.bck` | the config's `archive` (`<name>.szs`) | 201 folders (one per config) |
| `texts/{otakara,item}/<name>/animmgr.txt`, `collinfo.txt` | `otakara_texts.szs`, `item_texts.szs` | carry loop frames, collision spheres |
| `message/pikmin2.bmg` | `message/mesRes_eng.szs` | names and descriptions (English) |
| `effects/ebisawa_game.jpc` | `user/Ebisawa/effect/game.jpc` | JPA2 container (`JPAC2-10`) |
| `streams/ff_treasure{get,glad,comp}.ast` | `AudioRes/Stream/` | the treasure jingles |

Sound effects need no extra files: they are in `AudioRes/` from `p2::extract_audio` (Louie's extraction).

Oddities: `dia_c_red`, `juji_key` and `loozy` have no `collinfo.txt` on the disc (pellets never read it, so
it is optional); an archive folder name can differ in case from its texts folder (`Xmas_item` / `xmas_item`).

## The table (`libsms_assets::treasure`)

`Table::load(cache/treasure)` gives `treasures: Vec<Config>` (188) and `upgrades` (13). A `Config` is one
`PelletConfig::TParms` block. The fields a host cares about:

| Field | Meaning (cite) |
|---|---|
| `min` / `max` | Pikmin needed to lift it / carry slots (`Pellet::getPelletConfigMin/Max`). `min == 128` is the Doomsday Apparatus, which the game forces to 1000 carriers (`Pellet::onInit`) |
| `money` | Pokos paid on delivery (`onyonMgr.cpp`): 10 to 3000, 25 385 for all 188 |
| `radius`, `p_radius`, `height` | footprint, carriers' ring, collision height (`getBottomRadius`, `getStickRadius`) |
| `archive`, `bmd`, `anim_mgr`, `coll_tree` | files; `model_dir()` is the cache folder |
| `indirect` | `Yes` (8) / `Use` (4): refracting gems; the bake shows their plain texture |
| `dictionary` | 1-based collection number (`TItemZukan`) |
| `kind` | `Treasure` or `Upgrade` |

Names and descriptions: `Table::display_name(c)` / `description(c)`. Message group = `101 + offset` for the name,
`401 + offset` for what it is, `701 + offset` for Louie's note, where `offset` is the config's index (upgrades
after the 188 treasures) (`getOffsetFromDictionaryNo`; the `TOffsetMsgSet` base ids in `zukan2D.cpp`). Checked:
`yoyo_yellow` is "Doomsday Apparatus", `fue_a` "Brute Knuckles", `ahiru` "Rubber Ugly".

Spread of the 188 treasures (for value tiers): 40 are worth 50 Pokos or less, 92 are 51-100, 42 are 101-200, 14 are
over 200 (five are 1000: Monster Pump, Flare Cannon, Comedy Bomb, Shock Therapist, Lustrous Element; the Doomsday
Apparatus is 3000, Silencer 670). Carrier counts: 2 to 50 for almost all (31 need `5-10`, 30 need `15-25`, 24 need
`20-30`), 1 for the small ones (be_dama gems, feathers), 101 for the two Spherical Atlas halves (`map01/02`, upgrades).
`cargo run --release -p libsms-present --example treasures` prints the whole table as CSV (name, title, min, max,
Pokos, radius, height, baked size, loop frames).

Also parsed: `AnimMgr` (`carry.bck`, loop start/end frames, e.g. 8 to 43) and `CollSphere` trees (`collinfo.txt`).

## Models (`libsms_present::treasure::Treasure`)

`Treasure::load(cache/treasure, &config)` bakes the `.bmd` with `view::build` (TEV per texel to one RGBA
texture, like Louie), loads the rig and `carry.bck`, and offers `bounds()`, `size()`, `pose(frame)` and
`carry_frame(t)` (loops between the `animmgr.txt` frames at 30 fps). The test
`every_treasure_bakes_and_is_measured` bakes all 201 (188 + 13): every one has batches, a baked texture, finite
bounds between 1 and 2000 units and all 201 have a carry motion that poses finitely. Largest: `bell_blue`
(176 tall, 191 wide); most are 20 to 100 units, the units of the Pikmin games (the config `radius` is roughly half the baked footprint). 176 of the 201 rigs are a single joint (the wobble is that joint's scale/rotation); 25 have 2 to 15.

Not done: `Indirect::Yes` refraction (the framebuffer-sampling material), the drop shadow, and the carcass
models. A host must pick its own `Textures` per treasure (each `Treasure` owns one) or merge them.

## Sounds

`libsms-audio/tests/pikmin2_treasure_sounds.rs` (skips without the Pikmin 2 sound files) plays each effect
through `SoundSystem::play_se` and requires signal; all are audible:

| Moment | Name (SoundID.h) | Id |
|---|---|---|
| ship/Onion sucks the pellet in, then done | `PSSE_EV_HOME_PELLET_BACUUM` / `_FINISH` | 0x3804 / 0x3805 |
| treasure rises / hops out of its hiding place | `PSSE_EV_TREASURE_RISE_UP` / `_JUMP_OUT` | 0x3832 / 0x3833 |
| lands on soil (small, medium, large) | `PSSE_EV_ITEM_LAND_SOIL_S/M/L` | 0x3808-0x380A |
| lands in water (S/M/L) | `PSSE_EV_ITEM_LAND_WATER1_S/M/L` | 0x380B-0x380D |
| bounces off the Onion | `PSSE_EV_ONYON_BOUND_PLAYER` | 0x3806 |
| Pikmin try / succeed / move (voice; leaf-bud-flower) | `PSSE_PK_VC_LIFT_TRY/SUCCESS/MOVE`, `PSSE_PK_HAPPA_LIFT_*` | 0x282C-E, 0x2870-2 |
| all treasures got | `PSSE_OTAKARA_COMPLETE` | 0x1845 |
| shine demo, collection screen | `PSSE_DEMOG04_TREASURE_FX`, `PSSE_SY_OTAKARA_ZOOMIN/OUT/SCROLL/NAME` | 0x3870, 0x1082-5, 0x1885 |

Two caveats: `PSSE_SY_OTAKARA_SCROLL` is faint (peak 0.05, a menu tick); `PSSE_MP_INSECT_OTAKARA_01` (0x4002) is an
ambient bed that is nearly silent on its own and was left out. The treasure jingles are `.ast` streams, not
SEs: `PSSTR_TREASURE_GET` (`ff_treasureget`, 30 s, loops from sample 395200), `PSSTR_TREASURE_GLAD`
(`ff_treasureglad`, 25 s, loop at 289856) and the collection jingle `ff_treasurecomp` (6.6 s, no loop). A new
`Stream::decode_ast` (`libsms-assets/src/sound/stream.rs`) reads the AST container (4-bit ADPCM, 32 kHz stereo); the test
`treasure_jingles_decode` checks length and signal. They have not been played through speakers. The
`PSM::PelletOtakara` event layer (`otakaraEventStart` etc., the music that swells while carrying) is a BGM
sequence system and is not ported.

## Effects (JPA2)

Resource ids, all present in `ebisawa_game.jpc` (`Jpc::look` resolves each base shape and first texture;
`libsms_assets::treasure::EFFECTS` lists them with the `efx::` struct that plays them):

| Effect | Ids | When |
|---|---|---|
| `OtakaraApL` / `OtakaraApS` | 0x127-0x129 / 0x12A-0x12B | a large / small treasure appears (dust) |
| `OtakaraDive` | 0x279-0x27B | treasure lands in water (splash) |
| `PodGepu`, `PodKira`, `PodSuck` | 0x17E-0x17F, 0x180, 0x185-0x186 | Onion delivery |
| `UfoPodGeku`, `UfoPodOpenSuck`, `UfoPodSuck` | 0x1C4-0x1C5, 0x1C8, 0x1C9 | the ship receiving a treasure (the ship rainbow rings) |
| `PelKira` | 0x142-0x145 | pellet sparkles (unused by any `efx` struct) |

Needed to draw them: `libsms-assets/src/jpa2.rs` only reads base shape (`BSP1`) and textures. The emitter
(`BEM1`), shape extras (`ESP1`), `KFA1` keyframes and `FLD1` fields, and the particle simulation, are not read; the
blend mode, scale and colour animation come from those. A stand-in using the base shape alone (a billboard
with the first texture, scaled by `size`, tinted by `prm`) is possible today.

## Verified and not verified

Verified by headless tests (`cargo test --release`, caches present): extraction, the table, names, all 201 bakes and
measurements, every sound effect audible, the jingles decode, every effect id resolves. Each skips
cleanly without the caches.

Not verified: nothing has been drawn in a window, and nothing has been seen or heard in RoR2 or played by ear. The
bake's colours were not compared with the game's screenshots, `Indirect::Yes` models look flatter than in game, and the
AST decoder and the faint `OTAKARA_SCROLL` sound have only been judged by their numbers.

## Ship Repair integration (shared repair and the race)

Wired 2026-10-28 (models, sounds and sparkles the same day), built and staged, **never played**: nothing here has run in RoR2, alone or with two players. Treasures are
bonus objectives in both modes (the race is the plugin's default, `CompetitiveMode = true`).

- **Core** (`libpikmin/crates/libpikmin/src/carry.rs`): a treasure is a ship-part pellet whose `ship_part` is
  `TREASURE_PART_BASE (1000) + its index`. The Dolphin's Pikmin carry it to the receiver exactly like a part
  (including the host receipt, `Event::ShipPartCollected`); a ship never keeps it among its fitted parts. In the race a
  treasure goes only to its carrier's own Dolphin (never a rival's nearer one, never an Onion). Tests:
  `a_treasure_is_received_by_the_ship_without_fitting_it`, `competitive_treasures_go_only_to_the_carriers_own_ship`.
- **FFI** (`libpikmin-ffi`, no struct change, ABI stays 51): `pik_sim_treasure_count/weight/value/name`,
  `pik_sim_spawn_treasure`, and `pik_p2_treasures_complete` / `pik_p2_extract_treasures` (first-run extraction from the
  user's Pikmin 2 disc beside Louie's files). The sim reads `<assets>/p2/treasure` at creation; no cache means zero
  treasures and an unchanged Ship Repair.
- **Rule** (`ShipRepairTreasures.cs`, pure and tested in `tests/ShipRepairCheck`): per stage `TreasuresPerStage`
  (default 5, 0-16; the race adds 2 per extra racer, shared pool) distinct treasures, drawn with the run seed and the
  stage number, never one already collected this run, and subject to an optional `TreasureMaxCarriers` (default 0 = no
  limit, as the mod has no carrier limit; the table goes up to 101, and the Doomsday Apparatus asks 128, which the FFI
  passes as 128 carriers where the game forced 1000). The tier comes first (Pokos under 60 common 55%, under 130 fine
  28%, under 300 rare 14%, above that legendary 3%), then a treasure of the tier. Sites use the parts' rules: ground
  the Dolphin's Pikmin can carry from, 15-170 units from the receiver at first, 14 units clear of parts and of each
  other, off the arrival point.
- **Score**: a separate category. A treasure adds `TreasureScoreWeight` (default 0.1, capped at 0.2) of a Dolphin part
  times 1, 1.5, 2.5 or 4 by tier: 0.10 / 0.15 / 0.25 / 0.40 at the default, never 1.0 whatever the setting (tested).
  The final score is parts + Pikmin/50 + whole parts' worth of treasure points (rounded down), so many small
  treasures can add a point but no single one is a part. In the race the tally is per racer (`ShipRepairRacer`), counts in
  `Score` and so can decide the winner between close racers; treasures delivered after someone has all 30 parts do not
  count. Weight 0 turns the category off.
- **UI and text** (`ShipRepairSession.Treasure.cs`, `ShipRepairUi.cs`): a "Treasure: NAME - needs N Pikmin" toast on
  discovery, a floating name and carrier count until Pikmin lift it, a "+x.xx treasure points" toast on delivery (with
  the racer's name in the race), the HUD tally "TREASURES n +x.xx" under the parts counter (the local racer's own in the
  race), a TREASURE column of the results table (the other columns shift left when treasures are on, and names are
  cut to 13 letters then), the count in the results detail line, and the saved report.
- **Multiplayer**: the state message (`ShipRepairPacket` protocol 6) carries the crew's tally, each racer's tally and the
  stage's treasures (index and pellet), so clients show the same labels and scores; the packet checks reject a point total
  above 99 per treasure and a duplicate treasure. A treasure that falls through the map goes back to its last safe ground.

## Looking and sounding like Pikmin 2 treasures (libpikmin-present)

`libpikmin-present/src/treasure.rs`, `View::load_treasures`, `pik_view_load_treasures` (no struct change, ABI stays 51).

- **Models:** `load_treasures(<cache>/p2)` appends 188 models to the view's table, in `otakara_config.txt` order (13 MB of bakes,
  127 000 vertices, about half a second). A treasure pellet (ship part id 1000+index) is drawn with its own model at the model's
  own scale (Pikmin 2 units are Pikmin 1's), its lowest point on the pellet's ground, turned with the pellet, shrinking as the
  ship sucks it in, and on its `carry.bck` wobble (looped between the `animmgr.txt` frames) while carried or sucked in. It uses
  the Hocotate Ship's CPU-skinned `push_body` path. Without the cache it stays a plain pellet.
- **Sounds** (Pikmin 2 output, the one Louie's voice uses; started on the first cue): the moments are found by
  `treasure::moments` from the pellet's state between frames: it appears (`PSSE_EV_TREASURE_JUMP_OUT`, 0x3833), is lifted
  (`PSSE_PK_VC_LIFT_SUCCESS` 0x282D, then `_MOVE` 0x282E, repeated every 2.5 s while carried), is dropped (landing on soil,
  0x3808 + size by footprint 10 and 30; the water landings 0x380B-D are not triggered, the view does not know the water), is
  sucked in (`PSSE_EV_HOME_PELLET_BACUUM` 0x3804) and is taken (`_FINISH` 0x3805). These ids are the ones verified audible
  in `pikmin2_treasure_sounds.rs`. The jingles (`PSSTR_TREASURE_GET` and friends) are not played.
- **Effects** (stand-ins, no JPA2 emitter): like Louie's navi light, a glow from the resource's own texture and colours,
  additive. A treasure on the map gets three twinkles circling it (`PID_PelKira_3`, 0x144); a treasure appearing gets an
  expanding dust glow (`PID_OtakaraApL_1`, 0x127); one taken in gets a pale flash (`PID_UfoPodGeku_2`, 0x1C5); each burst lasts
  0.8 s. The real effects are particle emitters with many particles, keys and fields, so these are only a hint of them.
- **Host** (`ShipRepairSession.LoadTreasuresInto`, called by `PikminModels.Create` when Ship Repair is on): loads the models
  into a new view before its meshes are built, when the treasure cache is complete.

Tests: `treasure::tests` (the moments in order, the landing sizes, all 188 into the table with consistent model and texture
offsets, the three effects' quads), `tests/treasure_view.rs` (a treasure pellet in a real view: drawn with model
`first + 5`, resting on its ground, one appearing cue, quiet while still, sparkle quads), and the FFI test
`treasure_models_join_the_view_table_through_the_abi`.

## Verified and not verified (Ship Repair)

Verified headless: the core delivery (both modes), the FFI spawn and extraction exports, the pick rule, the worth bounds,
the race tally and score, and the packet round trips and rejections (`tests/ShipRepairCheck`'s treasure check; run on
its own because the file's earlier roster assertions currently fail on the Pikmin 2 enemy kinds another session added:
`TekiRoster` now has 30 kinds), and `build.ps1` builds and stages.

Never done or seen: any run in RoR2; two players or the race with treasures; how the results columns, labels and HUD text
sit on screen (the layout is arithmetic, not looked at); how the treasure models look in the game's lighting (they are baked
the way Louie's are, at full diffuse light) and whether their size reads right beside Pikmin 1's Pikmin; how the sparkles and
bursts look (they are stand-ins from base shapes alone); whether the sounds are mixed at a good level and place next to
Pikmin 1's own pellet sounds (Pikmin 1's own lift sounds very likely play as well; not checked); whether the sites are carryable on real
stages; the first-run extraction inside the plugin (the same function was run from the command line); the host building 188 more
meshes at view creation (the load time and memory were not measured in the game).

Not done: the water landing sounds, the treasure jingles, the real JPA2 emitters (no engine), refracting `Indirect::Yes`
materials, the drop shadow, Pokos as a currency, and tuning for the very heavy treasures (101 and 128 carriers can now lie in a
stage).
