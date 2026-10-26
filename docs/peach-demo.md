# Playable Peach retarget demo

Run `target/release/sms-viewer.exe assets --peach-compare --peach-melee` from libsms.
Tab switches between Melee and Sunshine Peach without resetting the Mario simulation.
WASD moves, Space jumps, F dives/grabs, Z ground pounds, Shift sprays, 1–4 select
nozzles, arrow keys orbit the camera, R respawns and Escape closes the viewer.
This is a viewer prototype, not a staged RoR2 skin.

## Exact local assets and source

- Sunshine: `airport0.szs:peach/peach_model.bmd`, 26 joints. The same archive supplies
  `peach_hair_normal.bmd`, `peach_hand1_l.bmd`, `peach_hand1_r.bmd` and hair animation.
  `sms-main/src/NPC/NpcManager.cpp::TPeachManager::createModelData` identifies the body;
  `NpcInitData.cpp::sPeach_*Data` supplies attachment joints;
  `NpcParts.cpp::TNpcParts` supplies the body-to-parts texture replacement.
- Melee: `PlPeNr.dat:PlyPeach5K_Share_joint`, 114 joints, plus `PlPe.dat:ftDataPeach`.
  `melee-master/melee-master/src/melee/ft/kinds/ftPeach/ftpeach.c` names the costume
  and `ftPe_Init_OnDeath` specifies the normal pink costume's part visibility.
  `PlCo.dat:ftLoadCommonData`'s `FighterPartsTable` entry 9 supplies the semantic
  bone mapping (`ft/ftparts.c::ftParts_GetBoneIndex`, `ft/forward.h::Fighter_Part`).
- Motion: `mario.szs:bmd/ma_mdl1.bmd`, 29 joints, with `bck/ma_*.bck`.

The existing ISO/CISO, Yaz0, RARC and HSD readers extract these exact files locally.
Peach data lives in ignored `assets/npc/peach` and `assets/melee-peach`; no Nintendo
data belongs in source control or a distributed demo package.

## Retargeting and limitations

`libsms_assets::retarget::Retarget` transfers world rotation changes relative to
each skeleton's bind pose, reconstructs local rotations and keeps target lengths.
Equal names and explicit aliases handle Sunshine Peach; the Melee version resolves
body functions through the disc's common part table. Both use the same retarget.
The Melee body and Luigi reuse one HSD renderer. Luigi's established mapping is unchanged.
Each body anchors its lowest skinned vertex to Mario's current lowest vertex.
FLUDD uses the same component and muzzle feedback as Luigi, aligned to Peach's chest.

Dresses (`libsms_assets::dress`, host adaptation, not a port of Nintendo's cloth):
the legs' gait is softened (`gait_weight`), and the garment is posed on top of the body.
Melee Peach's eight five-bone dress chains (`PlPe.dat:ftDataPeach+0x2C`) play their
native Run/Wait1 clips from `PlPeAJ.dat` and are pushed out of each leg's sphere
chain (`panels`). Sunshine's mesh only weights `jnt_skirt` plus two helpers at the waist,
so those get the authored `peach_wait.bck` sway and a front/back flare plus a depth-growing
shear fitted to the legs (`cone`). `SMS_DRESS=off` turns both off to compare.
Sunshine hair plays its authored idle; Melee hair/fingers keep bind local rotations.
The simulation, sounds, interaction reach, carried-fruit attachment and movement
effects remain Mario's. This demo evaluates movement on the two Peach models.

## Evidence

`SMS_POSE=run2,21` selects an exact source clip/frame for a capture and fails on an
unknown clip. Ordinary play samples Mario's current layered animation instead.
Captures in `../output/sms-peach-demo/` include idle, run frames 6/21/36, jump,
dive and pound for both exact assets, with source labels on each capture.
`libsms-assets/tests/retarget.rs` checks Sunshine Peach's bind preservation and
skinned vertex bounds across seven Mario clips and four frames each.
The viewer's `melee_peach` test checks finite HSD skinning across five clips.

## Pianta and Piantissimo

`sms-viewer assets --pianta` (Tab cycles; `SMS_PIANTA=0|1|2` picks the first,
`SMS_PIANTA_COLOR=<n>` picks the body colour from `sMonteM_BodyColorBuf` /
`sMonteW_ColorBuf`):

- **Piantissimo** (`mamma4.szs:map/map/pad/monteman_model.bmd`, extracted to
  `assets/npc/monteman`) has Mario's 29 joints and parent list, so Mario's joint
  matrices drive it as they are, as `TEnemyMario` does (`enemyMario.cpp`). Only two
  attachment joints are spelled differently.
- **Pianta** (`montem/mom_model.bmd`, `montew/mow_model.bmd`) is a separate 20-joint rig:
  no waist/chest split, arms, legs and head hang off `body_jnt`. `pianta::ALIASES` maps
  limb to limb and `Retarget` keeps the Pianta's own (longer) arm and leg lengths.
  The straw skirt, grass `mino_null`, palm and nose follow their parents; nothing
  animates the skirt panels yet.

Drawing them needs the runtime edits the decomp makes: the `_mino_dummyI4` texture becomes
`i_mom_mino_rgba.bti` / `i_mow_mino_rgba.bti` (`TMonte*BaseManager::changeTextureToStraw_`),
`_hand_mat`'s TEV register 0 gets the body colour (`NpcInitData.cpp` `sMonte*_BodyColor`,
`SMS_InitChangeNpcColor`), and every non-eye material's KColor0 is white with alpha 0,
the clean-NPC pollution value (`NpcBase.cpp`, `NpcInitPrg.cpp`). The hats, glasses and
cloth variants (`moma`..`momd`, `mowa`..) are not worn. The Pianta has no foot contact
model, so long strides slide a little; the lowest vertex is anchored like Peach's.
Captures: `../output/sms-peach-demo/pianta/`. Tests: `pianta::tests` (bone lengths held
across five Mario clips, Piantissimo's hierarchy equals Mario's).
