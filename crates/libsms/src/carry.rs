//! Picking things up, carrying, putting down and throwing (Player/MarioAction.cpp `taking`,
//! `takePose`, `putting`, `pitching`; MarioCollision.cpp `isTakeSituation`, `canTake`,
//! `dropObject`; MarioWait.cpp `canPut`; MarioRun.cpp `isThrowStart`).
//!
//! B at a standstill or a slow walk makes Mario reach forward (`TAKE_POSE`, the `get_fail`
//! clip when there's nothing there). While B is held, touching a takeable object he faces
//! starts `TAKE`: the `raise` clip, closing his hand on it at frame 11. He then carries it
//! with the `carry_p` arms over whatever his legs do. B again puts it down in front of him
//! (`put`, let go at frame 20), or throws it (`throw`, let go at frame 4) when he is moving
//! faster than 16 or the object is a throwing kind.

use glam::Vec3;

use crate::event::{Event, SoundCue};
use crate::host::{Host, Takeable};
use crate::mario::{Mario, Status, UpperState};
use crate::math::{cos, diff, matan, sin};

/// Clips (`ANIM_RAISE`, `ANIM_GET_FAIL`...).
pub mod clip {
    use crate::anim::ClipName;
    pub const RAISE: ClipName = "raise";
    pub const GRAB_MISS: ClipName = "get_fail";
    pub const PUT: ClipName = "put";
    pub const THROW: ClipName = "throw";
    /// The arms while carrying (`addUpper`, `UPPER_STATE_HOLDING_OBJECT`, anim 0x4B).
    pub const CARRY: ClipName = "carry_p";
}

impl Mario {
    /// `isTakeSituation`.
    fn is_take_situation(&self, t: &Takeable) -> bool {
        if matches!(
            self.status,
            Status::Putting
                | Status::Pitching
                | Status::Catch
                | Status::Hanging
                | Status::Ascend
                | Status::Descend
                | Status::HangJumping
        ) {
            return false;
        }
        if self.upper != UpperState::Idle || self.held.is_some() {
            return false;
        }
        let d = t.pos - self.pos;
        let angle = diff(matan(d.z, d.x), self.face_yaw);
        if angle <= -0x2aaa || angle >= 0x2aaa {
            return false;
        }
        d.length() <= self.params.mario.hold_radius + t.radius
    }

    /// The hit check's `hitWantToTake` -> `wantToTakeActor`: B held and touching something
    /// takeable in front of him starts `TAKE`.
    pub(crate) fn check_take(&mut self, host: &mut dyn Host) {
        if !self.input.b_held || self.held.is_some() {
            return;
        }
        let mut found = Vec::new();
        host.takeables(self.pos, self.params.mario.hold_radius, &mut found);
        if let Some(t) = found.into_iter().find(|t| self.is_take_situation(t)) {
            self.take_target = Some(t.id);
            self.change_status(Status::Take, 0, false);
        }
    }

    fn exert(&mut self) {
        self.events.push(Event::Sound(SoundCue {
            name: "MSD_SE_MV15_EXERT_INST_01",
            pos: self.pos,
            info: None,
        }));
    }

    /// `taking`.
    pub(crate) fn taking(&mut self, host: &mut dyn Host) -> bool {
        if self.input.airborne {
            return self.change_status(Status::Landing, 0, false);
        }
        self.set_anim(clip::RAISE, 1.0);
        if let Some(id) = self.take_target
            && self.anim.passed(11.0)
        {
            if host.take(id) {
                self.exert();
                self.held = Some(id);
            } else {
                self.take_target = None;
                return self.change_status(Status::Wait, 0, false);
            }
            self.take_target = None;
        }
        if self.anim.at_last_frame() {
            if self.held.is_some() {
                self.upper = UpperState::HoldingObject;
            }
            self.pump_cooldown = 0;
            return self.change_status(Status::Wait, 0, false);
        }
        self.wait_process(host);
        false
    }

    /// `takePose`: the grab at nothing.
    pub(crate) fn take_pose(&mut self, host: &mut dyn Host) -> bool {
        if self.input.airborne {
            return self.change_status(Status::Landing, 0, false);
        }
        if self.anim.clip == clip::GRAB_MISS && self.anim.at_last_frame() {
            self.set_anim(crate::anim::clip::WAIT, 1.0);
            return self.change_status(Status::Wait, 0, false);
        }
        self.stop_common(host, clip::GRAB_MISS, Status::Wait);
        false
    }

    /// `putting`.
    pub(crate) fn putting(&mut self, host: &mut dyn Host) -> bool {
        if self.input.airborne {
            self.drop_object();
            return self.change_status(Status::Landing, 0, false);
        }
        self.stop_common(host, clip::PUT, Status::Wait);
        if let Some(id) = self.held
            && self.anim.passed(20.0)
        {
            self.events.push(Event::Put {
                id,
                pos: self.pos,
                yaw: self.face_yaw,
            });
            self.held = None;
        }
        false
    }

    /// `pitching` / `checkThrowObject`.
    pub(crate) fn pitching(&mut self, host: &mut dyn Host) -> bool {
        if self.input.airborne {
            self.drop_object();
            return self.change_status(Status::Landing, 0, false);
        }
        self.stop_common(host, clip::THROW, Status::Wait);
        if self.anim.passed(4.0) && self.held.is_some() {
            self.exert();
            self.drop_object();
        }
        false
    }

    /// `dropObject`: let go with a throw.
    pub(crate) fn drop_object(&mut self) {
        if let Some(id) = self.held.take() {
            self.events.push(Event::Throw {
                id,
                pos: self.pos,
                yaw: self.face_yaw,
                power: self.params.mario.throw_power,
                vel: self.vel,
            });
        }
    }

    /// `canPut`: room in front of Mario for the object.
    fn can_put(&self, host: &mut dyn Host) -> bool {
        let r = 40.0;
        let mut hits = Vec::new();
        let ahead = Vec3::new(
            self.pos.x + 100.0 * sin(self.face_yaw),
            self.pos.y + 10.0,
            self.pos.z + 100.0 * cos(self.face_yaw),
        );
        host.walls(ahead, r, 1, &mut hits);
        host.walls(self.pos + Vec3::new(0.0, 10.0, 0.0), r, 1, &mut hits);
        hits.is_empty()
    }

    /// `isThrowStart` (running) and `waitMain`'s put/throw: B with something in hand.
    pub(crate) fn check_throw_start(&mut self, host: &mut dyn Host, running: bool) -> bool {
        if self.held.is_none() || !self.input.b_pressed {
            return false;
        }
        if running && self.forward_vel > 16.0 {
            return self.change_status(Status::Pitching, 0, false);
        }
        if self.can_put(host) {
            return self.change_status(Status::Putting, 0, false);
        }
        false
    }
}
