# Mario skins: one mechanism, one row each

A skin changes Mario's **body**; the sim, moves, FLUDD, effects and camera stay Mario's. Luigi
was the first skin (`luigi-skin.md`); Piantissimo the second, and the shared parts were pulled
out when he arrived. A new skin should be a row, not a new code path.

## Kinds of body (`libsms_present::view::SkinBody`)

| Kind | Body | Posing | Examples |
|---|---|---|---|
| `Mario` | `ma_mdl1.bmd` + cap + hands | the clip's joints | Mario |
| `SameRig(file)` | a J3D model with **Mario's exact joint parents** | Mario's joint matrices as they are, no retarget, no anchor; the model carries its own cap and hands | Piantissimo (`npc/monteman/monteman_model.bmd`); Shadow Mario (`enemy/kagemario/default.bmd`) is the same shape, unverified |
| `Hsd` | an HSD model from the Melee cache | retarget from Mario's 29 joints, feet anchor, hang offset, FLUDD lean | Luigi |
| `Yoshi` | `yoshi.szs:yoshi_model.bmd`, a **different** 38-joint rig | his own wait/walk/run and flutter clips on his rig; Mario's air clips through `Retarget`; feet anchor; FLUDD on `jnt_neck_1` | Yoshi |
| (not a skin yet) | a J3D model on a **different** rig | `libsms_assets::retarget::Retarget` with an alias table; see `crates/viewer/src/{peach,pianta}.rs` | Sunshine Peach, Pianta |

How to tell: compare joint parent lists (`libsms::Skeleton::from_bmd`). Equal parents means
`SameRig`; `load_skin` refuses a model whose parents differ, so a wrong guess fails loudly.
Piantissimo's joint *names* differ in two attachment joints; only the parents matter.

## Adding a `SameRig` skin

1. **Extract**: add a row to `libsms::extract::OPTIONAL_EXTRACTS` (archive, path inside it,
   destination under `assets/`) and bump the array length. Older caches pick it up through
   `sms_extract_extras` on the next run; nothing is required of the host.
2. **Native**: add a variant to `Skin` (append: the id is the host ABI), list it in
   `Skin::ALL`, and give it a `SkinBody::SameRig("path")` in `Skin::body`.
3. **Host**: add a `SkinSpec` row in `ror2-mario/MarioTest/MarioSkin.cs`: id, name, language
   token and text, pause-sheet title and quip, and optionally `UnlockCoins`, `VoicePitch`,
   `JumpScale`, `EggLay`. The loadout, SkinDefs, unlocks, portrait, lobby display, statue
   entrance and pause sheet all read the row.
4. **Tests**: nothing new is needed beyond a `Skin` row; `piantissimo_wears_marios_pose_on_his_own_model`
   (libsms-present) and `CheckSameRigSkin` (ror2-mario `tests/PortraitCheck`) show the pattern.

Drawing needs nothing per skin: the body is another entry in `View::models`, drawn in place of
`models::BODY`, baked like any J3D model with a texture-cache key above every model index
(`RIG_KEY_BASE + id`). Model-table readers (`sms_view_model`, `batches`, ...) see it as a
normal model.

## ABI

- `sms_view_skin(view, id)`: which body draws (unknown id: Mario).
- `sms_view_load_skin(view, id, dir)`: make a skin drawable; `dir` is the Melee cache for
  Luigi and may be null for the rest. 1 when its files were there. Load every skin a view will
  show before the host reads the model table (`ModelView` fixes it at creation).
- `sms_view_set_luigi` stays for existing callers.
- `sms_view_set_voice_pitch(view, ratio)`: pitch (Luigi 1.14; Piantissimo and Mario 1).
- `sms_view_voice_set(view, set)`: 0 Mario's voice, 1 Piantissimo's, 2 Piantissimo strict.
  `sms_view_skin` already picks 0 or 1 from the skin (`Skin::voice_set`).

## Yoshi

Yoshi is the first skin whose body plays **its own clips**. `libsms-present::yoshi_skin` loads
`yoshi.szs:yoshi_model.bmd` (38 joints, `Player/Yoshi.cpp::TYoshi::init`) and keeps his wait,
walk, run and `yoshi_hold_jump` clips on his own rig. Mario's jump, dive, flip and pound clips
go through `Retarget` (aliases `chn_neck`, `jnt_neck_1/2`, `chn_head`), and his lowest skinned
point is anchored to Mario's. FLUDD rides `jnt_neck_1` in Mario's authored centred chest frame
(`WaterGun.cpp::init`), without Luigi's side lean.

The flutter is a host option, `Options::yoshi_skin` (`sms_sim_set_yoshi_skin`): `TYoshi`'s
`thinkHoldOut` runs without Yoshi being ridden, a fresh A press starts it after two ticks in the
air, it holds A for up to `FLUTTER_TIME`, and it plays `MSD_SE_YV_FUNBARI` at the start. The
same option draws FLUDD's water in Yoshi's juice colour (`Mario::juice_color` returns 1).
Landing re-arms it; each jump also resets it. A front Spray within the mouth's clearance
envelope leaves `null_tongue` (jaw at the held `yoshi_eat` end pose, head turned to the aim,
`NozzleYoshiMouth.prm`) while FLUDD plays its house clip, and FLUDD's belt shape is widened
1.5x for his hips. Both are ported from the viewer; the water also carries his full run
(`Mario::juice_mouth`, `WaterManager::emit_carried`) instead of vanilla's 0.125 share.
`yoshi_skin_draws_his_body_and_routes_a_front_spray_to_his_mouth` covers the body and mouth
emitter.

Not ported from the viewer yet: Yoshi's own voice (the skin speaks with Mario's). Not
played in game.

## What a skin gets for free vs what Luigi needed

| | Luigi | Piantissimo |
|---|---|---|
| mesh | HSD mesh, CPU-skinned, x10 | a J3D model, same path as Mario's body |
| pose | retarget + feet anchor + hang offset | none: Mario's joints |
| FLUDD | own pack, Spray neck leans past the head | Mario's, unchanged |
| particles | red effects turned green | Mario's |
| voice | Mario's, +14% | his jump voice for Mario's five jump cues; Mario's lines otherwise (see Voice) |
| unlock | 2401 coins held | none (available from the start) |
| secret move | Egg Lay (dpad up) | none |

Per-skin FLUDD, tint, voice and unlock are `Skin` / `SkinSpec` data, not branches in the
host: Luigi's lean runs only when the current skin is Luigi, even if his mesh is preloaded
(the statue entrance preloads every skin).

## Voice

The game picks whose voice speaks per player type: `TMario::getVoiceStatus` (MarioSound.cpp)
returns 0 for Mario, 2 for Shadow Mario, **6 for Piantissimo**, and `MSound::startMarioVoice`
(MSound.cpp) applies it. For status 6 it replaces only the five jump cues (`MV21` small,
`MV22` mid, `MV23` large, `MV24` special, `MV25A` huge) with `MSD_SE_NPC_VA_MTMAN_JUMP1` and
returns without playing anything for every other voice. Voices keyed in animations
(`MAnmSound::startAnimSound`, category 7) go through the same function with the status in the
sound flags (`0x60000000`), so they follow the same rule.

libsms does it at the one place every Mario voice already passes, `SoundSystem::mario_voice`,
with `VoiceSet::{Mario, Piantissimo, PiantissimoStrict}`:

- `Piantissimo` (what the skin selects): the jump cues play his cue; every other voice falls
  back to Mario's own line, because "silent" would leave him mute for hurt, dive, death and
  spin voices.
- `PiantissimoStrict` (RoR2 config `Sound.PiantissimoGameVoice`): the game's behaviour, his
  jump voice and nothing else.

Tests: `libsms-audio/tests/voice_set.rs` (his cue is in the banks and differs from Mario's wave;
fallback vs strict). Gaps are listed below.

## Not done / not verified

- Not played in game; only the headless view test, the ABI check through the real DLL
  (`PortraitCheck`), and baked-texture captures were run. Piantissimo's FLUDD sits on Mario's
  chest; whether Spray's neck clears his tall cap in play is unchecked.
- Blink: Mario's wink pattern is not applied to Piantissimo's face.
- Voice gaps: he has one voice cue (`MSD_SE_NPC_VA_MTMAN_JUMP1`), so damage, dive, death, fall,
  spin, ledge, wake and tired lines are Mario's (silent in strict mode). `MTMAN_JUMP2` exists in
  the banks but the game never plays it, so it is unused here too. His race dialogue lines
  (`MSD_SE_NPC_VM_MTMAN_NORMAL` / `_LOST`, Talk2D2.cpp) are text-box voices with no RoR2 event.
  The jump cue does not use Mario's tired-voice or random-alternate picks, as in the game, and
  the pitch is not applied (it is 1 for him anyway). Not heard in game; checked headlessly.
- Multiplayer puppets still draw Mario's body for other players (as Luigi does).
