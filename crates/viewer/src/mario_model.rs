//! Mario as the game draws him, from the user's extracted `mario.szs`: the body skinned by the
//! clip the sim is playing, the hand models `changeHand` picks per clip, the cap on `M_head`
//! (`TMarioCap`), FLUDD on `jnt_chest` and the current nozzle on FLUDD's `nozzle_center`
//! (`TWaterGun::init`, WaterGun.cpp), and the eyes' blink pattern.

use std::path::Path;

use glam::{Mat4, Vec3};
use libsms::mario::UpperState;
use libsms::{Clip, NozzleKind, Sim};
use libsms_assets::btp::TexPattern;
use libsms_assets::mario::{BODY_HAND_SHAPES, HIDDEN_SHAPE, Hands, SHIRT_SHAPE, hands_for_clip};
use libsms_assets::{Bmd, Pose, paths};

use crate::j3d::{GpuModel, Lights, Look, Pass, Renderer};

struct Part {
    model: GpuModel,
    bind: Vec<Mat4>,
}

impl Part {
    fn load(r: &mut Renderer, dir: &Path, rel: &str) -> Result<Part, String> {
        let data = std::fs::read(dir.join(rel)).map_err(|e| format!("{rel}: {e}"))?;
        let bmd = Bmd::parse(&data).map_err(|e| format!("{rel}: {e}"))?;
        let bind = bmd.bind_world();
        let model = r.load(bmd).map_err(|e| format!("{rel}: {e}"))?;
        Ok(Part { model, bind })
    }

    /// Model-space joint matrices with `clip` at `frame`; joints it doesn't animate keep
    /// their bind transform.
    fn posed(&self, clip: Option<&Clip>, frame: f32) -> Vec<Mat4> {
        let b = &self.model.bmd;
        let mut out: Vec<Mat4> = Vec::with_capacity(b.joints.len());
        for (j, jt) in b.joints.iter().enumerate() {
            let local = match clip.and_then(|c| c.sample(j, frame)) {
                Some(s) => libsms_assets::pose::srt_matrix(s.scale, s.rot, s.trans),
                None => libsms_assets::pose::srt_matrix(jt.scale, jt.rotation, jt.translation),
            };
            out.push(match jt.parent {
                Some(p) if p < out.len() => out[p] * local,
                _ => local,
            });
        }
        out
    }
}

pub struct MarioModel {
    /// Wearing the Shine Shirt (off at the start of the game).
    pub shirt: bool,
    renderer: Renderer,
    body: Part,
    /// [open, fists] x [right, left]
    hands: [[Part; 2]; 2],
    cap: Part,
    fludd: Part,
    /// Spray, Hover, Rocket, Turbo.
    nozzles: [Option<Part>; 4],
    /// FLUDD's handles folded away (`wg_house`) and held for pumping (`wg_pump`).
    fludd_house: Option<Clip>,
    fludd_pump: Option<Clip>,
    /// `TWaterGun::unk1CEC`: 1 = handles fully housed.
    housed: f32,
    last_time: f32,
    wink: Option<TexPattern>,
    eye_materials: Vec<usize>,
    hand_joints: [Option<usize>; 2],
    head: Option<usize>,
    chest: Option<usize>,
    blink_at: f32,
}

impl MarioModel {
    pub fn load(dir: &Path) -> Result<MarioModel, String> {
        let mut r = Renderer::new();
        let body = Part::load(&mut r, dir, paths::BODY)?;
        let load_pair = |r: &mut Renderer, p: [&str; 2]| -> Result<[Part; 2], String> {
            Ok([Part::load(r, dir, p[0])?, Part::load(r, dir, p[1])?])
        };
        let hands = [
            load_pair(&mut r, paths::HANDS_OPEN)?,
            load_pair(&mut r, paths::HANDS_FIST)?,
        ];
        let cap = Part::load(&mut r, dir, paths::CAPS[0])?;
        let fludd = Part::load(&mut r, dir, paths::FLUDD_BODY)?;
        let nozzles = [
            paths::NOZZLE_SPRAY,
            paths::NOZZLE_HOVER,
            paths::NOZZLE_ROCKET,
            paths::NOZZLE_TURBO,
        ]
        .map(|p| Part::load(&mut r, dir, p).ok());
        // The nozzles ship an 8x8 stand-in for FLUDD's env-map texture; the game points it at
        // FLUDD's own (`SMS_ChangeTextureAll(.., "H_watergun_main_dummy", wg_mdl1 texture 1)`,
        // `TWaterGun::init`).
        let mut nozzles = nozzles;
        if let Some(&real) = fludd.model.textures.get(1) {
            for part in nozzles.iter_mut().flatten() {
                for (i, t) in part.model.bmd.textures.iter().enumerate() {
                    if t.name == "H_watergun_main_dummy" {
                        part.model.textures[i] = real;
                    }
                }
            }
        }
        let clip = |name: &str| {
            std::fs::read(dir.join(format!("watergun2/body/{name}.bck")))
                .ok()
                .and_then(|d| Clip::parse(&d))
        };
        let wink = std::fs::read(dir.join(paths::WINK))
            .ok()
            .and_then(|d| TexPattern::parse(&d).ok());
        let b = &body.model.bmd;
        let eye_materials = wink
            .as_ref()
            .map(|w| {
                w.tracks
                    .iter()
                    .map(|t| b.material(&t.material).unwrap_or(usize::MAX))
                    .collect()
            })
            .unwrap_or_default();
        Ok(MarioModel {
            shirt: false,
            hand_joints: [b.joint("jnt_hand_R"), b.joint("jnt_hand_L")],
            head: b.joint("M_head"),
            chest: b.joint("jnt_chest"),
            renderer: r,
            body,
            hands,
            cap,
            fludd,
            nozzles,
            fludd_house: clip("wg_house"),
            fludd_pump: clip("wg_pump"),
            housed: 1.0,
            last_time: 0.0,
            wink,
            eye_materials,
            blink_at: 2.0,
        })
    }

    /// Draws Mario. `joints` are the body's model-space joint matrices for this frame (from
    /// `Skeleton::pose`); `root` places model space (decomp units) in the scene.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        sim: &Sim,
        joints: &[Mat4],
        root: Mat4,
        view: Mat4,
        proj: Mat4,
        time: f32,
    ) {
        let m = &sim.mario;
        // `SMS_HIDE=fludd,nozzle,cap,hands,body` leaves parts out (for checking placement).
        let hide = std::env::var("SMS_HIDE").unwrap_or_default();
        let shown = |part: &str| !hide.split(',').any(|h| h == part);
        let lights = Lights::daylight(view);
        let body_pose = Pose::new(&self.body.model.bmd, root, joints);
        let world = |j: Option<usize>| j.and_then(|j| body_pose.joints.get(j).copied());

        // Blink every few seconds (the game runs `ma_wink_tx` on idle; 30 frames per second).
        if time > self.blink_at + 2.0 {
            self.blink_at = time + 2.0 + (time * 7.13).fract() * 2.5;
        }
        let mut body_look = Look {
            hidden: vec![HIDDEN_SHAPE],
            k0_alpha: Some(0),
            ..Default::default()
        };
        if let Some(w) = &self.wink {
            let frame = (time - self.blink_at) * 30.0;
            if frame >= 0.0 {
                for (i, t) in w.tracks.iter().enumerate() {
                    if let (Some(&mat), Some(tex)) = (self.eye_materials.get(i), w.tex_no(i, frame))
                    {
                        body_look.tex_no.push((mat, t.tex_map, tex));
                    }
                }
            }
        }
        if !self.shirt {
            body_look.hidden.push(SHIRT_SHAPE);
        }
        let hands = hands_for_clip(m.anim.clip);
        if hands != Hands::Body {
            body_look.hidden.extend(BODY_HAND_SHAPES);
        }
        let body_tex0 = self.body.model.textures.first().copied();
        let part_look = Look {
            texture0: body_tex0,
            k0_alpha: Some(0),
            ..Default::default()
        };

        // FLUDD's handles (`TWaterGun::calcAnimation`): pumping holds `wg_pump`; otherwise
        // `wg_house` runs toward its last frame, folding them away.
        let dt = (time - self.last_time).clamp(0.0, 0.1);
        self.last_time = time;
        let pumping = matches!(m.upper, UpperState::Pumping | UpperState::HoldingPump);
        if pumping {
            self.housed = 0.0;
        } else {
            self.housed = (self.housed + dt * 3.0).min(1.0);
        }
        let fludd_joints = if pumping {
            self.fludd.posed(self.fludd_pump.as_ref(), 0.0)
        } else {
            let end = self.fludd_house.as_ref().map_or(0.0, |c| c.frames);
            self.fludd
                .posed(self.fludd_house.as_ref(), self.housed * end)
        };

        // FLUDD on the chest, the nozzle on FLUDD.
        let fludd_root = m.fludd.as_ref().and_then(|_| world(self.chest));
        let nozzle = m.fludd.as_ref().and_then(|f| match f.nozzle {
            NozzleKind::Spray => Some(0),
            NozzleKind::Hover => Some(1),
            NozzleKind::Rocket => Some(2),
            NozzleKind::Turbo => Some(3),
            _ => None,
        });

        for pass in [Pass::Opaque, Pass::Translucent] {
            let r = &mut self.renderer;
            if shown("body") {
                r.draw(
                    &mut self.body.model,
                    &body_pose,
                    view,
                    proj,
                    &lights,
                    &body_look,
                    pass,
                );
            }
            if let Some(head) = world(self.head).filter(|_| shown("cap")) {
                let pose = Pose::new(&self.cap.model.bmd, head, &self.cap.bind);
                r.draw(
                    &mut self.cap.model,
                    &pose,
                    view,
                    proj,
                    &lights,
                    &part_look,
                    pass,
                );
            }
            let set = match hands {
                Hands::Open => Some(0),
                Hands::Fists => Some(1),
                Hands::Body => None,
            };
            if let Some(s) = set.filter(|_| shown("hands")) {
                for side in 0..2 {
                    if let Some(at) = world(self.hand_joints[side]) {
                        let part = &mut self.hands[s][side];
                        let pose = Pose::new(&part.model.bmd, at, &part.bind);
                        r.draw(
                            &mut part.model,
                            &pose,
                            view,
                            proj,
                            &lights,
                            &part_look,
                            pass,
                        );
                    }
                }
            }
            if let Some(at) = fludd_root {
                let pose = Pose::new(&self.fludd.model.bmd, at, &fludd_joints);
                if shown("fludd") {
                    r.draw(
                        &mut self.fludd.model,
                        &pose,
                        view,
                        proj,
                        &lights,
                        &Look::default(),
                        pass,
                    );
                }
                let center = self
                    .fludd
                    .model
                    .bmd
                    .joint("nozzle_center")
                    .map(|j| at * fludd_joints[j]);
                let nozzle = nozzle.filter(|_| shown("nozzle"));
                if let (Some(c), Some(Some(part))) = (center, nozzle.map(|i| &mut self.nozzles[i]))
                {
                    let pose = Pose::new(&part.model.bmd, c, &part.bind);
                    r.draw(
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
}

/// Model space (decomp units, +Z forward) to the scene: scaled by `scale`, turned by `yaw`
/// radians about +Y, placed at `pos`.
pub fn root_matrix(pos: Vec3, yaw: f32, scale: f32) -> Mat4 {
    Mat4::from_translation(pos) * Mat4::from_rotation_y(yaw) * Mat4::from_scale(Vec3::splat(scale))
}
