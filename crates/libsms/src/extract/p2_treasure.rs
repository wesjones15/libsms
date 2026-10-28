//! Pikmin 2's treasures from the user's own disc, for hosts that place them in the world (the Ship Repair
//! bonus objectives in RoR2). A separate extraction from Louie's (`p2.rs`) with its own marker, so adding
//! it never redoes the player files.
//!
//! Spec: `PelletList::Mgr::globalInstance` (plugProjectKandoU/gamePelletList.cpp) reads
//! `otakara_config.txt` (the 188 treasures), `item_config.txt` (the 13 Onion-pod upgrades, which the game
//! counts in the same collection) and `carcass_config.txt` from `pelletlist_<region>.szs`; the loose copies in
//! `user/Abe/Pellet/<region>/` are the same text. Each config's `archive` is a `.szs` beside them holding the
//! model (`bmd`) and the carry wobble (`carry.bck`); `animmgr`/`colltree` live in `otakara_texts.szs` /
//! `item_texts.szs` under `<name>/` (`PelletConfig::TParms`, pelletConfig.cpp). The names and
//! descriptions are in `message/mesRes_eng.szs:pikmin2.bmg` (`zukan2D.cpp` `TOffsetMsgSet`).
//!
//! Layout under `<out>/treasure/`: `config/*.txt`, `models/<archive>/<file>`, `texts/<kind>/<name>/<file>`,
//! `message/pikmin2.bmg`, `effects/*.jpc`, `streams/*.ast`.

use std::path::Path;

use libcommon::disc::Disc;

use super::p2::disc_err;
use super::{ExtractError, rarc, safe_join, yaz0};

/// Bump when the set of files changes, so an older cache is redone.
pub const VERSION: u32 = 2;
const MARKER: &str = "p2treasure.ok";
/// Folder under the cache root everything here goes to.
pub const DIR: &str = "treasure";

/// The config files the table loads (inside `DIR/config/`).
pub const CONFIGS: [&str; 3] = ["otakara_config.txt", "item_config.txt", "carcass_config.txt"];
/// Where the names and descriptions are (inside `DIR/`).
pub const MESSAGE: &str = "message/pikmin2.bmg";

/// The treasure jingles (`PSSTR_TREASURE_GET` / `_GLAD` / `_COMP`, SoundID.h): streams under `streams/`.
pub const STREAMS: [&str; 3] = ["ff_treasureget", "ff_treasureglad", "ff_treasurecomp"];

/// Disc files whose bytes are copied verbatim: (disc path, path under `DIR/`).
const EFFECTS: &[(&str, &str)] = &[
    ("user/Ebisawa/effect/game.jpc", "effects/ebisawa_game.jpc"),
];

/// Whether `dir` (the cache root, the folder holding `p2.ok`) has a complete treasure extraction of this version.
pub fn is_complete(dir: &Path) -> bool {
    let root = dir.join(DIR);
    std::fs::read_to_string(dir.join(MARKER)).is_ok_and(|s| s.trim() == VERSION.to_string())
        && CONFIGS.iter().all(|f| root.join("config").join(f).is_file())
        && root.join(MESSAGE).is_file()
        && STREAMS.iter().all(|s| root.join(format!("streams/{s}.ast")).is_file())
}

/// The disc's per-region folder under `user/Abe/Pellet/` (`GPVE` USA, `GPVP` PAL, `GPVJ` Japan).
fn region(game_id: &str) -> &'static str {
    match game_id.get(3..4) {
        Some("P") => "pal",
        Some("J") => "jpn",
        _ => "us",
    }
}

fn write_to(root: &Path, rel: &str, data: &[u8]) -> Result<(), ExtractError> {
    let target = safe_join(root, rel)?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(target, data)?;
    Ok(())
}

fn read_archive(disc: &Disc, path: &str) -> Result<rarc::Archive, ExtractError> {
    let file = disc.find(path).ok_or_else(|| ExtractError::Missing(path.to_owned()))?;
    let raw = disc.read(file).map_err(disc_err)?;
    let bytes = if yaz0::is_yaz0(&raw) { yaz0::decompress(&raw)? } else { raw };
    rarc::Archive::parse(bytes)
}

/// The `archive` value of each `{ ... }` block in a config text (what `PelletConfig::TParms` reads as `archive`).
fn archives_in(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let mut w = l.split_whitespace();
            (w.next() == Some("archive")).then(|| w.next().map(str::to_owned)).flatten()
        })
        .collect()
}

/// Extracts the treasures from the Pikmin 2 disc at `iso` into `out` (the cache root; files go under
/// `out/treasure/`). Skips the work when it is complete. Returns how many files were written.
pub fn extract(iso: &Path, out: &Path, log: &mut dyn FnMut(&str)) -> Result<usize, ExtractError> {
    if is_complete(out) {
        log("Pikmin 2 treasure files already extracted");
        return Ok(0);
    }
    let disc = Disc::open(iso).map_err(disc_err)?;
    if !disc.game_id.starts_with("GPV") {
        return Err(ExtractError::WrongGame(disc.game_id.clone()));
    }
    let reg = region(&disc.game_id);
    let base = format!("user/Abe/Pellet/{reg}");
    let root = out.join(DIR);
    let mut wrote = 0;
    log(&format!("disc: {} ({reg})", disc.game_id));

    for name in CONFIGS {
        let path = format!("{base}/{name}");
        let file = disc.find(&path).ok_or_else(|| ExtractError::Missing(path.clone()))?;
        let data = disc.read(file).map_err(disc_err)?;
        write_to(&root, &format!("config/{name}"), &data)?;
        wrote += 1;
        // Treasures and upgrades have models; the carcasses are enemy corpses (another job).
        if name == "carcass_config.txt" {
            continue;
        }
        let kind = name.trim_end_matches("_config.txt");
        let text = String::from_utf8_lossy(&data).into_owned();
        let mut count = 0;
        for arc_name in archives_in(&text) {
            let stem = arc_name.trim_end_matches(".szs");
            let arc = read_archive(&disc, &format!("{base}/{arc_name}"))?;
            let paths: Vec<String> = arc.paths().map(str::to_owned).collect();
            for rel in paths {
                if let Some(d) = arc.file(&rel) {
                    write_to(&root, &format!("models/{stem}/{rel}"), d)?;
                    wrote += 1;
                }
            }
            count += 1;
        }
        // animmgr.txt and collinfo.txt for every config of the kind.
        let tex = read_archive(&disc, &format!("{base}/{kind}_texts.szs"))?;
        let paths: Vec<String> = tex.paths().map(str::to_owned).collect();
        for rel in paths {
            if let Some(d) = tex.file(&rel) {
                write_to(&root, &format!("texts/{kind}/{rel}"), d)?;
                wrote += 1;
            }
        }
        log(&format!("{kind}: {count} archives"));
    }

    let msg = read_archive(&disc, "message/mesRes_eng.szs")?;
    for (rel, dest) in [("pikmin2.bmg", MESSAGE), ("pikmin2.bmc", "message/pikmin2.bmc")] {
        if let Some(d) = msg.file(rel) {
            write_to(&root, dest, d)?;
            wrote += 1;
        }
    }
    for (path, dest) in EFFECTS {
        let file = disc.find(path).ok_or_else(|| ExtractError::Missing((*path).to_owned()))?;
        write_to(&root, dest, &disc.read(file).map_err(disc_err)?)?;
        wrote += 1;
    }
    for stem in STREAMS {
        let path = format!("AudioRes/Stream/{stem}.w.32.c4.ast");
        let file = disc.find(&path).ok_or_else(|| ExtractError::Missing(path.clone()))?;
        write_to(&root, &format!("streams/{stem}.ast"), &disc.read(file).map_err(disc_err)?)?;
        wrote += 1;
    }
    log(&format!("{wrote} treasure files"));
    std::fs::write(out.join(MARKER), VERSION.to_string())?;
    Ok(wrote)
}
