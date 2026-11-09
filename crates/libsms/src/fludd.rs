//! FLUDD: tank, nozzles, trigger charging, recoil (Player/WaterGun.cpp, NozzleBase/Trigger/
//! Deform, and the upper-body state in Player/MarioUpper.cpp).
//!
//! The sim owns the tank and the nozzle state machines and applies recoil to Mario. It does
//! **not** simulate water droplets: each firing tick it reports a [`WaterEmit`] event and the
//! host spawns particles and applies damage. (Sunshine's `TModelWaterManager` is a particle
//! system with its own collision; that belongs to the host or a later module.)
//!
//! Known gaps (see docs/behavior-spec.md): the Spray nozzle's secondary "bomb" trigger and
//! the diving helmet. Yoshi's juice is the Yoshi nozzle here (`yoshi.rs` has the rest).

use glam::Vec3;

use crate::event::{Event, WaterEmit};
use crate::host::Host;
use crate::input::{Pad, lr_level};
use crate::mario::{Mario, Status, UpperState};
use crate::math::{cos, sin};
use crate::params::{Nozzle, NozzleKind, Params};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TriggerState {
    /// Not firing; R is up or hasn't charged.
    #[default]
    Inactive,
    /// Firing (`TNozzleTrigger::ACTIVE`).
    Active,
    /// Fired out; R must be released before it can charge again.
    Dead,
}

#[derive(Debug, Clone, Default)]
struct Slot {
    /// Continuous nozzles: ramping pressure gate (`unk372`).
    gate: u16,
    /// Continuous nozzles: current output `0..=1` (`unk378`).
    amount: f32,
    /// Continuous nozzles: the last rise in trigger pressure before scaling (`unk374`); the
    /// Spray's air noise is pitched by it.
    pressure: f32,
    /// Fractional particle accumulator (`unk37C`).
    emit_acc: f32,
    /// Water owed for drops at a host's spray rate (`limits.rs`), less than one unit.
    water_carry: f32,
    /// The Spray under pressure fades, pressed lightly: the trigger alone, before the gate,
    /// is short of full output, so the press is a short burst (not the game's: a host may
    /// weight these drops' hits, see [`crate::WaterEmit::burst`]).
    burst: bool,
    /// Trigger nozzles.
    state: TriggerState,
    timer: i16,
    /// Trigger charge (`unk388`).
    fill: f32,
    /// Pitch of the nozzle in angle units, positive up (`unk36E`).
    gun_angle: f32,
}

/// The secondary nozzles in [`crate::Options::cycle_nozzles`]' order (after Spray).
pub const CYCLE: [NozzleKind; 3] = [NozzleKind::Hover, NozzleKind::Rocket, NozzleKind::Turbo];

/// Where the game's L-button camera looks (`TNozzleBase::calcGunAngle`: while it's on, the
/// nozzle takes the camera's pitch as it is). `yaw` is the camera's heading, which the host
/// also turns a crouching Mario to (in the game the camera turns with him; a host whose
/// camera the player steers turns him with it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Aim {
    /// Heading, Mario's facing angle units.
    pub yaw: i16,
    /// The nozzle's pitch, angle units (0x8000 = half a turn), up positive.
    pub pitch: i16,
}

#[derive(Debug, Clone)]
pub struct Fludd {
    pub nozzle: NozzleKind,
    /// The nozzle `X` swaps with Spray.
    pub second: NozzleKind,
    /// Water in the tank, in the current nozzle's units (`mCurrentWater`).
    pub water: i32,
    slots: [Slot; 6],
    /// Target hover-nozzle tilts `[left, right]` set by steering (`unk1CC2/4`).
    pub tilt_target: [i16; 2],
    /// Smoothed tilts (`unk1CC8/C`), for drawing and emit direction.
    pub tilt: [f32; 2],
    /// Turbo propeller angle and speed (`unk1CD0/2`).
    pub prop_angle: f32,
    prop_speed: f32,
    /// 0 = Spray fitted, 1 = secondary nozzle fitted (`mSwitchToSecondNozzleProgress`).
    pub switch_progress: f32,
    switch_speed: f32,
    /// [`crate::Options::cycle_nozzles`]: the secondary nozzle a swing back from one
    /// secondary is going to (it turns round at the halfway point instead of fitting Spray).
    cycle_to: Option<NozzleKind>,
    /// FLUDD's handles: 1 = folded away, 0 = held out for pumping (`unk1CEC`). Water only
    /// leaves once it reaches 0.
    pub housed: f32,
    /// Ticks to wait before folding the handles back (`unk1CFA`).
    pub(crate) house_hold: i16,
    /// FLUDD's body clip and frame (`calcAnimation`).
    pub body_clip: crate::anim::ClipName,
    pub body_frame: f32,
    /// Each nozzle's clip state, by `NozzleKind`.
    pub(crate) nozzle_anims: [crate::fludd_anim::NozzleAnim; 6],
    /// Muzzles reported by the host, in Mario's model space, for `emitters_for`.
    pub(crate) emitters: [Option<(Vec3, Vec3)>; 2],
    pub(crate) emitters_for: NozzleKind,
    /// Particles accepted this tick (`mIsEmitWater`).
    pub emitted: u32,
    /// Ticks of `ms_nozzlesmoke` still owed after water last left a muzzle.
    smoke_hold: u8,
}

impl Fludd {
    pub fn new(p: &Params) -> Fludd {
        Fludd {
            nozzle: NozzleKind::Spray,
            second: NozzleKind::Hover,
            water: p.nozzle(NozzleKind::Spray).amount_max,
            slots: Default::default(),
            tilt_target: [0, 0],
            tilt: [0.0, 0.0],
            prop_angle: 0.0,
            prop_speed: 0.0,
            switch_progress: 0.0,
            switch_speed: 0.0,
            cycle_to: None,
            housed: 1.0,
            house_hold: 0,
            body_clip: crate::fludd_anim::body_clip::HOUSE,
            body_frame: 0.0,
            nozzle_anims: Default::default(),
            emitters: [None; 2],
            emitters_for: NozzleKind::Spray,
            emitted: 0,
            smoke_hold: 0,
        }
    }

    pub fn nozzle_params<'a>(&self, p: &'a Params) -> &'a Nozzle {
        p.nozzle(self.nozzle)
    }

    pub fn max_water(&self, p: &Params) -> i32 {
        self.nozzle_params(p).amount_max
    }

    /// Tank level as a fraction of the current nozzle's capacity.
    pub fn level(&self, p: &Params) -> f32 {
        self.water as f32 / self.max_water(p).max(1) as f32
    }

    pub fn trigger_state(&self) -> TriggerState {
        self.slots[self.nozzle as usize].state
    }

    /// A trigger nozzle's charge and its maximum (`TWaterGun::getPressure`/`getPressureMax`),
    /// while it's building (`isPressureOn`); what the HUD's gauge shows in red.
    pub fn pressure(&self, p: &Params) -> Option<(f32, f32)> {
        let fill = self.slots[self.nozzle as usize].fill;
        (self.nozzle.is_trigger() && fill > 0.0)
            .then(|| (fill, self.nozzle_params(p).inside_pressure_max))
    }

    /// Continuous-nozzle output (`unk378`), `0..=1`.
    pub fn spray_amount(&self) -> f32 {
        self.slots[self.nozzle as usize].amount
    }

    /// The nozzle's angle in angle units, as the game keeps it (`getGunAngle`): drawn, a negative
    /// angle lifts the barrel (the rest angle is `-mLAngleBase`) and the Spray's neck only turns
    /// for those. A host aim sets it for the head and sends the water by its own pitch.
    pub fn gun_angle(&self) -> f32 {
        self.slots[self.nozzle as usize].gun_angle
    }

    /// `canSpray`/`isEmitting`: is the current nozzle putting water out right now?
    pub fn can_spray(&self, p: &Params) -> bool {
        let _ = p;
        if self.water == 0 {
            return false;
        }
        let s = &self.slots[self.nozzle as usize];
        if self.nozzle.is_trigger() {
            s.state == TriggerState::Active
        } else {
            s.amount > 0.0
        }
    }

    /// Fits `to` at once, cancelling any backpack swing in progress. A host's radial menu uses
    /// this: the player has already chosen the nozzle, so the game's halfway change must not
    /// override it a moment later.
    pub fn fit_nozzle(&mut self, to: NozzleKind, p: &Params) {
        self.switch_speed = 0.0;
        self.switch_progress = if to == NozzleKind::Spray { 0.0 } else { 1.0 };
        self.cycle_to = None;
        self.change_nozzle(to, false, p);
    }

    /// `changeNozzle`: swaps the fitted nozzle, rescaling the tank to the new nozzle's capacity.
    pub fn change_nozzle(&mut self, to: NozzleKind, animate: bool, p: &Params) {
        let used = self.water as f32 / self.max_water(p).max(1) as f32;
        if to == NozzleKind::Spray {
            if animate {
                self.switch_progress = 0.0;
            }
        } else {
            self.second = to;
            if animate {
                self.switch_progress = 1.0;
            }
        }
        self.nozzle = to;
        self.slots[to as usize] = Slot::default();
        self.nozzle_anims[to as usize].init(to);
        self.water = (used * p.nozzle(to).amount_max as f32) as i32;
    }

    /// `changeBackup` (X): start swapping between Spray and the second nozzle.
    /// Returns the selection sound it starts (`startSoundSystemSE`).
    ///
    /// With `cycle` ([`crate::Options::cycle_nozzles`]), X steps through every nozzle in turn
    /// instead: Spray, Hover, Rocket, Turbo, Spray. Between two secondaries the backpack swings
    /// halfway back, changes nozzle and swings out again.
    pub fn swap_nozzle(&mut self, p: &Params, cycle: bool) -> Option<&'static str> {
        let mut sound = None;
        if self.switch_progress == 0.0 {
            sound = Some(Self::switch_sound(true));
            self.switch_speed = p.water_gun.change_speed;
            if cycle {
                self.second = CYCLE[0];
            }
        }
        if self.switch_progress == 1.0 {
            sound = Some(Self::switch_sound(false));
            self.switch_speed = -p.water_gun.change_speed;
            if cycle {
                self.cycle_to = CYCLE
                    .iter()
                    .position(|&k| k == self.nozzle)
                    .and_then(|i| CYCLE.get(i + 1))
                    .copied();
            }
        }
        sound
    }

    /// WaterGun.cpp::TWaterGun::changeBackup: 0x0812 out to the alternate,
    /// 0x0811 back to Spray. These are distinct from a new-nozzle pickup.
    pub(crate) fn switch_sound(to_alternate: bool) -> &'static str {
        if to_alternate { "MSD_SE_SY_SELECT_POMP_BACK" } else { "MSD_SE_SY_SELECT_POMP" }
    }

    pub fn is_switching(&self) -> bool {
        self.switch_speed != 0.0
    }

    /// `isSwitchingToSecondaryNozzle`.
    pub fn switching_to_second(&self) -> bool {
        self.switch_speed > 0.0
    }

    /// `isSwitchingToSprayNozzle`.
    pub fn switching_to_spray(&self) -> bool {
        self.switch_speed < 0.0
    }

    /// The host-reported muzzle `index` for the fitted nozzle, in Mario's model space.
    pub fn emitter(&self, index: usize) -> Option<(Vec3, Vec3)> {
        if self.emitters_for != self.nozzle {
            return None;
        }
        self.emitters.get(index).copied().flatten()
    }

    /// Refill the tank to the current nozzle's maximum.
    pub fn refill(&mut self, p: &Params) {
        self.water = self.max_water(p);
    }

    /// `TWaterGun::movement`: smoothing, propeller, nozzle swap progress, nozzle clip.
    fn movement(&mut self, p: &Params, table: &crate::anim::AnimTable) {
        if !self.can_spray(p) {
            self.tilt_target = [0, 0];
        }
        let k = p.water_gun.hover_smooth;
        for i in 0..2 {
            self.tilt[i] += (self.tilt_target[i] as f32 - self.tilt[i]) * k;
        }

        if self.nozzle == NozzleKind::Turbo {
            let amount = self.slots[self.nozzle as usize].amount;
            self.rotate_prop(amount, p);
        } else {
            self.prop_speed = 0.0;
            self.prop_angle = 0.0;
        }
        // Yoshi's juice never runs out (his own timer does), and FLUDD's handles stay put.
        if self.nozzle == NozzleKind::Yoshi {
            self.water = self.max_water(p);
            self.housed = 0.0;
        }

        if self.switch_speed != 0.0 {
            let before = self.switch_progress;
            let after = before + self.switch_speed;
            self.switch_progress = after;
            if before < 0.5 && 0.5 <= after {
                let to = self.second;
                self.change_nozzle(to, false, p);
            }
            if after < 0.5 && 0.5 <= before {
                match self.cycle_to.take() {
                    Some(to) => {
                        self.change_nozzle(to, false, p);
                        self.switch_speed = -self.switch_speed;
                    }
                    None => self.change_nozzle(NozzleKind::Spray, false, p),
                }
            }
            if self.switch_progress < 0.0 {
                self.switch_progress = 0.0;
                self.switch_speed = 0.0;
            }
            if self.switch_progress > 1.0 {
                self.switch_progress = 1.0;
                self.switch_speed = 0.0;
            }
        }
        self.nozzle_animation(table);
    }

    /// `rotateProp`.
    pub(crate) fn rotate_prop(&mut self, rotation: f32, p: &Params) {
        if self.nozzle == NozzleKind::Turbo {
            self.prop_speed += rotation * p.water_gun.nozzle_angle_y_speed;
            self.prop_speed *= p.water_gun.nozzle_angle_y_brake;
            let max = p.water_gun.nozzle_angle_y_speed_max as f32;
            if self.prop_speed > max {
                self.prop_speed = max;
            }
            self.prop_angle += self.prop_speed;
        } else {
            self.prop_speed = 0.0;
            self.prop_angle = 0.0;
        }
    }
}

/// How far the Spray nozzle's neck points below level with no rotation (angle units; measured
/// from the disc's model: its muzzle looks about 8 degrees down at `gun_angle` 0).
const AIM_DROOP: f32 = 1500.0;

impl Mario {
    /// What both FLUDD gauges draw as the trigger nozzle's charge, as (value, maximum): the nozzle's pressure, and
    /// for the Turbo the slower of that and its dash wind-up (`dash_speed` to 32), so a full gauge means the
    /// dash is ready, not only that the trigger has built pressure (host extension; the game's gauge
    /// shows the pressure alone). None while nothing is built.
    pub fn nozzle_gauge(&self) -> Option<(f32, f32)> {
        let f = self.fludd.as_ref()?;
        let pressure = f.pressure(&self.params);
        if f.nozzle != NozzleKind::Turbo {
            return pressure;
        }
        let (fill, max) = pressure.map_or((0.0, 1.0), |(n, d)| (n, d.max(1e-3)));
        let wind = (self.dash_speed / 32.0).clamp(0.0, 1.0);
        let level = (fill / max).clamp(0.0, 1.0).min(wind);
        (level > 0.0).then_some((level * max, max))
    }

    /// Host extension (`Options::aim_alt`): whether a press of the Turbo or Hover is an aimed one. It is decided as R is
    /// pressed (a valid target and the experiment on for that nozzle); the stream then follows the target, and
    /// when the target breaks the hold ends at once: pumping stays off until R is let go, and pressing it again
    /// starts afresh (like a Spray held too long). The Rocket resolves at its discharge instead.
    fn update_alt_aim(&mut self, r_level: f32) {
        let kind = self.fludd.as_ref().map(|f| f.nozzle);
        let alt = matches!(kind, Some(NozzleKind::Turbo | NozzleKind::Hover));
        let pressed = r_level > 0.0 && !self.alt_aim_r_down;
        self.alt_aim_r_down = r_level > 0.0;
        if r_level == 0.0 || !alt {
            self.alt_aim_latched = false;
            self.alt_aim_block = false;
            return;
        }
        if pressed {
            self.alt_aim_block = false;
            let on = match kind {
                Some(NozzleKind::Turbo) => self.options.aim_turbo,
                _ => self.options.aim_hover,
            };
            self.alt_aim_latched = self.options.aim_alt && on && self.aim_now().is_some();
        }
        if self.alt_aim_latched && self.aim_now().is_none() {
            self.alt_aim_latched = false;
            self.alt_aim_block = true;
        }
    }

    /// A B dive or a belly slide (dry or wet) with `Options::slide_dive_nozzles` on: the poses where the
    /// Rocket and Turbo push him along level instead of upward (host extension).
    pub(crate) fn horizontal_exhaust_pose(&self) -> bool {
        self.options.slide_dive_nozzles
            && !self.on_yoshi()
            && matches!(self.status, Status::JumpCatch | Status::Catch)
    }

    /// The aim the water follows now: the host's lock-on for the Spray, Yoshi and the rest, but for the alternate
    /// nozzles only what the experiment has switched on (`Options::aim_alt` and its per-nozzle switch): a
    /// Rocket charge that fired with a target (the precision slug), a Turbo or Hover press that began with one.
    pub(crate) fn gameplay_aim(&self) -> Option<crate::Aim> {
        match self.fludd.as_ref().map(|f| f.nozzle) {
            Some(NozzleKind::Rocket) => if self.rocket_precision { self.aim_now() } else { None },
            Some(NozzleKind::Turbo | NozzleKind::Hover) => if self.alt_aim_latched { self.aim_now() } else { None },
            _ => self.aim_now(),
        }
    }

    /// The exact gameplay emitter, shared by particle emission and asset-free hosts.
    pub fn water_emitter(&self, index: usize) -> Option<(Vec3, Vec3)> {
        self.water_emitter_ex(index, true)
    }

    /// [`Mario::water_emitter`], or with `aimed` false the direction his movement gives it with no lock-on (the Hover
    /// keeps its lift while its jets aim).
    pub(crate) fn water_emitter_ex(&self, index: usize, aimed: bool) -> Option<(Vec3, Vec3)> {
        let kind = self.fludd.as_ref()?.nozzle;
        let (pos, dir) = match self.fludd.as_ref().and_then(|f| f.emitter(index)) {
            _ if self.on_yoshi() => self.yoshi.mouth_world(),
            Some((p, d)) => (
                self.model_to_world(p),
                self.rotate_model(d).try_normalize().unwrap_or(Vec3::Z),
            ),
            None => (
                self.nozzle_pos(kind, index),
                self.nozzle_pitch_dir(kind, index),
            ),
        };
        // A host aim (the lock-on) sends the water where it points, whichever way Mario faces
        // or runs; not in the game, where the camera turns Mario instead. Its heading and
        // pitch, whether or not the drawn head has swung that far yet.
        let aim = if aimed { self.gameplay_aim() } else { None };
        let dir = match aim {
            Some(a) if !self.on_yoshi() => {
                let pitch = a.pitch as f32 * (std::f32::consts::PI / 32768.0);
                Vec3::new(
                    pitch.cos() * sin(a.yaw),
                    pitch.sin(),
                    pitch.cos() * cos(a.yaw),
                )
            }
            _ => dir,
        };
        let face = self.face_yaw;
        // Host extension (`Options::slide_dive_nozzles`): in a dive or belly slide the Rocket's and
        // the Turbo's exhaust goes level out the back along his travel, so their usual recoil drives him
        // forward (the Rocket's upright exhaust is authored to blow downward). Spray and Hover keep theirs.
        let dir = if self.horizontal_exhaust_pose()
            && matches!(kind, NozzleKind::Rocket | NozzleKind::Turbo)
            && aim.is_none()
        {
            Vec3::new(-sin(face), 0.0, -cos(face))
        } else {
            dir
        };
        // Host extension: in a corkscrew the Spray turns with him about the line of flight, as a spin jump's
        // turns about his height: it comes out of his belly (down, flat as he is) and goes round with the
        // roll, so a pass carpets what is below and to either side.
        let dir = if kind == NozzleKind::Spray
            && self.status == Status::JumpCatch
            && self.corkscrew > 0
            && !self.on_yoshi()
        {
            let roll = (self.model_roll as u16) as f32 * (std::f32::consts::TAU / 65536.0);
            let right = Vec3::new(cos(face), 0.0, -sin(face));
            (Vec3::NEG_Y * roll.cos() + right * roll.sin()).normalize()
        } else {
            dir
        };
        Some((pos, dir))
    }

    pub(crate) fn nozzle_pitch_dir(&self, kind: NozzleKind, index: usize) -> Vec3 {
        let f = self.fludd.as_ref().expect("fludd");
        let face = self.face_yaw;
        match kind {
            NozzleKind::Hover | NozzleKind::Underwater => {
                // Straight down, tilted fore/aft by the smoothed tilt (angle units).
                let t = f.tilt[index.min(1)] * (std::f32::consts::PI / 32768.0);
                Vec3::new(sin(face) * t.sin(), -t.cos(), cos(face) * t.sin())
            }
            NozzleKind::Rocket => Vec3::NEG_Y,
            NozzleKind::Turbo => {
                // Out the back.
                Vec3::new(-sin(face), 0.0, -cos(face))
            }
            _ => {
                // `gun_angle` counts the neck's way (a negative angle lifts the barrel), but the
                // world direction is up-positive, so negate it: otherwise the fallback sprays down
                // while the barrel is up, and up while it droops.
                let pitch = -f.gun_angle() * (std::f32::consts::PI / 32768.0);
                Vec3::new(
                    sin(face) * pitch.cos(),
                    pitch.sin(),
                    cos(face) * pitch.cos(),
                )
            }
        }
    }

    /// The procedural tank mount in world space, also used to place the fallback emitters.
    pub fn stand_in_tank_pose(&self) -> (Vec3, glam::Quat) {
        use crate::mario::{POSE_DIVE, POSE_SLIDE, STAND_IN_HEIGHT};
        use glam::Quat;
        let angle = std::f32::consts::PI / 32768.0;
        let mount = Vec3::new(0.0, 90.0, -35.0);
        if self.model_pitch != 0 {
            let yaw = Quat::from_rotation_y(f32::from(self.model_yaw) * angle);
            let (root, rotation) = Self::model_root_pose(self.pos, yaw, self.model_pitch, self.model_roll);
            return (root + rotation * mount, rotation);
        }
        let yaw = Quat::from_rotation_y(f32::from(self.face_yaw) * angle);
        if self.pose() & (POSE_DIVE | POSE_SLIDE) != 0 {
            return (
                self.pos + yaw * Vec3::new(0.0, 65.0, 20.0),
                yaw * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
            );
        }
        let rotation = yaw * Quat::from_rotation_x(f32::from(self.stand_in_jump_pitch()) * angle);
        let middle = Vec3::Y * (STAND_IN_HEIGHT * 0.5);
        (self.pos + middle + rotation * (mount - middle), rotation)
    }

    /// Authored approximations of `WaterGun.cpp::nozzleBmdData`'s muzzle joints
    /// relative to the tank. The slide keeps a steady outlet above the belly.
    pub(crate) fn nozzle_pos(&self, kind: NozzleKind, index: usize) -> Vec3 {
        use crate::mario::{POSE_DIVE, POSE_SLIDE};
        let (tank, rotation) = self.stand_in_tank_pose();
        let sliding = self.model_pitch == 0 && self.pose() & (POSE_DIVE | POSE_SLIDE) != 0;
        let outlet = match kind {
            NozzleKind::Hover | NozzleKind::Underwater => {
                let side = if index == 0 { -33.0 } else { 33.0 };
                Vec3::new(side, 29.0, -8.0)
            }
            NozzleKind::Rocket => Vec3::new(0.0, 10.0, -24.0),
            NozzleKind::Turbo => Vec3::new(0.0, 54.0, -15.0),
            _ if sliding => Vec3::new(0.0, 72.0, -5.0),
            _ => Vec3::new(0.0, 60.0, 47.0),
        };
        tank + rotation * outlet
    }

    pub(crate) fn emitter_count(kind: NozzleKind) -> usize {
        match kind {
            NozzleKind::Underwater | NozzleKind::Hover => 2,
            _ => 1,
        }
    }

    /// The decomp's `addVelocity` (forward speed cap 99).
    fn add_velocity(&mut self, dv: f32) {
        self.forward_vel += dv;
        if self.forward_vel > 99.0 {
            self.forward_vel = 99.0;
        }
    }

    /// `checkPumpEnable`.
    fn check_pump_enable(&self) -> bool {
        let Some(f) = self.fludd.as_ref() else {
            return false;
        };
        let np = f.nozzle_params(&self.params);
        // Host extension to MarioUpper.cpp::checkPumpEnable: B dive and belly slide
        // share `sldct`, which has no pump clip. Only the Spray may bypass that gate.
        let dive_spray = self.options.spray_during_dive
            && f.nozzle == NozzleKind::Spray
            && matches!(self.status, Status::JumpCatch | Status::Catch);
        // Host extension: the long fall's `thrown` table entry has `isPumpOK` false, so the game
        // offers no Hover or Rocket out of a fall of more than two seconds. Hosts with long drops
        // may ask for the pump anyway (the `pump` arm clip draws over the flail).
        // Host extension (`Options::slide_dive_nozzles`): the Rocket and Turbo pump in a B dive and a
        // belly slide, dry or wet, for a horizontal push (see `Mario::horizontal_exhaust`).
        let dive_nozzle = self.horizontal_exhaust_pose()
            && matches!(f.nozzle, NozzleKind::Rocket | NozzleKind::Turbo);
        let long_fall_pump = self.options.pump_in_long_fall
            && self.status == Status::Landing
            && self.anim.clip == "thrown";
        !self.on_yoshi()
            // Host extension: an aimed Turbo or Hover that lost its target stays off until R is let go.
            && !self.alt_aim_block
            && (crate::anim::pump_clip(self.anim.clip).is_some() || dive_spray || dive_nozzle || long_fall_pump)
            // Host extension: not while a use the host granted this press (`Limits::air_uses`) starts the next one.
            && !(self.status == Status::RocketLanding && np.rocket_type == 1 && !self.limit_granted())
            && !(f.nozzle.is_trigger() && f.trigger_state() == TriggerState::Dead)
            && !f.is_switching()
    }

    /// `checkPumping`: enter the pumping stance on R.
    fn check_pumping(&mut self, r_level: f32) {
        if r_level > 0.0 && self.upper != UpperState::Pumping {
            self.upper = UpperState::Pumping;
            self.pump_cooldown = 0;
            return;
        }
        if self.status == Status::Squat && self.upper == UpperState::Idle {
            self.upper = UpperState::HoldingPump;
            self.pump_cooldown = 0;
            return;
        }
        if self.fludd_emitting {
            self.upper = UpperState::Pumping;
            self.pump_cooldown = 0;
        }
    }

    /// `stateMachineUpper` for the FLUDD stances.
    pub(crate) fn upper_state_update(&mut self, pad: &Pad) {
        let r_level = lr_level(pad.r_analog, &self.params.controller);
        self.update_alt_aim(r_level);
        match self.upper {
            UpperState::Pumping => {
                if !self.check_pump_enable() {
                    self.upper_ctrl.frame = 0.0;
                    self.upper = UpperState::Idle;
                }
                if r_level == 0.0 {
                    self.upper = UpperState::HoldingPump;
                    self.pump_cooldown = self.params.upper_body.pump_wait_time as i32;
                }
            }
            UpperState::HoldingPump => {
                if !self.check_pump_enable() {
                    self.upper_ctrl.frame = 0.0;
                    self.upper = UpperState::Idle;
                }
                if self.pump_cooldown != 0 {
                    self.pump_cooldown -= 1;
                } else {
                    self.upper_ctrl.frame = 0.0;
                    self.upper = UpperState::Idle;
                }
                self.check_pumping(r_level);
            }
            UpperState::Idle => {
                if self.check_pump_enable() {
                    self.check_pumping(r_level);
                }
            }
            UpperState::HoldingObject => {
                if self.status == Status::Putting || self.held.is_none() {
                    self.upper = UpperState::Idle;
                }
            }
        }
        if self.fludd.is_none() && self.upper != UpperState::HoldingObject {
            self.upper = UpperState::Idle;
        }
    }

    /// The turbo nozzle's dash charge (`checkController`): runs the stick magnitude up to a
    /// dash and, once charged, sets `fludd_emitting` (the dash/broad-jump flag).
    pub(crate) fn turbo_dash_control(&mut self, _pad: &Pad, r_level: f32) {
        let Some(f) = self.fludd.as_mut() else { return };
        let m = self.params.mario.clone();
        let turbo = f.nozzle == NozzleKind::Turbo;
        let state_active = f.trigger_state() == TriggerState::Active;
        let mut dash_began = false;
        self.turbo_held = turbo && self.upper == UpperState::Pumping && r_level > 0.0 && f.water > 0;
        if self.turbo_held {
            if self.intended_mag == 0.0 {
                self.intended_yaw = self.face_yaw;
            }
            self.dash_speed += m.dash_acc;
            if self.dash_speed > 32.0 {
                self.dash_speed = 32.0;
                self.dash_timer += 1;
                if self.dash_timer as f32 > m.dash_start_time as f32 {
                    self.dash_timer = m.dash_start_time as i32;
                    if !self.fludd_emitting && state_active {
                        self.fludd_emitting = true;
                        dash_began = true;
                    }
                }
                if self.status != Status::Run {
                    self.dash_timer = 0;
                    self.fludd_emitting = false;
                }
            } else {
                self.dash_timer = 0;
                self.fludd_emitting = false;
            }
            self.intended_mag = self.dash_speed;
            let sp = self.dash_speed;
            f.rotate_prop(sp, &self.params);
        } else {
            if self.dash_speed > 0.1 {
                if self.intended_mag == 0.0 {
                    self.intended_yaw = self.face_yaw;
                }
                self.dash_speed *= m.dash_brake;
                self.intended_mag = self.dash_speed;
            } else {
                self.dash_speed = 0.0;
            }
            self.dash_timer = 0;
            self.fludd_emitting = false;
        }
        if dash_began {
            // `startSoundActor(MSD_SE_PO_SNIPER_TRIGGER)` on Mario, as the dash begins.
            self.cue("MSD_SE_PO_SNIPER_TRIGGER");
        }
    }

    /// `gunExec`: per-tick FLUDD update (nozzle trigger, emission, recoil, tank).
    pub(crate) fn gun_exec(&mut self, host: &mut dyn Host, pad: &Pad) {
        if self.fludd.is_none() {
            return;
        }
        let r_level = lr_level(pad.r_analog, &self.params.controller);
        if let Some(f) = self.fludd.as_mut() {
            f.emitted = 0;
        }

        self.nozzle_movement(host, r_level, pad);

        let params_ptr = self.params.clone();
        if self.in_water {
            // TWaterGun::suck: standing in water, R draws water in at `suck_rate` x the
            // trigger's pressure instead of spraying it.
            let pressure = (r_level * 150.0) as i32;
            let mut sucked = false;
            if let Some(f) = self.fludd.as_mut() {
                let rate = (pressure as f32 * params_ptr.nozzle(f.nozzle).suck_rate) as i32;
                if f.nozzle != NozzleKind::Yoshi && rate > 0 {
                    let max = f.max_water(&params_ptr);
                    f.water = (f.water + rate).min(max);
                    sucked = f.water < max;
                }
            }
            if sucked {
                self.sound("MSD_SE_PO_SUCK_WATER_B", None);
            }
            // Only a full tank sprays there, and it stays full (`resetWaterToFull`).
            let full = self
                .fludd
                .as_ref()
                .is_some_and(|f| f.water >= f.max_water(&params_ptr));
            if full && self.upper == UpperState::Pumping {
                self.emit(host);
                self.refill_water();
            }
        } else if self.upper == UpperState::Pumping {
            self.emit(host);
        }
        self.nozzle_smoke();
        // Riding, R spits juice without pumping (`onYoshi() && mAnalogR > 0`).
        if self.on_yoshi() && pad.r_analog > 0 {
            self.emit(host);
        }

        let x_edge = pad.x && !self.pad_prev.x;
        if x_edge && self.status != Status::BackJump && !self.on_yoshi() {
            let cycle = self.options.cycle_nozzles;
            let sound = self
                .fludd
                .as_mut()
                .and_then(|f| f.swap_nozzle(&params_ptr, cycle));
            if let Some(name) = sound {
                self.sound(name, None);
            }
        }
        let anims = std::mem::take(&mut self.anims);
        if let Some(f) = self.fludd.as_mut() {
            let before = f.nozzle;
            f.movement(&params_ptr, &anims);
            if f.nozzle != before {
                host.event(Event::NozzleChanged(f.nozzle));
            }
        }
        self.anims = anims;

        if self.options.unlimited_water {
            self.refill_water();
            self.was_full = true;
        }
        let (water, max) = match self.fludd.as_ref() {
            Some(f) => (f.water, f.max_water(&params_ptr)),
            None => return,
        };
        // `soundMovement`: R pressed with the tank dry puffs empty air.
        if self.input.r_pressed && water == 0 {
            self.sound("MSD_SE_PO_ACTION_ON_EMPTY", None);
        }
        // `TGCConsole2`'s gauge chimes as the tank fills up (`SY_PO_WATER_FULL`).
        let full = water >= max;
        if full && !self.was_full {
            self.sound("MSD_SE_SY_PO_WATER_FULL", None);
        }
        self.was_full = full;
    }

    /// `TNozzleBase/Trigger/Deform::movement` for the fitted nozzle.
    fn nozzle_movement(&mut self, host: &mut dyn Host, r_level: f32, pad: &Pad) {
        let squatting = self.status == Status::Squat;
        let fades = self.options.spray_pressure_fades;
        let hover_time = self.limits.hover_time;
        let spray_fade = self.limits.spray_fade;
        let aim = self.gameplay_aim();
        let upper = self.upper;
        let in_water = self.in_water;
        let p = self.params.clone();
        let Some(f) = self.fludd.as_mut() else { return };
        let kind = f.nozzle;
        let np = p.nozzle(kind).clone();
        let water = f.water;
        let max_water = np.amount_max;
        let (mut charging, mut fired) = (false, None);

        if kind.is_trigger() {
            let s = &mut f.slots[kind as usize];
            if water <= 0 {
                s.state = TriggerState::Inactive;
                s.timer = 0;
                s.fill = 0.0;
                return;
            }
            if s.state == TriggerState::Active {
                s.timer -= 1;
                if upper != UpperState::Pumping || s.timer <= 0 {
                    s.state = TriggerState::Dead;
                    s.fill = 0.0;
                    s.timer = 0;
                }
            }
            let mut can = upper == UpperState::Pumping;
            if in_water && water < max_water {
                can = false;
            }
            if can {
                s.fill += 150.0 * r_level;
                if s.state == TriggerState::Inactive && r_level > 0.0 {
                    // The charging hiss, every tick it builds (`MSD_SE_SY_NEWP_AIR_TAME`).
                    charging = true;
                }
            }
            s.fill -= np.inside_pressure_dec;
            if s.fill < 0.0 {
                s.fill = 0.0;
            }
            if s.fill > np.inside_pressure_max {
                s.fill = np.inside_pressure_max;
                if s.state == TriggerState::Inactive {
                    s.state = TriggerState::Active;
                    s.timer = crate::limits::trigger_time(kind, np.trigger_time, hover_time);
                    host.event(Event::TriggerFired(kind));
                    // `unk38C`, set in `TWaterGun::init`; Hover has none.
                    fired = match kind {
                        NozzleKind::Rocket => Some("MSD_SE_PO_ROCKET_TRIGGER"),
                        NozzleKind::Turbo => Some("MSD_SE_PO_SNIPER_TRIGGER"),
                        _ => None,
                    };
                }
            }
            if s.state == TriggerState::Dead {
                s.fill = 0.0;
                if r_level == 0.0 {
                    s.state = TriggerState::Inactive;
                }
            }
        } else {
            let s = &mut f.slots[kind as usize];
            if water > 0 {
                // TNozzleBase::movement: the output follows how far the trigger is pressed
                // beyond the ramping gate.
                let var1 = (256.0 * r_level * 150.0) as i32;
                s.burst = fades
                    && kind == NozzleKind::Spray
                    && var1 as f32 * 0.000_015_258_789 * np.emit_pow_scale < 1.0;
                if !fades {
                    s.amount = var1 as f32 * 0.000_015_258_789;
                    if var1 > s.gate as i32 {
                        s.pressure = s.amount;
                    }
                    s.gate = var1 as u16;
                } else if var1 > s.gate as i32 {
                    s.amount = (var1 - s.gate as i32) as f32 * 0.000_015_258_789;
                    s.pressure = s.amount;
                    s.gate = s.gate.wrapping_add(crate::limits::gate_rate(
                        kind,
                        np.trigger_rate,
                        spray_fade,
                    ));
                    if var1 < s.gate as i32 {
                        s.gate = var1 as u16;
                    }
                } else {
                    s.amount = 0.0;
                    s.gate = var1 as u16;
                }
                if matches!(kind, NozzleKind::Spray | NozzleKind::Yoshi) {
                    // TNozzleDeform::movement
                    s.amount = (s.amount * np.emit_pow_scale).min(1.0);
                }
            }
        }

        // calcGunAngle
        let s = &mut f.slots[kind as usize];
        let rocket_active = kind == NozzleKind::Rocket && s.state == TriggerState::Active;
        if let Some(a) = aim {
            // The L-button camera: the nozzle takes its pitch, unclamped and at once. The neck's
            // angle counts the other way round (negative lifts the barrel: the rest angle,
            // `-mLAngleBase`, points it up a little) from a rest that droops a little itself.
            s.gun_angle = if kind == NozzleKind::Yoshi {
                a.pitch as f32
            } else {
                -(a.pitch as f32) - AIM_DROOP
            };
        } else {
            let target = if squatting {
                (s.gun_angle + pad.stick.y * 10_000.0).clamp(np_min(&p, kind), np_max(&p, kind))
            } else {
                (-np_base(&p, kind)).clamp(np_min(&p, kind), np_max(&p, kind))
            };
            s.gun_angle += (target - s.gun_angle) * np_chase(&p, kind);
        }
        // Host extension (`Options::aim_rocket`): a charge that fires with a valid target is the precision slug, decided
        // here at the discharge (no launch, one shot); otherwise the Rocket's own movement behavior.
        if kind == NozzleKind::Rocket {
            if !rocket_active {
                self.rocket_precision = false;
                self.precision_fired = false;
            } else if fired.is_some() {
                self.rocket_precision =
                    self.options.aim_alt && self.options.aim_rocket && self.aim_now().is_some();
                self.precision_fired = false;
            }
        }
        if charging {
            self.sound("MSD_SE_SY_NEWP_AIR_TAME", None);
        }
        if let Some(name) = fired {
            self.sound(name, None);
        }
    }

    /// `TNozzleTrigger::animation` / `TNozzleDeform::animation`, last lines: while water leaves
    /// the nozzle (`mIsEmitWater`), `ms_nozzlesmoke` is bound to each muzzle. Asked for every tick it
    /// lasts; a few ticks of grace stand in for the game's 30 Hz frame, since a light Spray emits
    /// on some ticks only. Yoshi's mouth has no muzzle.
    fn nozzle_smoke(&mut self) {
        let Some(f) = self.fludd.as_mut() else { return };
        if f.emitted > 0 {
            f.smoke_hold = 4;
        }
        if f.smoke_hold == 0 {
            return;
        }
        f.smoke_hold -= 1;
        let kind = f.nozzle;
        if kind == NozzleKind::Yoshi {
            return;
        }
        for i in 0..Self::emitter_count(kind) {
            self.fx("ms_nozzlesmoke", crate::event::FxAt::Muzzle(i as u8));
        }
    }

    /// `TWaterGun::emit` + the nozzle's `emit`: fires water and applies recoil.
    fn emit(&mut self, host: &mut dyn Host) {
        let p = self.params.clone();
        let Some(f) = self.fludd.as_ref() else { return };
        let kind = f.nozzle;
        // The Turbo puts no water out while he spins (a spin jump) or corkscrews (host extension).
        if kind == NozzleKind::Turbo && (self.corkscrew > 0 || self.spinning()) {
            return;
        }
        // The handles have to be out first (`unk1CEC > 0` returns), unless riding.
        if !self.on_yoshi() && f.housed > 0.0 {
            return;
        }
        // `TNozzleDeform::emit`: no juice before Yoshi has eaten.
        if kind == NozzleKind::Yoshi && self.yoshi.color == 0 {
            return;
        }
        for i in 0..Self::emitter_count(kind) {
            self.emit_one(host, kind, i, &p);
        }
        let Some(f) = self.fludd.as_ref() else { return };
        if f.water <= 0 {
            return;
        }
        let s = &f.slots[kind as usize];
        let (amount, pressure, emitted) = (s.amount, s.pressure, f.emitted);
        // The held sounds `TWaterGun::emit` starts after the nozzle emits.
        match kind {
            NozzleKind::Spray => {
                // The Spray case falls through into the Turbo one.
                self.sound("MSD_SE_PO_NORMAL_NOZZLE_IMI", Some(pressure));
                self.sound("MSD_SE_PO_WATER_HI", Some(amount));
            }
            NozzleKind::Turbo | NozzleKind::Yoshi => self.sound("MSD_SE_PO_WATER_HI", Some(amount)),
            NozzleKind::Underwater => self.sound("MSD_SE_PO_HOVER", None),
            NozzleKind::Hover if emitted > 0 => self.sound("MSD_SE_PO_HOVER", None),
            _ => {}
        }
    }

    fn emit_one(&mut self, host: &mut dyn Host, kind: NozzleKind, index: usize, p: &Params) {
        let np = p.nozzle(kind).clone();
        let rate = self.limit_spray_rate(kind);
        // `getEmitPosDirSpeed`: the drawn muzzle when the host reported it; riding, Yoshi's
        // tongue joint (`getEmitMtx`).
        let (pos, dir) = self.water_emitter(index).expect("FLUDD emission");
        let face = self.face_yaw;
        // Host extension (the experimental aimed modes, `Options::aim_alt`): see `gameplay_aim`.
        let aimed = self.gameplay_aim().is_some();
        let precision = kind == NozzleKind::Rocket && self.rocket_precision;
        if precision && self.precision_fired {
            return;
        }
        let turbo_aimed = kind == NozzleKind::Turbo && aimed;
        let inherit = if self.juice_mouth && kind == NozzleKind::Spray {
            // The skin's juice keeps Yoshi's full carried motion (`emit_carried` exempts it
            // from the droplet speed cap); ordinary spray keeps vanilla's eighth.
            self.vel
        } else {
            // Vanilla's eighth of his horizontal speed. Host extension: `vel` is the vanilla-scale
            // speed and his travel is multiplied by the host's speed and any jump boost (a Wax
            // Quail), so the part of that above 1x is carried in full, or he outruns his own water.
            let travel = if self.limits_airborne() { self.limit_air_speed() } else { self.limit_ground_speed() };
            let k = 0.125 + if travel.is_finite() { (travel - 1.0).clamp(0.0, 10.0) } else { 0.0 };
            Vec3::new(self.vel.x * k, 0.0, self.vel.z * k)
        };
        let (mut size, mut dir_tremble) = (np.size, np.dir_tremble);

        let (count, attack, power, reaction);
        let mut burst = false;
        {
            let Some(f) = self.fludd.as_mut() else { return };
            if f.water <= 0 {
                return;
            }
            let s = &mut f.slots[kind as usize];
            if kind.is_trigger() {
                // TNozzleTrigger::emit
                if s.state != TriggerState::Active {
                    return;
                }
                let pressure = s.fill / np.inside_pressure_max;
                if precision {
                    // The one slug: a single drop, as fast as `PRECISION_SPEED_MULT` times the Rocket's fastest, hitting for
                    // `PRECISION_SPRAY_SHARE` of what the Spray could deal in the same wind-up, narrow and straight.
                    let sp = p.nozzle(NozzleKind::Spray);
                    let windup = np.inside_pressure_max / (150.0 - np.inside_pressure_dec).max(1.0);
                    let spray = sp.attack as f32 * sp.num * windup;
                    count = 1;
                    attack = (spray * PRECISION_SPRAY_SHARE).clamp(1.0, i16::MAX as f32) as i16;
                    power = np.emit_pow * PRECISION_SPEED_MULT;
                    reaction = 0.0;
                    size = np.size * PRECISION_SIZE;
                    dir_tremble = 0.0;
                } else {
                    let num_rate = if turbo_aimed { TURBO_AIM_RATE } else { 1.0 };
                    s.emit_acc += (pressure * (np.num - np.num_min) + np.num_min) * num_rate;
                    let n = s.emit_acc as i32;
                    if n == 0 {
                        return;
                    }
                    s.emit_acc -= n as f32;
                    count = n;
                    attack = (pressure * (np.attack - np.attack_min) as f32 + np.attack_min as f32)
                        as i16;
                    power = pressure * (np.emit_pow - np.emit_pow_min) + np.emit_pow_min;
                    reaction =
                        pressure * (np.reaction_pow - np.reaction_pow_min) + np.reaction_pow_min;
                    if turbo_aimed {
                        dir_tremble *= TURBO_AIM_TREMBLE;
                    }
                }
            } else if matches!(kind, NozzleKind::Spray | NozzleKind::Yoshi) {
                // TNozzleDeform::emit (without the secondary bomb trigger)
                let a = s.amount;
                burst = s.burst;
                if !(a > 0.0) {
                    return;
                }
                s.emit_acc += (a * (np.num - np.num_min) + np.num_min) * rate;
                let n = s.emit_acc as i32;
                if n == 0 {
                    return;
                }
                s.emit_acc -= n as f32;
                count = n;
                attack = (a * (np.attack - np.attack_min) as f32 + np.attack_min as f32) as i16;
                power = a * (np.emit_pow - np.emit_pow_min) + np.emit_pow_min;
                reaction = a * (np.reaction_pow - np.reaction_pow_min) + np.reaction_pow_min;
                dir_tremble = a * (np.dir_tremble - np.dir_tremble_min) + np.dir_tremble_min;
                // Size grows with pressure between `mSizeMinPressure` and `mSizeMaxPressure`.
                // The decomp's `(min - a) / (max - a)` is negative inside the range (the
                // function is NonMatching); this is the plain ramp.
                let t = if a < np.size_min_pressure {
                    0.0
                } else if a < np.size_max_pressure {
                    (a - np.size_min_pressure) / (np.size_max_pressure - np.size_min_pressure)
                } else {
                    1.0
                };
                size = t * (np.size - np.size_min) + np.size_min;
            } else {
                // TNozzleBase::emit
                let a = s.amount;
                if !(a > 0.0) {
                    return;
                }
                s.emit_acc += np.num;
                let n = s.emit_acc as i32;
                if n == 0 {
                    return;
                }
                s.emit_acc -= n as f32;
                count = n;
                attack = np.attack;
                power = np.emit_pow * a * np.emit_ctrl + np.emit_pow * (1.0 - np.emit_ctrl);
                reaction = np.reaction_pow * power;
            }
            // emitRequest accepts every particle in this model. A host's spray rate
            // (`limits.rs`) takes that much less water a drop, so the tank lasts as long.
            // The precision slug costs what its whole charge would have (`num` drops for `trigger_time` ticks).
            let cost_count = if precision {
                ((np.num * np.trigger_time as f32).round() as i32).max(1)
            } else {
                count
            };
            let cost = if rate == 1.0 {
                cost_count * np.dec_rate as i32
            } else {
                let c = count as f32 * np.dec_rate as f32 / rate + s.water_carry;
                let whole = c.floor();
                s.water_carry = c - whole;
                whole as i32
            };
            f.water = (f.water - cost).max(0);
            f.emitted += count as u32;
        }

        host.event(Event::WaterEmit(WaterEmit {
            nozzle: kind,
            pos,
            dir,
            power,
            count: count as u32,
            attack,
            hit_radius: np.hit_radius,
            hit_height: np.hit_height,
            inherit,
            size,
            size_tremble: if precision { 0.0 } else { np.size_tremble },
            dir_tremble,
            pow_tremble: if precision { 0.0 } else { np.pow_tremble },
            particle_type: np.particle_type,
            burst,
            spin: self.spinning(),
        }));

        // Recoil: `addVelocity(reaction * -(dir . forward) * power)`; vertical reaction pushes
        // along -dir.y.
        if precision {
            // The slug's kick: opposite the shot vector, `PRECISION_KICK` (a fraction of the Rocket's own launch), and
            // nothing else. In a dive or slide it takes the place of the horizontal boost.
            self.precision_fired = true;
            let dir_scale = -dir.x * sin(face) - dir.z * cos(face);
            if self.status == Status::Catch {
                self.push_along(PRECISION_KICK * dir_scale, 99.0);
            } else {
                self.add_velocity(PRECISION_KICK * dir_scale);
            }
            self.vel.y += -dir.y * PRECISION_KICK;
            return;
        }
        // The Hover keeps its lift while its jets aim: the recoil comes from where its jets point without the aim.
        let rdir = if kind == NozzleKind::Hover && aimed {
            self.water_emitter_ex(index, false).map_or(dir, |(_, d)| d)
        } else {
            dir
        };
        let dir_scale = -rdir.x * sin(face) - rdir.z * cos(face);
        let (reaction_pow, reaction_y) = if kind.is_trigger() || kind == NozzleKind::Spray {
            (reaction * dir_scale * power, -rdir.y * power * np.reaction_y)
        } else {
            (reaction * dir_scale, -rdir.y * power * np.reaction_y)
        };
        // An aimed Turbo's thrust is steady and controlled, and away from the target.
        let reaction_pow = if turbo_aimed { reaction_pow * TURBO_AIM_THRUST_SCALE } else { reaction_pow };
        let reaction_y = if kind == NozzleKind::Rocket {
            reaction_y * self.options.rocket_thrust
        } else {
            reaction_y
        };
        self.add_velocity(reaction_pow);
        self.vel.y += reaction_y;
        // Host extension (`Options::slide_dive_nozzles`): the Rocket's thrust is its vertical term, nothing
        // for a level exhaust, so in a dive or slide the same thrust drives him along instead.
        if self.horizontal_exhaust_pose() && self.status == Status::Catch {
            // A slide's speed is its own vector: the exhaust's recoil goes into it too.
            let cap = 99.0;
            self.push_along(reaction_pow, cap);
        }
        if kind == NozzleKind::Rocket && self.horizontal_exhaust_pose() && !aimed {
            let push = power * np.reaction_y * self.options.rocket_thrust;
            if self.status == Status::Catch {
                self.push_along(push, 99.0);
            } else {
                self.add_velocity(push);
            }
        }
        if !(kind.is_trigger() || kind == NozzleKind::Spray) {
            // TNozzleBase::emit also pushes Mario's horizontal velocity.
            self.vel.x -= rdir.x * reaction;
            self.vel.z -= rdir.z * reaction;
        }
    }
}

/// The experimental aimed modes' tuning (host extension; Sunshine has none of these, so there are no disc values).
/// The Rocket's precision slug hits for this share of what the Spray could deal in the same wind-up.
const PRECISION_SPRAY_SHARE: f32 = 0.5;
/// How much faster than the Rocket's fastest drop the slug flies.
const PRECISION_SPEED_MULT: f32 = 2.0;
/// The slug's size next to a Rocket drop's (narrow).
const PRECISION_SIZE: f32 = 0.6;
/// Speed the slug kicks him back, opposite the shot (a fraction of the Rocket's own climb).
const PRECISION_KICK: f32 = 8.0;
/// An aimed Turbo fires this many times as many drops a tick, spread this much less, and thrusts this much less.
const TURBO_AIM_RATE: f32 = 3.0;
const TURBO_AIM_TREMBLE: f32 = 0.5;
const TURBO_AIM_THRUST_SCALE: f32 = 0.5;

fn np_min(p: &Params, k: NozzleKind) -> f32 {
    p.nozzle(k).l_angle_min as f32
}
fn np_max(p: &Params, k: NozzleKind) -> f32 {
    p.nozzle(k).l_angle_max as f32
}
fn np_base(p: &Params, k: NozzleKind) -> f32 {
    p.nozzle(k).l_angle_base as f32
}
fn np_chase(p: &Params, k: NozzleKind) -> f32 {
    p.nozzle(k).l_angle_chase
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fitting_a_nozzle_cancels_a_backpack_swing() {
        let p = crate::params::Params::default();
        let mut f = Fludd::new(&p);
        f.swap_nozzle(&p, true);
        assert!(f.is_switching());
        f.fit_nozzle(NozzleKind::Hover, &p);
        assert!(!f.is_switching());
        assert_eq!(f.nozzle, NozzleKind::Hover);
    }

    #[test]
    fn a_spray_barrel_pointed_up_sends_the_fallback_direction_up() {
        let mut m = crate::Sim::new(
            crate::params::Params::default(),
            crate::anim::AnimTable::default(),
            Vec3::ZERO,
            0,
        )
        .mario;
        m.give_fludd();
        // `gun_angle` counts the neck's way: a negative angle lifts the barrel (the game's
        // `-mLAngleBase` rest), so a barrel pitched up must spray up, not down.
        m.fludd.as_mut().unwrap().slots[NozzleKind::Spray as usize].gun_angle = -(0x4000 as f32);
        let dir = m.nozzle_pitch_dir(NozzleKind::Spray, 0);
        assert!(
            dir.y > 0.9,
            "a barrel pointed straight up must fall back upward, got {dir:?}"
        );
    }
}
