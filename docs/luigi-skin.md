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

## What is still a workaround

`--luigi` hides Mario's body/cap/hands (`SMS_HIDE`) and reuses his FLUDD; `--luigi-beside`
instantiates a second `MarioModel` as a FLUDD renderer. Both should go.

## Next change (additive, no extraction)

New `viewer/src/fludd.rs`:

- `pub struct Fludd { body: Part, nozzle: Part, clips }`, loaded from
  `dir/watergun2/body/wg_mdl1.bmd`, `dir/watergun2/normal_wg/normal_wg.bmd`, and the
  `wg_house` / `wg_pump` / `normal_wg_change_end` BCKs.
- `pub fn draw(&mut self, look: &FluddLook, chest: Mat4, root: Mat4, view, proj)`:
  pose the body, apply the +45 Spray lean to joints 4/5/6/7 about `(37,-40,0)`, hide shape 3
  while leaning, pose and draw the nozzle on `nozzle_center`, all in the chest frame.
- Mario keeps `MarioModel` as-is for now; Luigi uses `Fludd` directly with
  `chest = retarget_world[5] * align * 10`.
- `--luigi` then draws only Luigi + his `Fludd`; `--luigi-beside` draws Mario + Luigi + his
  `Fludd`. `SMS_HIDE` and the second-`MarioModel` trick are deleted from these paths.

Later, optionally, `MarioModel` can use the same `Fludd` (remove the duplicated code).
