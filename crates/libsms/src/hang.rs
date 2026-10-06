//! Hanging from ledges (Player/MarioSpecial.cpp `hanging`, `ascend`, `descend`,
//! `hangJumping`, `startHangLanding`; Player/MarioPhysics.cpp `hangonCheck`, `checkDescent`).
//!
//! A jump that meets a wall at waist height (`wall2`) with nothing at head height (`wall1`)
//! catches the ledge when Mario is falling and the top is more than 100 above his feet
//! (`hangonCheck`). His position then sits on the ledge top, 10 units in from the face; the
//! `hang` clip lowers the body to hang below it. Walking slowly off an edge does the same
//! from above (`checkDescent`, `hgdwn`). While hanging: stick toward the wall climbs up
//! (`hgup`), sideways shimmies along the face and round corners (`hmov_l/r`), away drops off,
//! A jumps up (`hgjmp`), B lets go. After `mLimitTime` ticks he falls.

use glam::Vec3;

use crate::event::{Event, SoundCue};
use crate::host::{Host, SurfaceFlags, WallHit};
use crate::mario::{Mario, Status};
use crate::math::{cos, diff, matan, sin};

/// Clips (`ANIM_HANG`, `ANIM_HGUP`...).
pub mod clip {
    use crate::anim::ClipName;
    pub const HANG: ClipName = "hang";
    pub const CLIMB_UP: ClipName = "hgup";
    pub const CLIMB_DOWN: ClipName = "hgdwn";
    pub const JUMP_UP: ClipName = "hgjmp";
    pub const MOVE_L: ClipName = "hmov_l";
    pub const MOVE_R: ClipName = "hmov_r";
}

impl Mario {
    fn wall_angle(w: &WallHit) -> i16 {
        matan(w.normal.z, w.normal.x)
    }

    /// The touched wall Mario faces (within 45 degrees), if any.
    fn facing_wall(&self, walls: &[WallHit]) -> Option<WallHit> {
        let back = self.face_yaw.wrapping_add(i16::MIN);
        walls
            .iter()
            .rev()
            .find(|w| {
                let a = diff(Self::wall_angle(w), back);
                -0x2000 < a && a < 0x2000
            })
            .copied()
    }

    /// `hangonCheck`: can Mario catch the ledge above `wall`? `prev` is where the jump step
    /// aimed, `curr` where the walls pushed it.
    pub(crate) fn hangon_check(
        &mut self,
        host: &mut dyn Host,
        wall: &WallHit,
        prev: Vec3,
        curr: Vec3,
    ) -> bool {
        if self.vel.y > 0.0 {
            return false;
        }
        if (curr.x - prev.x) * self.vel.x + (curr.z - prev.z) * self.vel.z > 0.0 {
            return false;
        }
        let at = Vec3::new(
            curr.x - 60.0 * wall.normal.x,
            curr.y + 160.0,
            curr.z - 60.0 * wall.normal.z,
        );
        let (top, ground) = self.check_ground_plane(host, at);
        if top - curr.y <= 100.0 {
            return false;
        }
        if self.roof_y < top + 160.0 {
            return false;
        }
        if ground.flags.has(SurfaceFlags::FORCE_SLIP)
            || ground.flags.has(SurfaceFlags::NO_LEDGE_GRAB)
            || ground.flags.has(SurfaceFlags::ILLEGAL)
        {
            return false;
        }
        self.pos = Vec3::new(at.x, top, at.z);
        self.ground = ground;
        self.floor_y = top;
        self.slope_angle = matan(ground.normal.z, ground.normal.x);
        self.face_yaw = Self::wall_angle(wall).wrapping_add(i16::MIN);
        true
    }

    /// `startHangLanding`: let go, stepping back off the ledge.
    pub(crate) fn start_hang_landing(&mut self, host: &mut dyn Host, to: Status) -> bool {
        self.vel.y = 0.0;
        self.forward_vel = 0.0;
        self.pos.x -= 60.0 * sin(self.face_yaw);
        self.pos.z -= 60.0 * cos(self.face_yaw);
        let below = host.ground(self.pos).map_or(f32::MIN, |g| g.height);
        if below < self.pos.y - 100.0 {
            self.pos.y -= 100.0;
        } else {
            self.pos.y = below;
        }
        self.change_status(to, 0, false)
    }

    fn voice(&mut self, name: &'static str) {
        self.events.push(Event::Sound(SoundCue {
            name,
            pos: self.pos,
            info: None,
        }));
    }

    pub(crate) fn special_main(&mut self, host: &mut dyn Host) -> bool {
        match self.status {
            Status::Hanging => self.hanging(host),
            Status::Ascend => self.hang_clip(host, clip::CLIMB_UP, Status::Wait),
            Status::Descend => self.hang_clip(host, clip::CLIMB_DOWN, Status::Hanging),
            Status::HangJumping => self.hang_clip(host, clip::JUMP_UP, Status::Wait),
            _ => false,
        }
    }

    /// `ascend` / `descend` / `hangJumping`: play the clip in place, then move on.
    fn hang_clip(&mut self, host: &mut dyn Host, c: crate::anim::ClipName, then: Status) -> bool {
        if self.input.airborne {
            return self.start_hang_landing(host, Status::LandSafeDown);
        }
        self.wait_process(host);
        self.set_anim(c, 1.0);
        if self.anim.at_last_frame() {
            return self.change_status(then, 0, false);
        }
        false
    }

    /// `hanging`.
    fn hanging(&mut self, host: &mut dyn Host) -> bool {
        let p = self.params.hanging.clone();
        let mut pulled = false;
        let yaw_diff = diff(self.intended_yaw, self.face_yaw);
        let on_ceil = self.roof_y - self.floor_y >= 160.0;
        if self.status_timer < p.limit_time as i32 {
            self.status_timer += 1;
        }

        if self.input.dive
            || self.input.airborne
            || self.ground.flags.has(SurfaceFlags::FORCE_SLIP)
            || self.ground.flags.has(SurfaceFlags::NO_LEDGE_GRAB)
        {
            self.input.dive = false;
            self.input.airborne = false;
            return self.start_hang_landing(host, Status::WallSlide);
        }
        if self.input.jump && on_ceil {
            self.voice("MSD_SE_MV15_EXERT_INST_01");
            return self.change_status(Status::HangJumping, 0, false);
        }

        let mut walls = Vec::new();
        host.walls(
            Vec3::new(self.pos.x, self.pos.y - 10.0, self.pos.z),
            30.0,
            4,
            &mut walls,
        );
        let Some(wall) = self.facing_wall(&walls) else {
            return self.change_status(Status::Landing, 0, false);
        };

        let mut tmp = Vec::new();
        self.pos = host.walls(
            Vec3::new(self.pos.x, self.pos.y + 10.0, self.pos.z),
            50.0,
            1,
            &mut tmp,
        ) - Vec3::new(0.0, 10.0, 0.0);

        if self.status_timer >= 0x28 && self.input.stick {
            if (-0x400..=0x400).contains(&yaw_diff) && on_ceil {
                self.set_anim(clip::CLIMB_UP, 1.0);
                return self.change_status(Status::Ascend, 0, false);
            }
            if yaw_diff <= -0x71c7 || yaw_diff >= 0x71c7 {
                return self.start_hang_landing(host, Status::LandSafeDown);
            }
            if self.status_timer < p.rapid_time as i32 && !walls.is_empty() {
                // Shimmy along the face (`mMoveSp`), round corners up to
                // `mHangWallMovableAngle`.
                let step = p.move_sp * self.intended_mag;
                let mut to = self.pos;
                if yaw_diff > 0x400 && yaw_diff < 0x71c7 {
                    to.x -= step * wall.normal.z;
                    to.z += step * wall.normal.x;
                }
                if yaw_diff > -0x71c7 && yaw_diff < -0x400 {
                    to.x += step * wall.normal.z;
                    to.z -= step * wall.normal.x;
                }
                let to = host.walls(to, 50.0, 1, &mut tmp);
                let gy = host
                    .ground(Vec3::new(to.x, to.y + 50.0, to.z))
                    .map_or(f32::MIN, |g| g.height);
                if self.pos.y - 100.0 < gy && gy < self.pos.y + 50.0 {
                    let probe = Vec3::new(
                        to.x - 30.0 * sin(self.face_yaw),
                        gy - 20.0,
                        to.z - 30.0 * cos(self.face_yaw),
                    );
                    let mut w2 = Vec::new();
                    let center = host.walls(probe, 30.0, 4, &mut w2);
                    if let Some(wall2) = self.facing_wall(&w2)
                        && wall.normal.dot(wall2.normal) > self.params.mario.hang_wall_movable_angle
                    {
                        self.face_yaw = Self::wall_angle(&wall2).wrapping_add(i16::MIN);
                        self.pos.x = center.x - 40.0 * wall2.normal.x;
                        self.pos.z = center.z - 40.0 * wall2.normal.z;
                        self.pos.y = host
                            .ground(Vec3::new(self.pos.x, self.pos.y + 160.0, self.pos.z))
                            .map_or(self.pos.y, |g| g.height);
                        pulled = true;
                    }
                }
            }
        }

        if self.status_timer >= p.limit_time as i32 {
            return self.start_hang_landing(host, Status::LandSafeDown);
        }
        // `checkPlayerAround(-0x8000, 30)`: a floor just below the hands means a short wall
        // he simply hops up.
        let behind = self.face_yaw.wrapping_add(i16::MIN);
        let below = self.pos.y
            - host
                .ground(Vec3::new(
                    self.pos.x + 30.0 * sin(behind),
                    self.pos.y + 100.0,
                    self.pos.z + 30.0 * cos(behind),
                ))
                .map_or(f32::MIN, |g| g.height);
        if on_ceil && below < 100.0 {
            self.voice("MSD_SE_MV15_EXERT_INST_01");
            return self.change_status(Status::HangJumping, 0, false);
        }

        self.set_player_velocity(0.0);
        self.vel.y = 0.0;
        self.pos.y = self.floor_y;
        self.model_yaw = self.face_yaw;
        if pulled {
            let d = self.pos - self.prev_pos;
            let rate = (d.x * d.x + d.z * d.z).sqrt() * p.anm_rate;
            let c = if yaw_diff < 0 {
                clip::MOVE_L
            } else {
                clip::MOVE_R
            };
            self.set_anim(c, rate);
        } else if self.status_timer < p.rapid_time as i32 {
            self.set_anim(clip::HANG, 1.0);
        } else {
            self.set_anim(clip::HANG, p.anm_rapid);
        }
        false
    }

    /// `checkDescent`: walking slowly off an edge turns round and lowers Mario to hang.
    pub(crate) fn check_descent(&mut self, host: &mut dyn Host) {
        let sp = self.params.hanging.descent_sp;
        if self.forward_vel >= sp {
            return;
        }
        let mut walls = Vec::new();
        let center = host.walls(
            Vec3::new(self.pos.x, self.pos.y - 10.0, self.pos.z),
            sp,
            1,
            &mut walls,
        );
        let Some(wall) = walls.first().copied() else {
            return;
        };
        let (fy, g) =
            self.check_ground_plane(host, Vec3::new(center.x, self.pos.y + 30.0, center.z));
        if g.flags.has(SurfaceFlags::ILLEGAL) || self.pos.y - fy <= 160.0 {
            return;
        }
        let ang = Self::wall_angle(&wall);
        let d = diff(ang, self.face_yaw);
        if -0x4000 < d && d < 0x4000 {
            let back = 20.0 + sp;
            self.pos.x = center.x - back * wall.normal.x;
            self.pos.z = center.z - back * wall.normal.z;
            self.face_yaw = ang.wrapping_add(i16::MIN);
            self.change_status(Status::Descend, 0, false);
            self.set_anim(clip::CLIMB_DOWN, 1.0);
        }
    }
}
