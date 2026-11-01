//! Sunshine's sambo flowers (`Enemy/hanasambo.cpp`) as the viewer draws them:
//!
//! * the plain flower (`enemy/samboflower/flower.bmd`) with **`flower_orange.bmt` swapped in**
//!   as its material table: the model's own material draws the placeholder `K_name_dummy`
//!   8x8 texture, and `TSamboFlowerManager::load` (`unk64`) plus
//!   `TSamboFlower::setMActorAndKeeper` (`setMaterialTable(table, J3DMatCopyFlag_All)`) give
//!   it the disc's orange flower texture instead. Its clips are `flower_wait` (a bud),
//!   `flower_hit` (opening) and `flower_fwait` (open);
//! * the flower tower (`enemy/sambo/sambo.bmd`, `THanaSambo`), whose `sambo_f*` clips burst it
//!   out of the ground and bite;
//! * the hopping head (`enemy/sambohead/sambohead.bmd`, `TSamboHead`).
//!
//! A tower or head that is buried leaves a **plain flower** standing where it is
//! (`THanaSambo::initFlower` / `TSamboHead::initFlower` spawn a `TSamboFlower` at the
//! enemy's position) and hides it again when the enemy bursts out (`TNerveHanaSamboAppear` /
//! `TNerveSamboHeadAppear`: `unk1A8->hide()`). `SMS_FLOWER=<frame>` places that scene in
//! front of the spawn: a bud, an open flower, a tower bursting out and a hopping head, all at
//! that clip frame (so captures at several frames show the models animating).

use std::collections::HashMap;
use std::path::Path;

use glam::{Mat4, Vec3};
use libsms::{Clip, Vec3 as V3};
use libsms_assets::{MaterialTable, Pose};

use crate::j3d::{Lights, Look, Pass, Renderer};
use crate::mario_model::Part;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Kind {
    /// `TSamboFlower`: the sprayable flower, and what a buried enemy shows in its place.
    Flower,
    /// `THanaSambo`: the flower-headed tower.
    Tower,
    /// `TSamboHead`: the hopping flower head.
    Head,
}

fn load_clips(dir: &Path, folder: &str) -> HashMap<String, Clip> {
    let mut clips = HashMap::new();
    let Ok(entries) = std::fs::read_dir(dir.join(folder)) else {
        return clips;
    };
    for e in entries.flatten() {
        let path = e.path();
        if path.extension().and_then(|x| x.to_str()) != Some("bck") {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|x| x.to_str()) else {
            continue;
        };
        if let Some(c) = std::fs::read(&path).ok().and_then(|d| Clip::parse(&d)) {
            clips.insert(name.to_lowercase(), c);
        }
    }
    clips
}

pub struct Flowers {
    renderer: Renderer,
    parts: HashMap<Kind, Part>,
    clips: HashMap<(Kind, String), Clip>,
}

impl Flowers {
    pub fn load(dir: &Path) -> Result<Flowers, String> {
        let mut renderer = Renderer::new();
        // The plain flower's material table, as the game applies it. `SMS_FLOWER_NOTABLE=1`
        // leaves it out, for the before/after capture (the model alone keeps its placeholder
        // `K_name_dummy` material).
        let path = dir.join("enemy/samboflower/flower_orange.bmt");
        let mut table = None;
        if std::env::var_os("SMS_FLOWER_NOTABLE").is_none()
            && let Ok(data) = std::fs::read(&path)
        {
            table = Some(
                MaterialTable::parse(&data).map_err(|e| format!("{}: {e}", path.display()))?,
            );
        }
        let flower = Part::load_edited(
            &mut renderer,
            dir,
            "enemy/samboflower/flower.bmd",
            |bmd| {
                if let Some(t) = &table {
                    bmd.set_material_table(t);
                }
            },
        )?;
        let tower = Part::load(&mut renderer, dir, "enemy/sambo/sambo.bmd")?;
        let head = Part::load(&mut renderer, dir, "enemy/sambohead/sambohead.bmd")?;
        let mut clips = HashMap::new();
        for (kind, folder) in [
            (Kind::Flower, "enemy/samboflower"),
            (Kind::Tower, "enemy/sambo"),
            (Kind::Head, "enemy/sambohead"),
        ] {
            for (name, c) in load_clips(dir, folder) {
                clips.insert((kind, name), c);
            }
        }
        Ok(Flowers {
            renderer,
            parts: HashMap::from([(Kind::Flower, flower), (Kind::Tower, tower), (Kind::Head, head)]),
            clips,
        })
    }

    /// Draws `kind` at `at` (decomp units to the scene) playing `clip` at `frame`. A clip the
    /// folder lacks is drawn in the bind pose.
    pub fn draw(&mut self, kind: Kind, clip: &str, frame: f32, at: Mat4, view: Mat4, proj: Mat4) {
        let Some(part) = self.parts.get(&kind) else {
            return;
        };
        let c = self.clips.get(&(kind, clip.to_lowercase()));
        let joints = part.posed(c, frame, |_| None);
        let pose = Pose::new(&part.model.bmd, at, &joints);
        let lights = Lights::daylight(view);
        let renderer = &mut self.renderer;
        let part = self.parts.get_mut(&kind).unwrap();
        for pass in [Pass::Opaque, Pass::Translucent] {
            renderer.draw(
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

/// A prop's placement: at `pos`, turned by `yaw` radians, scaled to the scene.
pub fn place(pos: V3, yaw: f32, scale: f32) -> Mat4 {
    Mat4::from_translation(Vec3::new(pos.x, pos.y, pos.z) * scale)
        * Mat4::from_rotation_y(yaw)
        * Mat4::from_scale(Vec3::splat(scale))
}
