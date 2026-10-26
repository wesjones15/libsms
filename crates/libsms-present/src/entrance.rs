//! Approved viewer entrance, shared with the RoR2 adapter. MarioAutodemo::warpOut,
//! MarioParticle::warpOutEffect (particle emitter 0x40), and JPAField.cpp.
use super::*;
use crate::portal_cache::{self, Frames};
use crate::{plaza, portal::Portal};
use libsms::{Assets, Status};
use libsms_assets::jpa::Jpa;
use std::sync::Arc;

pub const ENTRANCE_RETURN: Vec3 = Vec3::new(0., 250., 308.53);
const CLIPS: [&str; 3] = [
    "demo_gate_out_appear_get",
    "demo_gate_out_rolling_get",
    "demo_gate_out_get2",
];
const WINDS: [&str; 4] = [
    "ms_gatewind_a",
    "ms_gatewind_a2",
    "ms_gatewind_a3",
    "ms_gatewind_b",
];

/// MarioPhysics::jumpProcess: integrate the zero-speed fall before applying gravity.
pub fn entrance_fall(seconds: f32, gravity: f32) -> (f32, f32) {
    let (mut y, mut velocity, mut ticks) = (ENTRANCE_RETURN.y, 0f32, 0u32);
    while y > 0. {
        y = (y + velocity * 0.25).max(0.);
        ticks += 1;
        if ticks as f32 > seconds * 120. {
            return (y, ticks as f32 / 120.);
        }
        velocity = (velocity - gravity.max(0.01)).max(-75.);
    }
    (0., ticks as f32 / 120.)
}

pub fn entrance_pose(time: f32, gravity: f32) -> (&'static str, f32, Vec3, bool) {
    if time < 1.5 {
        return (
            CLIPS[0],
            (time.max(0.) * 60.).min(89.),
            ENTRANCE_RETURN,
            false,
        );
    }
    let seconds = time - 1.5;
    let (y, duration) = entrance_fall(seconds, gravity);
    if y > 0. {
        return (
            CLIPS[1],
            seconds * 60.,
            Vec3::new(0., y, ENTRANCE_RETURN.z),
            false,
        );
    }
    let frame = ((seconds - duration) * 60.).max(0.);
    (
        CLIPS[2],
        frame.min(181.),
        Vec3::new(0., 0., ENTRANCE_RETURN.z),
        frame >= 181.,
    )
}

/// RoR2's short hybrid: compressed emitter/reveal, normal-rate original spin with a gentle fall,
/// ordinary landing (MarioRun::jumpSlip selects jmped), without the held-cap demo.
pub fn entrance_fast_pose(time: f32) -> (&'static str, f32, Vec3, bool) {
    if time < portal_cache::REVEAL {
        return (
            CLIPS[0],
            (time.max(0.) / portal_cache::REVEAL * 89.).min(89.),
            ENTRANCE_RETURN,
            false,
        );
    }
    let air = time - portal_cache::REVEAL;
    if air < portal_cache::HOP {
        let u = (air / portal_cache::HOP).clamp(0., 1.);
        return (
            CLIPS[1],
            air * 60.,
            Vec3::new(0., ENTRANCE_RETURN.y * (1. - u * u), ENTRANCE_RETURN.z),
            false,
        );
    }
    (
        "jmped",
        ((air - portal_cache::HOP) * 60.).clamp(0., 12.),
        Vec3::new(0., 0., ENTRANCE_RETURN.z),
        time >= portal_cache::COMPLETE,
    )
}

pub struct Entrance {
    sim: Sim,
    portal: Portal,
    statue: u32,
    gate: u32,
    texture: i32,
    texture_frame: u32,
    tick: u64,
    emitted: bool,
    fast: Option<Fast>,
}
impl Entrance {
    /// Pedestal geometry, excluding sculpture shape 23; host collision follows the asset.
    pub fn pedestal_bounds(&self, view: &View) -> (Vec3, Vec3) {
        let model = &view.models[self.statue as usize];
        let pose = Pose::new(
            &model.bmd,
            Mat4::from_translation(Vec3::new(0., -300., -4941.47)),
            &model.bind,
        );
        let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for v in model
            .batches
            .iter()
            .filter(|b| b.shape != 23)
            .flat_map(|b| &b.vertices)
        {
            let p = pose.skin(v).0;
            lo = lo.min(p);
            hi = hi.max(p);
        }
        (lo, hi)
    }
    pub fn load(dir: &Path, view: &mut View) -> Result<Self, String> {
        Self::load_mode(dir, view, false)
    }

    pub fn load_fast(dir: &Path, view: &mut View) -> Result<Self, String> {
        Self::load_mode(dir, view, true)
    }

    fn load_mode(dir: &Path, view: &mut View, fast: bool) -> Result<Self, String> {
        let frames = if fast { Some(Frames::load(dir)?) } else { None };
        let assets = Assets::load(dir);
        for clip in CLIPS {
            if assets.clips.get(clip).is_none() {
                return Err(format!("missing {clip}"));
            }
        }
        let mut sim = Sim::new(assets.params, assets.anim_table, Vec3::ZERO, 0);
        sim.mario.give_fludd();
        let mut portal = Portal::load(dir).map_err(|e| e.to_string())?;
        if !fast {
            portal.set_interior(&plaza::plaza_still(dir, 256).map_err(|e| e.to_string())?);
        }
        let map = load_bmd(dir, "portal/plaza.bmd")?;
        let batches = plaza::statue_batches(&map).map_err(|e| e.to_string())?;
        let statue = view.models.len() as u32;
        view.models.push(build_batches(
            map,
            batches,
            &|_, _| None,
            statue,
            &mut view.textures,
        )?);
        let gate = view.models.len() as u32;
        let mut model = build(portal.model.clone(), &|_, _| None, gate, &mut view.textures)?;
        let texture = model.batch_texture[0];
        if texture < 0 {
            return Err("portal has no texture".into());
        }
        model.batch_blend.fill(if fast { blend::TRANSLUCENT } else { blend::CUTOUT });
        model.batch_lit.fill(0);
        view.models.push(model);
        view.textures.list[texture as usize] = frames
            .as_ref()
            .map_or_else(|| portal.image(0., 0.), |f| f.image());
        view.textures.versions.insert(texture, 1);
        // MarioCap::perform / MarioDraw: cap held in the landing, ma_hnd4r before frame 158.
        let body0 = view.models[0].textures[0].clone();
        let mut return_parts = [0; 2];
        for (slot, file) in ["bmd/ma_cap3.bmd", "bmd/ma_hnd4r.bmd"]
            .into_iter()
            .enumerate()
        {
            let id = view.models.len() as u32;
            let mut bmd = load_bmd(dir, file)?;
            for m in &mut bmd.materials {
                m.konst[0][3] = 0;
            }
            view.models.push(build(
                bmd,
                &|i, _| if i == 0 { body0.clone() } else { None },
                id,
                &mut view.textures,
            )?);
            return_parts[slot] = id;
        }
        view.return_parts = Some(return_parts);
        let mut fx = Particles::load(dir, &["ms_m_gout_gate"]);
        if !fx.has("ms_m_gout_gate") {
            return Err("missing particle emitter 0x40".into());
        }
        fx.set_speed("ms_m_gout_gate", 2.);
        for name in WINDS {
            let jpa = Jpa::parse(
                &std::fs::read(dir.join(format!("portal/{name}.jpa")))
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            fx.add(name, jpa);
        }
        view.fx = fx;
        let mut made = Self {
            sim,
            portal,
            statue,
            gate,
            texture,
            texture_frame: 0,
            tick: 0,
            emitted: false,
            fast: None,
        };
        if let Some(frames) = frames {
            made.fast = Some(Fast::new(view, frames, statue, gate, dir)?);
            made.update_fast(view, 0., 0, Vec3::X, Vec3::Y);
            made.fast.as_mut().unwrap().uploaded = false;
        } else {
            made.update(view, 0., false, Vec3::X, Vec3::Y);
        }
        Ok(made)
    }

    pub fn is_fast(&self) -> bool {
        self.fast.is_some()
    }
    pub fn position(&self, time: f32) -> Vec3 {
        if self.is_fast() {
            entrance_fast_pose(time).2
        } else {
            entrance_pose(time, self.sim.mario.params.jump.gravity).2
        }
    }

    /// State 0 idle, 1 emerging, 2 handed off. Static meshes upload once; spent Mario is never posed.
    pub fn update_fast(
        &mut self,
        view: &mut View,
        time: f32,
        state: u8,
        right: Vec3,
        up: Vec3,
    ) -> bool {
        let Some(fast) = &mut self.fast else {
            return false;
        };
        let (clip, frame, pos, done) = entrance_fast_pose(time);
        let mut fx = std::mem::take(&mut view.fx);
        if view.skin == Skin::Luigi && !fast.luigi_particles {
            if let Some(tinted) = fast.luigi_fx.take() {
                fx = tinted;
            }
            fast.luigi_particles = true;
        }
        if state == 1 {
            self.sim.mario.pos = pos;
            self.sim.mario.prev_pos = pos;
            self.sim
                .mario
                .portrait_pose(Status::Jump, clip, frame, 0, false);
            view.set_camera(right, up);
            view.update(&mut self.sim, 0.);
            // The HSD retarget keeps rotations; apply the BCK reveal's centre scale
            // explicitly so Luigi and his FLUDD do not appear before the particles converge.
            if view.skin == Skin::Luigi && clip == CLIPS[0] {
                let scale = view
                    .clips
                    .get(CLIPS[0])
                    .and_then(|c| c.sample(1, frame))
                    .map_or(Vec3::ONE, |s| s.scale);
                for p in &mut view.positions {
                    *p *= scale;
                }
            }
        } else {
            clear_frame(view);
        }
        let image_index = fast.frames.index(time, state != 0);
        if fast.image != Some(image_index) {
            view.textures.list[self.texture as usize]
                .rgba
                .copy_from_slice(fast.frames.pixels(image_index));
            let version = view.textures.versions.entry(self.texture).or_insert(1);
            *version = version.wrapping_add(1).max(1);
            fast.image = Some(image_index);
        }
        if state == 1 {
            if !self.emitted {
                fx.emit(
                    "ms_m_gout_gate",
                    Mat4::from_translation(ENTRANCE_RETURN),
                    1.,
                );
                self.emitted = true;
            }
            let source_time = time * (1.5 / portal_cache::REVEAL);
            let end = (source_time * 120.) as u64;
            while self.tick <= end {
                if self.tick < 185 {
                    for (slot, name) in WINDS.iter().enumerate() {
                        fx.request(
                            slot as u32,
                            name,
                            Mat4::from_translation(Vec3::new(0., 200., 0.)),
                        );
                    }
                }
                fx.advance(self.tick);
                self.tick += 1;
            }
            let reflect = |v: Vec3| Vec3::new(-v.x, v.y, v.z);
            fx.build(reflect(right), reflect(up));
            for v in &mut fx.vertices {
                v.pos.x = -v.pos.x;
            }
        } else {
            fx.vertices.clear();
            fx.batches.clear();
        }
        view.fx = fx;
        for rigid in &fast.rigid {
            rigid.append(view, !fast.uploaded);
        }
        fast.uploaded = true;
        state != 0 && done
    }

    /// Time is seconds since X, or the idle swirl's clock while unactivated.
    /// Returns true once the landing/cap animation has finished.
    pub fn update(
        &mut self,
        view: &mut View,
        time: f32,
        activated: bool,
        right: Vec3,
        up: Vec3,
    ) -> bool {
        let time = time.max(0.);
        let mut fx = std::mem::take(&mut view.fx);
        let (clip, frame, pos, done) = entrance_pose(time, self.sim.mario.params.jump.gravity);
        self.sim.mario.pos = pos;
        self.sim.mario.prev_pos = pos;
        self.sim
            .mario
            .portrait_pose(Status::Jump, clip, frame, 0, false);
        view.set_camera(right, up);
        view.update(&mut self.sim, 0.);
        if !activated {
            view.draws.clear();
            view.positions.clear();
            view.normals.clear();
            view.uvs.clear();
            view.colors.clear();
            view.texture_ids.clear();
        }
        let image_frame = (time * 30.) as u32 + 1;
        if image_frame != self.texture_frame || activated != self.emitted {
            view.textures.list[self.texture as usize] = self.portal.image(
                time * 60.,
                if activated {
                    (time / 0.6).clamp(0., 1.)
                } else {
                    0.
                },
            );
            let version = view.textures.versions.entry(self.texture).or_insert(1);
            *version = version.wrapping_add(1).max(1);
            self.texture_frame = image_frame;
        }
        if activated {
            if !self.emitted {
                fx.emit(
                    "ms_m_gout_gate",
                    Mat4::from_translation(ENTRANCE_RETURN),
                    1.,
                );
                self.emitted = true;
            }
            let end = (time * 120.) as u64;
            while self.tick <= end {
                if (self.tick as f32) / 120.
                    < 1.5 + entrance_fall(10., self.sim.mario.params.jump.gravity).1
                {
                    for (slot, name) in WINDS.iter().enumerate() {
                        fx.request(
                            slot as u32,
                            name,
                            Mat4::from_translation(Vec3::new(0., 200., 0.)),
                        );
                    }
                }
                fx.advance(self.tick);
                self.tick += 1;
            }
        }
        let reflect = |v: Vec3| Vec3::new(-v.x, v.y, v.z);
        fx.build(reflect(right), reflect(up));
        view.fx = fx;
        for v in &mut view.fx.vertices {
            v.pos.x = -v.pos.x;
        }
        append_rigid(
            view,
            self.statue,
            Mat4::from_translation(Vec3::new(0., -300., -4941.47)),
            false,
        );
        append_rigid(
            view,
            self.gate,
            Mat4::from_translation(Vec3::new(0., 0., 3.5)),
            true,
        );
        activated && done
    }
}

fn append_rigid(view: &mut View, model: u32, root: Mat4, flip_uv: bool) {
    let mdl = &view.models[model as usize];
    let pose = Pose::new(&mdl.bmd, root, &mdl.bind);
    let first_vertex = view.positions.len() as u32;
    let first_color = view.colors.len() as u32;
    for (bi, b) in mdl.batches.iter().enumerate() {
        for v in &b.vertices {
            let (p, n) = pose.skin(v);
            view.positions.push(p);
            view.normals.push(n.normalize_or_zero());
            // Keep the established interior/row mapping. Portal::load compensates
            // the M's fixed mask independently, without flipping the Plaza planes.
            view.uvs.push(if flip_uv {
                [1. - v.uv[0].x, 1. - v.uv[0].y]
            } else {
                [v.uv[0].x, v.uv[0].y]
            });
        }
        view.colors.push(mdl.batch_color[bi]);
        view.texture_ids.push(mdl.batch_texture[bi]);
    }
    view.draws.push(Draw {
        key: (model as u64) << 56,
        model,
        kind: if flip_uv { 241 } else { 240 },
        flags: 0,
        pos: Vec3::ZERO,
        rot: Quat::IDENTITY,
        first_vertex,
        first_color,
        alpha: 1.,
    });
}

fn clear_frame(view: &mut View) {
    view.draws.clear();
    view.positions.clear();
    view.normals.clear();
    view.uvs.clear();
    view.colors.clear();
    view.texture_ids.clear();
}

struct Rigid {
    draw: Draw,
    positions: Vec<Vec3>,
    normals: Vec<Vec3>,
    uvs: Vec<[f32; 2]>,
    colors: Vec<C4>,
    textures: Vec<i32>,
}
impl Rigid {
    fn new(view: &mut View, model: u32, root: Mat4, flip_uv: bool) -> Self {
        clear_frame(view);
        append_rigid(view, model, root, flip_uv);
        Self {
            draw: view.draws[0],
            positions: std::mem::take(&mut view.positions),
            normals: std::mem::take(&mut view.normals),
            uvs: std::mem::take(&mut view.uvs),
            colors: std::mem::take(&mut view.colors),
            textures: std::mem::take(&mut view.texture_ids),
        }
    }
    fn append(&self, view: &mut View, upload: bool) {
        let mut draw = self.draw;
        draw.first_vertex = view.positions.len() as u32;
        draw.first_color = view.colors.len() as u32;
        if upload {
            view.positions.extend_from_slice(&self.positions);
            view.normals.extend_from_slice(&self.normals);
            view.uvs.extend_from_slice(&self.uvs);
        } else {
            draw.flags |= 4;
        }
        view.colors.extend_from_slice(&self.colors);
        view.texture_ids.extend_from_slice(&self.textures);
        view.draws.push(draw);
    }
}

struct Fast {
    frames: Arc<Frames>,
    image: Option<usize>,
    rigid: [Rigid; 2],
    uploaded: bool,
    luigi_fx: Option<Particles>,
    luigi_particles: bool,
}
impl Fast {
    fn new(
        view: &mut View,
        frames: Arc<Frames>,
        statue: u32,
        gate: u32,
        dir: &Path,
    ) -> Result<Self, String> {
        let rigid = [
            Rigid::new(
                view,
                statue,
                Mat4::from_translation(Vec3::new(0., -300., -4941.47)),
                false,
            ),
            Rigid::new(
                view,
                gate,
                Mat4::from_translation(Vec3::new(0., 0., 3.5)),
                true,
            ),
        ];
        let mut jpa = Jpa::parse(
            &std::fs::read(dir.join("particle/ms_m_gout_gate.jpa")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        // Luigi alt-skin: swap only the strongly red palette entries (the same rule the
        // gameplay effects use). Peach/brown skin, blue overalls, white gloves and yellow
        // accessories keep their original colours.
        crate::particles::red_to_green(&mut jpa);
        let mut fx = Particles::default();
        fx.add("ms_m_gout_gate", jpa);
        fx.set_speed("ms_m_gout_gate", 2.);
        for name in WINDS {
            let jpa = Jpa::parse(
                &std::fs::read(dir.join(format!("portal/{name}.jpa")))
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            fx.add(name, jpa);
        }
        Ok(Self {
            frames,
            image: None,
            rigid,
            uploaded: false,
            luigi_fx: Some(fx),
            luigi_particles: false,
        })
    }
}

#[cfg(test)]
mod fast_tests {
    use super::*;
    #[test]
    fn approved_particle_fall_and_spin_clocks_are_independent() {
        assert_eq!(entrance_fast_pose(0.4).0, CLIPS[0]);
        assert!((entrance_fast_pose(0.4).1 - 44.5).abs() < 0.001);
        let mid = entrance_fast_pose(1.2);
        assert_eq!(mid.0, CLIPS[1]);
        assert!(
            (mid.1 - 24.).abs() < 0.001,
            "spin must retain its native 60 fps rate"
        );
        assert!(
            (mid.2.y - 187.5).abs() < 0.01,
            "approved gentle descent changed"
        );
        assert_eq!(entrance_fast_pose(1.6).0, "jmped");
        assert!(!entrance_fast_pose(1.74).3);
        assert!(entrance_fast_pose(1.75).3);
        assert_eq!(
            entrance_pose(3., 1.).0,
            CLIPS[2],
            "full Sunshine landing must remain available"
        );
        assert_eq!(entrance_pose(1.6, 1.).0, CLIPS[1]);
    }
}
