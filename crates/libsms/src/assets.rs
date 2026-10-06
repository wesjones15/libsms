//! Locating the user's extracted disc files.
//!
//! The repository ships no game data. `tools/extract_assets.py` builds an `assets/` folder from
//! the user's own disc image; this module finds it and loads what the sim needs from it.
//!
//! ```text
//! assets/params_mario/*.prm       tuning
//! assets/bck/ma_*.bck             animation clips
//! assets/bmd/ma_mdl1.bmd          skeleton source
//! ```

use std::path::{Path, PathBuf};

use crate::anim::AnimTable;
use crate::params::Params;
use crate::skeleton::{ClipLibrary, Skeleton};

/// Everything extracted from the disc that the library can use. Each part is optional so a
/// partial extraction still works.
#[derive(Debug, Clone)]
pub struct Assets {
    pub dir: PathBuf,
    pub params: Params,
    /// How many `.prm` files were found and parsed (0 means code defaults are in use).
    pub prm_files: usize,
    pub anim_table: AnimTable,
    pub clips: ClipLibrary,
    pub skeleton: Option<Skeleton>,
}

/// Looks for an `assets` folder: an explicit path, `$SMS_ASSETS`, then `assets/` in the current
/// directory and a few levels up (so it works from the workspace root, a crate folder, or
/// `target/debug`).
pub fn find(explicit: Option<&Path>) -> Option<PathBuf> {
    let mut cands: Vec<PathBuf> = Vec::new();
    cands.extend(explicit.map(Path::to_path_buf));
    cands.extend(std::env::var_os("SMS_ASSETS").map(PathBuf::from));
    for base in [".", "..", "../..", "../../.."] {
        cands.push(Path::new(base).join("assets"));
    }
    cands.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets"));
    cands.into_iter().find(|p| p.join("params_mario").is_dir())
}

impl Assets {
    /// Loads whatever is in `dir`.
    pub fn load(dir: &Path) -> Assets {
        let (params, prm_files) = Params::from_dir(&dir.join("params_mario"));
        let skeleton = std::fs::read(dir.join("bmd/ma_mdl1.bmd"))
            .ok()
            .and_then(|b| Skeleton::from_bmd(&b));
        let mut anim_table = AnimTable::from_dir(&dir.join("bck"));
        crate::fludd_anim::insert_fludd_clips(&mut anim_table, &dir.join("watergun2"));
        crate::yoshi::insert_yoshi_clips(
            &mut anim_table,
            &dir.join("yoshi"),
            &dir.join("scene/mapobj"),
        );
        Assets {
            dir: dir.to_path_buf(),
            params,
            prm_files,
            anim_table,
            clips: ClipLibrary::from_dir(&dir.join("bck")),
            skeleton,
        }
    }

    /// First-run helper: if `dir` is not a complete extraction, build it from the disc image at
    /// `iso` (see [`crate::extract`]), then load it. The disc is not touched again afterwards.
    pub fn ensure(
        iso: &Path,
        dir: &Path,
        log: &mut dyn FnMut(&str),
    ) -> Result<Assets, crate::extract::ExtractError> {
        if !crate::extract::is_complete(dir) {
            crate::extract::extract_assets(iso, dir, log)?;
        }
        Ok(Assets::load(dir))
    }

    /// `find` then `load`.
    pub fn discover(explicit: Option<&Path>) -> Option<Assets> {
        find(explicit).map(|d| Assets::load(&d))
    }

    /// The first-run path for hosts: use a complete `assets/` if there is one; otherwise find
    /// the user's disc (`$SMS_DISC`, or an image in `disc_dirs`) and extract into the folder
    /// `find` picked, or `assets/` at the workspace root. An older, partial extraction is
    /// completed the same way. Without a disc, a partial folder still loads.
    pub fn discover_or_extract(
        explicit: Option<&Path>,
        disc_dirs: &[PathBuf],
        log: &mut dyn FnMut(&str),
    ) -> Option<Assets> {
        let dir = find(explicit);
        if let Some(d) = &dir
            && crate::extract::is_complete(d)
        {
            return Some(Assets::load(d));
        }
        let out = dir.clone().unwrap_or_else(|| {
            explicit
                .map(Path::to_path_buf)
                .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets"))
        });
        match crate::extract::find_disc(disc_dirs) {
            Some(iso) => {
                log(&format!(
                    "extracting {} into {}",
                    iso.display(),
                    out.display()
                ));
                match Assets::ensure(&iso, &out, log) {
                    Ok(a) => return Some(a),
                    Err(e) => log(&format!("extraction failed: {e}")),
                }
            }
            None => log("no Super Mario Sunshine disc found (set SMS_DISC or pass --iso)"),
        }
        dir.map(|d| Assets::load(&d))
    }
}
