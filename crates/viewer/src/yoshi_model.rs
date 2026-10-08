//! Yoshi, his tongue and his egg as the game draws them, from the user's extracted
//! `yoshi.szs` (`assets/yoshi/`), `mario.szs` (the tongue) and the stage archives (the egg).
//!
//! Yoshi (`TYoshi::calcAnim`/`entry`, Yoshi.cpp): the body clip the sim chose, with the walk
//! clip blended under the run, the eating clip over `jnt_18`'s subtree (`thinkUpper`), the head
//! (joint 21) pitched by the nozzle angle (`YoshiHeadCtrl`), the eye pattern (`thinkBtp`) and
//! TEV colour register 2 set to his body colour on every material. In the jump, fall and run
//! clips his built-in hands (shapes 0 and 1) give way to `yoshi_hand2_l/r` on joints 37 and 32,
//! which borrow his texture 0. The tongue (`TYoshiTongue::calcAnim`) lays its joints along an
//! arc from his mouth to the tip (`SMS_MakeJointsToArc`) with `yoshi_tongue_tip` on the end.
//! The egg (`TEggYoshi`) is `eggYoshi_normal` or `_broken` with its clip, and the thought bubble
//! on its root joint shows the wanted fruit through `eggyoshi_fukidashi.btp`.

use std::collections::HashMap;
use std::path::Path;

use glam::{Mat4, Vec3, Vec4};
use libsms::{Clip, EggLook, Srt, YoshiLook};
use libsms_assets::Pose;
use libsms_assets::btp::TexPattern;

use crate::j3d::{Lights, Look, Pass, Renderer};
use crate::mario_model::{Part, root_matrix};

/// Decomp units to scene units.
const S: f32 = 0.01;
/// `mMirrorModels[0/1]` follow these joints.
const HAND_JOINTS: [usize; 2] = [37, 32];
/// `thinkUpper` swaps this joint's (and its children's) calculation for the eating clip.
const EAT_ROOT: usize = 18;
/// `YoshiHeadCtrl`'s joint.
const HEAD: usize = 21;

struct EggParts {
    normal: Part,
    broken: Part,
    bubble: Part,
    clips: HashMap<String, Clip>,
    bubble_btp: Option<TexPattern>,
}

pub struct YoshiModel {
    renderer: Renderer,
    body: Part,
    /// `yoshi_hand2_l`, `yoshi_hand2_r`.
    hands: [Part; 2],
    tongue: Option<(Part, Part)>,
    clips: HashMap<String, Clip>,
    btps: HashMap<String, TexPattern>,
    egg: Option<EggParts>,
    mouth: Option<usize>,
}

fn load_clips(dir: &Path, prefix: &str) -> (HashMap<String, Clip>, HashMap<String, TexPattern>) {
    let (mut clips, mut btps) = (HashMap::new(), HashMap::new());
    for f in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = f.path();
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if !stem.starts_with(prefix) {
            continue;
        }
        let Ok(d) = std::fs::read(&path) else {
            continue;
        };
        match path.extension().and_then(|e| e.to_str()) {
            Some("bck") => {
                if let Some(c) = Clip::parse(&d) {
                    clips.insert(stem.to_owned(), c);
                }
            }
            Some("btp") => {
                if let Ok(t) = TexPattern::parse(&d) {
                    btps.insert(stem.to_owned(), t);
                }
            }
            _ => {}
        }
    }
    (clips, btps)
}

/// The material slots a texture pattern changes at `frame`.
fn pattern(model: &libsms_assets::Bmd, p: &TexPattern, frame: f32) -> Vec<(usize, u8, u16)> {
    p.tracks
        .iter()
        .enumerate()
        .filter_map(|(i, t)| Some((model.material(&t.material)?, t.tex_map, p.tex_no(i, frame)?)))
        .collect()
}

/// Blends two joint transforms, `t` of `b`: scale and translation lerp, each angle by its
/// shortest way round (`J3DMtxCalcBlend` mixes the transform infos).
fn blend(a: Srt, b: Srt, t: f32) -> Srt {
    let tau = std::f32::consts::TAU;
    let angle = |x: f32, y: f32| {
        let d = (y - x + std::f32::consts::PI).rem_euclid(tau) - std::f32::consts::PI;
        x + d * t
    };
    Srt {
        scale: a.scale.lerp(b.scale, t),
        rot: Vec3::new(
            angle(a.rot.x, b.rot.x),
            angle(a.rot.y, b.rot.y),
            angle(a.rot.z, b.rot.z),
        ),
        trans: a.trans.lerp(b.trans, t),
    }
}

impl YoshiModel {
    pub fn load(dir: &Path) -> Result<YoshiModel, String> {
        let mut r = Renderer::new();
        let body = Part::load(&mut r, dir, "yoshi/yoshi_model.bmd")?;
        let mut hands = [
            Part::load(&mut r, dir, "yoshi/yoshi_hand2_l.bmd")?,
            Part::load(&mut r, dir, "yoshi/yoshi_hand2_r.bmd")?,
        ];
        // `TYoshi::init`: the hands use the body's texture 0.
        if let Some(&t0) = body.model.textures.first() {
            for h in &mut hands {
                if let Some(t) = h.model.textures.first_mut() {
                    *t = t0;
                }
            }
        }
        let tongue = Part::load(&mut r, dir, "bmd/yoshi_tongue.bmd")
            .and_then(|t| Ok((t, Part::load(&mut r, dir, "bmd/yoshi_tongue_tip.bmd")?)))
            .ok();
        let (clips, btps) = load_clips(&dir.join("yoshi"), "yoshi_");
        let mapobj = dir.join("scene/mapobj");
        let egg = (|| -> Result<EggParts, String> {
            let (clips, btps) = load_clips(&mapobj, "eggyoshi_");
            Ok(EggParts {
                normal: Part::load(&mut r, dir, "scene/mapobj/eggyoshi_normal.bmd")?,
                broken: Part::load(&mut r, dir, "scene/mapobj/eggyoshi_broken.bmd")?,
                bubble: Part::load(&mut r, dir, "scene/mapobj/eggyoshi_fukidashi.bmd")?,
                clips,
                bubble_btp: btps.get("eggyoshi_fukidashi").cloned(),
            })
        })()
        .map_err(|e| eprintln!("egg: {e}"))
        .ok();
        let mouth = body.model.bmd.joint("null_tongue");
        Ok(YoshiModel {
            renderer: r,
            body,
            hands,
            tongue,
            clips,
            btps,
            egg,
            mouth,
        })
    }

    /// Yoshi's model-space joints for `look`. His model chains Softimage-style
    /// ([`libsms::MtxCalc`]): the riding clips stretch `jnt_neck_2` and shrink `chn_head` back.
    fn pose(&self, look: &YoshiLook) -> Vec<Mat4> {
        let b = &self.body.model.bmd;
        let clip = self.clips.get(look.clip);
        let walk = look.blend.and_then(|(c, w)| Some((self.clips.get(c)?, w)));
        let eat = look.eat.and_then(|(c, f)| Some((self.clips.get(c)?, f)));
        let mut eating = vec![false; b.joints.len()];
        for (j, jt) in b.joints.iter().enumerate() {
            eating[j] = j == EAT_ROOT || jt.parent.is_some_and(|p| p < j && eating[p]);
        }
        b.pose(
            |j| {
                let jt = &b.joints[j];
                let bind = Srt {
                    scale: jt.scale,
                    rot: jt.rotation,
                    trans: jt.translation,
                };
                let sample = |c: &Clip, f: f32| c.sample(j, f).unwrap_or(bind);
                Some(match eat {
                    Some((c, f)) if eating[j] => sample(c, f),
                    _ => {
                        let run = clip.map_or(bind, |c| sample(c, look.frame));
                        match walk {
                            Some((w, t)) => blend(run, sample(w, look.frame), t),
                            None => run,
                        }
                    }
                })
            },
            // `YoshiHeadCtrl` on joint 21.
            |j| {
                (j == HEAD && look.head_angle != 0.0).then(|| {
                    Mat4::from_rotation_z(look.head_angle * std::f32::consts::PI / 32768.0)
                })
            },
        )
    }

    /// Draws Yoshi. `root` is Mario's while he rides (the scene from model space); returns the
    /// posed `null_tongue` joint in Yoshi's model space (position, X axis) for
    /// `Mario::set_yoshi_mouth`.
    pub fn draw(
        &mut self,
        look: &YoshiLook,
        root: Option<Mat4>,
        view: Mat4,
        proj: Mat4,
    ) -> Option<(Vec3, Vec3)> {
        let root = root.unwrap_or_else(|| {
            root_matrix(
                look.pos * S,
                look.yaw as f32 * std::f32::consts::PI / 32768.0,
                S,
            )
        });
        let joints = self.pose(look);
        let lights = Lights::daylight(view);
        let c = look.body_color;
        let tev = Some((2, [c[0], c[1], c[2], 255.0]));
        let mut body_look = Look {
            tev_color: tev,
            ..Default::default()
        };
        if let Some(p) = self.btps.get(look.btp) {
            body_look.tex_no = pattern(&self.body.model.bmd, p, look.btp_frame);
        }
        if look.hands {
            body_look.hidden = vec![0, 1];
        }
        let hand_look = Look {
            tev_color: tev,
            ..Default::default()
        };
        let body_pose = Pose::new(&self.body.model.bmd, root, &joints);
        let tongue = look.tongue.and_then(|(head, dir, tip)| {
            let (t, _) = self.tongue.as_ref()?;
            Some(tongue_arc(t, head, dir, tip + Vec3::Y * 50.0))
        });
        for pass in [Pass::Opaque, Pass::Translucent] {
            let r = &mut self.renderer;
            r.draw(
                &mut self.body.model,
                &body_pose,
                view,
                proj,
                &lights,
                &body_look,
                pass,
            );
            if look.hands {
                for (part, &j) in self.hands.iter_mut().zip(&HAND_JOINTS) {
                    if let Some(&at) = body_pose.joints.get(j) {
                        let pose = Pose::new(&part.model.bmd, at, &part.bind);
                        r.draw(
                            &mut part.model,
                            &pose,
                            view,
                            proj,
                            &lights,
                            &hand_look,
                            pass,
                        );
                    }
                }
            }
            if let (Some((t, tip)), Some((joints, tip_at))) = (self.tongue.as_mut(), &tongue) {
                let scene = Mat4::from_scale(Vec3::splat(S));
                let pose = Pose::new(&t.model.bmd, scene, joints);
                r.draw(
                    &mut t.model,
                    &pose,
                    view,
                    proj,
                    &lights,
                    &Look::default(),
                    pass,
                );
                let pose = Pose::new(&tip.model.bmd, scene * *tip_at, &tip.bind);
                r.draw(
                    &mut tip.model,
                    &pose,
                    view,
                    proj,
                    &lights,
                    &Look::default(),
                    pass,
                );
            }
        }
        self.mouth
            .and_then(|m| joints.get(m))
            .map(|m| (m.w_axis.truncate(), m.x_axis.truncate()))
    }

    /// Draws an egg and its thought bubble; `time` runs the bubble's idle clip.
    pub fn draw_egg(&mut self, look: &EggLook, view: Mat4, proj: Mat4, time: f32) {
        let Some(e) = self.egg.as_mut() else {
            return;
        };
        let root = root_matrix(
            look.pos * S,
            look.yaw as f32 * std::f32::consts::PI / 32768.0,
            S,
        );
        let lights = Lights::daylight(view);
        let part = if look.broken {
            &mut e.broken
        } else {
            &mut e.normal
        };
        let joints = part.posed(e.clips.get(look.clip), look.frame, |_| None);
        let pose = Pose::new(&part.model.bmd, root, &joints);
        let bubble = look.bubble.map(|frame| {
            // `TEggYoshi::perform`: the bubble rides the egg's root joint.
            let at = pose.joints.first().copied().unwrap_or(root);
            let wait = e.clips.get("eggyoshi_fukidashi_wait");
            let len = wait.map_or(1.0, |c| c.frames.max(1.0));
            let js = e.bubble.posed(wait, (time * 30.0) % len, |_| None);
            let mut l = Look::default();
            if let Some(p) = &e.bubble_btp {
                l.tex_no = pattern(&e.bubble.model.bmd, p, frame);
            }
            (Pose::new(&e.bubble.model.bmd, at, &js), l)
        });
        for pass in [Pass::Opaque, Pass::Translucent] {
            let r = &mut self.renderer;
            r.draw(
                &mut part.model,
                &pose,
                view,
                proj,
                &lights,
                &Look::default(),
                pass,
            );
            if let Some((pose, l)) = &bubble {
                r.draw(&mut e.bubble.model, pose, view, proj, &lights, l, pass);
            }
        }
    }
}

/// `SMS_MakeJointsToArc(model, head, dir, end)` and the tip's matrix
/// (`TYoshiTongue::calcAnim`), in world decomp units.
fn tongue_arc(t: &Part, start: Vec3, head_dir: Vec3, end: Vec3) -> (Vec<Mat4>, Mat4) {
    let n = t.model.bmd.joints.len().max(2);
    let to = end - start;
    let mag = to.length().max(1.0e-3);
    let dir = to / mag;
    let up = head_dir.normalize_or(Vec3::Z);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let f = i as f32 / (n - 1) as f32;
        let c = (dir * f + up * (1.0 - f)).normalize_or(dir);
        let z = t
            .bind
            .get(i)
            .map_or(Vec3::Z, |m| m.z_axis.truncate().normalize_or(Vec3::Z));
        let side = z.cross(c).normalize_or(Vec3::Y);
        let fwd = c.cross(side).normalize_or(Vec3::Z);
        let p = start + c * (i as f32 * mag / (n - 1) as f32);
        out.push(Mat4::from_cols(
            c.extend(0.0),
            side.extend(0.0),
            fwd.extend(0.0),
            p.extend(1.0),
        ));
    }
    let (a, b) = (out[n - 2].w_axis.truncate(), out[n - 1].w_axis.truncate());
    let d = Vec3::new(b.x - a.x, 0.0, b.z - a.z).normalize_or(Vec3::Z);
    let x = Vec3::Y.cross(d);
    let tip = Mat4::from_cols(x.extend(0.0), Vec4::Y, d.extend(0.0), end.extend(1.0));
    (out, tip)
}
