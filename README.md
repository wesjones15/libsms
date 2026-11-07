# libsms

Native runtime used by [ror2-mario](https://github.com/wesjones15/ror2-mario). This workspace provides fixed-step character simulation, asset readers, animation, presentation, audio and a C interface. The host supplies world collision and connects simulation events to its own gameplay systems.

This is a source dependency. Installation and player controls belong in the [host README](https://github.com/wesjones15/ror2-mario).

## Structure

| Crate | Responsibility |
| --- | --- |
| [`libsms`](crates/libsms/) | Simulation, input, collision contract, tuning, extraction and gameplay events. |
| [`libsms-assets`](crates/libsms-assets/) | Model, texture, material, layout and sound-data readers. |
| [`libsms-audio`](crates/libsms-audio/) | Sequencing, mixing and animation sound cues. |
| [`libsms-present`](crates/libsms-present/) | Model posing/skinning, material textures, particles, water and HUD data. |
| [`libsms-ffi`](crates/libsms-ffi/) | C interface for non-Rust hosts; builds `sms.dll` on Windows. |
| [`viewer`](crates/viewer/) | Standalone inspection and development host. |

The simulation has no Unity or host-game types. Host-specific scaling, resource limits and integration rules are supplied through the API. Implementation scope, source references and known gaps are recorded in the [behavior spec](docs/behavior-spec.md).

## Dependencies and build

Use a Rust toolchain supporting edition 2024. The maintained branch is `modern/1.5.0`. Cargo expects these sibling checkouts:

```text
workspace/
  libsms/         modern/1.5.0
  libcommon/      modern/1.5.0
  libssbm/        main
```

[`libcommon`](https://github.com/wesjones15/libcommon) supplies shared Rust utilities. Presentation and viewer code also use the asset crate from [`libssbm`](https://github.com/wesjones15/libssbm). The C# host shares its integration code through [`ror2-common`](https://github.com/wesjones15/ror2-common).

Build the native host library from this directory:

```powershell
cargo build --release -p libsms-ffi
```

On Windows the output is `target/release/sms.dll`. A host-game installation and extracted assets are not required to compile it.

## Local data

Runtime assets are read from a user-supplied disc image into an ignored local cache. Disc images, extracted assets and generated caches are not included in the repository or release packages.

The native extractor accepts ISO, GCM and CISO:

```powershell
cargo run --release -p libsms --bin sms-extract -- "C:\path\to\disc.iso"
```

The default output is `assets/`. For other image containers, convert to a supported format or use [`tools/extract_assets.py`](tools/extract_assets.py), which requires `dtk`.

[`Assets`](crates/libsms/src/assets.rs) provides extraction, loading and discovery helpers. Standalone discovery supports an explicit directory, `SMS_ASSETS` and `SMS_DISC`. A complete cache can be reused without reopening the disc. Partial data can load with missing pieces, so callers must check completeness before starting features that require them.

The current C# host resolves its paths through SharedFiles and passes them to the library. Follow its README for that setup; the standalone environment-variable flow is separate.

## Viewer

```powershell
cargo run --release -p viewer
```

To extract on first run:

```powershell
cargo run --release -p viewer -- --iso "C:\path\to\disc.iso"
```

The viewer requires a graphics environment. Without assets it can use a capsule and code-default tuning; that fallback does not validate asset-dependent behavior or appearance.

| Keys | Action |
| --- | --- |
| WASD / Space | Move / jump. |
| F / Z / Left Ctrl | Dive / ground pound / crouch. |
| Left Shift | Use the equipped nozzle. |
| X / 1–4 | Swap / select nozzle. |
| Arrow keys | Orbit the camera. |
| R / M | Respawn / cycle music, including off. |

Add `--demo` for scripted playback. See [`crates/viewer/examples/`](crates/viewer/examples/) for focused captures and inspection tools.

## Host interface

The [`Host` trait](crates/libsms/src/host.rs) supplies ground, roof and wall collision, with optional queries for enemies, carryable objects, water and surface state. Events let the host apply damage and interactions and trigger presentation.

The simulation runs at **120 Hz**. [`Sim::advance`](crates/libsms/src/lib.rs) accumulates elapsed time; `advance_observed` provides per-tick observation for animation/audio scheduling. Coordinates use Y up and approximately centimeter units. The host owns scale and axis conversion.

Tuning is loaded from local `.prm` data. Code defaults support fallback operation and tests. `Options` and `Limits` expose host-specific adjustments without embedding the adapter's item or balance rules in the simulation.

Non-Rust hosts use the [C interface](crates/libsms-ffi/src/lib.rs). Check `sms_abi_version` and `sms_struct_sizes` against the bindings before creating handles. The current modern-branch ABI is **50**; the source constant is authoritative. Use matching native libraries and host bindings, release owned handles, and drain `sms_take_panic` for diagnostics.

Presentation is exposed through `libsms-present` and the C view API. See [audio integration](docs/audio.md) for mixing, scheduling and device playback responsibilities.

## Checks and limitations

```powershell
cargo test --release
cargo run --release -p libsms --example report
```

The report uses the local cache when available and warns before falling back to code defaults. Disc-dependent tests report a skip when their local data is absent. The default workspace members cover the libraries; run the viewer separately.

The [behavior spec](docs/behavior-spec.md) records missing mechanics and fidelity questions. Feature documents may retain older ABI or validation notes; current source and host bindings take precedence. Headless checks and captures establish the behavior they cover, not complete host-game or hardware validation.

## License and credits

Original contributions by **wesleyasaurus** are dedicated to the public domain under
[CC0 1.0 Universal](LICENSE), to the extent wesleyasaurus owns the relevant rights.
Attribution for those contributions is appreciated but not required. Third-party software
and original game content retain their respective rights and terms.
See [CREDITS.md](CREDITS.md) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
