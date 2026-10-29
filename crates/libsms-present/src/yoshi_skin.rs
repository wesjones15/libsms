//! Sunshine Yoshi as a Mario skin: `yoshi.szs:yoshi_model.bmd` (38 joints, `TYoshi::init` in
//! `Player/Yoshi.cpp`) wearing Mario's moves.
//!
//! Ported from the viewer's proven `viewer/src/yoshi_skin.rs` (2026-10-26 commits 88e57bf,
//! 5606eb4, 69e3dfc). What the viewer established, and this keeps:
//! - Yoshi's own wait/walk/run and `yoshi_hold_jump` clips play on his own rig; Mario's jumps,
//!   dives, flips and pounds retarget onto it through `libsms_assets::retarget::Retarget`
//!   (aliases `chn_neck`, `jnt_neck_1/2`, `chn_head`).
//! - The lowest skinned point is anchored to Mario's, so feet land where Mario's do.
//! - FLUDD rides `jnt_neck_1` in Mario's authored centred chest frame (`WaterGun.cpp::init`);
//!   no Luigi-style side lean.
//!
//! The mouth-juice routing and 1.5x belt fit are ported from the viewer here: within the
//! front clearance envelope a Spray leaves Yoshi's mouth (`null_tongue`,
//! `NozzleYoshiMouth.prm`) while FLUDD rests, and FLUDD's belt shape is widened for his hips.
use std::collections::HashMap;
use std::path::Path;

use glam::{Mat4, Quat, Vec3};
use libsms::{Clip, NozzleKind, Params, Skeleton};
use libsms_assets::retarget::Retarget;
use libsms_assets::Pose;

use crate::view::{Model, Textures, build, load_bmd, RIG_KEY_BASE};

/// Mario joints mapped onto Yoshi's (the viewer's table). The neck's first joint is Yoshi's
/// upper torso, which carries FLUDD.
const ALIASES: &[(&str, &str)] = &[
    ("chn_neck", "chn_chest"),
    ("jnt_neck_1", "jnt_chest"),
    ("jnt_neck_2", "jnt_chest"),
    ("chn_head", "jnt_head"),
];

/// Yoshi's clips used on the ground, by the Mario clip they stand in for.
fn ground_clip(clip: &str) -> Option<&'static str> {
    match clip {
        "wait" => Some("yoshi_wait"),
        "walk" => Some("yoshi_walk"),
        "run1" | "run2" => Some("yoshi_run"),
        "turn" | "pivot" => Some("yoshi_pivot"),
        _ => None,
    }
}

/// Viewer adaptation, from `viewer/src/yoshi_skin.rs`: the front clearance envelope uses the
/// mouth; outside it FLUDD handles the aim. Physical head pitch bounds come from
/// `NozzleYoshiMouth.prm`.
#[derive(Clone, Copy, Debug)]
pub struct SprayRoute {
    pub mouth: bool,
    pub yaw: f32,
    pub pitch: f32,
}

impl SprayRoute {
    pub fn new(yaw: f32, pitch: f32, nozzle: NozzleKind, params: &Params) -> Self {
        let n = params.nozzle(NozzleKind::Yoshi);
        let unit = std::f32::consts::PI / 32768.0;
        let side = (n.side_angle_max_side as f32 * unit).min(60f32.to_radians());
        let down = -n.l_angle_max as f32 * unit;
        let up = -n.l_angle_min as f32 * unit;
        let yaw = (yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        Self {
            mouth: nozzle == NozzleKind::Spray && yaw.abs() <= side && pitch >= down && pitch <= up,
            yaw: yaw.clamp(-side, side),
            pitch: pitch.clamp(-n.l_angle_max as f32 * unit, -n.l_angle_min as f32 * unit),
        }
    }

    pub fn for_mario(m: &libsms::Mario) -> Self {
        let unit = std::f32::consts::PI / 32768.0;
        let (yaw, pitch) = m
            .aim
            .map_or((0., 0.), |a| (a.yaw.wrapping_sub(m.model_yaw) as f32 * unit, a.pitch as f32 * unit));
        Self::new(yaw, pitch, m.fludd.as_ref().map_or(NozzleKind::Spray, |f| f.nozzle), &m.params)
    }

    pub fn direction(self) -> Vec3 {
        Vec3::new(self.yaw.sin() * self.pitch.cos(), self.pitch.sin(), self.yaw.cos() * self.pitch.cos())
    }
}

/// A Yoshi body plus its clips, posed either on his own rig or through the retarget.
pub struct Yoshi {
    /// Index into `View::models` of the body the view draws.
    pub model: u32,
    rig: Skeleton,
    retarget: Retarget,
    /// `jnt_neck_1` in Yoshi's rig, carrying FLUDD in Mario's chest frame.
    chest: usize,
    chest_align: Mat4,
    /// `chn_head`, `jnt_mouth` and `null_tongue`: the spray-aim pivot, jaw and emitter.
    head: usize,
    jaw: usize,
    mouth: usize,
    eat: Clip,
    /// Loaded Yoshi clips by file stem (`yoshi_wait`, `yoshi_run`, ...).
    clips: HashMap<&'static str, Clip>,
    dir: std::path::PathBuf,
}

impl Yoshi {
    /// Loads Yoshi's model and gives it a model-table entry; `source` is Mario's skeleton (the
    /// retarget's source and the frame FLUDD is authored in).
    pub fn load(
        dir: &Path,
        source: &Skeleton,
        textures: &mut Textures,
        index: u32,
    ) -> Result<(Yoshi, Model), String> {
        let raw = std::fs::read(dir.join("yoshi/yoshi_model.bmd")).map_err(|e| e.to_string())?;
        let rig = Skeleton::from_bmd(&raw).ok_or("invalid Yoshi JNT1/INF1")?;
        if rig.len() != 38 {
            return Err(format!("expected Yoshi's 38 joints, got {}", rig.len()));
        }
        for &(target, name) in ALIASES {
            if rig.find(target).is_none() || source.find(name).is_none() {
                return Err(format!("missing retarget joint: {target} <- {name}"));
            }
        }
        let retarget = Retarget::new(source, &rig, ALIASES);
        let chest = rig.find("jnt_neck_1").ok_or("Yoshi torso joint missing")?;
        let head = rig.find("chn_head").ok_or("Yoshi head pivot missing")?;
        let jaw = rig.find("jnt_mouth").ok_or("Yoshi jaw missing")?;
        let mouth = rig.find("null_tongue").ok_or("Yoshi mouth emitter missing")?;
        let eat = std::fs::read(dir.join("yoshi/yoshi_eat.bck"))
            .ok()
            .and_then(|d| Clip::parse(&d))
            .ok_or("Yoshi eat/jaw animation missing")?;
        let mario_chest = source.find("jnt_chest").ok_or("Mario chest missing")?;
        let rotation = |m: Mat4| m.to_scale_rotation_translation().1;
        let chest_align = Mat4::from_quat(
            rotation(rig.bind_world()[chest]).inverse() * rotation(source.bind_world()[mario_chest]),
        );
        let bmd = load_bmd(dir, "yoshi/yoshi_model.bmd")?;
        let model = build(bmd, &|_, _| None, RIG_KEY_BASE + 3, textures)?;
        Ok((
            Yoshi {
                model: index,
                rig,
                retarget,
                chest,
                chest_align,
                head,
                jaw,
                mouth,
                eat,
                clips: HashMap::new(),
                dir: dir.join("yoshi"),
            },
            model,
        ))
    }

    fn clip(&mut self, name: &'static str) -> Option<&Clip> {
        if !self.clips.contains_key(name) {
            let data = std::fs::read(self.dir.join(format!("{name}.bck"))).ok()?;
            let clip = Clip::parse(&data)?;
            self.clips.insert(name, clip);
        }
        self.clips.get(name)
    }

    /// Yoshi joint matrices from one of his own clips, in decomp units.
    pub fn pose_native(&mut self, name: &'static str, frame: f32) -> Option<Vec<Mat4>> {
        let clip = self.clip(name)?.clone();
        Some(self.rig.pose(&clip, frame))
    }

    /// Yoshi joint matrices from Mario's pose through the retarget.
    pub fn pose_retarget(&mut self, source: &[Mat4]) -> Vec<Mat4> {
        self.retarget.pose(&self.rig, source)
    }

    /// The clip's length for phase mapping, 1 when it is not loaded.
    pub fn frames(&mut self, name: &'static str) -> f32 {
        self.clip(name).map_or(1.0, |c| c.frames.max(1.0))
    }

    /// `jnt_neck_1` turned into Mario's chest frame, where FLUDD straps on.
    pub fn chest(&self, world: &[Mat4]) -> Mat4 {
        world[self.chest] * self.chest_align
    }

    /// The mouth opening and head aim on a posed body (`TYoshi::thinkUpper`): the authored jaw
    /// held at the end of `yoshi_eat`, then the head turned to the spray direction.
    pub fn spray_world(&self, mut world: Vec<Mat4>, route: SprayRoute, active: bool) -> Vec<Mat4> {
        if route.mouth && active {
            if let Some(local) = self.eat.sample(self.jaw, self.eat.frames) {
                world[self.jaw] = world[self.rig.parents[self.jaw].unwrap()] * local.matrix();
            }
            let forward = world[self.mouth].x_axis.truncate().normalize();
            let turn = Quat::from_rotation_arc(forward, route.direction());
            let pivot = world[self.head].w_axis.truncate();
            let delta =
                Mat4::from_translation(pivot) * Mat4::from_quat(turn) * Mat4::from_translation(-pivot);
            for j in 0..self.rig.len() {
                if self.rig.under(j, self.head) {
                    world[j] = delta * world[j];
                }
            }
        }
        world
    }

    /// Where juice leaves the mouth, in Mario's model space (Yoshi is anchored there).
    pub fn mouth(&self, world: &[Mat4]) -> Mat4 {
        world[self.mouth]
    }
}

/// The lowest skinned point of `world`, in model units.
pub fn lowest(model: &Model, world: &[Mat4]) -> f32 {
    let pose = Pose::new(&model.bmd, Mat4::IDENTITY, world);
    let mut low = f32::INFINITY;
    for b in &model.batches {
        for v in &b.vertices {
            low = low.min(pose.skin(v).0.y);
        }
    }
    low
}

/// Picks the clip for this frame. Mario's own clip stays for jumps, dives, flips and pounds;
/// Yoshi's wait/walk/run replace Mario's on the ground, and the flutter plays `yoshi_hold_jump`.
/// Returns the clip, its frame, and whether it is one of Yoshi's own (else it retargets).
pub fn clip_for(
    m: &libsms::Mario,
    back: f32,
    yoshi: &mut Yoshi,
    clips: &libsms::ClipLibrary,
) -> Option<(&'static str, f32, bool)> {
    if m.fluttering() {
        let frames = yoshi.frames("yoshi_hold_jump");
        return Some(("yoshi_hold_jump", (m.ticks as f32 * 0.125).rem_euclid(frames), true));
    }
    let mario = clips.get(m.anim.clip)?;
    let frame = m.anim.frame_before(back);
    if let Some(name) = ground_clip(m.anim.clip) {
        let frames = yoshi.frames(name);
        // Keep the phase of Mario's clip so footfalls stay in step.
        let phase = (frame / mario.frames.max(1.0)).rem_euclid(1.0);
        return Some((name, phase * frames, true));
    }
    Some((m.anim.clip, frame, false))
}
