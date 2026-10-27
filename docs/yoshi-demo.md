# Sunshine Yoshi wearing Mario motion

Run `target/release/sms-viewer.exe assets --yoshi-mario` from libsms.
The standalone viewer uses Mario's simulation and `ma_*.bck` motion on Yoshi's
own 38-joint skeleton and geometry. WASD moves, Space jumps, F dives/grabs,
Z ground pounds, arrows orbit the camera, R respawns and Escape quits.
This runaround has no FLUDD and is not a RoR2 deployment.

Exact asset: `yoshi.szs:yoshi_model.bmd`, identified by
`sms-main/src/Player/Yoshi.cpp::TYoshi::init`; its normal hand shapes, materials,
green `BODY_COLORS[0]` and `yoshi_wink_tx.btp` use the existing Yoshi renderer.
Source motion: `mario.szs:bmd/ma_mdl1.bmd` (29 joints) and `bck/ma_*.bck`.

The shared `libsms_assets::retarget::Retarget` transfers rotation changes while
preserving Yoshi's bind translations/scales. Matching limb names map directly;
`chn_neck` maps to Mario's `chn_chest`, `jnt_neck_1/2` map to `jnt_chest`, and
`chn_head` maps to `jnt_head`. The tail and mouth retain their authored local
transforms, inheriting the posed parents. These are host retargeting choices.
The body anchors its lowest skinned point to Mario's current lowest point.
No Yoshi riding/body animation replaces Mario's motion in this mode.

Viewer-first evidence: labelled idle, run frames 6/21/36, jump and dive captures
in `../output/sms-yoshi-demo/`. The `yoshi_skin` test samples seven Mario clips
at four frames, checking bind-pose identity, every target bone length and finite
skinned vertices. Original Yoshi riding/egg rendering still uses its own pose path.

The model and clips remain in the ignored local disc cache. Never commit or
distribute the extracted data. A local launcher is in `../output/sms-yoshi-demo/`.
