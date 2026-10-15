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
    /// Holding the Shine Sprite over his head (`ANIM_DEMO_SHINE_GET`).
    pub const DEMO_SHINE_GET: ClipName = "demo_shine_get";
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
        self.clips
            .get(name)
            .copied()
            .or_else(|| crate::fludd_anim::default_info(name))
            .unwrap_or(Self::FALLBACK)
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

    /// The frame `ticks` 120 Hz ticks of playback before this one, for hosts that draw a little
    /// behind the sim (fractional ticks give in-between poses). Loops wrap; other clips stop at
    /// their first frame, and one that has run to its end stays there.
    pub fn frame_before(&self, ticks: f32) -> f32 {
        let len = self.info.frames;
        let ticks = if ticks.is_finite() {
            ticks.max(0.0)
        } else {
            0.0
        };
        let f = self.frame - self.rate * 0.5 * ticks;
        match self.info.mode {
            LoopMode::Loop | LoopMode::MirrorLoop if len > 0.0 => f.rem_euclid(len),
            _ if self.finished_once && self.frame >= len => len,
            _ => f.clamp(0.0, len.max(0.0)),
        }
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

/// The upper-body clip Mario pumps FLUDD with over each body clip (`TMarioAnimeData::unk2`,
/// `gMarioAnimeData` in Player/MarioDraw.cpp). `None` is the table's 200: no pumping in that
/// clip (`isPumpOK`).
pub fn pump_clip(body: &str) -> Option<ClipName> {
    Some(match body {
        "hang" | "rope_hgwat" | "fence_move_down" => "hgpmp",
        "fence_wait" | "fence_move_l" | "fence_move_r" | "fence_move_up" => "fepmp",
        "float" | "paddle_end" | "paddle_start" | "paddle_to_wait" | "swim" | "swim_damage"
        | "swim_down" | "swim_start" | "swim_to_wait" | "swim_wait" | "wait_to_paddle"
        | "wait_to_swim" => "swpmp",
        "2jmed" | "2jmp1" | "2jmp2" | "brake" | "brend" | "broad_jump" | "damage_wait"
        | "damage_wait_start" | "dive_land" | "dive_wait" | "fepmp" | "giant_rolling" | "hipat"
        | "hipsr" | "hot_wait" | "jmped" | "jump" | "jump_rolling" | "laend" | "land" | "pivot"
        | "ride_shell_wait" | "roll" | "roll_jump" | "rope_run" | "rope_swait" | "rope_wait"
        | "rope_walk" | "rope_wtosw" | "rope_wtosw_r" | "run1" | "run2" | "sink_down"
        | "sinking" | "spin_p" | "sqend" | "sqsta" | "sqwat" | "sstep" | "step1" | "step2"
        | "step3" | "swait" | "swlkl" | "swlkr" | "swpmp" | "t_wait" | "tjmp1" | "tjmp2"
        | "trned" | "turbo_dash" | "turn" | "wait" | "walk" | "wjmp" => "pump",
        _ => return None,
    })
}

/// A frame controller driven by explicit rates (`J3DFrameCtrl`, J3DAnimation.cpp): the upper
/// body layer and FLUDD's clips, whose rates the game sets directly instead of through
/// `setAnimation`'s `rate * 0.5`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameCtrl {
    pub mode: LoopMode,
    pub end: f32,
    pub frame: f32,
    pub rate: f32,
    /// `STATE_COMPLETED_ONCE | STATE_LOOPED_ONCE` from the last update.
    pub done: bool,
}

impl FrameCtrl {
    /// `J3DFrameCtrl::init(end)` with the clip's own attribute: frame 0, rate 1.
    pub fn new(info: ClipInfo) -> FrameCtrl {
        FrameCtrl {
            mode: info.mode,
            end: info.frames,
            frame: 0.0,
            rate: 1.0,
            done: false,
        }
    }

    /// `J3DFrameCtrl::update`.
    pub fn update(&mut self) {
        self.done = false;
        self.frame += self.rate;
        let (start, end) = (0.0, self.end);
        match self.mode {
            LoopMode::Once | LoopMode::OnceReset => {
                let reset = self.mode == LoopMode::OnceReset;
                if self.frame < start {
                    self.frame = start;
                    self.rate = 0.0;
                    self.done = true;
                }
                if self.frame >= end {
                    self.frame = if reset { start } else { end - 0.001 };
                    self.rate = 0.0;
                    self.done = true;
                }
            }
            LoopMode::Loop => {
                if end > start {
                    while self.frame < start {
                        self.done = true;
                        self.frame += end - start;
                    }
                    while self.frame >= end {
                        self.done = true;
                        self.frame -= end - start;
                    }
                }
            }
            LoopMode::MirrorOnce => {
                if self.frame >= end {
                    self.frame = end - 0.001;
                    self.rate = -self.rate;
                }
                if self.frame < start {
                    self.frame = start;
                    self.rate = 0.0;
                    self.done = true;
                }
            }
            LoopMode::MirrorLoop => {
                if self.frame >= end {
                    self.frame = end - 0.001;
                    self.rate = -self.rate;
                }
                if self.frame < start {
                    self.frame = start;
                    self.rate = -self.rate;
                    self.done = true;
                }
            }
        }
    }

    /// `J3DFrameCtrl::checkPass` for the next update (once and ping-pong attributes).
    pub fn will_pass(&self, mark: f32) -> bool {
        let cur = self.frame;
        let next = (self.frame + self.rate).clamp(0.0, (self.end - 0.001).max(0.0));
        if cur <= next {
            cur <= mark && mark < next
        } else {
            next <= mark && mark < cur
        }
    }

    /// The "finished" test the nozzles use: completed or looped, or within 0.1 of the end.
    pub fn finished(&self) -> bool {
        self.done || self.frame > self.end - 0.1
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
    fn frame_before_steps_back_within_the_clip() {
        let t = table();
        let mut a = AnimState::new("loop", &t);
        a.advance(); // 0.5
        assert_eq!(a.frame_before(0.5), 0.25);
        // Loops wrap back past the start.
        assert_eq!(a.frame_before(3.0), 9.0);
        let mut o = AnimState::new("once", &t);
        o.advance();
        assert_eq!(o.frame_before(3.0), 0.0);
        // A finished one-shot holds its last frame.
        for _ in 0..40 {
            o.advance();
        }
        assert_eq!(o.frame_before(2.0), 10.0);
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
    fn ping_pong_loop_bounces_and_once_holds() {
        let mut c = FrameCtrl::new(ClipInfo {
            frames: 10.0,
            mode: LoopMode::MirrorLoop,
        });
        c.rate = 4.0;
        for _ in 0..3 {
            c.update();
        }
        assert!(c.rate < 0.0 && c.frame < 10.0, "{c:?}");
        for _ in 0..3 {
            c.update();
        }
        assert!(c.rate > 0.0 && c.frame == 0.0 && c.done, "{c:?}");
        let mut o = FrameCtrl::new(ClipInfo {
            frames: 5.0,
            mode: LoopMode::Once,
        });
        for _ in 0..10 {
            o.update();
        }
        assert!(o.finished() && o.rate == 0.0);
    }

    #[test]
    fn pump_clips_follow_the_anime_table() {
        assert_eq!(pump_clip("wait"), Some("pump"));
        assert_eq!(pump_clip("hang"), Some("hgpmp"));
        assert_eq!(pump_clip("swim"), Some("swpmp"));
        assert_eq!(pump_clip("sldct"), None);
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
