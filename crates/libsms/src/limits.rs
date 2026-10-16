//! Host limits: rules a host lays over the game's for its own balance, when it bills some of
//! Mario's moves as abilities with charges and cooldowns (Risk of Rain 2 makes FLUDD's mid-air
//! use and the wet belly slide skills that its items recharge and extend). None of this is
//! Sunshine's; every default is the game's, nothing limited and nothing stretched, so a host
//! that never sets [`Limits`] plays the game's Mario.
//!
//! The sim only gates when a move may start and scales how far it goes. What it spends comes
//! back as events ([`Event::AirNozzle`], [`Event::WetSlide`], [`Event::AirJump`]), so the
//! host keeps the books: it hands the sim what's left before each step (`air_extra`,
//! `wet_slide.ready`), and the sim counts those down as it spends them within the step.

use crate::event::Event;
use crate::input::{Pad, lr_level};
use crate::mario::{Mario, Status};
use crate::params::NozzleKind;

/// A host's wet-slide availability and duration rules. Use [`Self::timed`] and refresh
/// availability before each host step for a wet-ground-only move without a recharge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WetSlide {
    /// Availability for a new wet slide. Set false at activation; duration-only hosts
    /// refresh this each step, while charged hosts wait for a recharge.
    pub ready: bool,
    /// Any belly slide may turn wet, not only one over FLUDD's water on the ground.
    pub anywhere: bool,
    /// Ticks a wet slide glides before it turns back into a plain one. 0: as long as it lasts.
    pub max_ticks: u32,
    /// A slide over FLUDD's water glides for free, as in the game (to the slide's end, with no
    /// cap): the charge is only for slides without water under them.
    pub water_free: bool,
    /// A wet slide keeps its speed as if the stick were held forward (`doSliding`'s stick
    /// term at full), unless the stick pulls back against it. false: the game's (the stick
    /// decides).
    pub glide: bool,
}

impl WetSlide {
    /// Duration-limited wet slides: wet ground is required and every slide uses the cap,
    /// including slides in shallow water. Hosts refresh this each step; no cooldown needed.
    pub fn timed(max_ticks: u32, glide: bool) -> Self {
        Self {
            ready: true,
            anywhere: false,
            water_free: false,
            max_ticks,
            glide,
        }
    }
}

/// See the module docs. `Default` is the game.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    /// Count FLUDD's uses in the air: each start of a nozzle (R pressed, or a different nozzle
    /// fitted while it's held) while airborne spends one, except the Spray's, which are free
    /// and as many as he likes. One is free each time Mario leaves
    /// the ground; after that each takes one of `air_extra`, and with none left R does
    /// nothing until it's let go and Mario lands or is handed more. A use lasts as long as R
    /// stays down. Uses started on the ground cost nothing. false: the game's (the tank is
    /// the only limit).
    pub air_uses: bool,
    /// Extra mid-air uses on hand ([`Self::air_uses`]); counted down as they're spent.
    pub air_extra: u32,
    /// Jumps in mid-air, each the next jump of the chain as if A had been timed perfectly on a
    /// landing (a fall's first is a plain jump, then the double, then triples). They come back
    /// on landing. Spin and wall jumps, stomp jumps and the ground pound keep A. 0: the game's.
    pub air_jumps: u32,
    /// The Spray fires this many times as many drops a tick, each taking that much less water,
    /// so the tank lasts as long. 1: the game's.
    pub spray_rate: f32,
    /// Ticks the Spray can fire without letting go of R; then it stops until R is released.
    /// 0: no limit (the game's).
    pub spray_ticks: u32,
    /// The Hover nozzle's burst (its trigger time, `mTriggerTime`) times this. 1: the game's.
    pub hover_time: f32,
    /// A Spray press lasts this many times as long before it fades
    /// ([`crate::Options::spray_pressure_fades`]: the gate climbs this much slower). 1: the
    /// game's.
    pub spray_fade: f32,
    /// Ground covered on the Turbo nozzle's dash times this, on top of `Options::move_speed`.
    /// 1: the game's.
    pub turbo_speed: f32,
    /// None: the game's (any belly slide over FLUDD's water glides).
    pub wet_slide: Option<WetSlide>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AnimTable, Params, Sim, Vec3};

    #[test]
    fn timed_slides_require_wet_ground_and_cannot_restart_their_cap_mid_slide() {
        let mut m = Sim::new(Params::default(), AnimTable::default(), Vec3::ZERO, 0).mario;
        m.status = Status::Catch;
        m.limits.wet_slide = Some(WetSlide::timed(3, true));
        assert!(!m.limit_wet_slide(), "a dry slide turned wet");
        m.wet_ground = true;
        for tick in 0..8 {
            // RoR2 always refreshes availability, even after the duration has run out.
            m.limits.wet_slide = Some(WetSlide::timed(3, true));
            let wet = m.limit_wet_slide();
            assert_eq!(wet, tick < 3, "wet slide at tick {tick}");
            m.status_state = wet as u32;
            m.wet_ground = false; // it may carry on from the wet approach onto dry ground
        }
        assert_eq!(
            m.events
                .iter()
                .filter(|e| matches!(e, Event::WetSlide))
                .count(),
            1
        );
        m.change_status(Status::Wait, 0, false);
        m.change_status(Status::Catch, 0, false);
        m.limits.wet_slide = Some(WetSlide::timed(3, true));
        assert!(!m.limit_wet_slide(), "a fresh dry slide turned wet");
        m.wet_ground = true;
        assert!(m.limit_wet_slide(), "a fresh wet slide needed a cooldown");
    }

    #[test]
    fn a_longer_timed_limit_extends_the_glide_even_on_continuously_wet_ground() {
        for max_ticks in [60, 120, 180] {
            let mut m = Sim::new(Params::default(), AnimTable::default(), Vec3::ZERO, 0).mario;
            m.status = Status::Catch;
            m.wet_ground = true;
            let mut active = 0;
            for _ in 0..max_ticks + 20 {
                m.limits.wet_slide = Some(WetSlide::timed(max_ticks, true));
                let wet = m.limit_wet_slide();
                m.status_state = wet as u32;
                active += u32::from(wet);
            }
            assert_eq!(active, max_ticks, "wet ground bypassed the duration cap");
        }
    }
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            air_uses: false,
            air_extra: 0,
            air_jumps: 0,
            spray_rate: 1.0,
            spray_ticks: 0,
            hover_time: 1.0,
            spray_fade: 1.0,
            turbo_speed: 1.0,
            wet_slide: None,
        }
    }
}

/// What the limits remember between ticks.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LimitState {
    /// R is down, and this press started a use with this nozzle.
    r_down: bool,
    r_nozzle: Option<NozzleKind>,
    /// This press is refused (no use left, or the Spray ran its limit): R reads as up until
    /// it's let go.
    r_refused: bool,
    /// The free mid-air use is spent; it comes back on landing.
    free_spent: bool,
    /// Ticks this press has sprayed.
    spray_ticks: u32,
    /// Mid-air jumps made since the last landing.
    air_jumps: u32,
    /// This belly slide has decided whether it's wet, whether a charge paid for it, and its
    /// wet ticks left.
    pub(crate) wet_decided: bool,
    wet_charged: bool,
    wet_left: u32,
    /// Horizontal travel in the air times this until Mario lands ([`Mario::boost_jump`]).
    jump_boost: f32,
}

impl Default for LimitState {
    fn default() -> LimitState {
        LimitState {
            r_down: false,
            r_nozzle: None,
            r_refused: false,
            free_spent: false,
            spray_ticks: 0,
            air_jumps: 0,
            wet_decided: false,
            wet_charged: false,
            wet_left: 0,
            jump_boost: 1.0,
        }
    }
}

/// How far the continuous nozzles' pressure gate climbs a tick (`mTriggerRate`), the Spray's
/// slowed by [`Limits::spray_fade`].
pub(crate) fn gate_rate(kind: NozzleKind, rate: i16, spray_fade: f32) -> u16 {
    let rate = rate.max(0) as f32;
    if kind == NozzleKind::Spray && spray_fade != 1.0 && spray_fade.is_finite() && spray_fade > 0.0
    {
        (rate / spray_fade.clamp(0.1, 10.0))
            .round()
            .clamp(1.0, u16::MAX as f32) as u16
    } else {
        rate as u16
    }
}

/// A trigger nozzle's burst in ticks (`mTriggerTime`), the Hover's times [`Limits::hover_time`].
pub(crate) fn trigger_time(kind: NozzleKind, ticks: i16, hover_time: f32) -> i16 {
    if kind == NozzleKind::Hover && hover_time != 1.0 && hover_time.is_finite() {
        (ticks as f32 * hover_time.max(0.0))
            .round()
            .clamp(1.0, i16::MAX as f32) as i16
    } else {
        ticks
    }
}

impl Mario {
    /// Airborne for the limits: the jumping family (hovering and rocketing too).
    fn limits_airborne(&self) -> bool {
        self.status.is_jumping()
    }

    /// The pad this tick runs with: R refused while a mid-air use isn't on hand or the
    /// Spray has run its limit. Also restores the free use and the air jumps on landing.
    pub(crate) fn limit_pad(&mut self, pad: &Pad) -> Pad {
        let airborne = self.limits_airborne();
        if !airborne {
            self.lim.free_spent = false;
            self.lim.air_jumps = 0;
            self.lim.jump_boost = 1.0;
        }
        let l = self.limits;
        let mut out = *pad;
        let down = pad.r || lr_level(pad.r_analog, &self.params.controller) > 0.0;
        let nozzle = self.fludd.as_ref().map(|f| f.nozzle);
        if !down {
            self.lim = LimitState {
                r_down: false,
                r_nozzle: None,
                r_refused: false,
                spray_ticks: 0,
                ..self.lim
            };
            return out;
        }
        // A new press, or another nozzle fitted while R stays down: a new use.
        let starts =
            !self.lim.r_down || (self.lim.r_nozzle.is_some() && self.lim.r_nozzle != nozzle);
        self.lim.r_down = true;
        if starts {
            self.lim.r_nozzle = nozzle;
            self.lim.spray_ticks = 0;
            // The Spray costs nothing to start again in the air: only the other nozzles are counted.
            if l.air_uses
                && airborne
                && nozzle.is_some()
                && nozzle != Some(NozzleKind::Spray)
                && !self.lim.r_refused
            {
                if !self.lim.free_spent {
                    self.lim.free_spent = true;
                    self.events.push(Event::AirNozzle { extra: false });
                } else if self.limits.air_extra > 0 {
                    self.limits.air_extra -= 1;
                    self.events.push(Event::AirNozzle { extra: true });
                } else {
                    self.lim.r_refused = true;
                }
            }
        }
        if l.spray_ticks > 0 && nozzle == Some(NozzleKind::Spray) && !self.lim.r_refused {
            self.lim.spray_ticks += 1;
            if self.lim.spray_ticks > l.spray_ticks {
                self.lim.r_refused = true;
            }
        }
        if self.lim.r_refused {
            out.r = false;
            out.r_analog = 0;
        }
        out
    }

    /// A mid-air jump ([`Limits::air_jumps`]) for this tick's A, if one's on hand and nothing
    /// else wants A. Runs after the input is read, before the state machine.
    pub(crate) fn limit_air_jump(&mut self) -> bool {
        if !self.input.jump || self.lim.air_jumps >= self.limits.air_jumps || self.on_yoshi() {
            return false;
        }
        // A pad's flight isn't broken by an A held through the launch.
        if self.pad_gravity.is_some() && self.pad_age < crate::air::PAD_ACTION_GRACE {
            return false;
        }
        use Status::*;
        let next = match self.status {
            Jump => SecondJump,
            SecondJump | UltraJump => UltraJump,
            ForceJump | Landing | RocketLanding | BackJump | BroadJump | RotateBroadJump
            | UTurnJump | MissJump | SlipFall | WallJump | LeftRotateJump | RightRotateJump => Jump,
            // A dive in flight: a mid-air jump pulls out of it into a fresh jump.
            JumpCatch => Jump,
            // A stomp's own late jump keeps A while its window is open.
            Trample if self.stomp_late.is_none() => Jump,
            _ => return false,
        };
        // A spin of the stick makes A a spin jump.
        if self.check_stick_rotate().is_some() {
            return false;
        }
        self.lim.air_jumps += 1;
        self.events.push(Event::AirJump);
        self.change_status(next, 0, true)
    }

    /// Ground covered per tick, times this: the host's speed ([`crate::Options::move_speed`])
    /// and, dashing on the Turbo, [`Limits::turbo_speed`].
    pub(crate) fn limit_ground_speed(&self) -> f32 {
        let turbo = self.fludd_emitting
            && self
                .fludd
                .as_ref()
                .is_some_and(|f| f.nozzle == NozzleKind::Turbo);
        self.options.move_speed * if turbo { self.limits.turbo_speed } else { 1.0 }
    }

    /// Travel per tick in the air, times this: the host's speed and any jump boost.
    pub(crate) fn limit_air_speed(&self) -> f32 {
        self.options.move_speed * self.lim.jump_boost
    }

    /// `catching`'s wet slide with [`Limits::wet_slide`]: whether the belly slide glides
    /// this tick. The game's rule is any slide over FLUDD's water.
    pub(crate) fn limit_wet_slide(&mut self) -> bool {
        let Some(mut w) = self.limits.wet_slide else {
            return self.wet_ground || self.status_state == 1;
        };
        if !self.lim.wet_decided {
            if w.water_free && self.wet_ground {
                // The game's: over FLUDD's water it glides, and stays wet to the end.
                self.lim.wet_decided = true;
                self.lim.wet_charged = false;
                return true;
            }
            if w.ready && (w.anywhere || self.wet_ground) {
                self.lim.wet_decided = true;
                self.lim.wet_charged = true;
                self.lim.wet_left = w.max_ticks;
                w.ready = false;
                self.limits.wet_slide = Some(w);
                self.events.push(Event::WetSlide);
                return true;
            }
            // Without a charge it stays plain; it may still turn wet on reaching water, or if
            // a charge comes back mid-slide.
            return false;
        }
        if self.status_state != 1 {
            return false;
        }
        if !self.lim.wet_charged {
            return true;
        }
        if w.max_ticks == 0 {
            return true;
        }
        self.lim.wet_left = self.lim.wet_left.saturating_sub(1);
        self.lim.wet_left > 0
    }

    /// `doSliding`'s stick term for a wet slide under [`WetSlide::glide`]: full ahead unless
    /// the stick pulls back. `stick * cs` otherwise (the game's).
    pub(crate) fn limit_slide_push(&self, stick: f32, cs: f32) -> f32 {
        let glide = self.status == Status::Catch
            && self.status_state == 1
            && self.limits.wet_slide.is_some_and(|w| w.glide);
        if glide && (stick < 0.1 || cs >= 0.0) {
            1.0
        } else {
            stick * cs
        }
    }

    /// The Spray's drops a tick, and the water each takes, with [`Limits::spray_rate`].
    pub(crate) fn limit_spray_rate(&self, kind: NozzleKind) -> f32 {
        if kind == NozzleKind::Spray && self.limits.spray_rate.is_finite() {
            self.limits.spray_rate.clamp(0.1, 10.0)
        } else {
            1.0
        }
    }

    /// Boosts the jump Mario is in: horizontal travel times `factor` until he lands (a host's
    /// sprint-jump item). Not the game's. Does nothing on the ground; 1 ends it.
    pub fn boost_jump(&mut self, factor: f32) {
        if self.limits_airborne() {
            self.lim.jump_boost = if factor.is_finite() {
                factor.clamp(0.0, 10.0)
            } else {
                1.0
            };
        }
    }
}
