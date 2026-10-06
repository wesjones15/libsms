//! JASystem's state, as one struct: the track pool (`TrackMgr`), the channel managers each track
//! owns (`TChannelMgr`) plus the global one (`ChGlobal`), the 256 logical channels (`TChannel`),
//! the 64 DSP voices (`TDSPChannel` + `DSPInterface::DSPBuffer`) and the banks.
//!
//! Behaviour lives in `track.rs` (JASTrack.cpp, JASSeqParser.cpp), `channel.rs` (JASChannel.cpp,
//! JASChannelMgr.cpp, JASChGlobal.cpp, JASChAllocQueue.cpp, JASBankMgr.cpp) and `dsp.rs`
//! (JASDSPChannel.cpp, JASDSPInterface.cpp and the microcode's mixer).

use std::collections::VecDeque;
use std::sync::Arc;

use libsms_assets::sound::bank::Program;
use libsms_assets::sound::{SoundData, WaveKey};

use crate::calc::{PlayerRandom, RandomFast};
use crate::osc::{Osc, Oscillator, presets};

/// `JAIGlobalParameter::systemTrackMax` as Sunshine sets it (`MSound::MSound`).
pub const TRACKS: usize = 0xB9;
/// `systemRootTrackMax` (`MSound::MSound`).
pub const ROOTS: usize = 8;
/// `ChGlobal::init`
pub const CHANNELS: usize = 256;
/// `TDSPChannel::initAll`
pub const VOICES: usize = 64;
/// The global channel manager's index in [`Engine::mgrs`]; track `i` owns manager `i`.
pub const GLOBAL: usize = TRACKS;
/// `Driver::DSP_VOICE_WAITS_MAX`
pub const QUEUE: usize = 0x20;
/// `Driver::MAX_MIXERLEVEL`: `setMixerLevel(inputGainDown = 0.802, ...)` * 16384.
pub const MIXER_LEVEL: f32 = 0.802 * 16384.0;

/// `TTrack::MoveParam_`: a parameter gliding to a target.
#[derive(Debug, Clone, Copy, Default)]
pub struct Move {
    pub current: f32,
    pub target: f32,
    pub time: f32,
    pub step: f32,
}

/// `TTrack::TimedParamType`
pub mod timed {
    pub const VOLUME: usize = 0;
    pub const PITCH: usize = 1;
    pub const FXMIX: usize = 2;
    pub const PAN: usize = 3;
    pub const DOLBY: usize = 4;
    pub const DIST_FILTER: usize = 5;
    pub const OSC0_WIDTH: usize = 6;
    pub const OSC1_VERTEX: usize = 11;
    pub const IIR0: usize = 12;
    pub const PAUSE_VOLUME: usize = 16;
    pub const PAN_DELAY: usize = 17;
    pub const COUNT: usize = 18;
}

/// `TTrack::ParamUpdateFlag`
pub mod update {
    pub const VOLUME: u32 = 1;
    pub const PITCH: u32 = 2;
    pub const FXMIX: u32 = 4;
    pub const PAN: u32 = 8;
    pub const DOLBY: u32 = 0x10;
    pub const DIST_FILTER: u32 = 0x20;
    pub const TEMPO: u32 = 0x40;
    pub const FIR: u32 = 0x80;
    pub const IIRS: u32 = 0xF000;
    pub const PAN_DELAY: u32 = 0x20000;
}

/// `TTrack::TOuterParam`: what the game layers over a track.
#[derive(Debug, Clone, Copy, Default)]
pub struct OuterParam {
    pub switch: u16,
    pub update: u16,
    pub volume: f32,
    pub pitch: f32,
    pub fxmix: f32,
    pub dolby: f32,
    pub pan: f32,
    pub tempo: f32,
    pub fir: [i16; 8],
}

impl OuterParam {
    pub fn check(&self, flag: u32) -> bool {
        self.switch as u32 & flag != 0
    }

    /// `setParam`
    pub fn set(&mut self, flag: u32, v: f32) {
        let slot = match flag {
            update::VOLUME => &mut self.volume,
            update::PITCH => &mut self.pitch,
            update::FXMIX => &mut self.fxmix,
            update::DOLBY => &mut self.dolby,
            update::PAN => &mut self.pan,
            update::TEMPO => &mut self.tempo,
            _ => return,
        };
        *slot = v;
        self.update |= flag as u16;
    }

    /// `onSwitch`
    pub fn on(&mut self, flag: u32) {
        self.switch |= flag as u16;
        self.update |= flag as u16;
    }
}

/// `TSeqCtrl`: a track's read position, wait, call/loop stack and interrupt return.
#[derive(Debug, Clone, Default)]
pub struct SeqCtrl {
    pub data: Option<Arc<[u8]>>,
    pub pos: u32,
    pub wait: i32,
    pub stack: [u32; 8],
    pub loop_timers: [u16; 8],
    pub depth: u32,
    pub saved_wait: i32,
    /// `mPreviousFilePtr`: where an interrupt returns to.
    pub intr_return: Option<u32>,
}

impl SeqCtrl {
    pub fn byte_at(&self, at: u32) -> u8 {
        self.data
            .as_ref()
            .and_then(|d| d.get(at as usize).copied())
            .unwrap_or(0)
    }
}

/// `TTrackPort`
#[derive(Debug, Clone, Copy, Default)]
pub struct TrackPort {
    pub import: [bool; 16],
    pub export: [bool; 16],
    pub value: [u16; 16],
}

/// `TIntrMgr`
#[derive(Debug, Clone, Copy, Default)]
pub struct IntrMgr {
    pub enabled: bool,
    pub pending: u8,
    pub mask: u8,
    pub timer_count: u8,
    pub timer: u32,
    pub timer_reload: u32,
    pub vectors: [u32; 8],
}

/// `TTrack::TNoteMgr`
#[derive(Debug, Clone, Copy, Default)]
pub struct NoteMgr {
    /// Channel and its generation (`unkC6`) when the note started.
    pub channels: [Option<(usize, u16)>; 8],
    pub base_time: u32,
    pub connect: u8,
    pub last_note: u8,
    pub before_tie: bool,
}

/// What a channel oscillator follows: its own copy, or a track's live `mOscData` (the
/// C stores the pointer, so a sequence changing its vibrato depth affects playing notes).
#[derive(Debug, Clone, Default)]
pub enum OscSrc {
    #[default]
    None,
    Shared(Arc<Osc>),
    Track(usize, usize),
}

/// `TTrack`
#[derive(Debug, Clone)]
pub struct Track {
    pub seq: SeqCtrl,
    pub port: TrackPort,
    pub intr: IntrMgr,
    pub notes: NoteMgr,
    pub timed: [Move; timed::COUNT],
    /// `TRegisterParam` viewed as u16 registers: 0-5 general, 3 the flag, 6 bank << 8 |
    /// program, 7 bend range, 8-12 pan powers, 13 priority; 16-23 the four address registers.
    pub regs: [u16; 24],
    pub parent: Option<usize>,
    pub children: [Option<usize>; 16],
    pub route: u32,
    pub osc_data: [Osc; 2],
    pub oscs: [Oscillator; 2],
    pub ads_table: [[i16; 3]; 4],
    pub rel_table: [[i16; 3]; 2],
    pub osc_mode: [u8; 2],
    pub connect_name: u32,
    pub tick_counter: f32,
    pub tick_rate: f32,
    /// `unk3B4`: update flags waiting for `updateSeq`.
    pub pending_update: u32,
    pub tempo: u16,
    pub time_base: u16,
    /// `unk3BC`: how the track was opened (bit 0 = ignore the parent's mix, 2 = fresh
    /// registers, 4 = notes go to the parent's channel manager).
    pub open_mode: u8,
    /// `unk3BD`: follows its own tempo against the parent's.
    pub own_tempo: bool,
    pub inner_memory: bool,
    pub transpose: i8,
    pub transpose_total: i8,
    pub pause_status: u8,
    pub mute: bool,
    pub volume_mode: u8,
    /// `mSeqState`: 0 closed, 1 running, 2 loaded, 3 stopping.
    pub state: u8,
    pub pan_switch_ext: [u8; 3],
    pub pan_switch_parent: [u8; 3],
    pub time_relate: bool,
    pub connected: bool,
    pub paused: bool,
}

impl Default for Track {
    fn default() -> Self {
        Track {
            seq: SeqCtrl::default(),
            port: TrackPort::default(),
            intr: IntrMgr::default(),
            notes: NoteMgr::default(),
            timed: [Move::default(); timed::COUNT],
            regs: [0; 24],
            parent: None,
            children: [None; 16],
            route: 0,
            osc_data: [presets::envelope(), presets::vibrato()],
            oscs: [Oscillator::default(), Oscillator::default()],
            ads_table: presets::ADS_TABLE,
            rel_table: [[0, 0xA, 0], [0xF, 1, 0]],
            osc_mode: [0xF; 2],
            connect_name: 0,
            tick_counter: 0.0,
            tick_rate: 0.0,
            pending_update: 0,
            tempo: 0x78,
            time_base: 0x78,
            open_mode: 0,
            own_tempo: false,
            inner_memory: false,
            transpose: 0,
            transpose_total: 0,
            pause_status: 0,
            mute: false,
            volume_mode: 0,
            state: 0,
            pan_switch_ext: [0; 3],
            pan_switch_parent: [0; 3],
            time_relate: false,
            connected: false,
            paused: false,
        }
    }
}

/// `TChannelMgr`: the channels a track (or the global pool) owns and the mix it applies.
#[derive(Debug, Clone)]
pub struct Mgr {
    /// 0 free, 1 playing, 2 releasing, 3 waiting for a DSP voice.
    pub lists: [VecDeque<usize>; 4],
    /// `unk4`: channels borrowed from the global pool on demand.
    pub borrowed: u32,
    pub managed: u32,
    /// `unk70`: steal a playing channel when out of free ones.
    pub steal: bool,
    pub volume: f32,
    pub pitch: f32,
    pub pan: f32,
    pub fxmix: f32,
    pub dolby: f32,
    pub fir: [i16; 8],
    pub iir: [i16; 4],
    pub dist_filter: i16,
    /// `unk4E`: mixer output configs (`MixConfig`).
    pub bus: [u16; 6],
    pub delays: [u8; 6],
    pub delay_max: u8,
    pub filter_mode: u8,
    /// `unk62`: pan/fx/dolby calc types (`calc_sw_table`).
    pub calc: [u8; 3],
    /// `unk68`: note-on, release and lifetime priorities.
    pub priority: u32,
    pub priority_time: u16,
}

impl Default for Mgr {
    fn default() -> Self {
        let mut m = Mgr {
            lists: Default::default(),
            borrowed: 0,
            managed: 0,
            steal: true,
            volume: 1.0,
            pitch: 1.0,
            pan: 0.5,
            fxmix: 0.0,
            dolby: 0.0,
            fir: [0; 8],
            iir: [0; 4],
            dist_filter: 0,
            bus: [0; 6],
            delays: [0; 6],
            delay_max: 0,
            filter_mode: 0,
            calc: [0; 3],
            priority: 0,
            priority_time: 0,
        };
        m.init();
        m
    }
}

impl Mgr {
    /// `TChannelMgr::init`. It clears the list heads too: channels still in them drop out.
    pub fn init(&mut self) {
        *self = Mgr {
            lists: Default::default(),
            borrowed: 0,
            managed: 0,
            steal: true,
            volume: 1.0,
            pitch: 1.0,
            pan: 0.5,
            fxmix: 0.0,
            dolby: 0.0,
            fir: [0x7FFF, 0, 0, 0, 0, 0, 0, 0],
            iir: [0x7FFF, 0, 0, 0],
            dist_filter: 0,
            bus: [0x150, 0x210, 0x352, 0x412, 0, 0],
            delays: [0; 6],
            delay_max: 0,
            filter_mode: 0,
            calc: [0xD; 3],
            priority: 0x20103,
            priority_time: 600,
        };
    }
}

/// `Driver::PanMatrix_`
#[derive(Debug, Clone, Copy, Default)]
pub struct PanMatrix {
    pub sound: f32,
    pub effect: f32,
    pub channel: f32,
}

/// What a channel plays.
#[derive(Debug, Clone, Default)]
pub enum Source {
    #[default]
    None,
    Wave(WaveKey),
    /// `noteOnOsc`: a synthesized waveform (programs 0xF0+).
    Osc(u32),
}

/// `TChannel`
#[derive(Debug, Clone, Default)]
pub struct Channel {
    /// `unk0`, `unk1`: velocity and key; key 0xFF = no note.
    pub velocity: u8,
    pub key: u8,
    pub paused: bool,
    /// `unk3`: parameters changed, push them to the DSP.
    pub dirty: u8,
    pub mgr: usize,
    /// The list it's in (`unk8`): (manager, list).
    pub list: Option<(usize, u8)>,
    pub source: Source,
    /// The wave's recorded key (`mWaveData->mKey`).
    pub wave_key: u8,
    pub voice: Option<usize>,
    /// `unk28 != null`: the channel runs `updatecallLogicalChannel`.
    pub logical_cb: bool,
    /// `unk2C != null`: a key sweep is running (`extraUpdate`).
    pub sweeping: bool,
    /// `unk30`, `unk34`: note length in updates (-1 = until released) and its countdown.
    pub length: i32,
    pub countdown: i32,
    pub oscs: [Oscillator; 4],
    pub osc_src: [OscSrc; 4],
    /// `unk48`: base pitch (instrument pitch x wave rate / DAC rate).
    pub base_pitch: f32,
    /// `unk4C`: instrument volume.
    pub inst_volume: f32,
    /// `unk50`: base pitch at the current key.
    pub pitch: f32,
    /// `unk54`: instrument volume x velocity curve x effects.
    pub volume: f32,
    pub calc: [u8; 3],
    pub pan_power: PanMatrix,
    /// Pan, fx and dolby from the sound, its oscillators and the channel manager.
    pub mix: [PanMatrix; 3],
    /// `unk8C`, `unk90`: oscillator pitch and volume multipliers.
    pub osc_pitch: f32,
    pub osc_volume: f32,
    pub sweep_target: f32,
    pub dsp_pitch: u16,
    pub sweep_time: u16,
    /// `unk9C`: the manager it started under; parameters follow it only while it's the same.
    pub play_mgr: Option<usize>,
    pub mgr_pitch: f32,
    pub mgr_volume: f32,
    pub bus: [u16; 6],
    pub mixer: [i16; 6],
    pub priority: u32,
    pub priority_time: u16,
    pub generation: u16,
    /// `unkC8`: the polyphony-limit key.
    pub limit_key: u32,
    /// `unkD0`: orphaned (its track closed); only volume and pitch updates go out.
    pub orphan: bool,
}

/// `DSPInterface::DSPBuffer::Channel`
#[derive(Debug, Clone, Copy, Default)]
pub struct MixOut {
    pub bus: u16,
    pub target: i16,
    pub current: i16,
}

/// A DSP voice: `TDSPChannel` plus its `DSPBuffer`.
#[derive(Debug, Clone)]
pub struct Voice {
    /// 0 allocated, 1 free, 2 force-stopped.
    pub status: u8,
    pub sign: Option<usize>,
    pub priority: u8,
    pub priority_time: u16,
    /// `mCallback` is `updatecallDSPChannel`.
    pub callback: bool,
    pub cb_interval: u16,
    // DSPBuffer
    pub enabled: bool,
    pub done: bool,
    pub end_requested: bool,
    pub pause: bool,
    pub pitch: u16,
    pub mix: [MixOut; 6],
    pub age: u32,
    pub source: Source,
    pub pcm: Option<Arc<Vec<i16>>>,
    pub looping: bool,
    pub loop_start: u32,
    pub end: u32,
    pub pos: f64,
    pub osc_phase: f64,
    /// Volume ramps start from `current` on the first frame after a (re)start.
    pub fresh: bool,
}

impl Default for Voice {
    fn default() -> Self {
        Voice {
            status: 1,
            sign: None,
            priority: 0,
            priority_time: 0,
            callback: false,
            cb_interval: 0,
            enabled: false,
            done: false,
            end_requested: false,
            pause: false,
            pitch: 0,
            mix: [MixOut::default(); 6],
            age: 0,
            source: Source::None,
            pcm: None,
            looping: false,
            loop_start: 0,
            end: 0,
            pos: 0.0,
            osc_phase: 0.0,
            fresh: false,
        }
    }
}

/// `DSPInterface::FXBuffer`: an echo line fed by one send bus.
#[derive(Debug, Clone, Default)]
pub struct FxLine {
    pub enabled: bool,
    pub delay: Vec<i16>,
    pub pos: usize,
    pub out: [(u16, i16); 2],
    pub fir: [i16; 8],
    pub history: [i16; 8],
}

/// A bank as the engine uses it: the parsed bank plus its oscillators as engine `Osc`s.
pub struct BankRt {
    pub wave_bank: usize,
    /// Per program, its two oscillators (`TBasicInst::mOscillators`; a missing one stays a
    /// null slot, as `createBasicBank` leaves it).
    pub oscs: Vec<[Option<Arc<Osc>>; 2]>,
}

/// The callback a sequence's `SyncCPU` command reaches (`TTrack::sCallBackFunc`): the game
/// layer answers it.
pub trait SyncHandler {
    fn sync(&mut self, engine: &mut Engine, track: usize, value: u16) -> u16;
}

/// A handler that answers every sync with 0xFFFF (no callback registered).
pub struct NoSync;

impl SyncHandler for NoSync {
    fn sync(&mut self, _: &mut Engine, _: usize, _: u16) -> u16 {
        0xFFFF
    }
}

pub struct Engine {
    pub data: Arc<SoundData>,
    pub banks: Vec<BankRt>,
    /// The scene wave group currently loaded in the scene wave bank (`MSound::loadWave`).
    pub scene_group: Option<usize>,
    pub tracks: Vec<Track>,
    pub outer: Vec<OuterParam>,
    pub free_tracks: VecDeque<usize>,
    pub roots: [Option<usize>; ROOTS],
    /// `Kernel::registerSubframeCallback` entries: root tracks being ticked.
    pub running: Vec<usize>,
    /// `TrackMgr::sTrackList`: connect name -> track.
    pub registry: Vec<(u32, Option<usize>)>,
    pub mgrs: Vec<Mgr>,
    pub channels: Vec<Channel>,
    pub voices: Vec<Voice>,
    pub queue: VecDeque<Option<usize>>,
    pub fx: [FxLine; 4],
    pub rng: PlayerRandom,
    pub inst_rng: RandomFast,
    /// `TTrack::sUpdateSyncMode`
    pub update_sync_mode: u8,
}

impl Engine {
    pub fn new(data: Arc<SoundData>) -> Engine {
        let banks = data
            .init
            .banks
            .iter()
            .map(|b| BankRt {
                wave_bank: b.wave_bank,
                oscs: b
                    .bank
                    .programs
                    .iter()
                    .map(|p| match p {
                        Some(Program::Inst(i)) => {
                            let o =
                                |k: usize| i.oscs[k].as_ref().map(|o| Arc::new(Osc::from_bank(o)));
                            [o(0), o(1)]
                        }
                        _ => [None, None],
                    })
                    .collect(),
            })
            .collect();
        let mut e = Engine {
            data,
            banks,
            scene_group: None,
            tracks: vec![Track::default(); TRACKS],
            outer: vec![OuterParam::default(); TRACKS],
            free_tracks: (0..TRACKS).collect(),
            roots: [None; ROOTS],
            running: Vec::new(),
            registry: Vec::new(),
            mgrs: vec![Mgr::default(); TRACKS + 1],
            channels: vec![Channel::default(); CHANNELS],
            voices: vec![Voice::default(); VOICES],
            queue: VecDeque::new(),
            fx: Default::default(),
            rng: PlayerRandom::default(),
            inst_rng: RandomFast::default(),
            update_sync_mode: 0,
        };
        // `ChGlobal::init`: every channel starts free in the global manager.
        for c in 0..CHANNELS {
            e.channels[c].mgr = GLOBAL;
            e.channel_init(c);
            e.add_list_head(GLOBAL, c, 0);
        }
        e.mgrs[GLOBAL].managed = CHANNELS as u32;
        e
    }

    /// `TrackMgr::handleToSeq`
    pub fn root(&self, handle: usize) -> Option<usize> {
        *self.roots.get(handle)?
    }

    /// `TrackMgr::getNewTrack`
    pub fn new_track(&mut self) -> Option<usize> {
        let t = self.free_tracks.pop_front()?;
        self.tracks[t].state = 2;
        self.tracks[t].inner_memory = true;
        Some(t)
    }

    /// `TrackMgr::backTrack`
    pub fn back_track(&mut self, t: usize) {
        if !self.free_tracks.contains(&t) {
            self.free_tracks.push_back(t);
        }
    }

    /// The registered track with this connect name (`TrackMgr` lookups).
    pub fn registered(&self, name: u32) -> Option<usize> {
        self.registry
            .iter()
            .find(|(n, t)| *n == name && t.is_some())
            .and_then(|(_, t)| *t)
    }

    /// Voices currently sounding.
    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.enabled && !v.done).count()
    }
}
