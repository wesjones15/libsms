# Sunshine Yoshi wearing Mario motion

Run `target/release/sms-viewer.exe assets --yoshi-mario` from libsms.
The standalone viewer uses Mario's simulation and `ma_*.bck` motion on Yoshi's
own 38-joint skeleton and geometry. WASD moves, Space jumps, F dives/grabs,
Z ground pounds, Shift sprays, 1–4 choose Spray/Hover/Rocket/Turbo, X switches
nozzles, G refills, arrows orbit the camera, R respawns and Escape quits.
This is a standalone Rust viewer demo, not a RoR2 deployment.

Exact asset: `yoshi.szs:yoshi_model.bmd`, identified by
`sms-main/src/Player/Yoshi.cpp::TYoshi::init`; its normal hand shapes, materials,
green `BODY_COLORS[0]` and `yoshi_wink_tx.btp` use the existing Yoshi renderer.
Source motion: `mario.szs:bmd/ma_mdl1.bmd` (29 joints) and `bck/ma_*.bck`.

The shared `libsms_assets::retarget::Retarget` transfers rotation changes while
preserving Yoshi's bind translations/scales. Matching limb names map directly;
`chn_neck` maps to Mario's `chn_chest`, `jnt_neck_1/2` map to `jnt_chest`, and
`chn_head` maps to `jnt_head`. The tail inherits its posed parent. The jaw uses
the disc's `yoshi_eat.bck` held end pose while spitting. These are host retargeting choices.
The body anchors its lowest skinned point to Mario's current lowest point.
No Yoshi riding/body animation replaces Mario's motion in this mode.

FLUDD reuses the existing viewer component and water simulation. Its exact assets
are `mario.szs:watergun2/body/wg_mdl1.bmd` and the `normal_wg`, `hover_wg`,
`rocket_wg`, `back_wg` nozzle models named by `WaterGun.cpp::nozzleBmdData/init`.
The pack attaches to `jnt_neck_1`, Yoshi's upper torso mapped to Mario's chest,
with a bind-frame rotation correction. FLUDD keeps its authored centered neck;
Mari-Yoshi does not use Luigi's side lean.
The returned posed muzzle matrices drive both water emission and particle placement.

## Mouth juice and FLUDD routing

Spray aim within 60 degrees either side of the body, and within the disc's mouth
pitch bounds, comes from Yoshi's `null_tongue` joint. The PAL/USA mouth parameters
are -9000/+10000 gun-angle units, or about +49.4/-54.9 degrees of physical head pitch.
Outside this front clearance envelope, FLUDD aims normally. Hover/Rocket/Turbo
always use FLUDD. This clearance envelope and the independent yaw are host adaptations.
The body no longer automatically rotates to a stationary crosshair in this demo,
so the head can turn independently and the back-facing FLUDD aim remains reachable.

`Yoshi.cpp::YoshiHeadCtrl/getEmitPosDir/thinkUpper` identifies head pivot 21,
mouth emitter `null_tongue` (23), and the eating/jaw animation. Mouth spray uses
`NozzleYoshiMouth.prm` particle type 12, its power/size/spread, and Sunshine's
orange juice palette. The existing nozzle mist follows the same posed mouth.
FLUDD plays `wg_house`/`normal_wg_house` during mouth mode and returns to its
normal animation when aiming outside that envelope. No Luigi side lean is used.

Juice inherits the full emitter velocity, replacing vanilla's 0.125 share
(`WaterGun.cpp::getEmitPosDirSpeed`). `WaterManager::emit_carried` exempts that
carried motion from the droplet's own speed cap, so it can overtake a running Yoshi.
Ordinary `emit` keeps its original behavior. Juice and water use separate instances
of the same simulation/renderer so existing drops keep their color across mode switches.
Mouth position is the most recently posed animation frame, placed at each tick's
current body position; steering direction and inherited velocity are updated per tick.

Only FLUDD belt shape 1 (`_mat_froater`, rigid on `G_center` in `wg_mdl1.bmd`) is
expanded 1.5 times in its ring plane for Yoshi. Tank, straps and nozzle remain unchanged.
Both Peach viewer variants use 0.8 for this same belt-only adjustment.
`yoshi_fludd_audit` reports the exact mesh, material, joints and mouth parameters.
Labelled front/side/above/behind/running captures are `hybrid-*.png` in the demo folder.

Viewer-first evidence: labelled idle, run frames 6/21/36, jump and dive captures
in `../output/sms-yoshi-demo/`. The `yoshi_skin` test samples seven Mario clips
at four frames, checking bind-pose identity, every target bone length and finite
skinned vertices. Original Yoshi riding/egg rendering still uses its own pose path.

The model and clips remain in the ignored local disc cache. Never commit or
distribute the extracted data. A local launcher is in `../output/sms-yoshi-demo/`.
