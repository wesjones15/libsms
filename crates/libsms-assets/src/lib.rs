//! Readers for Super Mario Sunshine's model and sound data, from the user's extracted `assets/`
//! folder (see `libsms::extract`). Nothing here contains Nintendo data.
//!
//! - [`bmd`]: J3D models (`.bmd`): geometry, skinning, materials and textures.
//! - [`texture`]: GX texture decoding and `.bti` images.
//! - [`pose`]: skinning a model with joint matrices from `libsms::Skeleton`.
//! - [`btp`]: texture pattern animations (Mario's blinking and face swaps).
//! - [`blo`]: J2D screens (`.blo`): the HUD's pane layout.
//! - [`jpa`]: JParticle effects (`.jpa`): emitters, shapes, colours and textures.
//! - [`sound`]: JAudio's banks, waves, sequences, streams and the sound table.
//!
//! The sim (`libsms`) only needs tuning and clip timing; hosts that draw Mario use this crate.

pub mod bmd;
pub mod blo;
pub mod btp;
pub mod jpa;
pub mod mario;
pub mod pose;
mod reader;
pub mod sound;
pub mod texture;

use std::fmt;
use std::path::PathBuf;

pub use bmd::{Batch, BatchVertex, Bmd, Material, Skin};
pub use pose::Pose;
pub use texture::Image;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    /// Ran off the end of the data at this offset.
    Truncated(usize),
    Bad(String),
    NotFound(PathBuf),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::Truncated(at) => write!(f, "data ends early (at {at:#x})"),
            Error::Bad(m) => write!(f, "bad data: {m}"),
            Error::NotFound(p) => write!(f, "not found: {}", p.display()),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

/// Paths under `assets/` (the layout of `mario.szs`, which the extractor keeps).
pub mod paths {
    /// Mario's body (`TMario::initModel`, MarioDraw.cpp).
    pub const BODY: &str = "bmd/ma_mdl1.bmd";
    /// Open hands (`mHandModels[0]`), right then left.
    pub const HANDS_OPEN: [&str; 2] = ["bmd/ma_hnd2r.bmd", "bmd/ma_hnd2l.bmd"];
    /// Fists (`mHandModels[1]`), right then left.
    pub const HANDS_FIST: [&str; 2] = ["bmd/ma_hnd3r.bmd", "bmd/ma_hnd3l.bmd"];
    /// The cap and the cap with sunglasses lens holder (`TMarioCap::TMarioCap`, MarioCap.cpp).
    pub const CAPS: [&str; 2] = ["bmd/ma_cap1.bmd", "bmd/ma_cap3.bmd"];
    pub const SUNGLASSES: &str = "bmd/ma_glass1.bmd";
    /// FLUDD's body and its nozzles (`TWaterGun::init`, WaterGun.cpp).
    pub const FLUDD_BODY: &str = "watergun2/body/wg_mdl1.bmd";
    pub const NOZZLE_SPRAY: &str = "watergun2/normal_wg/normal_wg.bmd";
    pub const NOZZLE_HOVER: &str = "watergun2/hover_wg/hover_wg.bmd";
    pub const NOZZLE_ROCKET: &str = "watergun2/rocket_wg/rocket_wg.bmd";
    pub const NOZZLE_TURBO: &str = "watergun2/back_wg/back_wg.bmd";
    /// The Yoshi slot's model (`nozzleBmdData[3]`): FLUDD's head folded away while Mario rides.
    pub const NOZZLE_YOSHI: &str = "watergun2/dummy_wg/dummy_wg.bmd";
    /// Blinking (`marioAnimeTexPatternFilenames[0]`, MarioDraw.cpp).
    pub const WINK: &str = "btp/ma_wink_tx.btp";
}
