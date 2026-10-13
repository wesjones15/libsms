# Smoke presentation

`libsms-present::View` gives dust and smoke a host presentation override.
The effects in `SMOKE_EFFECTS` keep each particle's birth anchor instead of following Mario's later pose.
An active emitter still follows him to place new puffs, but old puffs remain in the world when he jumps or the emitter releases.
Their own JPA motion is preserved; this does not flatten the dust onto a plane.

The host queries `sms_view_fx_texture_world_space` once per texture and omits Mario's render-smoothing offset for these batches.
Applying that offset to every particle would tug old smoke upward or sideways with his current movement.
No C ABI structs change.

JPA integration still uses the disc's frame-based values at 30 Hz by default.
Billboard/directional motion, scale, alpha, colours and spin interpolate between particle frames at the host's display rate without advancing the simulation during rendering.
The host's sim time and render lag determine the interpolation fraction, so pausing freezes the effects.
Stripes retain their existing drawing path.

`sms_view_set_smoke_speed` is optional host tuning, independent of gameplay and other effects.
It advances smoke's motion, animation and lifetime together at a clamped 0.25-4 times the disc rate.
The RoR2 plugin sets this from `Look.SmokeSpeed`, default 2, to make the smoke move and clear twice as fast.
Set it to 1 for the original timing.

Regression tests check old smoke against a jumping emitter, placement of new smoke, interpolation between frames without simulation advancement, and faster motion with shorter lifetime.
