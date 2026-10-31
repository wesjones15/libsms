//! Pikmin 2's player models and motions from the user's own disc, for hosts that draw Louie (Olimar's
//! alt-skin in RoR2). Same job as the Sunshine extractor: open the image, pull what is needed out of
//! the archives, write it to a cache folder, never touch the disc again.
//!
//! Spec: `NaviMgr::load` and `loadResources_float` (plugProjectKandoU/naviMgr.cpp): Olimar is
//! `orima_model/orima1.bmd` and **Louie is `orima3.bmd`** (`syatyou.bmd`, the President, once the debt is
//! paid) in `user/Kando/piki/pikis.szs`; their motions are its `motion/*.bca`; `texts.szs` holds the
//! motion table (`animmgr.txt`) and collision (`navicoll.txt`).

use std::path::Path;

use libcommon::disc::Disc;

use super::{ExtractError, rarc, safe_join, yaz0};

/// Bump when the set of files changes, so an older cache is redone.
pub const VERSION: u32 = 4;
const MARKER: &str = "p2.ok";

/// (disc path, folder under the cache the archive's files go to).
const ARCHIVES: &[(&str, &str)] = &[
    ("user/Kando/piki/pikis.szs", "piki"),
    ("user/Kando/piki/texts.szs", "piki_texts"),
    // The Hocotate Ship (`ufo.bmd`, `ufo.bck`, `ufo.brk`, `ufoAnimMgr.txt`, `ufoColl.txt`; `ufo_gold` the gold one).
    ("user/Kando/ufo/arc.szs", "ufo"),
];
/// Plain files (the tuning text Louie and the Pikmin use).
const PLAIN: &[(&str, &str)] = &[
    ("user/Abe/piki/naviParms.txt", "parms/naviParms.txt"),
    ("user/Abe/piki/pikiParms.txt", "parms/pikiParms.txt"),
    // The particle container with the navi lights (JPA2, `PID_OrimaLight_*`).
    ("user/Ebisawa/effect/game.jpc", "effects/game.jpc"),
];

/// What a Louie needs from the cache.
pub const NAVI_FILES: [&str; 4] =
    ["piki/orima_model/orima3.bmd", "piki/motion/wait.bca", "piki_texts/animmgr.txt", "effects/game.jpc"];

/// Whether `dir` holds a complete extraction of this version.
pub fn is_complete(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join(MARKER)).is_ok_and(|s| s.trim() == VERSION.to_string())
        && NAVI_FILES.iter().all(|f| dir.join(f).is_file())
        && audio_is_complete(dir)
}

pub(super) fn disc_err(e: libcommon::disc::DiscError) -> ExtractError {
    match e {
        libcommon::disc::DiscError::Io(e) => ExtractError::Io(e),
        libcommon::disc::DiscError::Missing(p) => ExtractError::Missing(p),
        libcommon::disc::DiscError::Format(m) | libcommon::disc::DiscError::Unsupported(m) => ExtractError::Format(m),
    }
}

/// Extracts the player files from the Pikmin 2 disc at `iso` into `out`. Skips the work when `out` is
/// complete. `log` gets short progress messages. Returns how many files were written.
pub fn extract_navi(iso: &Path, out: &Path, log: &mut dyn FnMut(&str)) -> Result<usize, ExtractError> {
    if is_complete(out) {
        log("Pikmin 2 files already extracted");
        return Ok(0);
    }
    let disc = Disc::open(iso).map_err(disc_err)?;
    // GPVE01 / GPVP01 / GPVJ01.
    if !disc.game_id.starts_with("GPV") {
        return Err(ExtractError::WrongGame(disc.game_id.clone()));
    }
    log(&format!("disc: {}", disc.game_id));
    let mut wrote = 0;
    for (path, dest) in ARCHIVES {
        let file = disc.find(path).ok_or_else(|| ExtractError::Missing((*path).to_owned()))?;
        let raw = disc.read(file).map_err(disc_err)?;
        let bytes = if yaz0::is_yaz0(&raw) { yaz0::decompress(&raw)? } else { raw };
        let arc = rarc::Archive::parse(bytes)?;
        let paths: Vec<String> = arc.paths().map(str::to_owned).collect();
        for rel in paths {
            let Some(data) = arc.file(&rel) else { continue };
            let target = safe_join(&out.join(dest), &rel)?;
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(target, data)?;
            wrote += 1;
        }
        log(&format!("{path} -> {dest}"));
    }
    for (path, dest) in PLAIN {
        let file = disc.find(path).ok_or_else(|| ExtractError::Missing((*path).to_owned()))?;
        let data = disc.read(file).map_err(disc_err)?;
        let target = safe_join(out, dest)?;
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(target, data)?;
        wrote += 1;
    }
    wrote += extract_audio(iso, out, log)?;
    std::fs::write(out.join(MARKER), VERSION.to_string())?;
    Ok(wrote)
}

/// Bump when the set of sound files changes.
pub const AUDIO_VERSION: u32 = 2;
const AUDIO_MARKER: &str = "p2audio.ok";

/// The sound files Louie's voice and effects need: the init data (instrument and wave banks, the sound table), the sound
/// names, the sequences, and the wave archives (`Banks/wScene_N.aw`, all of them: which scene holds what is in the init data).
pub const AUDIO_FILES: [&str; 3] = ["AudioRes/PSound.aaf", "AudioRes/PSound.asn", "AudioRes/Seqs/Seq.arc"];

/// Whether `dir` holds a complete sound extraction of this version.
pub fn audio_is_complete(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join(AUDIO_MARKER)).is_ok_and(|s| s.trim() == AUDIO_VERSION.to_string())
        && AUDIO_FILES.iter().all(|f| dir.join(f).is_file())
        && ["trackMap_Seq_W.txt","trackMap_Seq_T.txt"].iter().all(|f|dir.join("music").join(f).is_file())
}

/// Copies the sound files out of the Pikmin 2 disc at `iso` into `out` (same layout as the disc's `AudioRes/`).
/// Skips the work when `out` is complete. Returns how many files were written.
pub fn extract_audio(iso: &Path, out: &Path, log: &mut dyn FnMut(&str)) -> Result<usize, ExtractError> {
    if audio_is_complete(out) {
        log("Pikmin 2 sound files already extracted");
        return Ok(0);
    }
    let disc = Disc::open(iso).map_err(disc_err)?;
    if !disc.game_id.starts_with("GPV") {
        return Err(ExtractError::WrongGame(disc.game_id.clone()));
    }
    let mut wrote = 0;
    let mut wanted: Vec<String> = AUDIO_FILES.iter().map(|s| (*s).to_owned()).collect();
    for f in disc.files() {
        if f.path.starts_with("AudioRes/Banks/") && f.path.ends_with(".aw") {
            wanted.push(f.path.clone());
        }
    }
    for path in wanted {
        let file = disc.find(&path).ok_or_else(|| ExtractError::Missing(path.clone()))?;
        let data = disc.read(file).map_err(disc_err)?;
        let target = safe_join(out, &path)?;
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(target, data)?;
        wrote += 1;
    }
    // PSM::DirectorMgr_Scene::initTrackMap selects these source layer maps.
    for (source,name) in [("user/Wakai/trackMap_Seq_W.txt","trackMap_Seq_W.txt"),("user/Totaka/trackMap_Seq_T.txt","trackMap_Seq_T.txt")] {
        let file=disc.find(source).ok_or_else(||ExtractError::Missing(source.into()))?;
        let data=disc.read(file).map_err(disc_err)?;
        std::fs::create_dir_all(out.join("music"))?;
        std::fs::write(out.join("music").join(name),data)?;
        wrote+=1;
    }
    log(&format!("{wrote} sound files"));
    std::fs::write(out.join(AUDIO_MARKER), AUDIO_VERSION.to_string())?;
    Ok(wrote)
}

/// Finds a Pikmin 2 image (`.iso`, `.gcm`, `.ciso`) in `dirs`, one level deep.
pub fn find_disc(dirs: &[std::path::PathBuf]) -> Option<std::path::PathBuf> {
    for dir in dirs {
        let Ok(rd) = std::fs::read_dir(dir) else { continue };
        let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            let ext = p.extension().and_then(|e| e.to_str()).map(str::to_lowercase);
            if matches!(ext.as_deref(), Some("iso" | "gcm" | "ciso"))
                && Disc::open(&p).is_ok_and(|d| d.game_id.starts_with("GPV"))
            {
                return Some(p);
            }
        }
    }
    None
}
