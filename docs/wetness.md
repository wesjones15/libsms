# Mario's wet runoff and burn hop

The wet VFX is **model-water type 1 (`WTP01_Wide`)** emitted with
`/Mario/WetWaterEmit.prm`. It has no JPA cue, particle-emitter ID, or effect-model ID.
`MarioInit.cpp` allocates that emit-info object; `MarioMove.cpp::thinkWaterSurface`
sets `mWetWaterParticleTimer = 120` on pool exit. `checkWet` decrements it and emits
whenever the remaining timer is divisible by eight, at Mario's feet plus 5 units,
with inherited velocity `0.3 * mVel`. It skips while riding Yoshi, over a water
surface, submerged, or in a status carrying `MARIO_STATUS_FLAG_UNK200`.
The timer lasts one second at 120 Hz, with 15 emission requests.

The owned disc's emit parameters give three drops per request, lifetime 128 water
frames, upward direction, power 5 with tremble 1, size 15 with tremble 10, direction
tremble 0.5, attack 1, and particle type 1. The normal water manager and renderer
provide the trajectory, coverage and refraction; no replacement smoke/sparkle
effect is used. Spray reach/speed tuning does not affect these drips, and the
FLUDD muzzle does not clip their tails.

Viewer proof (run from `libsms/`):

```powershell
cargo run --release -p viewer --example fire_panic -- wet.png 0.5 1.2 --wet
cargo run --release -p viewer --example fire_panic -- hop.png 0.35 0.7 --fixed-camera
```

Captures have source/identity sidecars. The wet example starts in stage water and
leaves it, then consumes the sim's `BodyDrip` requests through `WaterManager` and
`WaterLook`. The native game adapter consumes the same requests. They do not update
FLUDD's last-spray record, drain the tank, or provide fresh spray contacts that
could re-wet their emitter indefinitely.

The burn hop is `MarioMove.cpp::setStatusToJumping(FIRE_DOWN, arg 1)`:
`mFireDownForce` from `Jump.prm` (50 on the owned disc), without a normal jump-height
multiplier. `MarioJump.cpp::fireDowning` retains `ma_firejmp` (`ANIM_FIREJMP` 0x29,
file index 0x0B), rebounds twice at 40% of the downward speed, then enters
`FIRE_JUMP_END`. `MarioWait.cpp::fireJumpEnd` uses `ma_fjpend` (`ANIM_FJPEND` 0x28,
file index 0x0A). The native host enables `host_fire_reaction`, not `fire_panic_run`.

Host adaptations retained by request: alternating Fire 02/01 once per ignition,
five seconds of escape grace, extinguishing on actual body water contact, and
applying the vanilla runoff timer to FLUDD contact too. The decomp's runoff timer
itself does not implement burn immunity. RoR2 damage continues normally.

While Mario is wet, that same runoff timer also qualifies him for a wet slide on
dry ground. It uses the existing slide rules: wetness renews the slide's duration,
then its configured timer runs down once both Mario and the ground are dry.
This is a host adaptation; it does not mark the underlying ground wet.

Checks cover the owned-disc launch force, hop apex and rebound count, absence of
`FireDash`, runoff cadence/expiry, disc emit parameters, exclusion of Spray tuning
and runoff feedback, and droplets reaching the native water renderer. In-game
verification remains manual.
