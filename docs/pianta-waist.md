# Pianta waist and palm

The skirt is not rigid in Sunshine. Looking only at `mino_null`'s BCK rotation
missed the matrix callback on its parent. The old assertion that it was rigid in
the game was incorrect.

`sms-main/src/NPC/NpcInitPrg.cpp::TBaseNPC::setMtxEffect_` installs effect type 0
(`TMTX_EFFECT_TIME_LAG`) on `koshi_null` for the male and female Piantas.
`MarioUtil/MtxUtil.cpp::TMultiMtxEffect::setup` loads the joint's parameters from
`/Npc/MonteM/MtxEffect1.prm` (female: `MonteW`). These are already extracted as
`params_npc/montem/mtxeffect1.prm` and `montew/mtxeffect1.prm`.
The male disc values are position acceleration 1, brake 0.1, per-axis limit 30;
quaternion acceleration 1, brake 0.05. The implementation reads the files.

`TMtxTimeLag::calc` follows world translation and quaternion components with
damped springs. It preserves column lengths and normalises only when writing
the quaternion matrix. Applying its correction to waist joint 1 and child
`mino_null` joint 2 before envelope skinning deforms the skirt; the legs, arms
and head are siblings and remain on their own animation. No invented skirt sway
is needed.

The shared implementation is `libsms-present::mtx_effect::TimeLag`, invoked by
`body::Pianta::apply_waist_lag`. Hosts give each actor its own history and world
transform in Sunshine model units. A fixed 30 Hz callback clock interpolates
targets between host frames. First use, a clock rewind, a gap over 0.25 seconds,
or a world displacement over four source position limits resets the history;
the latter checks adapt the game's reset-on-uncull flag to host discontinuities.
Without the parameter file the body still loads, with no procedural substitute.

For the palm, `NpcAnm.cpp::npcTalkIn` calls `requestTalkAnm_`, whose ordinary
talk is kind 6. `NpcInitAnmData.cpp::sMonteM_AnmIndex[6]` selects BCK 12,
`mom_talk.bck`. Its `yashi_jnt`, `leafL_jnt` and `leafR_jnt` tracks supply the
perk. `Pianta::talk_plant` grafts those authored local transforms over any body
action. Olimar's skin blends this overlay in for a live enemy lock and out on
release or target loss. Pellet locks are excluded. The blend duration comes
from `npcnormal.prm`'s `mMotionBlendFrame` (source default 20).

Headless proof, without opening a window:

```powershell
cargo run --offline --release -p viewer --example pianta_warp -- build/pianta-warp/koshi1-timelag-talk.png
```

The PNG's companion text identifies the exact assets and source functions.
Columns show walk frames 0/10/20/30; rows show the source clip, the waist
callback, and the callback plus the talk palm. The software renderer uses
the same shared posing and skinning as the host. Captures contain extracted
assets and remain local, outside Git.

Verified: numerical spring steps, 30/60/144 Hz host consistency, resets,
male/female skirt vertices, exact talk locals and unchanged limbs; Olimar
tests cover live enemy locks, pellet/stale locks, unlock and independent
captains. The existing Pianta ABI test passes across standing, walking,
running, jumping and whistling. Not yet played in RoR2.
