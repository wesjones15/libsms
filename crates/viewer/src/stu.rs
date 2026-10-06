//! Strollin' Stus (`THamuKuri`, Enemy/hamukuri.cpp) for Yoshi's tongue to catch: the disc's
//! `hamukuri/default.bmd` playing `hamukuri_walk`, walking a circle. A stand-in, not the
//! enemy's AI: no chasing, damage or juice blocks. Sizes are the disc's `hamukuri.prm`
//! (`mSLDamageRadius` 60, `mSLDamageHeight` 100, body scale 1).

use std::path::Path;

use glam::{Mat4, Vec3};
use libsms::{Clip, Enemy, Vec3 as V3};
use libsms_assets::Pose;

use crate::j3d::{Lights, Look, Pass, Renderer};
use crate::mario_model::Part;

pub const RADIUS: f32 = 60.0;
pub const HEIGHT: f32 = 100.0;
/// Walking speed, units a tick (a viewer choice).
const SPEED: f32 = 1.0;
/// Ticks an eaten Stu stays away.
const RESPAWN: u32 = 900;

pub struct Stu {
    center: V3,
    path_radius: f32,
    angle: f32,
    pub pos: V3,
    pub yaw: f32,
    /// On Yoshi's tongue.
    pub held: bool,
    /// Ticks until it walks again after being eaten.
    pub gone: u32,
    frame: f32,
}

impl Stu {
    pub fn new(center: V3, path_radius: f32) -> Stu {
        let mut s = Stu {
            center,
            path_radius,
            angle: 0.0,
            pos: center,
            yaw: 0.0,
            held: false,
            gone: 0,
            frame: 0.0,
        };
        s.tick();
        s
    }

    pub fn loose(&self) -> bool {
        !self.held && self.gone == 0
    }

    pub fn enemy(&self, id: usize) -> Enemy {
        Enemy {
            id: id as u64,
            pos: self.pos,
            height: HEIGHT,
        }
    }

    pub fn eaten(&mut self) {
        self.held = false;
        self.gone = RESPAWN;
    }

    /// One 120 Hz tick: around the circle, the walk clip at 60 fps.
    pub fn tick(&mut self) {
        if self.gone > 0 {
            self.gone -= 1;
            return;
        }
        if self.held {
            return;
        }
        self.angle += SPEED / self.path_radius;
        let (s, c) = self.angle.sin_cos();
        self.pos = self.center + V3::new(s, 0.0, c) * self.path_radius;
        // Facing along the circle (+angle direction).
        self.yaw = self.angle + std::f32::consts::FRAC_PI_2;
        self.frame += 0.5;
    }
}

pub struct StuModel {
    renderer: Renderer,
    body: Part,
    walk: Option<Clip>,
}

impl StuModel {
    pub fn load(dir: &Path) -> Result<StuModel, String> {
        let mut renderer = Renderer::new();
        let body = Part::load(&mut renderer, dir, "enemy/hamukuri/default.bmd")?;
        let walk = std::fs::read(dir.join("enemy/hamukurianm/hamukuri_walk.bck"))
            .ok()
            .and_then(|d| Clip::parse(&d));
        Ok(StuModel {
            renderer,
            body,
            walk,
        })
    }

    /// Draws a Stu with its model space placed by `at` (decomp units to the scene).
    pub fn draw(&mut self, stu: &Stu, at: Mat4, view: Mat4, proj: Mat4) {
        let len = self.walk.as_ref().map_or(1.0, |c| c.frames.max(1.0));
        let joints = self.body.posed(self.walk.as_ref(), stu.frame % len, |_| None);
        let pose = Pose::new(&self.body.model.bmd, at, &joints);
        let lights = Lights::daylight(view);
        for pass in [Pass::Opaque, Pass::Translucent] {
            self.renderer.draw(
                &mut self.body.model,
                &pose,
                view,
                proj,
                &lights,
                &Look::default(),
                pass,
            );
        }
    }
}

/// A walking Stu's placement: at `pos`, turned by `yaw` radians, scaled to the scene.
pub fn place(pos: V3, yaw: f32, scale: f32) -> Mat4 {
    Mat4::from_translation(Vec3::new(pos.x, pos.y, pos.z) * scale)
        * Mat4::from_rotation_y(yaw)
        * Mat4::from_scale(Vec3::splat(scale))
}
