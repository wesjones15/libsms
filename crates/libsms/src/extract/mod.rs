//! Builds the `assets/` folder from the user's own disc image, with no external tools.
//!
//! This is the same job as `tools/extract_assets.py`, in Rust so a host (a mod DLL, for example)
//! can do it on first run: open the disc, pull the three archives' contents, write them to a cache
//! folder, and never touch the disc again.
//!
//! ```no_run
//! # use std::path::Path;
//! let report = libsms::extract::extract_assets(
//!     Path::new("Super Mario Sunshine.iso"),
//!     Path::new("assets"),
//!     &mut |msg| println!("{msg}"),
//! )?;
//! println!("{} prm, {} clips", report.prm_files, report.clips);
//! # Ok::<(), libsms::extract::ExtractError>(())
//! ```
//!
//! Supported inputs: plain GameCube images (`.iso`, `.gcm`) and CISO. Dolphin's compressed formats
//! (RVZ, WIA, WBFS) are refused with a clear message; convert them or use the Python script.

pub mod gcm;
pub mod p2;
pub mod p2_enemy;
pub mod p2_treasure;
pub mod rarc;
pub mod yaz0;

use std::fmt;
use std::path::{Component, Path, PathBuf};

pub use gcm::Disc;

#[derive(Debug)]
pub enum ExtractError {
    Io(std::io::Error),
    /// The data is not what the format says it should be.
    Format(&'static str),
    /// A file expected on the disc or in an archive is not there.
    Missing(String),
    Unsupported(&'static str),
    /// A valid disc, but not Super Mario Sunshine.
    WrongGame(String),
}

impl fmt::Display for ExtractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExtractError::Io(e) => write!(f, "I/O error: {e}"),
            ExtractError::Format(m) => write!(f, "bad data: {m}"),
            ExtractError::Missing(p) => write!(f, "not found on the disc: {p}"),
            ExtractError::Unsupported(m) => write!(f, "{m}"),
            ExtractError::WrongGame(id) => {
                write!(
                    f,
                    "this disc is {id}, not Super Mario Sunshine (GMSE01/GMSJ01/GMSP01)"
                )
            }
        }
    }
}

impl std::error::Error for ExtractError {}

impl From<std::io::Error> for ExtractError {
    fn from(e: std::io::Error) -> Self {
        ExtractError::Io(e)
    }
}

/// Where each piece comes from: `(archive under data/, path inside it, destination under out)`.
/// A path ending in `/` is a whole folder, and an empty path the whole archive. Matches
/// `tools/extract_assets.py`.
///
/// `mario.szs` is Mario's whole archive (`TMarioPlayer` loads it, MarioInit.cpp): `bmd/` models
/// (body, caps, hands, sunglasses), `bck/` clips, `bas/` per-clip sound keys, `btp/` face
/// texture swaps, `watergun2/` FLUDD's models and the water effect models and textures.
/// `msound.aaf` is the sound system's init data (banks, wave systems, sound tables), which
/// `TApplication::initialize_bootAfter` reads from `nintendo.arc:audi/`.
///
/// `params.szs:npc/` is the NPCs' tuning (`npcMonteM*.prm` / `npcMonteW*.prm` for the
/// Piantas; `TNpcParams::TNpcParams`, NpcSave.cpp), `params.szs:enemy/` the enemies'
/// (`TSmallEnemyParams` and kin: `hamukuri.prm` for the Strollin' Stu).
///
/// `yoshi.szs` is Yoshi's archive, mounted at `/yoshi/` (`TYoshi::init`, Yoshi.cpp): his
/// model and hands, clips, eye textures (`*_tx.btp`) and per-clip sound keys (`bas/`).
///
/// `particle.szs` holds the common particle effects (`.jpa`, JPA1) the game loads by bare name
/// (`TMarDirector::loadParticleMario` and kin, MarDirectorLoadResource.cpp), Mario's among them:
/// `ms_m_sphipd_a..d.jpa` are the super ground pound's flames.
///
/// `game_6` is the HUD's archive (`TGCConsole2::load` mounts it): `scrn/standard_1.blo` lays out
/// the panes and `timg/` holds their pictures, FLUDD's water gauge among them (`water_*.bti`).
/// European discs carry one per language (`game_6_en.szs`, ...); `|` separates the names tried.
pub const EXTRACTS: [(&str, &str, &str); 8] = [
    ("params.szs", "mario/", "params_mario"),
    ("params.szs", "npc/", "params_npc"),
    ("params.szs", "enemy/", "params_enemy"),
    ("mario.szs", "", ""),
    ("nintendo.szs", "audi/msound.aaf", "AudioRes/msound.aaf"),
    ("yoshi.szs", "", "yoshi"),
    ("particle.szs", "", "particle"),
    ("game_6_en.szs|game_6.szs", "", "game_6"),
];

/// Pieces a folder works without (not in [`REQUIRED`]): a host that finds them draws more.
/// The screen wipes' pictures (`TSMSFader::getGameOverBtiResource`, `getMmarkBtiResource`,
/// ScrnFader.cpp): the GAME OVER wipe's text and the circle wipe's Mario mark. The archives and
/// the paths in them are alternatives in the same order (`|`): a USA disc keeps them in
/// `common.szs` and `subtitle.szs`, a European one in a pair per language (`cmn2d_en.szs`,
/// `subtitle_en.szs`).
pub const OPTIONAL_EXTRACTS: [(&str, &str, &str); 26] = [
    // Map/Shimmer.cpp::TShimmer; dolpic1's scene record names ShimmerLow.
    ("scene/dolpic1.szs", "mapobj/shimmerlow.bmd", "heat_haze/shimmerlow.bmd"),
    ("scene/dolpic1.szs", "mapobj/shimmerlow.btk", "heat_haze/shimmerlow.btk"),
    (
        "common.szs|cmn2d_en.szs",
        "2d/wipe_gameover.bti|wipe_gameover.bti",
        "wipe/wipe_gameover.bti",
    ),
    (
        "subtitle.szs|subtitle_en.szs",
        "timg/wipe_m_mark.bti|timg/wipe_m_mark.bti",
        "wipe/wipe_m_mark.bti",
    ),
    // The Shine Sprite and its Shine Get demo. The camera is `TCameraBck` (Camera/CameraBck.cpp):
    // `camera_model.bmd` is a two-joint rig (`cam_position`, `cam_interest`) the demo's `.bck`
    // animates; `common.szs` holds them. The Shine itself (`TShine::makeMActors`, Item.cpp) is
    // `/scene/mapobj/shine.bmd` with `shine_float.bck` (it bobs while it waits) and
    // `shine_demo_shine_get.bck` (it rises over Mario's head). Every stage archive carries the
    // same copies; Airstrip's is read.
    (
        "common.szs",
        "camera/camera_model.bmd",
        "camera/camera_model.bmd",
    ),
    (
        "common.szs",
        "camera/camera_demo_shine_get_outside.bck",
        "camera/camera_demo_shine_get_outside.bck",
    ),
    (
        "common.szs",
        "camera/camera_demo_shine_get_inside.bck",
        "camera/camera_demo_shine_get_inside.bck",
    ),
    (
        "scene/airport0.szs",
        "mapobj/shine.bmd",
        "scene/mapobj/shine.bmd",
    ),
    (
        "scene/airport0.szs",
        "mapobj/shine_float.bck",
        "scene/mapobj/shine_float.bck",
    ),
    (
        "scene/airport0.szs",
        "mapobj/shine_demo_shine_get.bck",
        "scene/mapobj/shine_demo_shine_get.bck",
    ),
    (
        "scene/airport0.szs",
        "mapobj/shine_empty.bmd",
        "scene/mapobj/shine_empty.bmd",
    ),
    // ModelGate.cpp: Bianco M gate and Plaza map; all paths verified on the disc.
    (
        "scene/dolpic1.szs",
        "map/map/gate/05_gate01.bmd",
        "portal/05_gate01.bmd",
    ),
    (
        "scene/dolpic1.szs",
        "map/map/gate/05_gate01.btk",
        "portal/05_gate01.btk",
    ),
    (
        "scene/dolpic1.szs",
        "map/map/gate/05_gate01.brk",
        "portal/05_gate01.brk",
    ),
    (
        "scene/dolpic1.szs",
        "map/map/gate/ms_gatewind_a.jpa",
        "portal/ms_gatewind_a.jpa",
    ),
    (
        "scene/dolpic1.szs",
        "map/map/gate/ms_gatewind_a2.jpa",
        "portal/ms_gatewind_a2.jpa",
    ),
    (
        "scene/dolpic1.szs",
        "map/map/gate/ms_gatewind_a3.jpa",
        "portal/ms_gatewind_a3.jpa",
    ),
    (
        "scene/dolpic1.szs",
        "map/map/gate/ms_gatewind_b.jpa",
        "portal/ms_gatewind_b.jpa",
    ),
    // Piantissimo's body (`TEnemyMario`, enemyMario.cpp): Mario's 29 joints in Mario's order, so
    // Mario's pose drives it as it is (the Piantissimo alt-skin). Gelato Beach's race stage.
    (
        "scene/mamma4.szs",
        "map/map/pad/monteman_model.bmd",
        "npc/monteman/monteman_model.bmd",
    ),
    ("scene/dolpic1.szs", "map/map/map.bmd", "portal/plaza.bmd"),
    ("scene/dolpic1.szs", "map/scene.bin", "portal/scene.bin"),
    (
        "scene/dolpic1.szs",
        "mapobj/efdokangate.bmd",
        "portal/efdokangate.bmd",
    ),
    (
        "scene/dolpic1.szs",
        "mapobj/manhole.bmd",
        "portal/manhole.bmd",
    ),
    (
        "scene/dolpic1.szs",
        "mapobj/monumentshine.bmd",
        "portal/monumentshine.bmd",
    ),
    (
        "scene/dolpic1.szs",
        "mapobj/palmleaf.bmd",
        "portal/palmleaf.bmd",
    ),
    (
        "scene/dolpic1.szs",
        "mapobj/palmnormal.bmd",
        "portal/palmnormal.bmd",
    ),
];

/// Folders gathered from every stage archive (`data/scene/*.szs`) into one folder under `out`:
/// `(path prefix inside the archives, destination)`. Each stage carries only the NPC kinds it
/// uses, so the union over all stages is the full set. `monte*` are the Piantas: each
/// `TMonteM*Manager::createModelData` / `TMonteW*Manager` (NpcManager.cpp) loads its body from
/// `/scene/montem<x>/` or `/scene/montew<x>/` (some share `montem/`), and `montemcommon/` /
/// `montewcommon/` hold the shared clips (`mom_*` / `mow_*` .bck, .btp, .bas) and accessory
/// models (hats, glasses, ...). `mapobj/fruit*` are the fruit Mario picks up and throws
/// (`TResetFruit`, MapObjBall.cpp; `FruitPapaya` and friends in MapObjInit.cpp).
/// `mapobj/eggyoshi*` is Yoshi's egg and the bubble showing the fruit it wants (`TEggYoshi`,
/// Item.cpp; `eggYoshi` in MapObjInit.cpp). `hamukuri*` is the Strollin' Stu (`THamuKuri`,
/// hamukuri.cpp): `hamukuri/default.bmd` and its clips in `hamukurianm/`, a small enemy Yoshi
/// can eat. `mapobj/coin*` are the coins (`coin.bmd`, `coin_red`, `coin_blue`; `coin_data` in
/// MapObjInit.cpp). `kagemario/` is Shadow Mario (`TEnemyMario`, enemyMario.cpp loads
/// `/scene/kagemario/kagemario_model.bmd`): his body's look, for hosts that draw a dark Mario.
/// `sambo*` is the flower tower and the flowers (hanasambo.cpp): `sambo/` the tower's body,
/// `sambohead/` its flower head and the seed it shoots, `samboflower/` the plain flower you spray
/// into bloom and its leaf; models, clips and `bas/` sound keys, all in the Bianco and Monte stages.
pub const SCENE_FOLDERS: [(&str, &str); 7] = [
    ("monte", "npc"),
    ("mapobj/fruit", "scene"),
    ("mapobj/eggyoshi", "scene"),
    ("hamukuri", "enemy"),
    ("mapobj/coin", "scene"),
    ("kagemario", "enemy"),
    ("sambo", "enemy"),
];

/// Disc folders copied as they are: `AudioRes/` holds the wave archives (`Banks/*.aw`), the
/// sequence archive (`Seqs/sequence.arc`), streams and `mSound.asn` (MSound.cpp,
/// `MSound::MSound` sets these paths).
pub const DISC_FOLDERS: [&str; 1] = ["AudioRes/"];

/// Files that must exist for the library and viewer to work. Adding one here makes an older
/// `assets/` incomplete, so hosts re-extract it on the next run.
pub const REQUIRED: [&str; 38] = [
    "params_mario/run.prm",
    "params_mario/jump.prm",
    "params_mario/mario.prm",
    "params_mario/watergun/nozzledeform.prm",
    "bck/ma_wait.bck",
    "bck/ma_run2.bck",
    "bmd/ma_mdl1.bmd",
    "bmd/ma_cap1.bmd",
    "bmd/ma_hnd2r.bmd",
    "bas/ma_jump.bas",
    "btp/ma_wink_tx.btp",
    "watergun2/body/wg_mdl1.bmd",
    "watergun2/normal_wg/normal_wg.bmd",
    "AudioRes/msound.aaf",
    "AudioRes/mSound.asn",
    "AudioRes/Seqs/sequence.arc",
    "params_npc/npcmontem.prm",
    "npc/montemcommon/mom_wait.bck",
    "npc/montewcommon/mow_wait.bck",
    "npc/montema/moma_model.bmd",
    "npc/montew/mow_model.bmd",
    "scene/mapobj/fruitpapaya.bmd",
    "scene/mapobj/eggyoshi_normal.bmd",
    "yoshi/yoshi_model.bmd",
    "enemy/hamukuri/default.bmd",
    "scene/mapobj/coin.bmd",
    "params_mario/dmghamukuri.prm",
    "particle/ms_m_sphipd_a.jpa",
    "particle/ms_m_watslide_a.jpa",
    "particle/ms_m_watslide_b.jpa",
    "particle/ms_m_watslide_c.jpa",
    "particle/ms_coinget_a.jpa",
    "particle/ms_coinget_b.jpa",
    "game_6/scrn/standard_1.blo",
    "enemy/kagemario/default.bmd",
    "enemy/sambo/sambo.bmd",
    "enemy/sambohead/sambohead.bmd",
    "enemy/samboflower/flower.bmd",
];

pub const GAME_IDS: [(&str, &str); 3] = [
    ("GMSJ01", "Japan"),
    ("GMSE01", "USA"),
    ("GMSP01", "Europe (PAL)"),
];

/// Is `dir` a complete extraction?
pub fn is_complete(dir: &Path) -> bool {
    REQUIRED.iter().all(|r| dir.join(r).is_file())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub game_id: String,
    pub prm_files: usize,
    pub clips: usize,
    pub out: PathBuf,
}

/// A destination path built from archive-supplied names must stay under the output folder.
fn safe_join(base: &Path, rel: &str) -> Result<PathBuf, ExtractError> {
    let p = Path::new(rel);
    if p.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(ExtractError::Format(
            "archive path escapes the output folder",
        ));
    }
    Ok(base.join(p))
}

/// Whether the optional pieces ([`OPTIONAL_EXTRACTS`]) are all in `dir`.
pub fn has_extras(dir: &Path) -> bool {
    OPTIONAL_EXTRACTS.iter().all(|e| dir.join(e.2).is_file())
}

/// Extracts the optional pieces ([`OPTIONAL_EXTRACTS`]) from the disc at `iso` into `out`, for a
/// folder that is complete without them. Returns how many it wrote; one the disc lacks is
/// skipped (and logged), never an error.
pub fn extract_extras(
    iso: &Path,
    out: &Path,
    log: &mut dyn FnMut(&str),
) -> Result<usize, ExtractError> {
    let disc = Disc::open(iso)?;
    let mut wrote = 0;
    // The archive read last: several pieces come out of one.
    let mut cache: Option<(&str, rarc::Archive)> = None;
    for (archive, inner, dest) in OPTIONAL_EXTRACTS {
        let target = safe_join(out, dest)?;
        if target.is_file() {
            continue;
        }
        let mut found = Err(ExtractError::Missing(format!("{archive}:{inner}")));
        for (arc_name, path) in archive.split('|').zip(inner.split('|')) {
            if cache.as_ref().map(|c| c.0) != Some(arc_name) {
                cache = disc
                    .read_file(&format!("data/{arc_name}"))
                    .map_err(ExtractError::from)
                    .and_then(|raw| {
                        let bytes = if yaz0::is_yaz0(&raw) {
                            yaz0::decompress(&raw)?
                        } else {
                            raw
                        };
                        rarc::Archive::parse(bytes)
                    })
                    .ok()
                    .map(|a| (arc_name, a));
            }
            found = match &cache {
                Some((_, arc)) => arc
                    .file(path)
                    .map(|d| d.to_vec())
                    .ok_or_else(|| ExtractError::Missing(format!("{arc_name}:{path}"))),
                None => Err(ExtractError::Missing(format!("data/{arc_name}"))),
            };
            if found.is_ok() {
                break;
            }
        }
        match found {
            Ok(data) => {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(target, data)?;
                log(&format!("{archive}:{inner} -> {dest}"));
                wrote += 1;
            }
            Err(e) => log(&format!("skipped {archive}:{inner}: {e}")),
        }
    }
    Ok(wrote)
}

/// Extracts everything libsms needs from the disc at `iso` into `out`. Skips the work if `out`
/// is already complete. `log` receives short progress messages.
pub fn extract_assets(
    iso: &Path,
    out: &Path,
    log: &mut dyn FnMut(&str),
) -> Result<Report, ExtractError> {
    let disc = Disc::open(iso)?;
    if !GAME_IDS.iter().any(|(id, _)| *id == disc.game_id) {
        return Err(ExtractError::WrongGame(disc.game_id.clone()));
    }
    let region = GAME_IDS
        .iter()
        .find(|(id, _)| *id == disc.game_id)
        .map(|x| x.1)
        .unwrap_or("?");
    log(&format!("disc: {} {region}", disc.game_id));

    let count = |dir: &str, ext: &str| -> usize {
        std::fs::read_dir(out.join(dir))
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some(ext))
                    .count()
            })
            .unwrap_or(0)
    };
    let prm_count = |out: &Path| -> usize {
        walk(&out.join("params_mario"))
            .iter()
            .filter(|p| p.ends_with(".prm"))
            .count()
    };

    if is_complete(out) {
        log("assets already complete; nothing to do");
        return Ok(Report {
            game_id: disc.game_id,
            prm_files: prm_count(out),
            clips: count("bck", "bck"),
            out: out.to_path_buf(),
        });
    }

    let mut cache: Option<(String, rarc::Archive)> = None;
    for (archive, inner, dest) in EXTRACTS {
        if cache.as_ref().map(|c| c.0.as_str()) != Some(archive) {
            log(&format!("reading {archive}"));
            let mut names = archive.split('|');
            let first = names.next().unwrap_or(archive);
            let mut raw = disc.read_file(&format!("data/{first}"));
            for name in names {
                if raw.is_err() {
                    raw = disc.read_file(&format!("data/{name}"));
                }
            }
            let raw = raw?;
            let bytes = if yaz0::is_yaz0(&raw) {
                yaz0::decompress(&raw)?
            } else {
                raw
            };
            cache = Some((archive.to_owned(), rarc::Archive::parse(bytes)?));
        }
        let arc = &cache.as_ref().unwrap().1;

        if inner.is_empty() || inner.ends_with('/') {
            let mut n = 0;
            for (rel, data) in arc.under(inner) {
                let path = safe_join(&out.join(dest), rel)?;
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(path, data)?;
                n += 1;
            }
            if n == 0 {
                return Err(ExtractError::Missing(format!("{archive}:{inner}")));
            }
            log(&format!("{archive}:{inner} -> {dest}/ ({n} files)"));
        } else {
            let data = arc
                .file(inner)
                .ok_or_else(|| ExtractError::Missing(format!("{archive}:{inner}")))?;
            let path = safe_join(out, dest)?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, data)?;
            log(&format!("{archive}:{inner} -> {dest}"));
        }
    }

    // The wipes' pictures, if this disc has them.
    extract_extras(iso, out, log)?;

    for folder in DISC_FOLDERS {
        let mut n = 0;
        for path in disc.paths_under(folder) {
            let data = disc.read_file(&path)?;
            let dest = safe_join(out, &path)?;
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(dest, data)?;
            n += 1;
        }
        if n == 0 {
            return Err(ExtractError::Missing(folder.to_owned()));
        }
        log(&format!("{folder} ({n} files)"));
    }

    // Same path in two stages: keep the first copy (they are identical on the discs checked;
    // a difference is logged). Each stage archive is read once for every prefix.
    let mut seen: Vec<std::collections::HashMap<String, usize>> =
        vec![Default::default(); SCENE_FOLDERS.len()];
    let mut scenes = disc.paths_under("data/scene/");
    scenes.retain(|p| p.ends_with(".szs"));
    log(&format!("reading {} stage archives", scenes.len()));
    for scene in &scenes {
        let raw = disc.read_file(scene)?;
        let bytes = if yaz0::is_yaz0(&raw) {
            yaz0::decompress(&raw)?
        } else {
            raw
        };
        let Ok(arc) = rarc::Archive::parse(bytes) else {
            continue;
        };
        for (rel, data) in arc.under("") {
            let Some(i) = SCENE_FOLDERS.iter().position(|(p, _)| rel.starts_with(p)) else {
                continue;
            };
            if let Some(&len) = seen[i].get(rel) {
                if len != data.len() {
                    log(&format!(
                        "{scene}:{rel} differs from an earlier copy; kept the first"
                    ));
                }
                continue;
            }
            let path = safe_join(&out.join(SCENE_FOLDERS[i].1), rel)?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, data)?;
            seen[i].insert(rel.to_owned(), data.len());
        }
    }
    for ((prefix, dest), seen) in SCENE_FOLDERS.iter().zip(&seen) {
        if seen.is_empty() {
            return Err(ExtractError::Missing(format!("data/scene/*.szs:{prefix}*")));
        }
        log(&format!("{prefix}* -> {dest}/ ({} files)", seen.len()));
    }

    if !is_complete(out) {
        return Err(ExtractError::Missing(
            "expected files after extraction".into(),
        ));
    }
    let report = Report {
        game_id: disc.game_id.clone(),
        prm_files: prm_count(out),
        clips: count("bck", "bck"),
        out: out.to_path_buf(),
    };
    std::fs::write(
        out.join("SOURCE.txt"),
        format!(
            "extracted from: {}\ngame id: {}\nfiles: {} prm, {} bck, mario.szs, AudioRes/\nby: libsms::extract\n",
            iso.file_name().and_then(|s| s.to_str()).unwrap_or("?"),
            report.game_id,
            report.prm_files,
            report.clips
        ),
    )?;
    log(&format!(
        "done: {} .prm files, {} clips",
        report.prm_files, report.clips
    ));
    Ok(report)
}

/// The Game ID of a plain or CISO image, without reading its file system.
fn game_id_of(path: &Path) -> Option<String> {
    gcm::open_image(path).ok().and_then(|r| {
        let mut id = [0u8; 6];
        r.read_at(0, &mut id).ok()?;
        Some(String::from_utf8_lossy(&id).into_owned())
    })
}

/// Looks for the user's Super Mario Sunshine disc: `$SMS_DISC`, then `.iso`/`.gcm`/`.ciso`
/// files in `dirs` (names containing "sunshine" first), keeping the first whose Game ID is
/// Sunshine's.
pub fn find_disc(dirs: &[PathBuf]) -> Option<PathBuf> {
    let is_sms = |p: &Path| game_id_of(p).is_some_and(|id| GAME_IDS.iter().any(|g| g.0 == id));
    if let Some(p) = std::env::var_os("SMS_DISC").map(PathBuf::from)
        && is_sms(&p)
    {
        return Some(p);
    }
    let mut cands: Vec<PathBuf> = Vec::new();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let ext = p
                .extension()
                .and_then(|s| s.to_str())
                .map(str::to_ascii_lowercase);
            if matches!(ext.as_deref(), Some("iso" | "gcm" | "ciso")) {
                cands.push(p);
            }
        }
    }
    let named = |p: &PathBuf| {
        let n = p.to_string_lossy().to_ascii_lowercase();
        !(n.contains("sunshine") || n.contains("gms"))
    };
    cands.sort_by_key(named);
    cands.into_iter().find(|p| is_sms(p))
}

/// All files under `dir`, as `/`-separated paths relative to it.
fn walk(dir: &Path) -> Vec<String> {
    fn go(base: &Path, dir: &Path, out: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                go(base, &p, out);
            } else if let Ok(rel) = p.strip_prefix(base) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut v = Vec::new();
    go(dir, dir, &mut v);
    v.sort();
    v
}

#[cfg(test)]
mod tests {
    use libcommon::disc::{image_from, synthetic::disc_with};
    use super::*;

    /// Yaz0 "compression" that stores everything as literals (valid, just not small).
    fn yaz0_literal(data: &[u8]) -> Vec<u8> {
        let mut v = b"Yaz0".to_vec();
        v.extend_from_slice(&(data.len() as u32).to_be_bytes());
        v.extend_from_slice(&[0; 8]);
        for chunk in data.chunks(8) {
            v.push(0xFFu8 << (8 - chunk.len()));
            v.extend_from_slice(chunk);
        }
        v
    }

    #[test]
    fn wrong_game_is_refused() {
        let img = disc_with(b"GALE01", &[("opening.bnr", b"x")]);
        let dir = std::env::temp_dir().join("libsms_wrong_game.iso");
        std::fs::write(&dir, img).unwrap();
        let err = extract_assets(
            &dir,
            &std::env::temp_dir().join("libsms_wrong_game_out"),
            &mut |_| {},
        )
        .unwrap_err();
        assert!(
            matches!(err, ExtractError::WrongGame(ref id) if id == "GALE01"),
            "{err}"
        );
        let _ = std::fs::remove_file(dir);
    }

    #[test]
    fn disc_to_archive_round_trip() {
        // A disc whose data/mario.szs is a Yaz0-wrapped RARC: read it back through all layers.
        let szs = yaz0_literal(&rarc::tests::sample());
        let img = disc_with(b"GMSP01", &[("data/mario.szs", &szs)]);
        let len = img.len() as u64;
        let disc = Disc::from_reader(image_from(std::io::Cursor::new(img), len).unwrap()).unwrap();
        let raw = disc.read_file("data/mario.szs").unwrap();
        let arc = rarc::Archive::parse(yaz0::decompress(&raw).unwrap()).unwrap();
        assert_eq!(arc.file("sub/b.bin").unwrap(), [1, 2, 3]);
    }

    #[test]
    fn path_traversal_is_rejected() {
        assert!(safe_join(Path::new("out"), "../evil").is_err());
        assert!(safe_join(Path::new("out"), "/abs").is_err());
        assert!(safe_join(Path::new("out"), "watergun/nozzle.prm").is_ok());
    }
}
