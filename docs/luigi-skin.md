# Luigi alt-skin (libsms viewer)

Branch: `luigi` (from `modern/1.5.0`). Merge when the viewer shows Luigi with a FLUDD
that he owns.

## Goal

Luigi is an unlockable Sunshine skin that has **all of Mario's moves** (the sim is Mario's)
and a FLUDD that behaves identically except that the **Spray** neck leans to the side so it
clears his head. No height change: Luigi keeps his authored height, `model_scaling` included.

## Facts already established (do not re-derive)

- HSD model load: `libssbm_assets::model::Model::load_for(dir, costume, &fighter::LUIGI)`.
  `Model::load` defaults to **Samus** — never use it for Luigi.
- Unit conversion: Melee model units x10 -> Sunshine cm (MetresPerUnit 0.1). Applied in the
  viewer draw, not by changing Luigi.
- Clip: `sms.mario.anim.clip` (`wait`, `run1`, `jump`, ...) maps to `bck/ma_<clip>.bck`;
  pose Luigi with `libssbm_assets::sms::Retarget` from the 29-joint Mario rig to his 61.
- Retarget mapping is the one in `libssbm-assets/src/sms.rs` (`MAP`); it is correct and
  must not be changed.
- FLUDD lean (Spray only): joints **4, 5, 6, 7** (`null_pump_wA`, `null_pump_wB`,
  `nozzle_center`, `jnt_G_handle`), pivot `(37, -40, 0)`, rotation **+45 deg about Y**
  (the model's side axis). Tank (`jnt_G_main`, shape 3) must not move.
- The thin frame rod is **shape 3, `_mat_G_main`**; it is hidden while the lean is active.
  `_mat_waterline` is not the rod.
- Lean and rod-hide are gated on FLUDD being deployed: `m.fludd.housed < 0.5` (idle returns
  to the authored pose).
- Captures live under `libsms/target/`: `luigi_posed.png` (idle), `luigi_moves.png`
  (run/jump/dive/pound/spray), `spray_compare2.png` (lean A/B), `beside2.png` (both FLUDDs).

## FLUDD

`viewer/src/fludd.rs` is FLUDD as a component of its own: the pack
(`watergun2/body/wg_mdl1.bmd`), all five nozzles, every `watergun2` clip, the Spray neck lean,
the frame-rod hide and the nozzle aiming callbacks. Mario's model owns one (`MarioModel::fludd`)
and so does Luigi's character: a host passes the chest frame the pack straps to, the sim's
`FluddLook` and `housed < 0.5`, and gets the posed muzzles back for `set_emitters`.

- Mario: `chest = joints[jnt_chest]`.
- Luigi: `Luigi::chest(world) = world[WaistN] * (lg_bind[5]^T * sms_bind[14])`, translation
  x10 to Sunshine's decomp units (the retarget maps `jnt_chest` onto `WaistN`).
- `--luigi` draws only Luigi plus his FLUDD; `--luigi-beside` draws Mario, Luigi and Luigi's
  FLUDD as independent packs. `SMS_HIDE` and the second-`MarioModel` trick are gone from both.

Tunables live at the top of `fludd.rs`: `LEAN_DEG` (45), `LEAN_PIVOT` (37, -40, 0),
`LEAN_JOINTS` (4, 5, 6, 7), `ROD_SHAPE` (3) and `LEAN_RATE` (1800 deg/s, so the two-frame
swing from one side to the other). `Fludd::draw` leans only when the caller says the wearer is
an alt-skin (`alt_skin`), so the game's Mario keeps his authored Spray neck; `SMS_LEAN=<deg>`
still forces an angle for A/B captures.

## Aiming and which side the neck leans

The host aim (`Mario::aim`, `Aim { yaw, pitch }`) is what the RoR2 adapter reports from the
crosshair. The viewer now sets it too: hold the right mouse button and the aim follows the
camera; `SMS_AIM=<deg>` holds an aim that far to Luigi's right (+) or left (-) of his facing
for captures. A cyan line from his chest draws the aim, a white one his facing.

`FluddLook::aim_turn` is the aim relative to the model's facing. The lean follows it: aiming
past his right shoulder leans the neck right, past the left leans it left, with a 512-unit
(~6 deg) deadband so aiming straight ahead cannot flicker. Past 120 deg off the facing the
lean fades out and is gone at 180: aiming behind, the swivelled head needs no clearance and
the neck is authored again (which also removes the sign flip a dead-behind aim would have).
The neck eases at `LEAN_RATE` instead of snapping, so the swing reads as a fast smear; the
water keeps flowing through it. An aimed nozzle also swivels its head to the aim
(`chn_muzzle_1`), matching `libsms-present`'s view, so the drawn spray points where the line
does. Aim pitch (above/below) does not touch the lean; it only feeds the sim's `Aim.pitch`.
In a spin `aim_now()` is None, so `aimed` is false and the neck keeps the side it last chose
while the body turns.

## Anchoring (why he used to float)

`fsms::Clip` (`libssbm-assets/src/sms.rs`) reads ANK1 rotations only; the clips also carry
joint translation tracks in the same chunk (Mario's root/`center` drops for the dive and the
hang), so a retarget that only takes rotations leaves Luigi standing at the sim's position —
floating at hip height in a dive, a body above a ledge in a hang, and a few centimetres off
the floor in idle. The viewer now anchors each frame: it takes the lowest skinned point of the
drawn Mario (`MarioModel::lowest`, model space) and the lowest point of Luigi's pose
(`Luigi::lowest`), and shifts every Luigi joint by the difference, so feet follow feet and a
prone body follows a prone body.

## Voice

Luigi's voice is Mario's, pitched up 14% with the line's length kept (the Melee trick, done
properly). `SoundData::set_voice_pitch` (the voice cues are SE category 7, Mario's `_MV`
lines) resamples the decoded wave by the ratio and then WSOLA-stretches it back to its own
sample count, so the line is not faster; it is baked once per wave and cached. The viewer sets
it for `--luigi` (1.14, `SMS_VOICE_PITCH` overrides). Measured on the jump cue: F0
286 -> 325 Hz, duration 1.20 s unchanged. `--example voice_pitch` renders an A/B.

## Hanging

Luigi hangs a little lower and further off the wall than the feet anchor puts him
(`SMS_HANG_OFFSET=<down>,<back>` in decomp cm, default 8,6), applied to the whole pose while
`Status::Hanging` so FLUDD follows.

## Water

The sim takes the drawn muzzles through `Mario::set_emitters`, but the viewer's particles
(`libsms_present::View`) posed its own Mario and overwrote them, so the spray left the
authored, un-leaned nozzle. `View::set_muzzles` (and `fx::FxLayer::set_muzzles`) now takes the
matrices the host drew; the viewer passes Luigi's, so the mist and the water leave the leaned
nozzle.

