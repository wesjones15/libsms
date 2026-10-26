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
- `sms_view_set_voice_pitch(view, ratio)`: a skin's voice (Piantissimo and Mario: 1).

## What a skin gets for free vs what Luigi needed

| | Luigi | Piantissimo |
|---|---|---|
| mesh | HSD mesh, CPU-skinned, x10 | a J3D model, same path as Mario's body |
| pose | retarget + feet anchor + hang offset | none: Mario's joints |
| FLUDD | own pack, Spray neck leans past the head | Mario's, unchanged |
| particles | red effects turned green | Mario's |
| voice | Mario's, +14% | Mario's |
| unlock | 2401 coins held | none (available from the start) |
| secret move | Egg Lay (dpad up) | none |

Per-skin FLUDD, tint, voice and unlock are `Skin` / `SkinSpec` data, not branches in the
host: Luigi's lean runs only when the current skin is Luigi, even if his mesh is preloaded
(the statue entrance preloads every skin).

## Not done / not verified

- Not played in game; only the headless view test, the ABI check through the real DLL
  (`PortraitCheck`), and baked-texture captures were run. Piantissimo's FLUDD sits on Mario's
  chest; whether Spray's neck clears his tall cap in play is unchecked.
- Blink: Mario's wink pattern is not applied to Piantissimo's face.
- Multiplayer puppets still draw Mario's body for other players (as Luigi does).
