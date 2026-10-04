//! FLUDD: tank, nozzles, trigger charging, recoil (Player/WaterGun.cpp, NozzleBase/Trigger/
//! Deform, and the upper-body state in Player/MarioUpper.cpp).
//!
//! The sim owns the tank and the nozzle state machines and applies recoil to Mario. It does
//! **not** simulate water droplets: each firing tick it reports a [`WaterEmit`] event and the
//! host spawns particles and applies damage. (Sunshine's `TModelWaterManager` is a particle
//! system with its own collision; that belongs to the host or a later module.)
//!
//! Known gaps (see docs/behavior-spec.md): the Spray nozzle's secondary "bomb" trigger,
//! Yoshi juice, the diving helmet, and the nozzle-swap animation timing.

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
    /// Fractional particle accumulator (`unk37C`).
    emit_acc: f32,
    /// Trigger nozzles.
    state: TriggerState,
    timer: i16,
    /// Trigger charge (`unk388`).
    fill: f32,
    /// Pitch of the nozzle in angle units, positive up (`unk36E`).
    gun_angle: f32,
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

    /// Continuous-nozzle output (`unk378`), `0..=1`.
    pub fn spray_amount(&self) -> f32 {
        self.slots[self.nozzle as usize].amount
    }

    /// Nozzle pitch in angle units, positive up.
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
        self.water = (used * p.nozzle(to).amount_max as f32) as i32;
    }

    /// `changeBackup` (X): start swapping between Spray and the second nozzle.
    pub fn swap_nozzle(&mut self, p: &Params) {
        if self.switch_progress == 0.0 {
            self.switch_speed = p.water_gun.change_speed;
        }
        if self.switch_progress == 1.0 {
            self.switch_speed = -p.water_gun.change_speed;
        }
    }

    pub fn is_switching(&self) -> bool {
        self.switch_speed != 0.0
    }

    /// Refill the tank to the current nozzle's maximum.
    pub fn refill(&mut self, p: &Params) {
        self.water = self.max_water(p);
    }

    /// `TWaterGun::movement`: smoothing, propeller, nozzle swap progress.
    fn movement(&mut self, p: &Params) {
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

        if self.switch_speed != 0.0 {
            let before = self.switch_progress;
            let after = before + self.switch_speed;
            self.switch_progress = after;
            if before < 0.5 && 0.5 <= after {
                let to = self.second;
                self.change_nozzle(to, false, p);
            }
            if after < 0.5 && 0.5 <= before {
                self.change_nozzle(NozzleKind::Spray, false, p);
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

impl Mario {
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
                let pitch = f.gun_angle() * (std::f32::consts::PI / 32768.0);
                Vec3::new(
                    sin(face) * pitch.cos(),
                    pitch.sin(),
                    cos(face) * pitch.cos(),
                )
            }
        }
    }

    /// Where the nozzle's water leaves, if the host doesn't supply a skeleton pose. Approximate:
    /// chest height, a little in front of Mario for forward nozzles.
    pub(crate) fn nozzle_pos(&self, kind: NozzleKind, index: usize) -> Vec3 {
        let face = self.face_yaw;
        match kind {
            NozzleKind::Hover | NozzleKind::Underwater => {
                let side = if index == 0 { -35.0 } else { 35.0 };
                self.pos
                    + Vec3::new(
                        side * cos(face) - 5.0 * sin(face),
                        65.0,
                        -side * sin(face) - 5.0 * cos(face),
                    )
            }
            NozzleKind::Rocket => self.pos + Vec3::new(-20.0 * sin(face), 50.0, -20.0 * cos(face)),
            NozzleKind::Turbo => self.pos + Vec3::new(-30.0 * sin(face), 90.0, -30.0 * cos(face)),
            _ => self.pos + Vec3::new(40.0 * sin(face), 85.0, 40.0 * cos(face)),
        }
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
        !(self.status == Status::RocketLanding && np.rocket_type == 1)
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
        match self.upper {
            UpperState::Pumping => {
                if !self.check_pump_enable() {
                    self.upper = UpperState::Idle;
                }
                if r_level == 0.0 {
                    self.upper = UpperState::HoldingPump;
                    self.pump_cooldown = 5;
                }
            }
            UpperState::HoldingPump => {
                if !self.check_pump_enable() {
                    self.upper = UpperState::Idle;
                }
                if self.pump_cooldown != 0 {
                    self.pump_cooldown -= 1;
                } else {
                    self.upper = UpperState::Idle;
                }
                self.check_pumping(r_level);
            }
            UpperState::Idle => {
                if self.check_pump_enable() {
                    self.check_pumping(r_level);
                }
            }
        }
        if self.fludd.is_none() {
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
        if turbo && self.upper == UpperState::Pumping && r_level > 0.0 && f.water > 0 {
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
    }

    /// `gunExec`: per-tick FLUDD update (nozzle trigger, emission, recoil, tank).
    pub(crate) fn gun_exec(&mut self, host: &mut dyn Host, pad: &Pad) {
        if self.fludd.is_none() {
            return;
        }
        let r_level = lr_level(pad.r_analog, &self.params.controller);

        self.nozzle_movement(host, r_level, pad);

        let params_ptr = self.params.clone();
        if self.in_water {
            // TWaterGun::suck: standing in water refills at `suck_rate` x current pressure.
            let pressure = (r_level * 150.0) as i32;
            if let Some(f) = self.fludd.as_mut() {
                let rate = (pressure as f32 * params_ptr.nozzle(f.nozzle).suck_rate) as i32;
                if f.nozzle != NozzleKind::Yoshi && rate > 0 {
                    let max = f.max_water(&params_ptr);
                    f.water = (f.water + rate).min(max);
                }
            }
        }
        if self.upper == UpperState::Pumping {
            self.emit(host);
        }

        let x_edge = pad.x && !self.pad_prev.x;
        if x_edge
            && self.status != Status::BackJump
            && let Some(f) = self.fludd.as_mut()
        {
            f.swap_nozzle(&params_ptr);
        }
        if let Some(f) = self.fludd.as_mut() {
            let before = f.nozzle;
            f.movement(&params_ptr);
            if f.nozzle != before {
                host.event(Event::NozzleChanged(f.nozzle));
            }
        }
    }

    /// `TNozzleBase/Trigger/Deform::movement` for the fitted nozzle.
    fn nozzle_movement(&mut self, host: &mut dyn Host, r_level: f32, pad: &Pad) {
        let squatting = self.status == Status::Squat;
        let fades = self.options.spray_pressure_fades;
        let upper = self.upper;
        let in_water = self.in_water;
        let p = self.params.clone();
        let Some(f) = self.fludd.as_mut() else { return };
        let kind = f.nozzle;
        let np = p.nozzle(kind).clone();
        let water = f.water;
        let max_water = np.amount_max;

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
            }
            s.fill -= np.inside_pressure_dec;
            if s.fill < 0.0 {
                s.fill = 0.0;
            }
            if s.fill > np.inside_pressure_max {
                s.fill = np.inside_pressure_max;
                if s.state == TriggerState::Inactive {
                    s.state = TriggerState::Active;
                    s.timer = np.trigger_time;
                    host.event(Event::TriggerFired(kind));
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
                if !fades {
                    s.amount = var1 as f32 * 0.000_015_258_789;
                    s.gate = var1 as u16;
                } else if var1 > s.gate as i32 {
                    s.amount = (var1 - s.gate as i32) as f32 * 0.000_015_258_789;
                    s.gate = s.gate.wrapping_add(np.trigger_rate as u16);
                    if var1 < s.gate as i32 {
                        s.gate = var1 as u16;
                    }
                } else {
                    s.amount = 0.0;
                    s.gate = var1 as u16;
                }
                if kind == NozzleKind::Spray {
                    // TNozzleDeform::movement
                    s.amount = (s.amount * 10.0).min(1.0);
                }
            }
        }

        // calcGunAngle
        let s = &mut f.slots[kind as usize];
        let target = if squatting {
            (s.gun_angle + pad.stick.y * 10_000.0).clamp(np_min(&p, kind), np_max(&p, kind))
        } else {
            (-np_base(&p, kind)).clamp(np_min(&p, kind), np_max(&p, kind))
        };
        s.gun_angle += (target - s.gun_angle) * np_chase(&p, kind);
    }

    /// `TWaterGun::emit` + the nozzle's `emit`: fires water and applies recoil.
    fn emit(&mut self, host: &mut dyn Host) {
        let p = self.params.clone();
        let Some(f) = self.fludd.as_ref() else { return };
        let kind = f.nozzle;
        if kind == NozzleKind::Yoshi {
            return;
        }
        for i in 0..Self::emitter_count(kind) {
            self.emit_one(host, kind, i, &p);
        }
    }

    fn emit_one(&mut self, host: &mut dyn Host, kind: NozzleKind, index: usize, p: &Params) {
        let np = p.nozzle(kind).clone();
        let dir = self.nozzle_pitch_dir(kind, index);
        let pos = self.nozzle_pos(kind, index);
        let face = self.face_yaw;

        let (count, attack, power, reaction);
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
                s.emit_acc += pressure * (np.num - np.num_min) + np.num_min;
                let n = s.emit_acc as i32;
                if n == 0 {
                    return;
                }
                s.emit_acc -= n as f32;
                count = n;
                attack =
                    (pressure * (np.attack - np.attack_min) as f32 + np.attack_min as f32) as i16;
                power = pressure * (np.emit_pow - np.emit_pow_min) + np.emit_pow_min;
                reaction = pressure * (np.reaction_pow - np.reaction_pow_min) + np.reaction_pow_min;
            } else if kind == NozzleKind::Spray {
                // TNozzleDeform::emit (without the secondary bomb trigger)
                let a = s.amount;
                if !(a > 0.0) {
                    return;
                }
                s.emit_acc += a * (np.num - np.num_min) + np.num_min;
                let n = s.emit_acc as i32;
                if n == 0 {
                    return;
                }
                s.emit_acc -= n as f32;
                count = n;
                attack = (a * (np.attack - np.attack_min) as f32 + np.attack_min as f32) as i16;
                power = a * (np.emit_pow - np.emit_pow_min) + np.emit_pow_min;
                reaction = a * (np.reaction_pow - np.reaction_pow_min) + np.reaction_pow_min;
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
            // emitRequest accepts every particle in this model.
            f.water = (f.water - count * np.dec_rate as i32).max(0);
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
        }));

        // Recoil: `addVelocity(reaction * -(dir . forward) * power)`; vertical reaction pushes
        // along -dir.y.
        let dir_scale = -dir.x * sin(face) - dir.z * cos(face);
        let (reaction_pow, reaction_y) = if kind.is_trigger() || kind == NozzleKind::Spray {
            (reaction * dir_scale * power, -dir.y * power * np.reaction_y)
        } else {
            (reaction * dir_scale, -dir.y * power * np.reaction_y)
        };
        self.add_velocity(reaction_pow);
        self.vel.y += reaction_y;
        if !(kind.is_trigger() || kind == NozzleKind::Spray) {
            // TNozzleBase::emit also pushes Mario's horizontal velocity.
            self.vel.x -= dir.x * reaction;
            self.vel.z -= dir.z * reaction;
        }
    }
}

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
