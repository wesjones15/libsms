//! GameCube disc images: plain ISO/GCM and CISO. The reader, file system table and CISO
//! handling are `libcommon::disc`, shared with the other GameCube libraries; this maps its
//! errors onto [`ExtractError`] so extraction code keeps using `?`.

use super::ExtractError;
use libcommon::disc::DiscError;

pub use libcommon::disc::{Disc, ReadAt, open_image};

impl From<DiscError> for ExtractError {
    fn from(e: DiscError) -> Self {
        match e {
            DiscError::Io(e) => ExtractError::Io(e),
            DiscError::Format(m) => ExtractError::Format(m),
            DiscError::Unsupported(m) => ExtractError::Unsupported(m),
            DiscError::Missing(p) => ExtractError::Missing(p),
        }
    }
}
