//! Mario as the game draws him, from the user's extracted `mario.szs`: the body skinned by the
//! clip the sim is playing, the hand models `changeHand` picks per clip, the cap on `M_head`
//! (`TMarioCap`), FLUDD on `jnt_chest` and the current nozzle on FLUDD's `nozzle_center`
//! (`TWaterGun::init`, WaterGun.cpp), and the eyes' blink pattern. FLUDD's and the nozzle's
//! clips come from the sim (`Mario::fludd_look`); the nozzle joints get the game's aiming
//! callbacks, and the posed muzzles go back to the sim so water leaves the drawn nozzle.

use std::path::Path;

use glam::{Mat4, Vec3};
use libsms::{Clip, Sim};
use libsms_assets::btp::TexPattern;
use libsms_assets::mario::{BODY_HAND_SHAPES, HIDDEN_SHAPE, Hands, SHIRT_SHAPE, hands_for_clip};
use libsms_assets::{Bmd, Pose, paths};

use crate::fludd::Fludd;
use crate::j3d::{GpuModel, Lights, Look, Pass, Renderer};

pub(crate) struct Part {
    pub(crate) model: GpuModel,
    pub(crate) bind: Vec<Mat4>,
}

impl Part {
    pub(crate) fn load(r: &mut Renderer, dir: &Path, rel: &str) -> Result<Part, String> {
        let data = std::fs::read(dir.join(rel)).map_err(|e| format!("{rel}: {e}"))?;
        let bmd = Bmd::parse(&data).map_err(|e| format!("{rel}: {e}"))?;
        let bind = bmd.bind_world();
        let model = r.load(bmd).map_err(|e| format!("{rel}: {e}"))?;
        Ok(Part { model, bind })
    }

    /// Model-space joint matrices with `clip` at `frame`; joints it doesn't animate keep
    /// their bind transform. `extra` is a joint callback's rotation
    /// (`MTXConcat(J3DSys::mCurrentMtx, rot)`), which turns the joint's children.
    pub(crate) fn posed(
        &self,
        clip: Option<&Clip>,
        frame: f32,
        extra: impl Fn(&str) -> Option<Mat4>,
    ) -> Vec<Mat4> {
        let b = &self.model.bmd;
        b.pose(
            |j| clip.and_then(|c| c.sample(j, frame)),
            |j| extra(&b.joints[j].name),
        )
    }
}

pub struct MarioModel {
    /// Wearing the Shine Shirt (off at the start of the game).
    pub shirt: bool,
    /// Parts to leave out, comma separated (`SMS_HIDE`); per instance so a second
    /// copy can draw only FLUDD for another character.
    pub hide: String,
    renderer: Renderer,
    body: Part,
    /// [open, fists] x [right, left]
    hands: [[Part; 2]; 2],
    cap: Part,
    /// TMarioCap::perform uses ma_cap3 during the successful gate-return landing.
    return_cap: Option<Part>,
    /// MarioDraw.cpp: the right hand carrying the cap before get2 frame 158.
    return_hand: Option<Part>,
    /// The pack he wears (`watergun2/`), also worn by other characters.
    fludd: Fludd,
    wink: Option<TexPattern>,
    eye_materials: Vec<usize>,
    hand_joints: [Option<usize>; 2],
    head: Option<usize>,
    chest: Option<usize>,
    blink_at: f32,
}

impl MarioModel {
    /// Native nozzle-only still life for Air Pump. WaterGun.cpp's nozzleBmdData:
    /// hover_wg, back_wg, rocket_wg. Each keeps its own disc geometry/materials.
    pub fn draw_nozzle_group(&mut self, view: Mat4, proj: Mat4) {
        let lights = Lights::daylight(view);
        for (i, slot) in [1usize, 3, 2].into_iter().enumerate() {
            let Some(part) = self.fludd.nozzles_mut()[slot].as_mut() else { continue };
            let bind_pose = Pose::new(&part.model.bmd, Mat4::IDENTITY, &part.bind);
            let mut lo = Vec3::splat(f32::INFINITY);
            let mut hi = Vec3::splat(f32::NEG_INFINITY);
            for b in part.model.bmd.batches().expect("nozzle geometry") {
                for v in b.vertices {
                    let p = bind_pose.skin(&v).0;
                    lo = lo.min(p); hi = hi.max(p);
                }
            }
            let scale = libsms_present::view::NOZZLE_PORTRAIT_SPANS[i] * 0.01
                / (hi - lo).max_element();
            let at = [Vec3::new(0.0, 0.55, 0.0), Vec3::new(-0.66, -0.52, 0.0), Vec3::new(0.66, -0.52, 0.0)][i];
            let root = Mat4::from_translation(at)
                * Mat4::from_rotation_y(2.54)
                * Mat4::from_scale(Vec3::splat(scale))
                * Mat4::from_translation(-(lo + hi) * 0.5);
            let pose = Pose::new(&part.model.bmd, root, &part.bind);
            for pass in [Pass::Opaque, Pass::Translucent] {
                self.renderer.draw(&mut part.model, &pose, view, proj, &lights, &Look::default(), pass);
            }
        }
    }

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
        let return_cap = Part::load(&mut r, dir, paths::CAPS[1]).ok();
        let return_hand = Part::load(&mut r, dir, "bmd/ma_hnd4r.bmd").ok();
        let fludd = Fludd::load(dir)?;
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
            hide: std::env::var("SMS_HIDE").unwrap_or_default(),
            hand_joints: [b.joint("jnt_hand_R"), b.joint("jnt_hand_L")],
            head: b.joint("M_head"),
            chest: b.joint("jnt_chest"),
            renderer: r,
            body,
            hands,
            cap,
            return_cap,
            return_hand,
            fludd,
            wink,
            eye_materials,
            blink_at: 2.0,
        })
    }

    /// The lowest skinned point of the body for `joints`, in model space (decomp units):
    /// Luigi's alt-skin anchors his own lowest point to this so he stands, dives and hangs
    /// where Mario does.
    pub fn lowest(&self, joints: &[Mat4]) -> f32 {
        let pose = Pose::new(&self.body.model.bmd, Mat4::IDENTITY, joints);
        let mut low = f32::INFINITY;
        for b in self.body.model.bmd.batches().expect("body geometry") {
            for v in b.vertices {
                low = low.min(pose.skin(&v).0.y);
            }
        }
        low
    }

    /// Draws Mario. `joints` are the body's model-space joint matrices for this frame (from
    /// `Skeleton::pose_layered`); `root` places model space (decomp units) in the scene.
    /// Returns FLUDD's posed muzzle matrices in model space (for `Mario::set_emitters` and the
    /// nozzle's effects).
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        sim: &Sim,
        joints: &[Mat4],
        root: Mat4,
        view: Mat4,
        proj: Mat4,
        time: f32,
    ) -> Vec<Mat4> {
        let m = &sim.mario;
        // `SMS_HIDE=fludd,nozzle,cap,hands,body` leaves parts out (for checking placement).
        let hide = self.hide.clone();
        let shown = |part: &str| !hide.split(',').any(|h| h == part);
        let lights = Lights::daylight(view);
        let body_pose = Pose::new(&self.body.model.bmd, root, joints);
        let world = |j: Option<usize>| j.and_then(|j| body_pose.joints.get(j).copied());

        // Blink every few seconds (the game runs `ma_wink_tx` on idle; 30 frames per second).
        if time > self.blink_at + 2.0 {
            self.blink_at = time + 2.0 + (time * 7.13).fract() * 2.5;
        }
        // `addDamageFog` / `SMS_AddDamageFogEffect` (DrawUtil.cpp): while invincible, pink fog
        // from 700 units in front of Mario to 500 behind, both swinging 300 with
        // `sin(moveTicks * 0x888)`, on the body, cap and hands.
        let fog = (m.invincible > 0).then(|| {
            let d = -(view * root.w_axis).z;
            let s = (m.ticks as f32 * 0x888 as f32 * std::f32::consts::PI / 32768.0).sin();
            let at = |base: f32, swing: f32| d + (base + (swing - base) * s) * 0.01;
            (
                [1.0, 0.0, 128.0 / 255.0],
                at(-700.0, -400.0),
                at(500.0, 800.0),
            )
        });
        let mut body_look = Look {
            hidden: vec![HIDDEN_SHAPE],
            k0_alpha: Some(0),
            fog,
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
        let return_landing = m.anim.clip == "demo_gate_out_get2";
        let hands = if return_landing { Hands::Fists } else { hands_for_clip(m.anim.clip) };
        if hands != Hands::Body {
            body_look.hidden.extend(BODY_HAND_SHAPES);
        }
        let body_tex0 = self.body.model.textures.first().copied();
        let part_look = Look {
            texture0: body_tex0,
            k0_alpha: Some(0),
            fog,
            ..Default::default()
        };

        // FLUDD in model space: on the chest. The pack owns its body, clips and nozzles and
        // does the game's `setBaseTRMtx` chest tilt itself.
        let look = m.fludd_look();
        let fludd_base = look
            .and(self.chest)
            .and_then(|c| joints.get(c))
            .copied();
        // Only a deployed FLUDD leans the Spray neck (Luigi's alt-skin).
        let spraying = m.fludd.as_ref().is_some_and(|f| f.housed < 0.5);

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
                let cap = if return_landing && m.anim.frame < 157. {
                    self.return_cap.as_mut().unwrap_or(&mut self.cap)
                } else { &mut self.cap };
                let pose = Pose::new(&cap.model.bmd, head, &cap.bind);
                r.draw(
                    &mut cap.model,
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
                        let part = if return_landing && side == 0 && m.anim.frame < 158. {
                            self.return_hand.as_mut().unwrap_or(&mut self.hands[s][side])
                        } else { &mut self.hands[s][side] };
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
        }
        fludd_base.map_or_else(Vec::new, |base| {
            self.fludd.draw(
                look,
                spraying,
                false,
                0.0,
                base,
                root,
                view,
                proj,
                (shown("fludd"), shown("nozzle")),
            )
        })
    }
}

/// Model space (decomp units, +Z forward) to the scene: scaled by `scale`, turned by `yaw`
/// radians about +Y, placed at `pos`.
pub fn root_matrix(pos: Vec3, yaw: f32, scale: f32) -> Mat4 {
    Mat4::from_translation(pos) * Mat4::from_rotation_y(yaw) * Mat4::from_scale(Vec3::splat(scale))
}




















