# Enemy water-hit flash and sound

The yellow burst in the supplied Sunshine frame is JPA **particle emitter 0xE7**,
`ms_enm_wathit.jpa`, not an effect model. `System/MarDirectorLoadResource.cpp:206`
loads that exact file at 0xE7; `System/Particles.hpp:198` names it
`PARTICLE_MS_ENM_WATHIT`. `Enemy/chuuhana.cpp::TChuuHana::receiveMessage`
(water-message branch, lines 556-567) emits it at the water actor's contact position
and plays **0x6802**, `MSD_SE_EN_COMMON_W_HIT_OK`, at the enemy.
`Enemy/smallEnemy.cpp::TSmallEnemy::receiveMessage:542-556` uses the same pair.
This is the enemy-hit marker, separate from `TModelWaterManager`'s ordinary splash
and `MSD_SE_WT_GND` sound. The screenshot enemy is ChuuHana; it does not establish
the Sambo flower tower's own reaction effects or animation audio.

## Viewer proof

Run from `libsms/` with the user's extracted Sunshine cache:

```powershell
cargo run --release -p viewer --example water_hit -- assets output/water-hit-0xe7.png
```

The GPU capture uses the live viewer's shader/uploader and shared JPA simulation,
with unchanged asset data, scale and colour. Its six labelled panels show particle
frames 0, 1, 2, 4, 7 and 12, including the final empty frame. It includes the cue,
file name and source citation. This capture was produced and inspected before
changing host integration. No second candidate asset was needed.

## Integration

`libsms-present::water_hit` emits an independent, world-space marker for each
accepted `WaterEvent::Hit`. `View::water_events`, the Rust viewer and the C ABI
all use it. RoR2 enemy contacts and native flower-world contacts take this path;
terrain splashes do not. Native flower targets receiving the common marker are
a host adaptation. Existing damage and basic splash processing remain separate.
The texture is marked world-space so render smoothing does not move the flash
with Mario. `Look.Effects` controls its visibility. The extraction completeness
check requires the exact JPA file so partial older caches refresh.

The sound uses the disc's JAI cue, independently of the splash sound. The shared
gate preserves the minimum ten 30 Hz frames between starts in `MSSetSound::init`,
`MSound/MSoundStruct.cpp` set 6. This is not a full MSSetSound reimplementation:
spatial aggregation, random timing shift, and continuous volume/pitch modulation
are not reproduced. The contact position is used for audio as well as the flash;
Sunshine supplies the enemy position for audio.

`SmsEventKind::WaterImpact` reports presentation-only contacts, including native
flower targets; damage remains WaterHit/BurstHit/SpinHit. RoR2 records the contact
after its Step operation. Remote puppets replay it through `sms_view_water_hit`,
which emits the same marker and gated sound without collision queries or damage.
The remote water simulation advances the sound gate; replaying several contacts
does not advance it again. This requires **ABI 39 / frame protocol 9** on all peers.

## Verification

Disc-backed tests check yellow colour, finite geometry at distinct contact
positions, world-space textures, terrain-hit exclusion and expiry. Device-free
JAI rendering verifies cue 0x6802 produces audible finite PCM and repeated contacts
cannot bypass the ten-frame sound gate. C ABI tests spray a real host enemy and
verify the public effect readers, Effects toggle, expiry and presentation-only
remote replay. Managed NetCheck covers contact order, roundtrip, finite positions
and truncated messages; PortraitCheck calls the replay export from C#.

The viewer proof and headless checks passed. The staged RoR2 build is **not yet
played**, including multiplayer timing, apparent scale and sound balance.
