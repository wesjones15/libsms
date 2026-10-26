//! Pre-render the exact ModelGate TEV on the asset worker, never the gameplay thread.
//! Cache bytes are derivatives of the user's disc, kept only in the ignored asset folder.
use crate::{plaza, portal::Portal, tev::Baked};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
};

pub const SIZE: u32 = 256;
pub const FPS: f32 = 15.;
pub const EXIT_FPS: f32 = 30.;
pub const REVEAL: f32 = 0.8;
// Host presentation: a gentler exit-only fall; BCK spin still advances at 60 fps.
pub const HOP: f32 = 0.8;
pub const LAND: f32 = 0.15;
pub const COMPLETE: f32 = REVEAL + HOP + LAND;
pub const CLOSE: f32 = COMPLETE + 0.14;
const MAGIC: &[u8; 8] = b"SMSPOD04";
const PIXELS: usize = (SIZE * SIZE * 4) as usize;
const HEADER: usize = 20;

pub fn activation(time: f32) -> f32 {
    if time <= COMPLETE {
        (time / 0.18).clamp(0., 1.)
    } else {
        (1. - (time - COMPLETE) / (CLOSE - COMPLETE)).clamp(0., 1.)
    }
}

fn cache_path(dir: &Path) -> Result<PathBuf, String> {
    let mut hash = 0xcbf29ce484222325u64;
    let mut add = |bytes: &[u8]| {
        for &b in bytes {
            hash = (hash ^ b as u64).wrapping_mul(0x100000001b3);
        }
    };
    add(MAGIC);
    for value in [SIZE as f32, FPS, EXIT_FPS, REVEAL, HOP, LAND, CLOSE] {
        add(&value.to_bits().to_le_bytes());
    }
    for (_, _, path) in libsms::extract::OPTIONAL_EXTRACTS
        .iter()
        .filter(|(_, _, p)| p.starts_with("portal/"))
    {
        add(path.as_bytes());
        add(&fs::read(dir.join(path)).map_err(|e| format!("{path}: {e}"))?);
    }
    Ok(dir
        .join("portal/cache")
        .join(format!("entrance-v4-{hash:016x}.bin")))
}

pub fn complete(dir: &Path) -> bool {
    cache_path(dir)
        .ok()
        .and_then(|p| fs::File::open(p).ok())
        .is_some_and(|mut f| read_header(&mut f).is_ok())
}

fn read_header(f: &mut fs::File) -> Result<(usize, usize), String> {
    let mut h = [0; HEADER];
    f.read_exact(&mut h).map_err(|e| e.to_string())?;
    let n = |at| u32::from_le_bytes(h[at..at + 4].try_into().unwrap()) as usize;
    let (idle, exit) = (n(12), n(16));
    if &h[..8] != MAGIC
        || n(8) != SIZE as usize
        || !(1..=1000).contains(&idle)
        || !(1..=120).contains(&exit)
        || f.metadata().map_err(|e| e.to_string())?.len()
            != (HEADER + (idle + exit) * PIXELS) as u64
    {
        return Err("invalid portal animation cache".into());
    }
    Ok((idle, exit))
}

/// First-run worker entry point. Stable cache identity includes every source file and timing.
pub fn prepare(dir: &Path) -> Result<PathBuf, String> {
    let path = cache_path(dir)?;
    if complete(dir) {
        return Ok(path);
    }
    let mut portal = Portal::load(dir).map_err(|e| e.to_string())?;
    portal.set_interior(&plaza::plaza_still(dir, (SIZE * 2) as usize).map_err(|e| e.to_string())?);
    let idle = (portal.loop_frames() / 60. * FPS).ceil().max(1.) as u32;
    let exit = (CLOSE * EXIT_FPS).ceil() as u32 + 1;
    fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let temp = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut f = fs::File::create(&temp).map_err(|e| e.to_string())?;
    f.write_all(MAGIC)
        .and_then(|_| f.write_all(&SIZE.to_le_bytes()))
        .and_then(|_| f.write_all(&idle.to_le_bytes()))
        .and_then(|_| f.write_all(&exit.to_le_bytes()))
        .map_err(|e| e.to_string())?;
    for i in 0..idle + exit {
        let (time, a) = if i < idle {
            (i as f32 / FPS, 0.)
        } else {
            let t = (i - idle) as f32 / EXIT_FPS;
            (t, activation(t))
        };
        f.write_all(&smooth_frame(&mut portal, time * 60., a)?)
            .map_err(|e| e.to_string())?;
    }
    f.sync_all().map_err(|e| e.to_string())?;
    drop(f);
    fs::rename(&temp, &path).map_err(|e| e.to_string())?;
    Ok(path)
}

// Rasterize the original TEV at 2x, then retain coverage from its alpha-tested edge.
// Store straight-alpha RGB: transparent subpixels must not darken the dark M border
// a second time when the host blends the fractional-coverage output pixel.
fn smooth_frame(portal: &mut Portal, frame: f32, activation: f32) -> Result<Vec<u8>, String> {
    let high = portal.image_sized(frame, activation, SIZE * 2);
    if high.width != SIZE * 2 || high.height != SIZE * 2 {
        return Err("portal supersampling requires a 512-square interior".into());
    }
    let mut rgba = vec![0; PIXELS];
    for y in 0..SIZE as usize {
        for x in 0..SIZE as usize {
            let mut sum = [0u32; 3];
            let mut covered = 0;
            for dy in 0..2 {
                for dx in 0..2 {
                    let at = ((y * 2 + dy) * high.width as usize + x * 2 + dx) * 4;
                    if high.rgba[at + 3] == 0 {
                        continue;
                    }
                    covered += 1;
                    for c in 0..3 {
                        sum[c] += high.rgba[at + c] as u32;
                    }
                }
            }
            let at = (y * SIZE as usize + x) * 4;
            if covered != 0 {
                for c in 0..3 {
                    rgba[at + c] = ((sum[c] + covered / 2) / covered) as u8;
                }
                rgba[at + 3] = ((covered * 255 + 2) / 4) as u8;
            }
        }
    }
    Ok(rgba)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cached_gate_has_smooth_coverage_but_an_opaque_interior() {
        let Some(dir) = libsms::assets::find(None) else {
            eprintln!("SKIP: Sunshine assets absent");
            return;
        };
        if !dir.join("portal/05_gate01.bmd").is_file() {
            eprintln!("SKIP: M gate absent");
            return;
        }
        let mut portal = Portal::load(&dir).unwrap();
        portal.set_interior(&plaza::plaza_still(&dir, (SIZE * 2) as usize).unwrap());
        for activation in [0., 1.] {
            let image = smooth_frame(&mut portal, 0., activation).unwrap();
            assert_eq!(image.len(), PIXELS);
            assert!(
                image.chunks_exact(4).any(|p| p[3] > 0 && p[3] < 255),
                "no antialiased edge coverage"
            );
            assert!(
                image.chunks_exact(4).any(|p| p[3] == 255),
                "M interior became translucent"
            );
            assert!(
                image.chunks_exact(4).any(|p| p[3] == 0),
                "outside M became an opaque square"
            );
        }
    }
}

pub struct Frames {
    bytes: Vec<u8>,
    idle: usize,
    exit: usize,
}
impl Frames {
    pub fn load(dir: &Path) -> Result<Arc<Self>, String> {
        static SHARED: OnceLock<Mutex<HashMap<PathBuf, Weak<Frames>>>> = OnceLock::new();
        let key = cache_path(dir)?;
        let cache = SHARED.get_or_init(Default::default);
        if let Some(old) = cache.lock().unwrap().get(&key).and_then(Weak::upgrade) {
            return Ok(old);
        }
        let mut file = fs::File::open(&key).map_err(|_| {
            "portal animation cache is not prepared; run the asset worker".to_string()
        })?;
        let (idle, exit) = read_header(&mut file)?;
        let mut bytes = Vec::with_capacity((idle + exit) * PIXELS);
        file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        let made = Arc::new(Self { bytes, idle, exit });
        cache.lock().unwrap().insert(key, Arc::downgrade(&made));
        Ok(made)
    }
    pub fn index(&self, time: f32, activated: bool) -> usize {
        if activated && time < CLOSE {
            self.idle + ((time * EXIT_FPS) as usize).min(self.exit - 1)
        } else {
            (time * FPS) as usize % self.idle
        }
    }
    pub fn pixels(&self, index: usize) -> &[u8] {
        &self.bytes[index * PIXELS..(index + 1) * PIXELS]
    }
    pub fn image(&self) -> Baked {
        Baked {
            width: SIZE,
            height: SIZE,
            wrap: [1, 1],
            rgba: self.pixels(0).to_vec(),
            env: None,
        }
    }
}
