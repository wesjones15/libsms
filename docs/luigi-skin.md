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
`LEAN_JOINTS` (4, 5, 6, 7) and `ROD_SHAPE` (3).

