//! The game's side of JAudio: JAInterface (`JAIBasic`, `JAISound`, JAIGFrameSe.cpp,
//! JAIGFrameSequence.cpp, JAIEntrySe.cpp) and the parts of Sunshine's `MSound` layer it needs.
//!
//! - Sound effects all play through one sequence, `se.scom` (`JAIBasic::startSeSequence`). Its
//!   tracks are the SE "channels": each scene gives every SE category a number of tracks
//!   (`JAICategoryInfo::mMaxPlaying`). Each game frame the registered sounds of a category are
//!   ranked by priority and distance and the best get its tracks (`checkNextFrameSe`); a newly
//!   placed sound is started by writing its index to ports of its track (`sendPlayingSeCommand`)
//!   and its volume, pan and pitch go to the track's outer parameters.
//! - Music is a sequence in its own root (`MSD_BGM_*`), its volume the product of the sound's
//!   volume parameters; stopping with a fade glides one of them to 0.
//! - The title stream (`MSD_STR_*`) is decoded on the CPU and mixed after the DSP.
//! - Positions are turned into distance volume, pan and doppler through the camera's view matrix
//!   (`setDistanceVolumeCommon`, `setDistancePanCommon`, JAISound.cpp).

use std::sync::Arc;

use glam::{Mat4, Vec3};
use libsms_assets::sound::aaf::id;
use libsms_assets::sound::{SoundData, SoundInfo};

use crate::calc::{DAC_RATE, RandomFast, SUBFRAME};
use crate::engine::{Engine, SyncHandler, update};

/// `JAISoundState`
mod state {
    pub const INACTIVE: u8 = 0;
    pub const STORED: u8 = 1;
    pub const PREPARED: u8 = 2;
    pub const STARTED: u8 = 3;
    pub const PLAYING: u8 = 4;
    pub const STOPPING: u8 = 5;
}

/// `JAISeSwBit`
mod sw {
    pub const NO_DISTANCE_VOLUME: u32 = 0x2;
    pub const NO_DISTANCE_FXMIX: u32 = 0x4;
    pub const RANDOM_PITCH_DOWN: u32 = 0x10;
    pub const DISTANCE_LIMIT: u32 = 0x20;
    pub const RANDOM_PITCH_WIDTH: u32 = 0xC0;
    pub const DOPPLER: u32 = 0x300;
    pub const MULTI: u32 = 0x80000;
    pub const MONO: u32 = 0x4000;
}

/// `JAIGlobalParameter`s as `MSound::MSound` leaves them.
mod global {
    /// The largest `smSeCategory` distance (`MSHandle.cpp`).
    pub const DISTANCE_MAX: f32 = 12000.0;
    pub const MAX_VOLUME_DISTANCE: f32 = 1200.0;
    pub const MIN_DISTANCE_VOLUME: f32 = 0.0;
    pub const DISTANCE_MOVE_TIME: u32 = 3;
    pub const PAN_DISTANCE_MAX: f32 = 500.0;
    pub const PAN_DISTANCE2_MAX: f32 = 1000.0;
    pub const PAN_ANGLE: f32 = 12.0;
    pub const PAN_ANGLE2: f32 = 2.4;
    pub const DOPPLER: f32 = 3200.0;
    pub const DOPPLER_MOVE_TIME: u32 = 15;
    pub const SE_DISTANCE_FX: f32 = 150.0;
    pub const SE_DISTANCE_PITCH_MAX: f32 = 0.2;
    pub const SE_DISTANCE_WAIT_MAX: u32 = 12;
    pub const SE_REGIST_MAX: usize = 20;
    pub const SEQ_PLAY_TRACKS: usize = 4;
}

/// `MSHandle::smSeCategory[c].unk8`: per-category SE volume (`setSeCategoryVolume`).
const CATEGORY_VOLUME: [f32; 16] = [
    0.76, 1.0, 1.0, 0.81, 0.84, 0.59, 0.9, 1.0, 0.76, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0,
];

/// `JAIMoveParaSet`
#[derive(Debug, Clone, Copy)]
struct MovePara {
    current: f32,
    target: f32,
    counter: u32,
    amount: f32,
}

impl MovePara {
    fn new(v: f32) -> Self {
        MovePara {
            current: v,
            target: v,
            counter: 0,
            amount: 0.0,
        }
    }

    /// `JAISound::setSeInterMovePara`
    fn se_move(&mut self, target: f32, time: u32) {
        self.target = target;
        if self.current != self.target {
            if time == 0 {
                self.current = self.target;
            } else {
                self.counter = time;
                self.amount = (self.target - self.current) / time as f32;
            }
        }
    }

    /// `JAIData::setSeMovePara`
    fn se_step(&mut self) {
        if self.counter == 0 {
            return;
        }
        self.counter -= 1;
        if self.counter == 0 {
            self.current = self.target;
        } else {
            self.current += self.amount;
        }
    }

    /// `JAISound::initMoveParameter`: false when nothing changes.
    fn seq_move(&mut self, target: f32, time: u32) -> bool {
        if self.counter == 0 && self.current == target {
            return false;
        }
        if self.counter != 0 && self.target == target {
            return false;
        }
        self.target = target;
        if time == 0 {
            self.current = target;
            return true;
        }
        self.amount = (self.current - self.target) / time as f32;
        self.counter = time;
        true
    }

    /// `JAIData::moveParameter`
    fn seq_step(&mut self) {
        if self.counter == 0 {
            return;
        }
        self.counter -= 1;
        if self.counter != 0 {
            self.current -= self.amount;
        } else {
            self.current = self.target;
        }
    }
}

/// `JAISeParameter`: eight layered values per parameter (index 7 = override when not -1).
#[derive(Debug, Clone)]
struct SeParam {
    volume: [MovePara; 8],
    pan: [MovePara; 8],
    pitch: [MovePara; 8],
    fxmix: [MovePara; 8],
    dolby: [MovePara; 8],
    port_data: [u16; 16],
    port_update: u16,
}

impl Default for SeParam {
    /// `JAIData::initSePara`
    fn default() -> Self {
        let mut p = SeParam {
            volume: [MovePara::new(1.0); 8],
            pan: [MovePara::new(0.5); 8],
            pitch: [MovePara::new(1.0); 8],
            fxmix: [MovePara::new(0.0); 8],
            dolby: [MovePara::new(30.0 / 127.0); 8],
            port_data: [0; 16],
            port_update: 0,
        };
        for a in [
            &mut p.volume,
            &mut p.pan,
            &mut p.pitch,
            &mut p.fxmix,
            &mut p.dolby,
        ] {
            a[7].current = -1.0;
        }
        p
    }
}

/// A registered sound effect (`JAISound` + its `JAISeParameter`).
#[derive(Debug, Clone)]
struct Se {
    id: u32,
    info: SoundInfo,
    state: u8,
    generation: u32,
    /// The emitter; `None` plays at the listener.
    pos: Option<Vec3>,
    /// Who started it (`mActor`), for the same-sound rules.
    owner: u64,
    wait: u8,
    fade: u32,
    /// `unk5`: distance parameter glide time.
    move_time: u32,
    priority: u32,
    track: usize,
    random: u8,
    cam_pos: Vec3,
    prev_cam_pos: Vec3,
    /// Squared distance, then (after `sendPlayingSeCommand`) the distance.
    dist: f32,
    param: SeParam,
    frames: u32,
}

/// A handle the host keeps to change or stop a sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoundHandle {
    slot: usize,
    generation: u32,
}

/// A playing sequence (`JAISeqUpdateData` + `JAISeqParameter`, reduced to what Sunshine uses).
#[derive(Debug, Clone)]
struct Seq {
    id: u32,
    root: usize,
    handle: usize,
    volume: [MovePara; 16],
    stopping: bool,
    sent_volume: f32,
}

/// What the SE sequence reads back through `SyncCPU(1)` (`FabricatedSeTrackParameter`).
#[derive(Debug, Clone, Copy)]
struct SeTrackParam {
    volume: f32,
    pan: f32,
    pitch: f32,
    fxmix: f32,
}

impl Default for SeTrackParam {
    fn default() -> Self {
        SeTrackParam {
            volume: 1.0,
            pan: 0.5,
            pitch: 1.0,
            fxmix: 0.0,
        }
    }
}

/// The JAI state the sync callback needs (kept apart from the engine it's called from).
struct Jai {
    sound_scene: usize,
    se_track_params: Vec<SeTrackParam>,
    se_root: Option<usize>,
    /// Roots of playing sequences, for `SyncCPU(0)`.
    seq_roots: Vec<usize>,
    ukulele: bool,
}

impl SyncHandler for Jai {
    /// `MSSeCallBack::setParameterSeqSync` over `JAIBasic::setParameterSeqSync`.
    fn sync(&mut self, e: &mut Engine, t: usize, value: u16) -> u16 {
        match value {
            0 => {
                // `outerInit` for a sequence's track: every outer parameter switched on at 1.
                if let Some(p) = e.tracks[t].parent
                    && self.seq_roots.contains(&p)
                {
                    let o = &mut e.outer[t];
                    for (f, v) in [
                        (update::VOLUME, 1.0),
                        (update::PITCH, 1.0),
                        (update::FXMIX, 0.0),
                        (update::PAN, 0.5),
                        (update::DOLBY, 0.0),
                    ] {
                        o.on(f);
                        o.set(f, v);
                    }
                }
                e.tracks[t].pause_status = 74;
                if let Some(p) = e.tracks[t].parent
                    && e.tracks[p].children[15] == Some(t)
                {
                    e.tracks[t].mute = true;
                }
                0
            }
            1 => {
                let i = (e.tracks[t].route & 0xFF) as usize;
                let p = self.se_track_params.get(i).copied().unwrap_or_default();
                let o = &mut e.outer[t];
                o.set(update::VOLUME, p.volume);
                o.set(update::PAN, p.pan);
                o.set(update::PITCH, p.pitch);
                o.set(update::FXMIX, p.fxmix);
                o.set(update::DOLBY, 0.0);
                0
            }
            12 => {
                // The underwater low-pass glide (ports 13-15); no water filter here.
                let parent = e.tracks[t].parent;
                let want = parent.map_or(0, |p| e.tracks[p].port.value[8]);
                let have = e.read_port_direct(t, 15);
                let mut step;
                if want != have {
                    let cur = e.read_port_direct(t, 14);
                    step = if cur > want { 0xFFEC } else { 0x14 };
                    e.write_port_direct(t, 13, step);
                    e.write_port_direct(t, 15, want);
                } else {
                    step = e.read_port_direct(t, 13);
                }
                if step != 0 {
                    let mut cur = e.read_port_direct(t, 14).wrapping_add(step);
                    if cur > 0x7FFF || cur == 0 {
                        step = 0;
                        cur = 0;
                        e.write_port_direct(t, 13, step);
                    } else if want != 0 && cur > want {
                        cur = want;
                        e.write_port_direct(t, 13, 0);
                    }
                    e.write_port_direct(t, 14, cur);
                    return 0x7F_u16.wrapping_sub(cur);
                }
                0xFF
            }
            13 => {
                let want = e.tracks[t].parent.map_or(0, |p| e.tracks[p].port.value[8]);
                e.write_port_direct(t, 14, want);
                e.write_port_direct(t, 15, want);
                0x7F_u16.wrapping_sub(want)
            }
            15 | 20 | 40 => 0,
            30 => {
                // Tracks that pan through the Dolby auto-mixer.
                for i in 0..3 {
                    e.tracks[t].pan_switch_parent[i] = 1;
                }
                for (r, v) in [(8, 0), (10, 0), (11, 0), (12, 0x7FFF)] {
                    e.write_reg(t, r, v);
                }
                if let Some(p) = e.tracks[t].parent {
                    for i in 0..3 {
                        e.tracks[p].pan_switch_ext[i] = 1;
                    }
                    for (r, v) in [(8, 0), (10, 0), (11, 0x7FFF), (12, 0)] {
                        e.write_reg(p, r, v);
                    }
                    for f in [update::PAN, update::FXMIX, update::DOLBY] {
                        e.outer[p].on(f);
                    }
                }
                e.mgrs[t].bus[0] = 0xFFFF;
                0
            }
            110 => 0xFF,
            120 => {
                self.ukulele = !self.ukulele;
                self.ukulele as u16
            }
            121 | 123..=126 => self.ukulele as u16,
            0x7F => {
                e.write_port_direct(t, 0, self.sound_scene as u16);
                0
            }
            _ => 0,
        }
    }
}

/// The game-facing sound system.
pub struct SoundSystem {
    pub engine: Engine,
    jai: Jai,
    data: Arc<SoundData>,
    ses: Vec<Se>,
    /// Per category, the registered sounds (newest first).
    regist: Vec<Vec<usize>>,
    /// Per category, its SE tracks' sounds.
    se_tracks: Vec<Vec<Option<usize>>>,
    se_seq: Option<Seq>,
    seqs: Vec<Seq>,
    next_generation: u32,
    rng: RandomFast,
    view: Mat4,
    camera: Vec3,
    prev_camera: Vec3,
    /// The listener's own position (Mario's voice plays from here).
    pub player: Vec3,
    /// Mario's health for voice selection (8 = full; 2 and below are the tired voices).
    pub health: i16,
    stream: Option<StreamPlay>,
    voice: Option<SoundHandle>,
    out: std::collections::VecDeque<f32>,
    resample_pos: f64,
    /// Overall volumes the host can set.
    pub master: f32,
    pub music: f32,
}

struct StreamPlay {
    pcm: Vec<i16>,
    rate: f32,
    pos: f64,
    looping: bool,
    loop_start: usize,
    volume: f32,
}

impl SoundSystem {
    /// Boots the sound system: echo lines from FX scene 0, then the SE sequence.
    pub fn new(data: Arc<SoundData>) -> SoundSystem {
        let mut engine = Engine::new(data.clone());
        if let Some(scene) = data.init.fx_scenes.first() {
            for (i, l) in scene.iter().enumerate().take(4) {
                engine.set_fx_line(
                    i,
                    l.enabled,
                    l.frames as usize,
                    [(l.out[0].0, l.out[0].1), (l.out[1].0, l.out[1].1)],
                    l.fir,
                );
            }
        }
        let cats = data.init.sound_table.se_categories();
        let mut s = SoundSystem {
            engine,
            jai: Jai {
                sound_scene: 0,
                se_track_params: vec![SeTrackParam::default(); 32],
                se_root: None,
                seq_roots: Vec::new(),
                ukulele: false,
            },
            data,
            ses: Vec::new(),
            regist: vec![Vec::new(); cats],
            se_tracks: vec![Vec::new(); cats],
            se_seq: None,
            seqs: Vec::new(),
            next_generation: 1,
            rng: RandomFast::default(),
            view: Mat4::IDENTITY,
            camera: Vec3::ZERO,
            prev_camera: Vec3::ZERO,
            player: Vec3::ZERO,
            health: 8,
            stream: None,
            voice: None,
            out: Default::default(),
            resample_pos: 0.0,
            master: 1.0,
            music: 1.0,
        };
        s.set_sound_scene(0);
        // `startSeSequence`: `MSD_SE_SEQ` (0x80000800).
        s.se_seq = s.start_seq_sound(0x8000_0800);
        s.jai.se_root = s.se_seq.as_ref().map(|q| q.root);
        s
    }

    pub fn data(&self) -> &Arc<SoundData> {
        &self.data
    }

    /// The sound scene picks each SE category's track count.
    pub fn set_sound_scene(&mut self, scene: usize) {
        let n = self.data.init.scenes.len().max(1);
        self.jai.sound_scene = scene % n;
        for c in 0..self.se_tracks.len() {
            let max = self.category_info(c).0;
            self.se_tracks[c].resize(max, None);
        }
    }

    fn category_info(&self, c: usize) -> (usize, usize) {
        self.data
            .init
            .scenes
            .get(self.jai.sound_scene)
            .and_then(|s| s.get(c))
            .map_or((1, 1), |i| (i.max_playing as usize, i.max_same as usize))
    }

    /// The camera (`MSound::setCameraInfo`): its world position and view matrix.
    pub fn set_listener(&mut self, view: Mat4, camera: Vec3) {
        self.view = view;
        self.prev_camera = self.camera;
        self.camera = camera;
    }

    /// Looks a sound up by its `MSD_*` name.
    pub fn id(&self, name: &str) -> Option<u32> {
        self.data.names.id(name)
    }

    // ------------------------------------------------------------ sequences

    /// Loads and starts a sequence sound in a new root.
    fn start_seq_sound(&mut self, sid: u32) -> Option<Seq> {
        let n = (sid & id::INDEX_MASK) as usize;
        let data: Arc<[u8]> = self.data.seqs.seq(n)?.into();
        let handle = self.engine.set_seq_data(data)?;
        let root = self.engine.root(handle)?;
        self.jai.seq_roots.push(root);
        let info = self.data.init.sound_table.info(sid).copied();
        let mut volume = [MovePara::new(1.0); 16];
        // `setSeExtParameter`: the table's volume.
        if let Some(i) = info {
            volume[1] = MovePara::new((i.volume as f32 / 127.0).min(1.0));
        }
        // `rootInit`: the root's outer parameters carry the sequence's mix.
        let o = &mut self.engine.outer[root];
        for f in [
            update::VOLUME,
            update::PITCH,
            update::FXMIX,
            update::PAN,
            update::DOLBY,
            update::TEMPO,
        ] {
            o.on(f);
        }
        o.set(update::PITCH, info.map_or(1.0, |i| i.pitch));
        o.set(update::PAN, 0.5);
        o.set(update::FXMIX, 0.0);
        o.set(update::DOLBY, 0.0);
        o.set(update::TEMPO, 1.0);
        let mut q = Seq {
            id: sid,
            root,
            handle,
            volume,
            stopping: false,
            sent_volume: -1.0,
        };
        self.push_seq_volume(&mut q);
        self.engine.start_seq(handle);
        Some(q)
    }

    fn push_seq_volume(&mut self, q: &mut Seq) {
        let music = if q.id == 0x8000_0800 { 1.0 } else { self.music };
        let v = q.volume.iter().map(|m| m.current).product::<f32>() * music;
        if v != q.sent_volume {
            q.sent_volume = v;
            self.engine.outer[q.root].set(update::VOLUME, v);
        }
    }

    /// Starts a music sequence (`MSD_BGM_*`), replacing any other.
    pub fn play_bgm(&mut self, sid: u32) -> bool {
        if sid & id::TYPE_MASK != id::TYPE_SEQUENCE {
            return false;
        }
        self.stop_bgm(0);
        if self.seqs.len() >= global::SEQ_PLAY_TRACKS - 1 {
            return false;
        }
        match self.start_seq_sound(sid) {
            Some(q) => {
                self.seqs.push(q);
                true
            }
            None => false,
        }
    }

    /// Stops the music, fading over `frames` game frames.
    pub fn stop_bgm(&mut self, frames: u32) {
        for q in &mut self.seqs {
            if frames == 0 {
                self.engine.stop_seq(q.handle);
                q.stopping = true;
                q.volume[6] = MovePara::new(0.0);
            } else if !q.stopping {
                q.stopping = true;
                q.volume[6].seq_move(0.0, frames);
            }
        }
        if frames == 0 {
            self.seqs.clear();
        }
    }

    pub fn bgm_playing(&self) -> Option<u32> {
        self.seqs.iter().find(|q| !q.stopping).map(|q| q.id)
    }

    // ------------------------------------------------------------ streams

    /// Plays a stream (`MSD_STR_*`, or by file name).
    pub fn play_stream(&mut self, name: &str) -> bool {
        match self.data.stream(name) {
            Ok(s) => {
                self.stream = Some(StreamPlay {
                    rate: s.header.sample_rate as f32,
                    looping: s.header.looping,
                    loop_start: s.header.loop_start as usize,
                    pcm: s.stereo,
                    pos: 0.0,
                    volume: 1.0,
                });
                true
            }
            Err(_) => false,
        }
    }

    pub fn stop_stream(&mut self) {
        self.stream = None;
    }

    // ------------------------------------------------------------ sound effects

    /// Starts a sound effect at `pos` (`None` = at the listener, like system sounds).
    /// `owner` identifies the emitter for the same-sound rules (`mActor`).
    pub fn play_se(&mut self, sid: u32, pos: Option<Vec3>, owner: u64) -> Option<SoundHandle> {
        if sid & id::TYPE_MASK != id::TYPE_SE {
            return None;
        }
        let table = &self.data.init.sound_table;
        let mut sid = sid;
        let mut info = *table.info(sid)?;
        if table.format(sid) & 1 != 0 && info.offset_no != 0xFFFF {
            // `startSoundIndirectID`
            sid = (sid & !id::INDEX_MASK) | info.offset_no as u32;
            info = *table.info(sid)?;
        }
        let cat = id::category(sid) as usize;
        if cat >= self.regist.len() {
            return None;
        }
        // `JAISeEntry::storeBuffer`: one per emitter unless the sound allows several; a full
        // category drops its lowest priority sound for a higher one.
        let (_, max_same) = self.category_info(cat);
        let mut same: Vec<usize> = Vec::new();
        for &s in &self.regist[cat].clone() {
            let se = &self.ses[s];
            if se.owner != owner {
                continue;
            }
            if se.id == sid && info.sw_bit & sw::MULTI == 0 {
                if sid & 0x800 == 0 && se.state == state::STOPPING {
                    self.ses[s].state = state::PLAYING;
                    return Some(self.handle(s));
                }
                self.release_se(s);
                same.clear();
                break;
            }
            same.push(s);
        }
        if same.len() >= max_same.max(1) {
            let lowest = *same.iter().min_by_key(|&&s| self.ses[s].info.priority)?;
            let lp = self.ses[lowest].info.priority;
            if lp > info.priority
                || (lp == info.priority && self.ses[lowest].state == state::STOPPING)
            {
                return None;
            }
            self.release_se(lowest);
        }
        if self.regist[cat].len() >= global::SE_REGIST_MAX {
            return None;
        }
        let slot = match self.ses.iter().position(|s| s.state == state::INACTIVE) {
            Some(i) => i,
            None => {
                self.ses.push(Se {
                    id: 0,
                    info,
                    state: 0,
                    generation: 0,
                    pos: None,
                    owner: 0,
                    wait: 0,
                    fade: 0,
                    move_time: 0,
                    priority: 0,
                    track: 0,
                    random: 0,
                    cam_pos: Vec3::ZERO,
                    prev_cam_pos: Vec3::ZERO,
                    dist: 0.0,
                    param: SeParam::default(),
                    frames: 0,
                });
                self.ses.len() - 1
            }
        };
        self.next_generation = self.next_generation.wrapping_add(1).max(1);
        let mut se = Se {
            id: sid,
            info,
            state: state::STORED,
            generation: self.next_generation,
            pos,
            owner,
            wait: 10,
            fade: 0,
            move_time: global::DISTANCE_MOVE_TIME,
            priority: 0,
            track: 0,
            random: 0,
            cam_pos: Vec3::ZERO,
            prev_cam_pos: Vec3::ZERO,
            dist: 0.0,
            param: SeParam::default(),
            frames: 0,
        };
        // `MSound::setSeExtParameter`
        se.param.volume[1].se_move((info.volume as f32 / 127.0).min(1.0), 0);
        se.param.pitch[1].se_move(info.pitch, 0);
        self.ses[slot] = se;
        self.regist[cat].insert(0, slot);
        Some(self.handle(slot))
    }

    /// Plays a sound by name.
    pub fn play(&mut self, name: &str, pos: Option<Vec3>, owner: u64) -> Option<SoundHandle> {
        let sid = self.id(name)?;
        if sid & id::TYPE_MASK == id::TYPE_SEQUENCE {
            self.play_bgm(sid);
            return None;
        }
        self.play_se(sid, pos, owner)
    }

    fn handle(&self, slot: usize) -> SoundHandle {
        SoundHandle {
            slot,
            generation: self.ses[slot].generation,
        }
    }

    fn live(&self, h: SoundHandle) -> Option<usize> {
        let s = self.ses.get(h.slot)?;
        (s.generation == h.generation && s.state != state::INACTIVE).then_some(h.slot)
    }

    pub fn is_playing(&self, h: SoundHandle) -> bool {
        self.live(h).is_some()
    }

    /// The current sound for a live handle (stale handles return `None`).
    pub fn sound_id(&self, h: SoundHandle) -> Option<u32> {
        self.live(h).map(|slot| self.ses[slot].id)
    }

    /// Renews a held SE without restarting its sequence, even when the host samples more
    /// often than JAI's 30 Hz frame (`JAISeEntry::storeBuffer` keep-alive).
    pub fn refresh(&mut self, h: SoundHandle, pos: Vec3) -> bool {
        let Some(slot) = self.live(h) else {
            return false;
        };
        let se = &mut self.ses[slot];
        se.pos = Some(pos);
        if se.id & id::SUSTAIN_MASK == 0 && se.state == state::STOPPING {
            se.state = state::PLAYING;
        }
        true
    }

    /// Moves a sound's emitter.
    pub fn set_position(&mut self, h: SoundHandle, pos: Vec3) {
        if let Some(s) = self.live(h) {
            self.ses[s].pos = Some(pos);
        }
    }

    /// `JAISound::setVolume` (layer `i`, 0-6) over `frames`.
    pub fn set_volume(&mut self, h: SoundHandle, v: f32, frames: u32, i: usize) {
        if let Some(s) = self.live(h) {
            self.ses[s].param.volume[i.min(6)].se_move(v, frames);
        }
    }

    /// `JAISound::setPitch`
    pub fn set_pitch(&mut self, h: SoundHandle, v: f32, frames: u32, i: usize) {
        if let Some(s) = self.live(h) {
            self.ses[s].param.pitch[i.min(6)].se_move(v, frames);
        }
    }

    /// `JAISound::setPan`
    pub fn set_pan(&mut self, h: SoundHandle, v: f32, frames: u32, i: usize) {
        if let Some(s) = self.live(h) {
            self.ses[s].param.pan[i.min(6)].se_move(v, frames);
        }
    }

    /// `JAISound::setSePortData`
    pub fn set_port(&mut self, h: SoundHandle, port: usize, v: u16) {
        let Some(s) = self.live(h) else {
            return;
        };
        if self.ses[s].state == state::STORED {
            self.ses[s].param.port_data[port & 0xF] = v;
            self.ses[s].param.port_update |= 1 << (port & 0xF);
        } else {
            let route = self.se_track_route(self.ses[s].track) + ((port as u32 & 0xF) << 16);
            if let Some(r) = self.jai.se_root {
                self.engine.write_port_app(r, route, v);
            }
        }
    }

    /// `stopSoundHandle` for an SE: at once, or fading over `frames`.
    pub fn stop(&mut self, h: SoundHandle, frames: u32) {
        let Some(s) = self.live(h) else {
            return;
        };
        if frames == 0 || self.ses[s].state == state::STORED {
            self.release_se(s);
        } else {
            self.ses[s].fade = frames;
            self.ses[s].param.volume[6].se_move(0.0, frames);
        }
    }

    /// The route of SE track `k` under the SE sequence's root (child `k >> 4`, then `k & 0xF`).
    fn se_track_route(&self, k: usize) -> u32 {
        0x2000_0000 + ((k as u32 >> 4) & 0xF) + ((k as u32 & 0xF) << 4)
    }

    /// `JAIBasic::releaseSeRegist`
    fn release_se(&mut self, s: usize) {
        if self.ses[s].state == state::INACTIVE {
            return;
        }
        if self.ses[s].state != state::STORED {
            self.stop_se_track(self.ses[s].track);
        }
        let cat = id::category(self.ses[s].id) as usize;
        if let Some(slots) = self.se_tracks.get_mut(cat) {
            for slot in slots.iter_mut() {
                if *slot == Some(s) {
                    *slot = None;
                }
            }
        }
        if let Some(r) = self.regist.get_mut(cat) {
            r.retain(|&x| x != s);
        }
        self.ses[s].state = state::INACTIVE;
        if self.voice.is_some_and(|v| v.slot == s) {
            self.voice = None;
        }
    }

    /// Port 0 = 0 then interrupt 5 on an SE track: the sequence stops the sound.
    fn stop_se_track(&mut self, k: usize) {
        let Some(r) = self.jai.se_root else {
            return;
        };
        let route = self.se_track_route(k);
        self.engine.write_port_app(r, route, 0);
        if let Some(t) = self.engine.route_track(r, route) {
            self.engine.set_interrupt(t, 5);
        }
    }

    /// `JAISound` distance curves: (volume, pan, fx, pitch-by-distance).
    fn distance_params(&self, se: &Se) -> (f32, f32, f32, f32) {
        let d2 = se.dist;
        let d = d2.sqrt();
        let swbit = se.info.sw_bit;
        // `setDistanceVolumeCommon`
        let volume = if swbit & sw::NO_DISTANCE_VOLUME != 0 {
            1.0
        } else {
            let curve = ((swbit & 0x70000) >> 16) as u8;
            if d < global::MAX_VOLUME_DISTANCE {
                1.0
            } else {
                let x = d - global::MAX_VOLUME_DISTANCE;
                let mut span = global::DISTANCE_MAX - global::MAX_VOLUME_DISTANCE;
                if curve > 3 {
                    span /= (1 << ((curve & 3) + 1)) as f32;
                } else if curve != 0 {
                    span *= (1 << (curve & 3)) as f32;
                }
                if x < span {
                    if curve > 3 {
                        1.0 - x / span
                    } else {
                        global::MIN_DISTANCE_VOLUME
                            + (1.0 - global::MIN_DISTANCE_VOLUME) * (1.0 - x / span)
                    }
                } else if curve > 3 {
                    0.0
                } else {
                    global::MIN_DISTANCE_VOLUME
                }
            }
        };
        // `setDistancePanCommon`
        let c = se.cam_pos;
        let (dx, dz) = (
            c.x.abs().min(global::PAN_DISTANCE_MAX),
            c.z.abs().min(global::PAN_DISTANCE_MAX),
        );
        let pan = if c.x.abs() < 1.0 && c.z.abs() < 1.0 || (c.x == 0.0 && c.z == 0.0) {
            0.5
        } else if c.x > 0.0 && dx >= dz {
            1.0 - (global::PAN_DISTANCE2_MAX - dx)
                / (global::PAN_ANGLE * (global::PAN_DISTANCE2_MAX - dz))
        } else if c.x <= 0.0 && dx >= dz {
            (global::PAN_DISTANCE2_MAX - dx)
                / (global::PAN_ANGLE * (global::PAN_DISTANCE2_MAX - dz))
        } else {
            c.x / (global::PAN_ANGLE2 * dz) + 0.5
        };
        // `setSeDistanceFxmix`
        let fx = if swbit & sw::NO_DISTANCE_FXMIX == 0 {
            if d < global::DISTANCE_MAX {
                global::SE_DISTANCE_FX * (d / global::DISTANCE_MAX)
            } else {
                global::SE_DISTANCE_FX
            }
        } else {
            0.0
        }
        .min(127.0)
            / 127.0;
        // `setSeDistancePitch`
        let mut pitch = 1.0;
        if swbit & sw::MONO != 0 && swbit & sw::NO_DISTANCE_VOLUME == 0 && swbit & sw::DOPPLER == 0
        {
            pitch += global::SE_DISTANCE_PITCH_MAX * (d / global::DISTANCE_MAX).min(1.0);
        }
        (volume, pan, fx, pitch)
    }

    /// `setPositionDopplarCommon` (with the decomp's mix of world and camera space).
    fn doppler(&self, se: &Se) -> f32 {
        let level = ((se.info.sw_bit & sw::DOPPLER) >> 8) as f32;
        let to = self.camera - se.cam_pos;
        let vel = (self.camera - self.prev_camera) - (se.cam_pos - se.prev_cam_pos);
        let l1 = to.length();
        let l2 = (to + vel).length();
        (1.0 / (1.0 - (l1 - l2) / (global::DOPPLER / (level * level)))).clamp(0.1, 2.0)
    }

    /// `JAISound::setSeDistanceParameters`
    fn set_se_distance_params(&mut self, s: usize) {
        let se = self.ses[s].clone();
        let time = if se.state == state::PREPARED {
            0
        } else {
            se.move_time
        };
        let (vol, pan, fx, mut pitch) = self.distance_params(&se);
        if se.info.sw_bit & sw::RANDOM_PITCH_DOWN != 0 {
            pitch = 1.0 - (self.rng.uint8(16) & 0xF) as f32 / 192.0;
        }
        if se.info.sw_bit & sw::RANDOM_PITCH_WIDTH != 0 {
            pitch += se.random as f32 / 192.0;
        }
        let doppler = (se.info.sw_bit & sw::DOPPLER != 0).then(|| self.doppler(&se));
        let p = &mut self.ses[s].param;
        p.volume[4].se_move(vol, time);
        p.pan[4].se_move(pan.clamp(0.0, 1.0), time);
        p.pitch[4].se_move(pitch, time);
        p.fxmix[4].se_move(fx, time);
        if let Some(d) = doppler {
            let t = if se.state == state::PREPARED {
                1
            } else {
                global::DOPPLER_MOVE_TIME
            };
            p.pitch[1].se_move(d, t);
        }
    }

    /// `JAIBasic::checkNextFrameSe`: ranks each category's sounds and gives the best its tracks.
    fn check_next_frame_se(&mut self) {
        let fvar1 = global::DISTANCE_MAX / 1000.0;
        for cat in 0..self.regist.len() {
            let (max_playing, _) = self.category_info(cat);
            let mut candidates: Vec<(u32, usize)> = Vec::new();
            for s in self.regist[cat].clone() {
                let se = &mut self.ses[s];
                if se.state == state::STORED && se.id & id::SUSTAIN_MASK != 0 {
                    se.wait = se.wait.saturating_sub(1);
                } else if se.id & id::SUSTAIN_MASK == 0 && se.state == state::STOPPING {
                    self.release_se(s);
                    continue;
                }
                if self.ses[s].wait == 0 {
                    self.release_se(s);
                    continue;
                }
                if self.ses[s].state == state::INACTIVE {
                    continue;
                }
                let pos = self.ses[s].pos;
                let cam = pos.map_or(Vec3::ZERO, |p| self.view.transform_point3(p));
                let se = &mut self.ses[s];
                se.prev_cam_pos = se.cam_pos;
                se.cam_pos = cam;
                se.dist = cam.length_squared();
                let prio = se.info.priority as u32;
                se.priority = ((0xFF - prio) * (0xFF - prio) * 0x1690) / fvar1 as u32
                    + (se.dist / fvar1) as u32;
                if cam.z > 0.0 {
                    se.priority += (cam.z * 6.0 / fvar1) as u32;
                }
                let limit = if se.info.sw_bit & sw::DISTANCE_LIMIT != 0 {
                    global::DISTANCE_MAX * global::DISTANCE_MAX
                } else {
                    1e10
                };
                if se.dist > limit {
                    if se.id & id::SUSTAIN_MASK == 0 {
                        if se.state != state::STORED {
                            let k = se.track;
                            se.state = state::STORED;
                            self.stop_se_track(k);
                        }
                        self.ses[s].state = state::STORED;
                    } else {
                        self.release_se(s);
                    }
                } else {
                    let (prio, st) = (se.priority, se.state);
                    let i = candidates
                        .iter()
                        .position(|&(p, c)| prio < p || (prio == p && self.ses[c].state >= st))
                        .unwrap_or(candidates.len());
                    if i < max_playing {
                        candidates.insert(i, (self.ses[s].priority, s));
                        candidates.truncate(max_playing);
                    }
                }
            }
            for &(_, s) in &candidates {
                let se = &mut self.ses[s];
                if se.state == state::STORED {
                    se.state = state::PREPARED;
                } else if se.state == state::PLAYING {
                    se.state = state::STARTED;
                }
            }
            let mut cands: Vec<Option<usize>> = candidates.iter().map(|&(_, s)| Some(s)).collect();
            for j in 0..max_playing {
                let cur = self.se_tracks[cat][j];
                let free = match cur {
                    None => true,
                    Some(s) if self.ses[s].state == state::PLAYING => {
                        // Lost its place this frame.
                        if self.ses[s].id & id::SUSTAIN_MASK != 0 {
                            self.release_se(s);
                        } else {
                            self.ses[s].state = state::STORED;
                            self.ses[s].wait = 0;
                        }
                        true
                    }
                    Some(s) if self.ses[s].state == state::INACTIVE => {
                        self.se_tracks[cat][j] = None;
                        true
                    }
                    Some(s) => {
                        if let Some(c) = cands.iter_mut().find(|c| **c == Some(s)) {
                            *c = None;
                        }
                        false
                    }
                };
                if free {
                    let pick = cands.iter_mut().find(|c| {
                        c.is_some_and(|s| {
                            self.ses[s].state != state::STARTED
                                && !self.se_tracks[cat].contains(&Some(s))
                        })
                    });
                    self.se_tracks[cat][j] = pick.and_then(|c| c.take());
                }
            }
        }
    }

    /// `JAIBasic::sendPlayingSeCommand`: starts newly placed sounds and pushes parameters.
    fn send_playing_se_command(&mut self) {
        let Some(root) = self.jai.se_root else {
            return;
        };
        let mut track = 0usize;
        for cat in 0..self.se_tracks.len() {
            for j in 0..self.se_tracks[cat].len() {
                let k = track;
                track += 1;
                let Some(s) = self.se_tracks[cat][j] else {
                    continue;
                };
                self.ses[s].frames += 1;
                let route = self.se_track_route(k);
                // The audio thread may not have run se.scom's startup yet. Keep a
                // prepared sound queued until its child track exists; writes to an
                // absent track are lost (not a completed sound with zero ports).
                let Some(status0) = self.engine.read_port_app(root, route + 0x20000) else {
                    continue;
                };
                let Some(status1) = self.engine.read_port_app(root, route) else {
                    continue;
                };
                self.ses[s].dist = self.ses[s].dist.sqrt();
                let st = self.ses[s].state;
                if st == state::PREPARED {
                    self.ses[s].track = k;
                    let swbit = self.ses[s].info.sw_bit;
                    if swbit & sw::RANDOM_PITCH_WIDTH != 0 {
                        let r = (255.0 * self.rng.ufloat_1()) as u8;
                        self.ses[s].random = match swbit & sw::RANDOM_PITCH_WIDTH {
                            0x40 => r & 0xF,
                            0x80 => r & 0x1F,
                            _ => r & 0x3F,
                        };
                    }
                    let ports = self.ses[s].param.port_data;
                    let mut mask = std::mem::take(&mut self.ses[s].param.port_update);
                    let mut i = 0;
                    while mask != 0 {
                        if mask & 1 != 0 {
                            self.engine
                                .write_port_app(root, route + ((i as u32) << 16), ports[i]);
                        }
                        mask >>= 1;
                        i += 1;
                    }
                    self.set_se_distance_params(s);
                    if self.ses[s].fade > 1 {
                        let f = self.ses[s].fade;
                        self.ses[s].param.volume[6] = MovePara::new(0.0);
                        self.ses[s].param.volume[6].se_move(1.0, f);
                        self.ses[s].fade = 0;
                    }
                    self.send_se_all_parameter(s, true);
                    let index = (self.ses[s].id & id::INDEX_MASK) as u16;
                    let wait = (global::SE_DISTANCE_WAIT_MAX as f32
                        * (self.ses[s].dist / global::DISTANCE_MAX).min(1.0))
                        as u16;
                    let wait = if self.ses[s].info.sw_bit & 0x1000 != 0 {
                        wait
                    } else {
                        0
                    };
                    self.engine.write_port_app(root, route + 0x30000, wait);
                    self.engine.write_port_app(root, route + 0x60000, 0);
                    self.engine.write_port_app(root, route + 0x40000, index);
                    self.engine.write_port_app(root, route, 1);
                    self.ses[s].state = if self.ses[s].id & id::SUSTAIN_MASK != 0 {
                        state::PLAYING
                    } else {
                        state::STOPPING
                    };
                } else if status0 == 0 && status1 != 1 {
                    self.release_se(s);
                } else if self.ses[s].fade != 0 {
                    if self.ses[s].param.volume[6].current != 0.0 {
                        self.set_se_distance_params(s);
                        self.send_se_all_parameter(s, false);
                        self.ses[s].state = if self.ses[s].id & id::SUSTAIN_MASK != 0 {
                            state::PLAYING
                        } else {
                            state::STOPPING
                        };
                    } else {
                        self.release_se(s);
                    }
                } else if st == state::STARTED {
                    self.set_se_distance_params(s);
                    self.send_se_all_parameter(s, false);
                    self.ses[s].state = if self.ses[s].id & id::SUSTAIN_MASK != 0 {
                        state::PLAYING
                    } else {
                        state::STOPPING
                    };
                }
            }
        }
    }

    /// `JAIBasic::sendSeAllParameter`: the layered values into the track's slot; a playing sound
    /// gets them straight onto its track's outer parameters.
    fn send_se_all_parameter(&mut self, s: usize, prepared: bool) {
        let se = &self.ses[s];
        let p = &se.param;
        let pick = |a: &[MovePara; 8], f: &dyn Fn(&[MovePara; 8]) -> f32| {
            if a[7].current == -1.0 {
                f(a)
            } else {
                a[7].current
            }
        };
        let cat = id::category(se.id) as usize;
        let volume = pick(&p.volume, &|a| a[..7].iter().map(|m| m.current).product())
            * CATEGORY_VOLUME.get(cat).copied().unwrap_or(1.0)
            * self.master;
        let pan = pick(&p.pan, &|a| {
            (a[..7]
                .iter()
                .filter(|m| m.current != 0.5)
                .map(|m| m.current - 0.5)
                .sum::<f32>()
                + 0.5)
                .clamp(0.0, 1.0)
        });
        let pitch = pick(&p.pitch, &|a| a[..7].iter().map(|m| m.current).product());
        let fxmix = pick(&p.fxmix, &|a| a[..7].iter().map(|m| m.current).sum());
        let k = se.track;
        let slot = &mut self.jai.se_track_params[k.min(31)];
        let changed = [
            slot.volume != volume,
            slot.pitch != pitch,
            slot.pan != pan,
            slot.fxmix != fxmix,
        ];
        *slot = SeTrackParam {
            volume,
            pan,
            pitch,
            fxmix,
        };
        if prepared {
            return;
        }
        let Some(r) = self.jai.se_root else {
            return;
        };
        let route = self.se_track_route(k);
        if let Some(t) = self.engine.route_track(r, route) {
            let o = &mut self.engine.outer[t];
            if changed[0] {
                o.set(update::VOLUME, volume);
            }
            if changed[1] {
                o.set(update::PITCH, pitch);
            }
            if changed[2] {
                o.set(update::PAN, pan);
            }
            if changed[3] {
                o.set(update::FXMIX, fxmix);
            }
        }
    }

    /// `JAIBasic::checkSeMovePara`
    fn check_se_move_para(&mut self) {
        for cat in 0..self.regist.len() {
            for &s in &self.regist[cat] {
                let p = &mut self.ses[s].param;
                for a in [
                    &mut p.volume,
                    &mut p.pan,
                    &mut p.fxmix,
                    &mut p.dolby,
                    &mut p.pitch,
                ] {
                    for m in a.iter_mut() {
                        m.se_step();
                    }
                }
            }
        }
    }

    /// Mario's voice (`MSound::startMarioVoice`): tired variants at low health and random
    /// alternates, one voice at a time.
    pub fn mario_voice(&mut self, name: &str) -> Option<SoundHandle> {
        let health = self.health;
        let mut r = |p: f32| self.rng.ufloat_1() < p;
        let tired = health <= 2;
        let pick = match name {
            "MSD_SE_MV01_DAMAGE_BIG_01" if r(0.25) => "MSD_SE_MA_VO_DAMAGE_BIG",
            "MSD_SE_MV02_DAMAGE_MID_01" | "MSD_SE_MV03_DAMAGE_LITLE_01" if r(0.25) => {
                "MSD_SE_MA_VO_DAMAGE_SMALL"
            }
            "MSD_SE_MV07_DAMAGE_REACT_01"
            | "MSD_SE_MV12_REACT_01"
            | "MSD_SE_MV26_JUMP_REACT_01"
                if tired =>
            {
                "MSD_SE_MV33_T_01"
            }
            "MSD_SE_MV08A_DOWN_01" if r(0.15) => "MSD_SE_MA_VO_DEAD",
            "MSD_SE_MV13_ACTION_SMALL_01" | "MSD_SE_MV17_EXERT_REACT_01" if tired => {
                "MSD_SE_MV34_ACTION_T_01"
            }
            "MSD_SE_MV15_EXERT_INST_01" if tired => "MSD_SE_MV38_EXERT_INST_T_01",
            "MSD_SE_MV21_JUMP_SMALL_01"
            | "MSD_SE_MV22_JUMP_MID_01"
            | "MSD_SE_MV23_JUMP_LARGE_01"
            | "MSD_SE_MV24_JUMP_SPECIAL_01"
                if tired =>
            {
                "MSD_SE_MV41_JUMP_T_01"
            }
            "MSD_SE_MV21_JUMP_SMALL_01" if r(0.3) => "MSD_SE_MA_VO_JUMP_SMALL_0",
            "MSD_SE_MV22_JUMP_MID_01" if r(0.7) => "MSD_SE_MA_VO_JUMP_MID_0",
            "MSD_SE_MV23_JUMP_LARGE_01" if r(0.5) => "MSD_SE_MA_VO_JUMP_BIG_0",
            "MSD_SE_MV24_JUMP_SPECIAL_01" if r(0.5) => "MSD_SE_MA_VO_JUMP_SPECIAL_0",
            "MSD_SE_MV25A_JUMP_HUGE_01" if tired => "MSD_SE_MV42_JUMP_HUGE_T_01",
            "MSD_SE_MV26_JUMP_REACT_01" if r(0.5) => "MSD_SE_MA_VO_LAND_LOW_0",
            other => other,
        };
        let sid = self.id(pick).or_else(|| self.id(name))?;
        let h = self.play_se(sid, Some(self.player), 0x4D41_5249)?;
        self.voice = Some(h);
        Some(h)
    }

    /// Plays a sound id from an animation key (`MAnmSound::startAnimSound`): Mario voice
    /// category ids go through [`Self::mario_voice`].
    pub fn play_anim_sound(&mut self, sid: u32, pos: Vec3, owner: u64) -> Option<SoundHandle> {
        if sid & id::TYPE_MASK == id::TYPE_SE && id::category(sid) == 7 && owner == 0x4D41_5249 {
            let name = self.data.names.name(sid)?.to_string();
            return self.mario_voice(&name);
        }
        self.play_se(sid, Some(pos), owner)
    }

    // ------------------------------------------------------------ frames and output

    /// `JAIBasic::processFrameWork`: once per game frame (30 Hz).
    pub fn frame(&mut self) {
        if self.se_seq.is_some() {
            self.check_next_frame_se();
        }
        self.send_playing_se_command();
        self.check_se_move_para();
        // Sequences: volume glides, finished fades.
        let mut seqs = std::mem::take(&mut self.seqs);
        for q in &mut seqs {
            for m in &mut q.volume {
                m.seq_step();
            }
            self.push_seq_volume(q);
            if q.stopping && q.volume[6].current == 0.0 {
                self.engine.stop_seq(q.handle);
            }
        }
        seqs.retain(|q| {
            !(q.stopping && q.volume[6].current == 0.0)
                && self.engine.root(q.handle) == Some(q.root)
        });
        let live: Vec<usize> = seqs.iter().map(|q| q.root).collect();
        self.jai
            .seq_roots
            .retain(|r| live.contains(r) || Some(*r) == self.jai.se_root);
        self.seqs = seqs;
        if let Some(mut q) = self.se_seq.take() {
            self.push_seq_volume(&mut q);
            self.se_seq = Some(q);
        }
    }

    /// Renders interleaved stereo at `rate` Hz into `out`.
    pub fn render(&mut self, out: &mut [f32], rate: f32) {
        let step = DAC_RATE as f64 / rate as f64;
        let mut buf = [0f32; SUBFRAME * 2];
        for frame in out.as_chunks_mut::<2>().0 {
            while self.out.len() < 6 {
                self.engine.subframe(&mut self.jai, &mut buf);
                self.mix_stream(&mut buf);
                self.out.extend(buf.iter());
            }
            let f = self.resample_pos as f32;
            let (l0, r0, l1, r1) = (self.out[0], self.out[1], self.out[2], self.out[3]);
            frame[0] = l0 + (l1 - l0) * f;
            frame[1] = r0 + (r1 - r0) * f;
            self.resample_pos += step;
            while self.resample_pos >= 1.0 {
                self.resample_pos -= 1.0;
                self.out.pop_front();
                self.out.pop_front();
            }
        }
    }

    /// The stream, mixed after the DSP (`Kernel::mixInterleaveTrack`).
    fn mix_stream(&mut self, buf: &mut [f32; SUBFRAME * 2]) {
        let Some(s) = &mut self.stream else {
            return;
        };
        let step = s.rate as f64 / DAC_RATE as f64;
        let n = s.pcm.len() / 2;
        for k in 0..SUBFRAME {
            let i = s.pos as usize;
            if i >= n {
                if s.looping && s.loop_start < n {
                    s.pos = s.loop_start as f64;
                } else {
                    self.stream = None;
                    return;
                }
            }
            let i = s.pos as usize;
            let v = s.volume * self.music * self.master / 32768.0;
            buf[k * 2] = (buf[k * 2] + s.pcm[i * 2] as f32 * v).clamp(-1.0, 1.0);
            buf[k * 2 + 1] = (buf[k * 2 + 1] + s.pcm[i * 2 + 1] as f32 * v).clamp(-1.0, 1.0);
            s.pos += step;
        }
    }

    pub fn active_voices(&self) -> usize {
        self.engine.active_voices()
    }

    /// Sounds registered (waiting or playing).
    pub fn active_sounds(&self) -> usize {
        self.regist.iter().map(Vec::len).sum()
    }
}
