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

The dresses keep their authored helper poses; there is no cloth simulation or
Melee dress dynamics yet. High knees and some dives clip through the skirt.
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
