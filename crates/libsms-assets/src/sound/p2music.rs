//! Pikmin 2 DirectedBgm child groups: PSM::BgmTrackMapFile::read and
//! DirectorMgr_Scene::newDirector. Stage selection and activity signals belong to hosts.
use std::{collections::HashMap, path::Path};
pub const SONGS: &[&str] = &["n_tutorial.bms", "forest.bms", "yakushima.bms", "last.bms"];
#[derive(Debug, Clone)]
pub struct TrackMap {
    pub groups: [u8; 6],
    pub pikmin: [bool; 16],
}
impl TrackMap {
    pub fn levels(&self, flags: u32) -> [f32; 16] {
        // Host context: enemy-near, battle, working, treasure, Pikmin present.
        let on = [
            true,
            flags & 4 != 0,
            flags & 8 != 0,
            flags & 1 != 0,
            flags & 2 != 0,
            true,
        ];
        let mut levels = [0.0; 16];
        let mut track = 0;
        for (g, &count) in self.groups.iter().enumerate() {
            for _ in 0..count {
                if track < 16 {
                    levels[track] = if on[g] { 1.0 } else { 0.0 };
                }
                track += 1;
            }
        }
        for i in 0..16 {
            if self.pikmin[i] && flags & 16 == 0 {
                levels[i] = 0.0;
            }
        }
        levels
    }
}
pub fn load(dir: &Path) -> Result<HashMap<String, TrackMap>, String> {
    let mut maps = HashMap::new();
    for file in ["trackMap_Seq_W.txt", "trackMap_Seq_T.txt"] {
        let bytes = std::fs::read(dir.join("music").join(file)).map_err(|e| e.to_string())?;
        // Tokens used here are ASCII; Shift-JIS comments are stripped before tokenizing.
        let text = String::from_utf8_lossy(&bytes);
        let clean = text
            .lines()
            .map(|l| l.split('#').next().unwrap_or(""))
            .collect::<Vec<_>>()
            .join(" ")
            .replace(['{', '}'], " ");
        let mut words = clean.split_whitespace();
        while let Some(name) = words.next() {
            if name == "endoffile" {
                break;
            }
            let mut values = [0u8; 30];
            for value in &mut values {
                *value = words
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or_else(|| format!("bad music map {name}"))?;
            }
            let groups = values[..6].try_into().unwrap();
            if values[..6].iter().map(|&v| v as u32).sum::<u32>() > 16 {
                return Err(format!("too many music tracks: {name}"));
            }
            maps.insert(
                name.to_owned(),
                TrackMap {
                    groups,
                    pikmin: std::array::from_fn(|i| values[6 + i] != 0),
                },
            );
        }
    }
    Ok(maps)
}
