//! Luigi's HSD mesh (`PlLgNr.dat`) and his own FLUDD's Spray-neck lean, ported from the
//! viewer's proven `viewer/src/luigi.rs` and `viewer/src/fludd.rs` (do not re-derive the
//! posing or the lean; the viewer already established both).
//!
//! HSD is not J3D: its materials are plain textured (no TEV), coloured by the source's
//! per-vertex GX CLR0, which `Model::load_for` folds into the batch colour for untextured
//! batches (`apply_untextured_color`). The host reads Luigi's static mesh through the same
//! model readers as Mario's parts, at the index after the J3D models, and each frame the view
//! skins his 61 joints with the shared Sunshine clips through [`sms::Retarget`], anchoring his
//! lowest point to Mario's (feet to feet, belly to belly) and leaning the shared FLUDD's Spray
//! neck out of his head's way.
use super::*;
use libsms::{FluddLook, NozzleKind};
use libssbm_assets::model as hsd;
use libssbm_assets::sms;

/// FLUDD alt-skin: the Spray neck leans to one side so it clears Luigi's head. Degrees about
/// [`LUIGI_LEAN_PIVOT`] around Y (the model's side axis), applied to [`LUIGI_LEAN_JOINTS`] only.
const LUIGI_LEAN_DEG: f32 = 45.0;
/// The collar base, measured on the posed model.
const LUIGI_LEAN_PIVOT: Vec3 = Vec3::new(37.0, -40.0, 0.0);
/// `null_pump_wA`, `null_pump_wB`, `nozzle_center`, `jnt_G_handle`: the neck above the collar
/// base. The levers and grips are not part of it, and the tank (`jnt_G_main`) stays put.
const LUIGI_LEAN_JOINTS: [usize; 4] = [4, 5, 6, 7];
/// The thin frame rod (`_mat_G_main`); it would poke out of the tank when the neck leans.
pub const LUIGI_ROD_SHAPE: usize = 3;
/// How fast the neck swings to the other side, degrees per second: about two frames for the
/// full +45 to -45, quick because the water visibly breaks while it turns.
const LUIGI_LEAN_RATE: f32 = 1800.0;
/// How much lower and further off the wall Luigi hangs than the feet anchor puts him, in
/// Sunshine decomp cm (`SMS_HANG_OFFSET` in the viewer, default 8,6).
pub const LUIGI_HANG_OFFSET: (f32, f32) = (8.0, 6.0);

/// Luigi's HSD materials carry Melee's 0.70 diffuse colour, which the source's stage lighting
/// makes up for and RoR2's does not. The Melee host brightens fighter bodies by this much
/// (`Visuals.FighterBrightness`, default 1.5); Sunshine Luigi gets the same lift here.
const LUIGI_BRIGHTNESS: f32 = 1.5;

/// One HSD batch as the host reads it (mirrors the FFI's `SmsBatch` fields).
pub struct HsdBatch {
    pub first_index: u32,
    pub index_count: u32,
    pub first_vertex: u32,
    pub vertex_count: u32,
    /// Plain texture id into [`Textures::list`], or -1 for an untextured batch.
    pub texture: i32,
    /// [`blend`] class: 0 opaque, 1 cutout, 2 translucent, 3 additive.
    pub blend: u8,
    /// Lit bits (1 colour, 2 alpha); HSD carries no vertex-alpha fade.
    pub lit: u8,
    /// Index into [`hsd::Model::batches`] this draw's vertices come from (the load filters out
    /// empty and DObj-hidden batches, so this is not the position in [`Self::batches`]).
    src: usize,
}

/// Luigi's HSD mesh plus the FLUDD lean state, drawn at the index after the J3D models.
pub struct Luigi {
    model: hsd::Model,
    rig: sms::Rig,
    retarget: sms::Retarget,
    /// Mario's `jnt_chest` frame in Luigi's `WaistN` frame: FLUDD's chest transform.
    chest_align: Mat4,
    bck_dir: PathBuf,
    clips: HashMap<String, sms::Clip>,
    /// The current Spray-neck lean, eased toward its target; and the side it last chose
    /// (+1 the right-hand lean, -1 the mirrored one, picked from where the host aims).
    lean: f32,
    side: f32,

    /// Static mesh for the host's model readers (at the index after the J3D models).
    pub batches: Vec<HsdBatch>,
    /// Flattened triangle indices into the concatenated vertex list (the FFI's convention).
    pub indices: Vec<u32>,
    /// One UV pair per vertex, bind order.
    pub uvs: Vec<[f32; 2]>,
    pub vertex_count: usize,
    /// Bind-pose bounds in Sunshine decomp units (Melee units ×10, like the skinned vertices).
    pub lo: Vec3,
    pub hi: Vec3,
}

impl Luigi {
    /// Loads Luigi's HSD mesh from the Melee cache and prepares its static host mesh; `sms_dir`
    /// supplies the Sunshine rig and clips the retarget needs. `textures` gains one plain baked
    /// texture per textured batch.
    pub fn load(
        melee_dir: &Path,
        sms_dir: &Path,
        textures: &mut Textures,
    ) -> Result<Luigi, String> {
        let model = hsd::Model::load_for(melee_dir, 0, &libssbm_assets::fighter::LUIGI)
            .map_err(|e| e.to_string())?;
        let rig =
            sms::Rig::from_file(&sms_dir.join("bmd/ma_mdl1.bmd")).map_err(|e| e.to_string())?;
        let retarget = sms::Retarget::new(&model, &rig);
        // FLUDD rides the chest: the retarget maps `jnt_chest` (14) onto `WaistN` (5), so its
        // bind rotations give the frame that carries FLUDD's Mario-space base to Luigi.
        let chest_align =
            Mat4::from_mat3(retarget.target_bind()[5].transpose() * retarget.source_bind()[14]);

        let mut batches = Vec::with_capacity(model.batches.len());
        let mut indices = Vec::new();
        let mut uvs = Vec::new();
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for (src, batch) in model.batches.iter().enumerate() {
            if batch.indices.is_empty() {
                continue;
            }
            // `ftParts_8007487C`'s DObj alternatives: Melee keeps one hand open and one closed
            // (among other variants) and shows only the chosen one.
            if !model
                .normal_visible
                .get(batch.dobj)
                .copied()
                .unwrap_or(true)
            {
                continue;
            }
            let first_vertex = uvs.len() as u32;
            let texture = match &batch.texture {
                Some(t) => {
                    textures.list.push(Baked {
                        width: t.width as u32,
                        height: t.height as u32,
                        wrap: [t.wrap_s, t.wrap_t],
                        rgba: t.rgba.clone(),
                        env: None,
                    });
                    textures.list.len() as i32 - 1
                }
                None => -1,
            };
            batches.push(HsdBatch {
                first_index: indices.len() as u32,
                index_count: batch.indices.len() as u32,
                first_vertex,
                vertex_count: batch.vertices.len() as u32,
                texture,
                blend: batch.blend,
                lit: batch.lit,
                src,
            });
            indices.extend(batch.indices.iter().map(|&i| first_vertex + i));
            for v in &batch.vertices {
                uvs.push([v.uv.x, v.uv.y]);
                // The skinned vertices are emitted ×10 into Sunshine's decomp units; the bind
                // bounds should match so a host's first box is not a tenth of the real size.
                lo = lo.min(v.position * 10.0);
                hi = hi.max(v.position * 10.0);
            }
        }
        if lo.x > hi.x {
            (lo, hi) = (Vec3::ZERO, Vec3::ZERO);
        }
        Ok(Luigi {
            model,
            rig,
            retarget,
            chest_align,
            bck_dir: sms_dir.join("bck"),
            clips: HashMap::new(),
            lean: 0.0,
            side: -1.0,
            batches,
            indices,
            vertex_count: uvs.len(),
            uvs,
            lo,
            hi,
        })
    }

    /// Luigi's joint matrices for a Sunshine clip (`wait`, `run1`, `jump`...), in Melee units.
    pub fn pose(&mut self, clip_name: &str, frame: f32) -> Option<Vec<Mat4>> {
        if !self.clips.contains_key(clip_name) {
            let path = self.bck_dir.join(format!("ma_{clip_name}.bck"));
            let clip = sms::Clip::from_file(&path).ok()?;
            self.clips.insert(clip_name.to_string(), clip);
        }
        let clip = self.clips.get(clip_name)?;
        Some(self.retarget.pose(&self.model, &self.rig, clip, frame))
    }

    /// The lowest skinned point of a pose from [`Self::pose`], in Luigi's model units.
    pub fn lowest(&self, world: &[Mat4]) -> f32 {
        let mut low = f32::INFINITY;
        for i in 0..self.model.batches.len() {
            for (p, _) in self.model.skin(i, world) {
                low = low.min(p.y);
            }
        }
        low
    }

    /// Where FLUDD straps on: `WaistN` turned into Mario's chest frame and scaled to Sunshine's
    /// decomp units (Melee models are a tenth of one).
    pub fn chest(&self, world: &[Mat4]) -> Mat4 {
        let Some(chest) = world.get(5).copied() else {
            return Mat4::IDENTITY;
        };
        let (_, r, t) = (chest * self.chest_align).to_scale_rotation_translation();
        Mat4::from_rotation_translation(r, t * 10.0)
    }

    /// Applies the Spray-neck lean to the shared FLUDD's body joints and reports whether the
    /// frame rod (shape 3) must hide. `deployed` is `housed < 0.5`; the game's Mario never
    /// leans, only the alt-skin's taller head needs the clearance.
    pub fn lean_fludd(
        &mut self,
        joints: &mut [Mat4],
        look: Option<FluddLook>,
        deployed: bool,
        dt: f32,
    ) -> bool {
        let spraying = deployed && look.is_some_and(|l| l.nozzle == NozzleKind::Spray);
        // Crossed well past the facing plane, so aiming straight ahead cannot flicker it.
        if let Some(l) = look
            && l.aimed
            && l.aim_turn.unsigned_abs() > 512
        {
            self.side = if l.aim_turn > 0 { -1.0 } else { 1.0 };
        }
        // Always lean fully while spraying (to the side the last aim picked). The RoR2 host's
        // facing is the movement direction, not the camera, so gating on aim_turn made the neck
        // drop out whenever Mario kited away from the locked target; the lean exists to clear
        // his head, and the head swivel already handles where the nozzle points.
        let target = if spraying { self.side * LUIGI_LEAN_DEG } else { 0.0 };
        let step = LUIGI_LEAN_RATE * dt.max(0.0);
        self.lean += (target - self.lean).clamp(-step, step);
        let leaning = self.lean.abs() > 0.5;
        if leaning {
            let pivot = LUIGI_LEAN_PIVOT;
            let m = Mat4::from_translation(pivot)
                * Mat4::from_rotation_y(self.lean.to_radians())
                * Mat4::from_translation(-pivot);
            for i in LUIGI_LEAN_JOINTS {
                if let Some(j) = joints.get_mut(i) {
                    *j = m * *j;
                }
            }
        }
        leaning
    }

    /// Skins Luigi's HSD mesh for `world` (Melee units) into the host's per-frame buffers,
    /// scaling Melee units to Sunshine's decomp units (×10) and appending one colour + texture
    /// id per batch. Returns the first vertex and colour offsets.
    pub fn skin_into(
        &self,
        world: &[Mat4],
        positions: &mut Vec<Vec3>,
        normals: &mut Vec<Vec3>,
        uvs: &mut Vec<[f32; 2]>,
        colors: &mut Vec<C4>,
        texture_ids: &mut Vec<i32>,
    ) -> (u32, u32) {
        let first_vertex = positions.len() as u32;
        let first_color = colors.len() as u32;
        for hb in &self.batches {
            let batch = &self.model.batches[hb.src];
            for (idx, (p, n)) in self.model.skin(hb.src, world).into_iter().enumerate() {
                positions.push(p * 10.0);
                normals.push(n);
                uvs.push([batch.vertices[idx].uv.x, batch.vertices[idx].uv.y]);
            }
            texture_ids.push(hb.texture);
            // Melee's 0.70 diffuse reads dark under RoR2's lighting; lift it like the Melee host does.
            colors.push([
                batch.color[0] * LUIGI_BRIGHTNESS,
                batch.color[1] * LUIGI_BRIGHTNESS,
                batch.color[2] * LUIGI_BRIGHTNESS,
                batch.color[3],
            ]);
        }
        (first_vertex, first_color)
    }
}
