//! The diving helmet on any skin's head (viewer-only; the Ship Repair idea).
//!
//! Spec: `TMarioCap::TMarioCap` (MarioCap.cpp) loads `/mario/watergun2/body/diver_helm.bmd`
//! as cap model 2 and parents it to Mario's `jnt_head` (`mJointIdHead`, MarioDraw.cpp), not
//! to the hat's `M_head`; `TMario::setDivHelm` / `MARIO_FLAG_HELMET_FLW_CAMERA` switch it on
//! (`TMarioCap::perform` draws the model while `E_CAP_MODEL_HELMET` is set). The hat is
//! left as it is, so it shows inside the glass. Other skins get it on the head joint their
//! rig maps to `jnt_head`, with a per-skin fit ([`Fit`]) because only Mario's head is the
//! size the model was built for.
use std::path::Path;

use glam::{Mat4, Vec3};
use libsms_assets::Pose;

use crate::j3d::{Lights, Look, Pass, Renderer};
use crate::mario_model::Part;

/// Extra placement in the head joint's frame, decomp units: offset, then uniform scale.
#[derive(Clone, Copy, Debug)]
pub struct Fit {
    pub offset: Vec3,
    pub scale: f32,
}

impl Fit {
    pub const MARIO: Fit = Fit { offset: Vec3::ZERO, scale: 1.0 };

    /// `SMS_HELMET_ADJ=x,y,z,scale` overrides every skin's fit (for tuning captures).
    pub fn from_env(default: Fit) -> Fit {
        let Ok(v) = std::env::var("SMS_HELMET_ADJ") else { return default };
        let n: Vec<f32> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        match n[..] {
            [x, y, z, s] => Fit { offset: Vec3::new(x, y, z), scale: s },
            _ => default,
        }
    }
}

pub struct Helmet {
    renderer: Renderer,
    part: Part,
}

impl Helmet {
    pub fn load(dir: &Path) -> Result<Helmet, String> {
        let mut renderer = Renderer::new();
        let part = Part::load(&mut renderer, dir, "watergun2/body/diver_helm.bmd")?;
        Ok(Helmet { renderer, part })
    }

    /// `head` is the head joint's matrix in model space (decomp units), `root` places model
    /// space in the scene. Drawn after the body so the glass blends over it.
    pub fn draw(&mut self, head: Mat4, fit: Fit, root: Mat4, view: Mat4, proj: Mat4) {
        let at = head * Mat4::from_translation(fit.offset) * Mat4::from_scale(Vec3::splat(fit.scale));
        let lights = Lights::daylight(view);
        let pose = Pose::new(&self.part.model.bmd, root * at, &self.part.bind);
        for pass in [Pass::Opaque, Pass::Translucent] {
            self.renderer.draw(&mut self.part.model, &pose, view, proj, &lights, &Look::default(), pass);
        }
    }
}
