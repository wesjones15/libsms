//! Coins (`TCoin`, Item.cpp) from the disc's `scene/mapobj/coin*.bmd`: they spin together on
//! the item manager's shared matrix (`TItemManager::perform`, 2 degrees a tick about Y) and
//! are taken when Mario's body touches their cylinder (`coin_hit_data_table`: radius 70,
//! height 100). Taken coins come back after a while here, which the game's don't.

use std::path::Path;

use glam::{Mat4, Vec3};
use libsms::{Coin as Kind, Vec3 as V3};
use libsms_assets::Pose;

use crate::j3d::{Lights, Look, Pass, Renderer};
use crate::mario_model::Part;

pub const RADIUS: f32 = 70.0;
pub const HEIGHT: f32 = 100.0;
/// Ticks a taken coin stays away (a viewer choice).
const RESPAWN: u32 = 1200;

pub struct Coin {
    pub kind: Kind,
    pub pos: V3,
    /// Ticks until it is back after being taken.
    pub gone: u32,
}

impl Coin {
    pub fn new(kind: Kind, pos: V3) -> Coin {
        Coin { kind, pos, gone: 0 }
    }

    pub fn tick(&mut self) {
        self.gone = self.gone.saturating_sub(1);
    }

    /// Does a body at `p` (feet), `radius` wide and `height` tall, touch it?
    pub fn touches(&self, p: V3, radius: f32, height: f32) -> bool {
        let d = V3::new(self.pos.x - p.x, 0.0, self.pos.z - p.z).length();
        self.gone == 0
            && d < RADIUS + radius
            && self.pos.y + HEIGHT > p.y
            && self.pos.y < p.y + height
    }

    pub fn take(&mut self) {
        self.gone = RESPAWN;
    }
}

pub struct CoinModel {
    renderer: Renderer,
    /// Gold, red, blue.
    parts: [Option<Part>; 3],
}

impl CoinModel {
    pub fn load(dir: &Path) -> Result<CoinModel, String> {
        let mut renderer = Renderer::new();
        let gold = Part::load(&mut renderer, dir, "scene/mapobj/coin.bmd")?;
        let red = Part::load(&mut renderer, dir, "scene/mapobj/coin_red.bmd").ok();
        let blue = Part::load(&mut renderer, dir, "scene/mapobj/coin_blue.bmd").ok();
        Ok(CoinModel {
            renderer,
            parts: [Some(gold), red, blue],
        })
    }

    /// Draws every coin still out; `ticks` turns them (`unk74 += 2` a tick).
    pub fn draw(&mut self, coins: &[Coin], ticks: u64, scale: f32, view: Mat4, proj: Mat4) {
        let spin = ((ticks as f32 * 2.0) % 360.0).to_radians();
        let lights = Lights::daylight(view);
        for pass in [Pass::Opaque, Pass::Translucent] {
            for c in coins.iter().filter(|c| c.gone == 0) {
                let i = match c.kind {
                    Kind::Gold => 0,
                    Kind::Red => 1,
                    Kind::Blue => 2,
                };
                let Some(part) = self.parts[i].as_mut() else {
                    continue;
                };
                let at = Mat4::from_translation(Vec3::new(c.pos.x, c.pos.y, c.pos.z) * scale)
                    * Mat4::from_rotation_y(spin)
                    * Mat4::from_scale(Vec3::splat(scale));
                let pose = Pose::new(&part.model.bmd, at, &part.bind);
                self.renderer.draw(
                    &mut part.model,
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
}
