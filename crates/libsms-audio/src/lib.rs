//! Super Mario Sunshine's sound system (JAudio's JASystem and JAInterface, the JSystem C++
//! generation), reimplemented from the doldecomp/sms decomp as a behavioural spec. It plays the
//! disc's own sequences, instruments, waves and streams into PCM.
//!
//! - [`engine`]: JASystem's state (tracks, channel managers, channels, DSP voices).
//! - `track`: sequence tracks and the BMS interpreter (JASTrack.cpp, JASSeqParser.cpp).
//! - `channel`: notes, envelopes and voice allocation (JASChannel.cpp, JASBankMgr.cpp).
//! - `dsp`: DSP voices and the mixer (JASDSPChannel.cpp; the microcode isn't in the decomp).
//! - [`osc`]: envelopes and LFOs (JASOscillator.cpp).
//! - [`jai`]: the game layer (JAInterface + MSound): sound effects through the SE sequence,
//!   music, the title stream, Mario's voice. Hosts use [`SoundSystem`].
//!
//! Nothing here contains Nintendo data: banks, sequences and the sound table are read from the
//! user's extracted `assets/` (`libsms_assets::sound`).

// The port indexes parallel arrays the way the C++ does; iterator rewrites would obscure that.
#![allow(clippy::needless_range_loop)]

pub mod animation;
pub mod calc;
mod channel;
mod dsp;
pub mod engine;
pub mod jai;
pub mod osc;
mod track;

pub use engine::{Engine, NoSync, SyncHandler};
pub use jai::{SoundHandle, SoundSystem};
