//! FLUDD as a model of its own (`watergun2/`), so a character other than Mario can wear one.
//!
//! Mario's model used to own FLUDD and every nozzle; Luigi then borrowed them by hiding
//! Mario's body. Here the pack, its five nozzles and their clips are one component: a host
//! hands it the chest frame it is strapped to and the sim's [`FluddLook`], and gets FLUDD's
//! posed muzzles back for `Mario::set_emitters`.
//!
//! The assembly follows `TWaterGun::init` / `TWaterGun::setBaseTRMtx` (WaterGun.cpp): the base
//! sits on the chest and tilts back as the chest leans; the body clip poses it; the nozzle
//! hangs on `nozzle_center` with its own clip and the aiming callbacks.

use std::collections::HashMap;
use std::path::Path;

use glam::{Mat4, Vec3};
use libsms::{Clip, FluddLook, NozzleKind};
use libsms_assets::{Pose, paths};

use crate::j3d::{Lights, Look, Pass, Renderer};
use crate::mario_model::Part;

/// Luigi alt-skin: the Spray neck leans to one side so it clears his head. Degrees about
/// [`LEAN_PIVOT`] around Y (the model's side axis), applied to [`LEAN_JOINTS`] only.
pub const LEAN_DEG: f32 = 45.0;
/// The collar base, measured on the posed model.
pub const LEAN_PIVOT: Vec3 = Vec3::new(37.0, -40.0, 0.0);
/// `null_pump_wA`, `null_pump_wB`, `nozzle_center`, `jnt_G_handle`: the neck above the collar
/// base. The levers and grips are not part of it, and the tank (`jnt_G_main`) stays put.
pub const LEAN_JOINTS: [usize; 4] = [4, 5, 6, 7];
/// The thin frame rod (`_mat_G_main`); it would poke out of the tank when the neck leans.
pub const ROD_SHAPE: usize = 3;
/// How fast the neck swings to the other side, degrees per second: about two frames for the
/// full +45 to -45, quick because the water visibly breaks while it turns.
pub const LEAN_RATE: f32 = 1800.0;
/// How far off the facing the aim can go before the lean starts to fade out, and where it is
/// gone: aiming behind, the swivelled head needs no clearance, so the neck is authored again.
pub const LEAN_FADE_FROM: f32 = 120.0;
pub const LEAN_FADE_TO: f32 = 180.0;

/// One FLUDD: its body, its nozzles and every `watergun2` clip.
pub struct Fludd {
    /// Fit only shape 1 (_mat_froater) around the wearer; all other shapes retain scale.
    pub belt_scale: f32,
    /// Its own renderer, so a character with no Mario model can own a FLUDD.
    renderer: Renderer,
    body: Part,
    /// Spray, Hover, Rocket, Turbo and Yoshi's `dummy_wg`.
    nozzles: [Option<Part>; 5],
    /// Every `watergun2/*/*.bck` by file stem: FLUDD's and the nozzles' clips.
    clips: HashMap<String, Clip>,
    /// `nozzle_center` on the body.
    centre: Option<usize>,
    /// The current lean, in degrees, eased towards its target; and the side it last chose
    /// (+1 the right-hand lean, -1 the mirrored one, picked from where the host aims).
    lean: f32,
    side: f32,
}

impl Fludd {
    pub fn load(dir: &Path) -> Result<Fludd, String> {
        let mut renderer = Renderer::new();
        let body = Part::load(&mut renderer, dir, paths::FLUDD_BODY)?;
        let mut nozzles = [
            paths::NOZZLE_SPRAY,
            paths::NOZZLE_HOVER,
            paths::NOZZLE_ROCKET,
            paths::NOZZLE_TURBO,
            paths::NOZZLE_YOSHI,
        ]
        .map(|p| Part::load(&mut renderer, dir, p).ok());
        // The nozzles ship an 8x8 stand-in for FLUDD's env-map texture; the game points it at
        // FLUDD's own (`SMS_ChangeTextureAll(.., "H_watergun_main_dummy", wg_mdl1 texture 1)`,
        // `TWaterGun::init`).
        if let Some(&real) = body.model.textures.get(1) {
            for part in nozzles.iter_mut().flatten() {
                for (i, t) in part.model.bmd.textures.iter().enumerate() {
                    if t.name == "H_watergun_main_dummy" {
                        part.model.textures[i] = real;
                    }
                }
            }
        }
        let mut clips = HashMap::new();
        for folder in std::fs::read_dir(dir.join("watergun2"))
            .into_iter()
            .flatten()
            .flatten()
        {
            for f in std::fs::read_dir(folder.path())
                .into_iter()
                .flatten()
                .flatten()
            {
                let path = f.path();
                if path.extension().and_then(|e| e.to_str()) != Some("bck") {
                    continue;
                }
                let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                if let Some(c) = std::fs::read(&path).ok().and_then(|d| Clip::parse(&d)) {
                    clips.insert(stem.to_owned(), c);
                }
            }
        }
        let centre = body.model.bmd.joint("nozzle_center");
        Ok(Fludd {
            belt_scale: 1.,
            renderer,
            body,
            nozzles,
            clips,
            centre,
            lean: 0.0,
            side: 1.0,
        })
    }

    /// The nozzles, for a host that wants to draw one on its own (the Air Pump portrait).
    pub(crate) fn nozzles_mut(&mut self) -> &mut [Option<Part>; 5] {
        &mut self.nozzles
    }

    /// Draws FLUDD on `chest` (in decomp units, relative to `root`) and returns its posed
    /// muzzle matrices in model space (for `Mario::set_emitters` and the nozzle effects).
    /// `deployed` is `housed < 0.5`. `alt_skin` is the tall head that needs the Spray neck
    /// leaned out of the way; the game's Mario does not. `dt` eases the neck from one side
    /// to the other. `show` is (body, nozzle).
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        look: Option<FluddLook>,
        deployed: bool,
        alt_skin: bool,
        dt: f32,
        chest: Mat4,
        root: Mat4,
        view: Mat4,
        proj: Mat4,
        show: (bool, bool),
    ) -> Vec<Mat4> {
        let lights = Lights::daylight(view);
        let deg = |a: f32| a * std::f32::consts::PI / 32768.0;
        // `setBaseTRMtx`: the pack tilts back as the chest leans, `|m[1][0]| * (0 - -0x1800)`.
        let lean = chest.x_axis.truncate().normalize_or_zero().y.abs();
        let base = chest * Mat4::from_rotation_z(deg(lean * 6144.0 - 6144.0));
        let mut joints = self.body.posed(
            look.and_then(|l| self.clips.get(l.body_clip)),
            look.map_or(0.0, |l| l.body_frame),
            |_| None,
        );
        // The Spray neck leans out of the way of a taller head, to the side the host aims
        // past (the water would cross his face otherwise). Only the neck joints above the
        // collar base move; the tank and the frame rod stay where the game puts them.
        let lean_env = std::env::var("SMS_LEAN")
            .ok()
            .and_then(|v| v.parse::<f32>().ok());
        let spraying = deployed && look.is_some_and(|l| l.nozzle == NozzleKind::Spray);
        // Crossed well past the facing plane, so aiming straight ahead cannot flicker it.
        if let Some(l) = look
            && l.aimed
            && l.aim_turn.unsigned_abs() > 512
        {
            self.side = if l.aim_turn > 0 { -1.0 } else { 1.0 };
        }
        // Aiming behind him, the head swivels the nozzle round past his neck, so the neck has
        // nothing to clear and goes back to the authored middle; the lean fades out over the
        // last stretch (`LEAN_FADE_FROM`..`LEAN_FADE_TO`) so a sweep reads as one motion. It
        // also takes the sign flip at a dead-behind aim out of the picture.
        let clear = look
            .filter(|l| l.aimed)
            .map_or(1.0, |l| {
                let deg = (l.aim_turn as f32 * 180.0 / 32768.0).abs();
                ((LEAN_FADE_TO - deg) / (LEAN_FADE_TO - LEAN_FADE_FROM)).clamp(0.0, 1.0)
            });
        // `SMS_LEAN=<deg>` forces an angle for A/B captures; 0 means "as authored" and keeps
        // the frame rod, which only has to go when the neck actually swings out.
        let target = match lean_env {
            Some(d) => d,
            None if alt_skin && spraying => self.side * LEAN_DEG * clear,
            None => 0.0,
        };
        let step = LEAN_RATE * dt.max(0.0);
        self.lean += (target - self.lean).clamp(-step, step);
        if lean_env.is_some() {
            self.lean = target;
        }
        let leaning = self.lean.abs() > 0.5;
        if leaning {
            let pivot = LEAN_PIVOT;
            let m = Mat4::from_translation(pivot)
                * Mat4::from_rotation_y(self.lean.to_radians())
                * Mat4::from_translation(-pivot);
            for i in LEAN_JOINTS {
                if let Some(j) = joints.get_mut(i) {
                    *j = m * *j;
                }
            }
        }
        let slot = look.map(|l| match l.nozzle {
            NozzleKind::Spray => 0,
            NozzleKind::Hover | NozzleKind::Underwater => 1,
            NozzleKind::Rocket => 2,
            NozzleKind::Turbo => 3,
            // The water leaves Yoshi's mouth (`getEmitMtx`); FLUDD shows `dummy_wg`.
            NozzleKind::Yoshi => 4,
        });
        let nozzle_at = self.centre.map(|c| base * joints[c]);
        let mut muzzles = Vec::new();
        let nozzle_joints = match (look, slot.and_then(|i| self.nozzles[i].as_ref())) {
            (Some(l), Some(part)) => {
                let extra = |name: &str| match name {
                    // `NozzleCtrl`: a host aim (the lock-on) swivels the head round to the
                    // target, and the neck dips with the aim, never above level.
                    "chn_muzzle_1" | "jnt_back_nozzle_neck" if l.aimed => {
                        Some(Mat4::from_rotation_y(deg(l.aim_turn as f32))
                            * Mat4::from_rotation_z(deg(l.gun_angle)))
                    }
                    "chn_muzzle_1" | "jnt_back_nozzle_neck" if l.gun_angle < 0.0 => {
                        Some(Mat4::from_rotation_z(deg(l.gun_angle)))
                    }
                    // `WaterGunDivingCtrlL/R`: the hover jets lean with the steering.
                    "jnt_nozzle_L" => Some(Mat4::from_rotation_z(deg(-l.tilt[0]))),
                    "jnt_nozzle_R" => Some(Mat4::from_rotation_z(deg(-l.tilt[1]))),
                    // `RotateCtrl`: the Turbo propeller.
                    "chn_back_nozzle_prop" => Some(Mat4::from_rotation_x(deg(l.prop_angle))),
                    _ => None,
                };
                let clip = l.nozzle_clip.and_then(|c| self.clips.get(c));
                let js = part.posed(clip, l.nozzle_frame, extra);
                if let Some(at) = nozzle_at.filter(|_| l.nozzle != NozzleKind::Yoshi) {
                    for name in ["null_G_muzzle", "null_G_muzzle2"] {
                        if let Some(j) = part.model.bmd.joint(name) {
                            muzzles.push(at * js[j]);
                        }
                    }
                }
                js
            }
            _ => Vec::new(),
        };
        let body_look = Look {
            hidden: if leaning { vec![ROD_SHAPE] } else { Vec::new() },
            ..Default::default()
        };
        for pass in [Pass::Opaque, Pass::Translucent] {
            if show.0 {
                let pose = Pose::new(&self.body.model.bmd, root * base, &joints);
                let mut body_look=body_look.clone();
                if (self.belt_scale-1.).abs()>0.001 {
                    let belt=pose.joints[1]; // G_center; shape 1 is rigidly attached here.
                    body_look.shape_transform=Some((1,belt*Mat4::from_scale(Vec3::new(1.,self.belt_scale,self.belt_scale))*belt.inverse()));
                }
                self.renderer.draw(
                    &mut self.body.model,
                    &pose,
                    view,
                    proj,
                    &lights,
                    &body_look,
                    pass,
                );
            }
            if let (Some(at), Some(i)) = (nozzle_at, slot.filter(|_| show.1))
                && let Some(part) = self.nozzles[i].as_mut()
            {
                let pose = Pose::new(&part.model.bmd, root * at, &nozzle_joints);
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
        muzzles
    }
}
