//! Getting a Shine Sprite: the touch, Mario's win pose, and what the host does around it.
//!
//! In the game a Shine (`TShine`, MoveBG/Item.cpp) is a pickup. Touching it (`touchPlayer`) plays
//! `MSD_SE_SY_GET_SHINE` and sends `HIT_MESSAGE_ATTACK`, which `TMario::receiveMessage` answers for
//! actor type 0x20000013 (MarioReceiveMsg.cpp) by facing the Shine's way, stopping, refilling
//! health and breath and entering `MARIO_STATUS_WIN_DEMO`. `winDemo` (MarioAutodemo.cpp) then
//! lets him land (`jumpProcess`), and the moment he has it tells the director (`fireGetStar`,
//! MarDirectorEvent.cpp: the demo camera, the "SHINE GET!" letters, the fanfare, the end of the
//! stage) and holds the `demo_shine_get` clip, the Shine over his head, until the world moves on.
//!
//! The sim does only Mario's part. The host owns the Shine itself, the camera, the letters, the
//! music and what the pickup means (RoR2's teleporter), and lets Mario go again with
//! [`Mario::end_win_demo`] if the stage doesn't end.

use glam::Vec3;

use crate::anim::clip;
use crate::event::{Event, SoundCue};
use crate::host::Host;
use crate::mario::{Mario, Status};

impl Mario {
    /// He touches a Shine at `pos`, turned `yaw` (`TShine::touchPlayer`, then the 0x20000013
    /// case of `receiveMessage`). `yaw` is the way he faces for the pose, and the way the demo
    /// camera is turned (`mInitialRotation.y`). Returns false, and does nothing, if he's already
    /// in the pose or down.
    pub fn get_shine(&mut self, pos: Vec3, yaw: i16) -> bool {
        if matches!(self.status, Status::WinDemo | Status::DownLoser) {
            return false;
        }
        self.events.push(Event::Sound(SoundCue {
            name: "MSD_SE_SY_GET_SHINE",
            pos,
            info: None,
        }));
        // `winDemo` state 0 lets go of what he holds (`HIT_MESSAGE_UNKD`); the host's takeable
        // simply stays where it is here.
        self.held = None;
        self.held_fruit = None;
        self.face_yaw = yaw;
        self.model_yaw = yaw;
        self.set_player_velocity(0.0);
        self.health = self.params.mario.hp_max;
        self.invincible = 0;
        self.change_status(Status::WinDemo, 0, true);
        self.events.push(Event::ShineTouch { pos, yaw });
        true
    }

    /// `winDemo`: state 0 falls until he has landed (`jumpProcess`), state 1 is the pose.
    pub(crate) fn win_demo(&mut self, host: &mut dyn Host) -> bool {
        match self.status_state {
            0 => {
                self.set_player_velocity(0.0);
                if self.jump_process(host, 0) == 1 {
                    self.status_state = 1;
                    self.status_timer = 0;
                    self.vel = Vec3::ZERO;
                    self.events.push(Event::ShineDemo {
                        pos: self.pos,
                        yaw: self.face_yaw,
                    });
                }
            }
            _ => {
                // `stopProcess`: where he is, held still, on the floor.
                self.status_timer += 1;
                self.set_player_velocity(0.0);
                self.vel = Vec3::ZERO;
                self.model_yaw = self.face_yaw;
                self.set_anim(clip::DEMO_SHINE_GET, 1.0);
            }
        }
        false
    }

    /// He's in the Shine Get pose (falling to it, or holding it).
    pub fn is_win_demo(&self) -> bool {
        self.status == Status::WinDemo
    }

    /// Whether he has landed and holds the Shine up (the demo proper).
    pub fn win_demo_posing(&self) -> bool {
        self.status == Status::WinDemo && self.status_state >= 1
    }

    /// The host ended the Shine Get demo without the stage ending: he stands again.
    pub fn end_win_demo(&mut self) {
        if self.status == Status::WinDemo {
            self.change_status(Status::Wait, 0, true);
        }
    }
}
