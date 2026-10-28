//! Pikmin 2's enemies from the user's own disc, for hosts that fight them (Ship Repair in RoR2). A separate
//! extraction from Louie's (`p2.rs`) and the treasures' (`p2_treasure.rs`) with its own marker, so adding it never
//! redoes their files.
//!
//! Spec: `EnemyMgrBase` (plugProjectYamashitaU/enemyMgrBase.cpp) loads, per enemy name, `/enemy/data/<name>/model.szs`
//! (`enemy.bmd`), `anim.szs` (the `.bca` clips) and loose textures (`changeTexture`, `.brk`/`.btk`), and from the
//! parameter archive `enemy/parm/enemyParms.szs` the text files `<name>/enemyParm.txt` (`EnemyParmsBase::read`),
//! `enemyColl.txt` (the collision tree), `enemyAnimMgr.txt` (which clip is which motion) and `enemyStoneInfo.txt`.
//!
//! Layout under `<out>/enemy/`: `parm/<name>/<file>` (names as the archive spells them, lower case),
//! `data/<Name>/model/<file>`, `data/<Name>/anim/<file>`, `data/<Name>/<loose file>`, `common/<file>`.

use std::path::Path;

use libcommon::disc::Disc;

use super::p2::disc_err;
use super::{ExtractError, rarc, safe_join, yaz0};

/// Bump when the set of files changes, so an older cache is redone.
pub const VERSION: u32 = 1;
const MARKER: &str = "p2enemy.ok";
/// Folder under the cache root everything here goes to.
pub const DIR: &str = "enemy";

/// The parameter archive on the disc.
const PARMS: &str = "enemy/parm/enemyParms.szs";
const COMMON: &str = "enemy/common/enemyCommon.szs";

/// Whether `dir` (the cache root, the folder holding `p2.ok`) has a complete enemy extraction of this version.
pub fn is_complete(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join(MARKER)).is_ok_and(|s| s.trim() == VERSION.to_string())
        && dir.join(DIR).join("parm/kogane/enemyparm.txt").is_file()
        && dir.join(DIR).join("data/Kogane/model/enemy.bmd").is_file()
}

fn write_to(root: &Path, rel: &str, data: &[u8]) -> Result<(), ExtractError> {
    let target = safe_join(root, rel)?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(target, data)?;
    Ok(())
}

fn open_archive(disc: &Disc, path: &str) -> Result<rarc::Archive, ExtractError> {
    let file = disc.find(path).ok_or_else(|| ExtractError::Missing(path.to_owned()))?;
    let raw = disc.read(file).map_err(disc_err)?;
    let bytes = if yaz0::is_yaz0(&raw) { yaz0::decompress(&raw)? } else { raw };
    rarc::Archive::parse(bytes)
}

fn unpack(arc: &rarc::Archive, root: &Path, prefix: &str) -> Result<usize, ExtractError> {
    let mut wrote = 0;
    let paths: Vec<String> = arc.paths().map(str::to_owned).collect();
    for rel in paths {
        if let Some(d) = arc.file(&rel) {
            write_to(root, &format!("{prefix}{rel}"), d)?;
            wrote += 1;
        }
    }
    Ok(wrote)
}

/// Extracts every enemy's model, clips, textures and parameter text from the Pikmin 2 disc at `iso` into `out` (the
/// cache root; files go under `out/enemy/`). Skips the work when it is complete. Returns how many files were written.
pub fn extract(iso: &Path, out: &Path, log: &mut dyn FnMut(&str)) -> Result<usize, ExtractError> {
    if is_complete(out) {
        log("Pikmin 2 enemy files already extracted");
        return Ok(0);
    }
    let disc = Disc::open(iso).map_err(disc_err)?;
    if !disc.game_id.starts_with("GPV") {
        return Err(ExtractError::WrongGame(disc.game_id.clone()));
    }
    let root = out.join(DIR);
    let mut wrote = unpack(&open_archive(&disc, PARMS)?, &root, "parm/")?;
    wrote += unpack(&open_archive(&disc, COMMON)?, &root, "common/")?;
    log(&format!("parameters and common parts: {wrote} files"));

    let mut enemies = 0;
    let files: Vec<(String, String)> = disc
        .files()
        .iter().filter_map(|f| {
            let rest = f.path.strip_prefix("enemy/data/")?;
            let (name, file) = rest.split_once('/')?;
            (!file.contains('/')).then(|| (name.to_owned(), f.path.clone()))
        })
        .collect();
    for (name, path) in files {
        let file = path.rsplit('/').next().unwrap_or_default();
        let dir = format!("data/{name}/");
        match file {
            "model.szs" => wrote += unpack(&open_archive(&disc, &path)?, &root, &format!("{dir}model/"))?,
            "anim.szs" => {
                wrote += unpack(&open_archive(&disc, &path)?, &root, &format!("{dir}anim/"))?;
                enemies += 1;
            }
            // `bti.szs` (the Cannon Beetle larva's texture set) is an archive of textures.
            "bti.szs" => wrote += unpack(&open_archive(&disc, &path)?, &root, &format!("{dir}bti/"))?,
            _ => {
                let f = disc.find(&path).ok_or_else(|| ExtractError::Missing(path.clone()))?;
                write_to(&root, &format!("{dir}{file}"), &disc.read(f).map_err(disc_err)?)?;
                wrote += 1;
            }
        }
    }
    log(&format!("{enemies} animated enemies, {wrote} files"));
    std::fs::write(out.join(MARKER), VERSION.to_string())?;
    Ok(wrote)
}
