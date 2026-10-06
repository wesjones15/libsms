//! Fruit for Mario to pick up, carry, put down and throw: the disc's `mapobj/fruit*.bmd`
//! (`TResetFruit`, MapObjBall.cpp) with the toss and bounce from their physical data
//! (MapObjInit.cpp: `*_physical_data`; `TMapObjGeneral::thrown` / `put`, MapObjGeneral.cpp).
//!
//! Physical data used: `[0]` gravity per tick, `[1]` bounce, `[4]` ground friction,
//! `[11]` throw speed (times Mario's `mThrowPower`), `[12]` throw rise. Velocities are per
//! frame like Mario's: positions move a quarter of it each 120 Hz tick.

use std::path::Path;

use glam::{Mat4, Vec3};
use libsms::Vec3 as V3;
use libsms::host::Takeable;
use libsms_assets::{Bmd, Pose};

use crate::j3d::{GpuModel, Lights, Look, Pass, Renderer};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Papaya,
    Coconut,
    Pine,
    Banana,
    Durian,
}

impl Kind {
    /// The sim's name for it (Yoshi's egg and tongue care which fruit it is).
    pub fn fruit(self) -> libsms::Fruit {
        match self {
            Kind::Papaya => libsms::Fruit::Papaya,
            Kind::Coconut => libsms::Fruit::Coconut,
            Kind::Pine => libsms::Fruit::Pine,
            Kind::Banana => libsms::Fruit::Banana,
            Kind::Durian => libsms::Fruit::Durian,
        }
    }

    fn file(self) -> &'static str {
        match self {
            Kind::Papaya => "scene/mapobj/fruitpapaya.bmd",
            Kind::Coconut => "scene/mapobj/fruitcoconut.bmd",
            Kind::Pine => "scene/mapobj/fruitpine.bmd",
            Kind::Banana => "scene/mapobj/fruitbanana.bmd",
            Kind::Durian => "scene/mapobj/fruitdurian.bmd",
        }
    }

    /// How far above the hand it rides (`TMapObjBall::control`, `unk190`).
    pub fn held_offset(self) -> f32 {
        match self {
            Kind::Papaya | Kind::Coconut => 20.0,
            Kind::Pine => 10.0,
            Kind::Durian => 45.0,
            Kind::Banana => 0.0,
        }
    }

    /// How far above its position (the bottom, on the ground) the model's root sits:
    /// `mBodyRadius` (`TMapObjBall::initMapObj`), less the banana's 50 when upright
    /// (`TMapObjBall::calcCurrentMtx`).
    fn body_lift(self) -> f32 {
        match self {
            Kind::Papaya | Kind::Coconut => 40.0,
            Kind::Pine => 50.0,
            Kind::Durian => 45.0,
            Kind::Banana => 0.0,
        }
    }

    /// (gravity, bounce, friction, throw speed, throw rise, hit radius).
    fn physics(self) -> (f32, f32, f32, f32, f32, f32) {
        match self {
            Kind::Papaya => (0.3, 0.6, 0.97, 5.0, 20.0, 40.0),
            Kind::Coconut => (0.3, 0.6, 0.97, 2.0, 15.0, 40.0),
            Kind::Pine => (0.3, 0.6, 0.97, 2.0, 22.0, 50.0),
            Kind::Banana => (0.3, 0.6, 0.97, 5.0, 20.0, 40.0),
            Kind::Durian => (0.3, 0.6, 0.97, 0.01, 5.0, 50.0),
        }
    }
}

pub struct Fruit {
    pub kind: Kind,
    pub pos: V3,
    pub vel: V3,
    pub yaw: f32,
    pub held: bool,
    /// Where it grows back after Yoshi eats it (`TResetFruit`).
    pub home: V3,
    /// Ticks until it is back.
    pub gone: u32,
}

/// Ticks an eaten fruit stays away (a viewer choice; `TResetFruit`'s wait is stage data).
const REGROW: u32 = 600;

impl Fruit {
    pub fn new(kind: Kind, pos: V3) -> Fruit {
        Fruit {
            kind,
            pos,
            vel: V3::ZERO,
            yaw: 0.0,
            held: false,
            home: pos,
            gone: 0,
        }
    }

    /// Yoshi ate it: it grows back at home.
    pub fn eaten(&mut self) {
        self.held = false;
        self.pos = self.home;
        self.vel = V3::ZERO;
        self.gone = REGROW;
    }

    /// Lying about and takeable.
    pub fn loose(&self) -> bool {
        !self.held && self.gone == 0
    }

    pub fn takeable(&self, id: usize) -> Takeable {
        Takeable {
            id: id as u64,
            pos: self.pos,
            radius: self.kind.physics().5,
            fruit: Some(self.kind.fruit()),
        }
    }

    /// `TMapObjGeneral::put`: in front of Mario, on the ground.
    pub fn put(&mut self, mario: V3, yaw: i16) {
        let a = yaw as f32 * std::f32::consts::PI / 32768.0;
        let d = self.kind.physics().5 + 50.0 + 10.0;
        self.pos = V3::new(mario.x + a.sin() * d, mario.y, mario.z + a.cos() * d);
        self.vel = V3::ZERO;
        self.held = false;
    }

    /// `TMapObjGeneral::thrown`.
    pub fn throw(&mut self, mario: V3, yaw: i16, power: f32, vel: V3) {
        let (_, _, _, speed, rise, _) = self.kind.physics();
        let a = yaw as f32 * std::f32::consts::PI / 32768.0;
        self.vel = V3::new(
            power * a.sin() * speed + 0.5 * vel.x,
            rise,
            power * a.cos() * speed + 0.5 * vel.z,
        );
        self.pos = mario + self.vel;
        self.yaw = a;
        self.held = false;
    }

    /// One 120 Hz tick of flight, bounce and roll over `floor(pos)`.
    pub fn tick(&mut self, floor: impl Fn(V3) -> f32) {
        if self.gone > 0 {
            self.gone -= 1;
            return;
        }
        if self.held {
            return;
        }
        let (g, bounce, friction, ..) = self.kind.physics();
        self.vel.y -= g;
        self.pos += self.vel * 0.25;
        let f = floor(self.pos + V3::Y * 30.0);
        if self.pos.y <= f {
            self.pos.y = f;
            if self.vel.y < -2.0 {
                self.vel.y = -self.vel.y * bounce;
            } else {
                self.vel.y = 0.0;
            }
            self.vel.x *= friction;
            self.vel.z *= friction;
        }
    }
}

pub struct Props {
    renderer: Renderer,
    models: Vec<(Kind, GpuModel, Vec<Mat4>)>,
}

impl Props {
    pub fn load(dir: &Path) -> Props {
        let mut renderer = Renderer::new();
        let mut models = Vec::new();
        for kind in [
            Kind::Papaya,
            Kind::Coconut,
            Kind::Pine,
            Kind::Banana,
            Kind::Durian,
        ] {
            let Some(bmd) = std::fs::read(dir.join(kind.file()))
                .ok()
                .and_then(|d| Bmd::parse(&d).ok())
            else {
                continue;
            };
            let bind = bmd.bind_world();
            if let Ok(m) = renderer.load(bmd) {
                models.push((kind, m, bind));
            }
        }
        Props { renderer, models }
    }

    /// Draws `fruit` at `at` (decomp units to scene through `scene`).
    pub fn draw(&mut self, kind: Kind, at: Mat4, view: Mat4, proj: Mat4) {
        let lights = Lights::daylight(view);
        let Some((_, model, bind)) = self.models.iter_mut().find(|m| m.0 == kind) else {
            return;
        };
        let pose = Pose::new(&model.bmd, at, bind);
        for pass in [Pass::Opaque, Pass::Translucent] {
            self.renderer
                .draw(model, &pose, view, proj, &lights, &Look::default(), pass);
        }
    }

    pub fn loaded(&self) -> bool {
        !self.models.is_empty()
    }
}

/// A loose fruit's placement: scaled, turned about Y, resting at `pos` (decomp units).
pub fn place(kind: Kind, pos: V3, yaw: f32, scale: f32) -> Mat4 {
    let lift = kind.body_lift();
    Mat4::from_translation(Vec3::new(pos.x, pos.y + lift, pos.z) * scale)
        * Mat4::from_rotation_y(yaw)
        * Mat4::from_scale(Vec3::splat(scale))
}
