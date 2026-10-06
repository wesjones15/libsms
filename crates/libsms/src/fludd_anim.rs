//! How FLUDD and Mario's pumping arm move: the upper-body layer (`TMario::addUpper`,
//! Player/MarioDraw.cpp), FLUDD's handles (`TWaterGun::calcAnimation`) and the nozzle clips
//! (`TNozzleBase/Trigger/Deform::animation`, Player/WaterGun.cpp).
//!
//! The sim keeps this state because part of it is gameplay: FLUDD only emits once its handles
//! are fully out (`TWaterGun::emit` returns while `unk1CEC > 0`). Hosts read the clip names and
//! frames to pose the models, and report the muzzle joints back with
//! [`Mario::set_emitters`] so water leaves the drawn nozzle (`getEmitPosDirSpeed`).
//!
//! Clip rates: Mario's frame controllers run once per 120 Hz tick at `rate * 0.5` (60 fps
//! clips in real time). The nozzles' `MActor` clips are set to `SMSGetAnmFrameRate()` = 2 per
//! update, which would be four times faster if they updated every tick; they are played at
//! Mario's 60 fps here (see the spec's "Unverified" list).

use glam::Vec3;

use crate::anim::{AnimTable, ClipInfo, ClipName, FrameCtrl, LoopMode, pump_clip};
use crate::event::{Event, SoundCue};
use crate::fludd::Fludd;
use crate::mario::{Mario, UpperState};
use crate::math::{cos, sin};
use crate::params::NozzleKind;

/// Frames per tick for FLUDD's `MActor` clips (see the module notes).
pub const NOZZLE_RATE: f32 = 0.5;

/// FLUDD's own clips (`/mario/watergun2/body`).
pub mod body_clip {
    use crate::anim::ClipName;
    /// Handles folded away (frame 119) to held out (frame 0).
    pub const HOUSE: ClipName = "wg_house";
    pub const PUMP: ClipName = "wg_pump";
    pub const HANG: ClipName = "wg_hgpmp";
    pub const SWIM: ClipName = "wg_swpmp";
    pub const FENCE: ClipName = "wg_fepmp";
}

/// The nozzle model's folder and file stem (`nozzleBmdData`, WaterGun.cpp).
pub fn nozzle_model(kind: NozzleKind) -> Option<&'static str> {
    Some(match kind {
        NozzleKind::Spray => "normal_wg",
        NozzleKind::Rocket => "rocket_wg",
        NozzleKind::Hover | NozzleKind::Underwater => "hover_wg",
        NozzleKind::Turbo => "back_wg",
        NozzleKind::Yoshi => return None,
    })
}

/// A nozzle's clips by `setBckFromIndex` index. `MActorAnmData::init` numbers the files in the
/// order the archive's directory lists them (read from the disc's `mario.szs`).
pub fn nozzle_clips(kind: NozzleKind) -> &'static [ClipName] {
    match kind {
        NozzleKind::Spray => &[
            "house_wg_change_end",
            "house_wg_change_start",
            "normal_wg_change_end",
            "normal_wg_change_start",
            "normal_wg_house",
            "normal_wg_shooting",
            "normal_wg_shoot_end",
            "normal_wg_shoot_start",
        ],
        NozzleKind::Rocket => &[
            "rocket_wg_change_end",
            "rocket_wg_change_start",
            "rocket_wg_shooting",
            "rocket_wg_shoot_end",
            "rocket_wg_shoot_start",
        ],
        NozzleKind::Hover | NozzleKind::Underwater => &[
            "hover_wg_change_end",
            "hover_wg_change_start",
            "hover_wg_shooting",
            "hover_wg_shoot_end",
            "hover_wg_shoot_start",
        ],
        NozzleKind::Turbo => &[
            "back_wg_change_end",
            "back_wg_change_start",
            "back_wg_shooting",
            "back_wg_shoot_end",
            "back_wg_shoot_start",
        ],
        NozzleKind::Yoshi => &[],
    }
}

/// One nozzle's `MActor` clip and its state machine (`unk36C`).
#[derive(Debug, Clone, PartialEq)]
pub struct NozzleAnim {
    state: u8,
    /// Index into [`nozzle_clips`].
    pub clip: Option<usize>,
    pub ctrl: FrameCtrl,
}

impl Default for NozzleAnim {
    fn default() -> Self {
        NozzleAnim {
            state: 2,
            clip: None,
            ctrl: FrameCtrl::new(AnimTable::FALLBACK),
        }
    }
}

impl NozzleAnim {
    /// `TNozzleBase::init` / `TNozzleTrigger::init`.
    pub(crate) fn init(&mut self, kind: NozzleKind) {
        self.state = if kind.is_trigger() { 0 } else { 2 };
    }

    /// `MActor::checkCurBckFromIndex` + `setBckFromIndex`: restarts only on a change.
    fn set(&mut self, kind: NozzleKind, i: usize, table: &AnimTable) {
        if self.clip != Some(i) {
            self.clip = Some(i);
            let info = nozzle_clips(kind)
                .get(i)
                .map_or(AnimTable::FALLBACK, |n| table.get(n));
            self.ctrl = FrameCtrl::new(info);
            self.ctrl.rate = NOZZLE_RATE;
        }
    }

    /// Plays the swap clip at a frame set from the swap progress (`setRate(0)`).
    fn hold(&mut self, kind: NozzleKind, i: usize, t: f32, table: &AnimTable) {
        self.set(kind, i, table);
        self.ctrl.frame = t * self.ctrl.end;
        self.ctrl.rate = 0.0;
    }
}

/// What a host draws for FLUDD this tick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FluddLook {
    pub body_clip: ClipName,
    pub body_frame: f32,
    /// The fitted nozzle; its model comes from [`nozzle_model`].
    pub nozzle: NozzleKind,
    pub nozzle_clip: Option<ClipName>,
    pub nozzle_frame: f32,
    /// Pitch of the Spray/Turbo neck (`NozzleCtrl`): applied when negative.
    pub gun_angle: f32,
    /// Hover nozzle tilts, left and right (`WaterGunDivingCtrlL/R`).
    pub tilt: [f32; 2],
    /// Turbo propeller angle (`RotateCtrl`).
    pub prop_angle: f32,
}

impl Fludd {
    /// Is the fitted nozzle putting water out (the nozzles' `updateAnimation` test)?
    fn spraying(&self) -> bool {
        self.water != 0
            && if self.nozzle.is_trigger() {
                self.trigger_state() == crate::fludd::TriggerState::Active
            } else {
                self.spray_amount() > 0.0
            }
    }

    /// `getCurrentNozzle()->animation(mCurrentNozzle)`, from `TWaterGun::movement`.
    pub(crate) fn nozzle_animation(&mut self, table: &AnimTable) {
        let kind = self.nozzle;
        let spraying = self.spraying();
        let to_second = self.switching_to_second();
        let to_spray = self.switching_to_spray();
        let p = self.switch_progress;
        let housed = self.housed;
        let a = &mut self.nozzle_anims[kind as usize];
        match kind {
            NozzleKind::Spray => {
                // TNozzleDeform::animation
                if to_second {
                    if a.state != 4 && a.state != 6 {
                        a.state = if housed > 0.5 { 4 } else { 6 };
                    }
                } else if to_spray {
                    if a.state != 5 && a.state != 7 {
                        a.state = if housed > 0.5 { 5 } else { 7 };
                    }
                } else if housed > 0.0 {
                    a.state = 0;
                } else if a.state == 0 {
                    a.state = 2;
                }
                match a.state {
                    0 => {
                        a.set(kind, 4, table);
                        a.ctrl.frame = a.ctrl.end * housed;
                    }
                    2 => {
                        a.set(kind, 7, table);
                        if a.ctrl.finished() {
                            a.state = 3;
                        }
                    }
                    3 => {
                        a.set(kind, 5, table);
                        if !spraying {
                            a.state = 8;
                        }
                    }
                    8 => {
                        a.set(kind, 6, table);
                        if spraying {
                            a.state = 2;
                        }
                        if a.ctrl.finished() && housed != 0.0 {
                            a.state = 0;
                        }
                    }
                    4 => a.hold(kind, 1, 2.0 * p, table),
                    5 => {
                        a.hold(kind, 0, 1.0 - 2.0 * p, table);
                        if p <= 0.0 {
                            a.state = 0;
                            self.housed = 0.0;
                        }
                    }
                    6 => a.hold(kind, 3, 2.0 * p, table),
                    7 => {
                        a.hold(kind, 2, 1.0 - 2.0 * p, table);
                        if p <= 0.0 {
                            a.state = 2;
                            self.housed = 0.0;
                        }
                    }
                    _ => {}
                }
            }
            NozzleKind::Rocket | NozzleKind::Hover | NozzleKind::Turbo | NozzleKind::Underwater => {
                // TNozzleTrigger::animation (and TNozzleBase::animation for Underwater, which
                // uses the same clip order).
                if to_second {
                    a.state = 4;
                }
                if to_spray {
                    a.state = 3;
                }
                match a.state {
                    0 => {
                        a.set(kind, 4, table);
                        if a.ctrl.finished() {
                            a.state = 1;
                        }
                    }
                    1 => {
                        a.set(kind, 2, table);
                        if !spraying {
                            a.state = 2;
                        }
                    }
                    2 => {
                        a.set(kind, 3, table);
                        if spraying {
                            a.state = 0;
                        }
                    }
                    3 => a.hold(kind, 1, -(2.0 * (p - 0.5) - 1.0), table),
                    4 => {
                        a.hold(kind, 0, 2.0 * (p - 0.5), table);
                        if p >= 1.0 {
                            a.state = 0;
                        }
                    }
                    _ => {}
                }
            }
            NozzleKind::Yoshi => {}
        }
    }

    /// Advances the fitted nozzle's clip (`MActor::frameUpdate`).
    pub(crate) fn nozzle_frame_update(&mut self) {
        self.nozzle_anims[self.nozzle as usize].ctrl.update();
    }
}

impl Mario {
    /// `TMario::addUpper` and the upper layer's frame update (`M3UModelMario::updateInMotion`):
    /// the pumping arm. Runs after the body clip each tick.
    pub(crate) fn add_upper(&mut self) {
        match self.upper {
            UpperState::Pumping | UpperState::HoldingPump => {
                if let Some(c) = pump_clip(self.anim.clip) {
                    self.upper_on = true;
                    if self.upper_clip != Some(c) {
                        // `changeMtxCalcSIAnmBQAnmTransform` -> `J3DFrameCtrl::init`.
                        self.upper_clip = Some(c);
                        let info = self.anims.get(c);
                        self.upper_ctrl = FrameCtrl::new(info);
                    }
                }
            }
            UpperState::Idle => self.upper_on = false,
        }
        if self.upper == UpperState::Pumping {
            self.upper_ctrl.mode = LoopMode::MirrorLoop;
            let rate = self.pad.r_analog as f32 * self.params.upper_body.pump_anm_speed;
            let empty = self.fludd.as_ref().is_some_and(|f| f.water <= 0);
            if empty {
                if self.upper_ctrl.rate >= 0.0 {
                    self.upper_ctrl.rate = rate;
                    if self.upper_ctrl.will_pass(8.0) {
                        self.sound("MSD_SE_PO_POMPING1", None);
                    }
                } else {
                    self.upper_ctrl.rate = -rate;
                    if self.upper_ctrl.will_pass(13.0) {
                        self.sound("MSD_SE_PO_POMPING2", None);
                    }
                }
            }
        }
        self.upper_ctrl.update();
    }

    /// `TWaterGun::calcAnimation`: FLUDD's handles follow the pumping arm, then fold away
    /// after `unk1CF8` (360) ticks without pumping.
    pub(crate) fn fludd_calc_animation(&mut self) {
        let pumping = matches!(self.upper, UpperState::Pumping | UpperState::HoldingPump);
        let in_water = self.in_water;
        let hanging = self.anim.clip == "hang";
        let pump_frame = self.upper_ctrl.frame;
        let house_end = self.anims.get(body_clip::HOUSE).frames;
        let Some(f) = self.fludd.as_mut() else {
            return;
        };
        if pumping {
            if f.housed == 0.0 {
                f.body_clip = if in_water {
                    body_clip::SWIM
                } else if hanging {
                    body_clip::HANG
                } else {
                    body_clip::PUMP
                };
                f.body_frame = pump_frame;
                f.house_hold = 360;
            } else {
                f.body_clip = body_clip::HOUSE;
                f.housed = (f.housed - 0.1).max(0.0);
                f.body_frame = f.housed * house_end;
            }
        } else if f.house_hold == 0 {
            f.housed = (f.housed + 0.005).min(1.0);
            f.body_clip = body_clip::HOUSE;
            f.body_frame = f.housed * house_end;
        } else {
            f.house_hold -= 1;
        }
    }

    /// Queues a sound at FLUDD's first emitter (or Mario when he has none).
    pub(crate) fn sound(&mut self, name: &'static str, info: Option<f32>) {
        let pos = self.emit_pos(0);
        self.events.push(Event::Sound(SoundCue { name, pos, info }));
    }

    /// Where emitter `index` is: the host's last reported muzzle, else the analytic stand-in.
    pub(crate) fn emit_pos(&self, index: usize) -> Vec3 {
        match self.fludd.as_ref() {
            Some(f) => match f.emitter(index) {
                Some((p, _)) => self.model_to_world(p),
                None => self.nozzle_pos(f.nozzle, index),
            },
            None => self.pos,
        }
    }

    /// Model space (decomp units, +Z forward, at Mario's feet) to the world.
    pub(crate) fn model_to_world(&self, v: Vec3) -> Vec3 {
        self.pos + self.rotate_model(v)
    }

    pub(crate) fn rotate_model(&self, v: Vec3) -> Vec3 {
        let (s, c) = (sin(self.model_yaw), cos(self.model_yaw));
        Vec3::new(v.x * c + v.z * s, v.y, -v.x * s + v.z * c)
    }

    /// The host's posed muzzle joints (`null_G_muzzle`, `null_G_muzzle2`) for `kind`, in
    /// Mario's model space: position and the joint's X axis (`TWaterGun::perform` copies
    /// them to `mEmitPos` after each animation pass). Water then leaves the drawn nozzle.
    pub fn set_emitters(&mut self, kind: NozzleKind, muzzles: &[(Vec3, Vec3)]) {
        if let Some(f) = self.fludd.as_mut() {
            f.emitters = [None; 2];
            for (i, m) in muzzles.iter().take(2).enumerate() {
                f.emitters[i] = Some(*m);
            }
            f.emitters_for = kind;
        }
    }

    /// What to draw for FLUDD now, if Mario has it.
    pub fn fludd_look(&self) -> Option<FluddLook> {
        let f = self.fludd.as_ref()?;
        let a = &f.nozzle_anims[f.nozzle as usize];
        Some(FluddLook {
            body_clip: f.body_clip,
            body_frame: f.body_frame,
            nozzle: f.nozzle,
            nozzle_clip: a.clip.and_then(|i| nozzle_clips(f.nozzle).get(i).copied()),
            nozzle_frame: a.ctrl.frame,
            gun_angle: f.gun_angle(),
            tilt: f.tilt,
            prop_angle: f.prop_angle,
        })
    }

    /// The upper-body clip over the chest and everything below it, with its frame, while
    /// pumping (`M3UModelMario` slot 1 on `jnt_chest`).
    pub fn upper_layer(&self) -> Option<(ClipName, f32)> {
        if !self.upper_on {
            return None;
        }
        self.upper_clip.map(|c| (c, self.upper_ctrl.frame))
    }
}

/// Clip lengths for the FLUDD clips, keyed by file stem, from an extracted `watergun2/`.
pub fn insert_fludd_clips(table: &mut AnimTable, watergun2: &std::path::Path) {
    let Ok(dirs) = std::fs::read_dir(watergun2) else {
        return;
    };
    for d in dirs.flatten() {
        let Ok(files) = std::fs::read_dir(d.path()) else {
            continue;
        };
        for e in files.flatten() {
            let path = e.path();
            if path.extension().and_then(|s| s.to_str()) != Some("bck") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if let Some(info) = std::fs::read(&path)
                .ok()
                .and_then(|b| AnimTable::parse_bck(&b))
            {
                table.insert(stem, info);
            }
        }
    }
}

/// Fallback lengths when the disc's clips are missing (frames from the shipped files).
pub(crate) fn default_info(name: &str) -> Option<ClipInfo> {
    let (frames, mode) = if name.ends_with("_shooting") {
        (30.0, LoopMode::Loop)
    } else if name.ends_with("_change_end") || name.ends_with("_change_start") {
        (5.0, LoopMode::Once)
    } else if name.ends_with("_shoot_end") || name.ends_with("_shoot_start") {
        (9.0, LoopMode::Once)
    } else if name.ends_with("house") {
        (119.0, LoopMode::Once)
    } else if name.ends_with("pmp") || name == "wg_pump" || name == "pump" {
        (23.0, LoopMode::Once)
    } else {
        return None;
    };
    Some(ClipInfo { frames, mode })
}
