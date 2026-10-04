//! Animation clock. Much of Mario's state machine advances on animation events
//! (`isLast1AnimeFrame`, `checkPass`), so the sim has to run the clips' timing even though it
//! draws nothing. Clips are named by their BCK stem (`mario.szs:bck/ma_<stem>.bck`).
//!
//! `TMario::setAnimation` sets the frame controller rate to `rate * 0.5` per 120 Hz tick, so
//! rate 1.0 plays the clip's 60 fps authoring speed in real time.

use std::collections::HashMap;

/// Clip name (the part of `ma_<stem>.bck` after `ma_`).
pub type ClipName = &'static str;

/// Clips the locomotion and FLUDD code selects (names from MarioAnimeFiles.inc).
pub mod clip {
    use super::ClipName;
    pub const WAIT: ClipName = "wait";
    pub const PIVOT: ClipName = "pivot";
    pub const RUN1: ClipName = "run1";
    pub const RUN2: ClipName = "run2";
    pub const SSTEP: ClipName = "sstep";
    pub const TURBO_DASH: ClipName = "turbo_dash";
    pub const TURN: ClipName = "turn";
    pub const TRNED: ClipName = "trned";
    pub const BRAKE: ClipName = "brake";
    pub const BREND: ClipName = "brend";
    pub const PUSH: ClipName = "push";
    pub const JUMP: ClipName = "jump";
    pub const JMPED: ClipName = "jmped";
    pub const JUMP2_UP: ClipName = "2jmp1";
    pub const JUMP2_DOWN: ClipName = "2jmp2";
    pub const JUMP2_END: ClipName = "2jmed";
    pub const ROLL: ClipName = "roll";
    pub const LAND: ClipName = "land";
    pub const LAND_END: ClipName = "laend";
    pub const UTURN_JUMP: ClipName = "tjmp1";
    pub const UTURN_END: ClipName = "tjmp2";
    pub const WALL_JUMP: ClipName = "wjmp";
    pub const WALL_SLIDE: ClipName = "wsld";
    pub const SPIN: ClipName = "spin_p";
    pub const SQUAT: ClipName = "sqwat";
    pub const SQUAT_END: ClipName = "sqend";
    pub const SLIP: ClipName = "slip";
    pub const SLIP_END: ClipName = "slped";
    pub const HIP_START: ClipName = "hipsr";
    pub const HIP_FALL: ClipName = "hipat";
    pub const HIP_END: ClipName = "hiped";
    pub const DIVE: ClipName = "sldct";
    pub const BACK_DOWN: ClipName = "bkdwn";
    pub const FORE_DOWN: ClipName = "jfdwn";
    pub const SLIDE_FORE: ClipName = "sdwnf";
}

/// BCK loop modes (`J3DFrameCtrl` attribute).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopMode {
    Once,
    OnceReset,
    Loop,
    MirrorOnce,
    MirrorLoop,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipInfo {
    /// Length in frames (60 fps authoring rate).
    pub frames: f32,
    pub mode: LoopMode,
}

/// Clip lengths. Falls back to a generic one-second clip for names that are not loaded, so the
/// sim still runs without the user's disc (timings will differ from the game).
#[derive(Debug, Clone, Default)]
pub struct AnimTable {
    clips: HashMap<String, ClipInfo>,
}

impl AnimTable {
    pub const FALLBACK: ClipInfo = ClipInfo {
        frames: 30.0,
        mode: LoopMode::Loop,
    };

    pub fn insert(&mut self, name: &str, info: ClipInfo) {
        self.clips.insert(name.to_owned(), info);
    }

    pub fn len(&self) -> usize {
        self.clips.len()
    }

    pub fn is_empty(&self) -> bool {
        self.clips.is_empty()
    }

    pub fn get(&self, name: &str) -> ClipInfo {
        self.clips.get(name).copied().unwrap_or(Self::FALLBACK)
    }

    /// Reads the length and loop mode out of a BCK file (`J3D1bck1`, first chunk `ANK1`).
    pub fn parse_bck(data: &[u8]) -> Option<ClipInfo> {
        if data.get(0..8)? != b"J3D1bck1" || data.get(0x20..0x24)? != b"ANK1" {
            return None;
        }
        let mode = match data.get(0x28)? {
            0 => LoopMode::Once,
            1 => LoopMode::OnceReset,
            2 => LoopMode::Loop,
            3 => LoopMode::MirrorOnce,
            4 => LoopMode::MirrorLoop,
            _ => return None,
        };
        let frames = u16::from_be_bytes([*data.get(0x2a)?, *data.get(0x2b)?]) as f32;
        Some(ClipInfo { frames, mode })
    }

    /// Loads every `ma_*.bck` in an extracted `mario.szs:bck/` folder.
    pub fn from_dir(dir: &std::path::Path) -> AnimTable {
        let mut t = AnimTable::default();
        let Ok(rd) = std::fs::read_dir(dir) else {
            return t;
        };
        for e in rd.flatten() {
            let path = e.path();
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let Some(name) = stem.strip_prefix("ma_") else {
                continue;
            };
            if path.extension().and_then(|s| s.to_str()) != Some("bck") {
                continue;
            }
            if let Some(info) = std::fs::read(&path).ok().and_then(|b| Self::parse_bck(&b)) {
                t.insert(name, info);
            }
        }
        t
    }
}

/// The playing clip.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimState {
    pub clip: ClipName,
    /// Current frame in `[0, frames]`.
    pub frame: f32,
    /// `setAnimation`'s `rate` argument (1.0 = authored speed).
    pub rate: f32,
    info: ClipInfo,
    /// Looped or completed at least once (`isAnimeLoopOrStop`).
    pub finished_once: bool,
}

impl AnimState {
    pub fn new(clip: ClipName, table: &AnimTable) -> AnimState {
        AnimState {
            clip,
            frame: 0.0,
            rate: 1.0,
            info: table.get(clip),
            finished_once: false,
        }
    }

    /// `TMario::setAnimation(clip, rate)`: restarts only if the clip changed.
    pub fn set(&mut self, clip: ClipName, rate: f32, table: &AnimTable) {
        if clip != self.clip {
            *self = AnimState::new(clip, table);
        }
        self.rate = rate;
    }

    /// One 120 Hz tick of playback.
    pub fn advance(&mut self) {
        let len = self.info.frames;
        let mut f = self.frame + self.rate * 0.5;
        match self.info.mode {
            LoopMode::Loop | LoopMode::MirrorLoop => {
                if len > 0.0 {
                    if f >= len {
                        self.finished_once = true;
                    }
                    f = f.rem_euclid(len);
                }
            }
            _ => {
                if f >= len {
                    f = len;
                    self.finished_once = true;
                }
                if f < 0.0 {
                    f = 0.0;
                }
            }
        }
        self.frame = f;
    }

    /// `isLast1AnimeFrame`.
    pub fn at_last_frame(&self) -> bool {
        self.frame >= self.info.frames - 1.0
    }

    /// `J3DFrameCtrl::checkPass(frame)`: did the last advance cross `mark`?
    pub fn passed(&self, mark: f32) -> bool {
        let step = self.rate * 0.5;
        self.frame >= mark && self.frame - step < mark
    }

    pub fn length(&self) -> f32 {
        self.info.frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> AnimTable {
        let mut t = AnimTable::default();
        t.insert(
            "once",
            ClipInfo {
                frames: 10.0,
                mode: LoopMode::Once,
            },
        );
        t.insert(
            "loop",
            ClipInfo {
                frames: 10.0,
                mode: LoopMode::Loop,
            },
        );
        t
    }

    #[test]
    fn once_clip_stops_and_reports_last_frame() {
        let t = table();
        let mut a = AnimState::new("once", &t);
        for _ in 0..30 {
            a.advance();
        }
        // rate 1.0 = 0.5 frames per tick, so 30 ticks = 15 frames > 10
        assert_eq!(a.frame, 10.0);
        assert!(a.at_last_frame());
        assert!(a.finished_once);
    }

    #[test]
    fn loop_clip_wraps() {
        let t = table();
        let mut a = AnimState::new("loop", &t);
        for _ in 0..30 {
            a.advance();
        }
        assert!((a.frame - 5.0).abs() < 1e-4, "{}", a.frame);
        assert!(a.finished_once);
    }

    #[test]
    fn set_same_clip_keeps_progress() {
        let t = table();
        let mut a = AnimState::new("once", &t);
        a.advance();
        a.set("once", 2.0, &t);
        assert_eq!(a.frame, 0.5);
        assert_eq!(a.rate, 2.0);
        a.set("loop", 1.0, &t);
        assert_eq!(a.frame, 0.0);
    }

    #[test]
    fn parses_bck_header() {
        let mut d = vec![0u8; 0x30];
        d[..8].copy_from_slice(b"J3D1bck1");
        d[0x20..0x24].copy_from_slice(b"ANK1");
        d[0x28] = 2;
        d[0x2a..0x2c].copy_from_slice(&180u16.to_be_bytes());
        let info = AnimTable::parse_bck(&d).unwrap();
        assert_eq!(info.frames, 180.0);
        assert_eq!(info.mode, LoopMode::Loop);
        assert!(AnimTable::parse_bck(&d[..0x10]).is_none());
    }
}
