//! C ABI over `libsms`. Hosts that aren't Rust (or that load us as a DLL) use this.
//! The header will be generated with cbindgen once the API settles.
//!
//! Not wired up yet: the Rust API is still moving. The planned shape mirrors `libpikmin-ffi`:
//! a `SmsHost` callback table (ground/roof/wall probes, event sink), `sms_sim_new/free`,
//! `sms_sim_advance(pad, camera_yaw, dt)` and plain-struct getters for Mario's pose, status
//! and animation clip.

/// ABI version, bumped when the C interface changes.
pub const SMS_ABI_VERSION: u32 = 0;

#[unsafe(no_mangle)]
pub extern "C" fn sms_abi_version() -> u32 {
    SMS_ABI_VERSION
}

/// Fixed simulation rate in Hz.
#[unsafe(no_mangle)]
pub extern "C" fn sms_tick_hz() -> u32 {
    libsms::TICK_HZ
}
