//! Luigi's HSD mesh (`PlLgNr.dat`) drawn beside Mario in the Sunshine viewer.
//!
//! HSD is not J3D: its materials are not TEV, so this uses a small textured pipeline of its
//! own (position, uv, vertex colour), keeps the mesh static on the GPU and skins it on the
//! CPU each frame from Luigi's 61 joint matrices.
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use glam::{Mat4, Vec3};
use libssbm_assets::model::{Model, Vertex};
use libssbm_assets::sms;
use macroquad::miniquad::{
    Bindings, BufferId, BufferLayout, BufferSource, BufferType, BufferUsage, Comparison,
    CullFace, PassAction, Pipeline, PipelineParams, RenderingBackend, ShaderId, ShaderMeta,
    ShaderSource, TextureId, TextureWrap, UniformBlockLayout, UniformDesc, UniformType,
    UniformsSource, VertexAttribute, VertexFormat,
};
use macroquad::miniquad;

use crate::j3d::ctx;

const FLOATS: usize = 9; // pos3 + uv2 + colour4

const VERTEX: &str = r#"#version 100
attribute vec3 pos;
attribute vec2 uv;
attribute vec4 col;
uniform mat4 Mvp;
varying highp vec2 v_uv;
varying lowp vec4 v_col;
void main() {
    gl_Position = Mvp * vec4(pos, 1.0);
    v_uv = uv;
    v_col = col;
}"#;

const FRAGMENT: &str = r#"#version 100
uniform sampler2D Tex;
varying highp vec2 v_uv;
varying lowp vec4 v_col;
void main() {
    gl_FragColor = texture2D(Tex, v_uv) * v_col;
}"#;

#[repr(C)]
struct Uniforms {
    mvp: Mat4,
}

struct GBatch {
    vbuf: BufferId,
    ibuf: BufferId,
    tex: TextureId,
    count: i32,
    verts: Vec<Vertex>,
    inverse: Vec<Mat4>,
}

pub struct Luigi {
    pipe: Pipeline,
    batches: Vec<GBatch>,
    scratch: Vec<f32>,
    model: Model,
    rig: sms::Rig,
    retarget: sms::Retarget,
    bck_dir: PathBuf,
    clips: HashMap<String, sms::Clip>,
}

impl Luigi {
    /// `melee_dir` holds `PlLgNr.dat`; `costume` selects the colour set.
    pub fn load(melee_dir: &Path, sms_dir: &Path, costume: usize) -> Result<Luigi, String> {
        let model = Model::load_for(melee_dir, costume, &libssbm_assets::fighter::LUIGI)
            .map_err(|e| e.to_string())?;
        let rig = sms::Rig::from_file(&sms_dir.join("bmd/ma_mdl1.bmd")).map_err(|e| e.to_string())?;
        let retarget = sms::Retarget::new(&model, &rig);
        let ctx = ctx();
        let shader = ctx
            .new_shader(
                ShaderSource::Glsl {
                    vertex: VERTEX,
                    fragment: FRAGMENT,
                },
                ShaderMeta {
                    images: vec!["Tex".into()],
                    uniforms: UniformBlockLayout {
                        uniforms: vec![UniformDesc::new("Mvp", UniformType::Mat4)],
                    },
                },
            )
            .map_err(|e| format!("Luigi shader: {e}"))?;
        let pipe = ctx.new_pipeline(
            &[BufferLayout::default()],
            &[
                VertexAttribute::new("pos", VertexFormat::Float3),
                VertexAttribute::new("uv", VertexFormat::Float2),
                VertexAttribute::new("col", VertexFormat::Float4),
            ],
            shader,
            PipelineParams {
                depth_test: Comparison::LessOrEqual,
                depth_write: true,
                cull_face: CullFace::Nothing,
                ..Default::default()
            },
        );
        let inverse: Vec<Mat4> = model.joints.iter().map(|j| j.inverse).collect();
        let mut batches = Vec::with_capacity(model.batches.len());
        let mut scratch = Vec::new();
        for batch in &model.batches {
            if batch.indices.is_empty() {
                continue;
            }
            let indices: Vec<u16> = batch.indices.iter().map(|&i| i as u16).collect();
            let vbuf = ctx.new_buffer(
                BufferType::VertexBuffer,
                BufferUsage::Stream,
                BufferSource::empty::<f32>(batch.vertices.len() * FLOATS),
            );
            let ibuf = ctx.new_buffer(
                BufferType::IndexBuffer,
                BufferUsage::Immutable,
                BufferSource::slice(&indices),
            );
            let tex = match &batch.texture {
                Some(t) => {
                    let id = ctx.new_texture_from_rgba8(t.width, t.height, &t.rgba);
                    ctx.texture_set_wrap(id, TextureWrap::Clamp, TextureWrap::Clamp);
                    id
                }
                None => {
                    let id = ctx.new_texture_from_rgba8(1, 1, &[255, 255, 255, 255]);
                    id
                }
            };
            scratch.resize(batch.vertices.len() * FLOATS, 0.0);
            batches.push(GBatch {
                vbuf,
                ibuf,
                tex,
                count: indices.len() as i32,
                verts: batch.vertices.clone(),
                inverse: inverse.clone(),
            });
        }
        Ok(Luigi {
            pipe,
            batches,
            scratch,
            model,
            rig,
            retarget,
            bck_dir: sms_dir.join("bck"),
            clips: HashMap::new(),
        })
    }

    /// `world` is one model-space matrix per Luigi joint; `root` maps model space to the scene.
    pub fn draw(&mut self, world: &[Mat4], root: Mat4, view: Mat4, proj: Mat4) {
        // Flush macroquad's batching so the depth buffer has the scene.
        // SAFETY: main thread, between draws.
        unsafe { macroquad::window::get_internal_gl() }.flush();
        // Melee model units to Sunshine cm (the Melee mod.s MetresPerUnit 0.1).
        let root = root * Mat4::from_scale(Vec3::splat(10.0));
        let mvp = proj * view * root;
        let ctx = ctx();
        ctx.begin_default_pass(PassAction::Nothing);
        for b in &mut self.batches {
            self.scratch.resize(b.verts.len() * FLOATS, 0.0);
            for (v, out) in b.verts.iter().zip(self.scratch.chunks_exact_mut(FLOATS)) {
                let mut p = Vec3::ZERO;
                if v.weights.is_empty() {
                    p = v.position;
                } else {
                    let mut sum = 0.0;
                    for &(j, w) in &v.weights {
                        if let (Some(m), Some(inv)) = (world.get(j), b.inverse.get(j)) {
                            p += (m * inv).transform_point3(v.position) * w;
                            sum += w;
                        }
                    }
                    if sum > 1e-6 {
                        p /= sum;
                    }
                }
                out[0] = p.x;
                out[1] = p.y;
                out[2] = p.z;
                out[3] = v.uv.x;
                out[4] = v.uv.y;
                out[5] = v.color[0];
                out[6] = v.color[1];
                out[7] = v.color[2];
                out[8] = v.color[3];
            }
            ctx.buffer_update(b.vbuf, BufferSource::slice(&self.scratch));
            ctx.apply_pipeline(&self.pipe);
            ctx.apply_bindings(&Bindings {
                vertex_buffers: vec![b.vbuf],
                index_buffer: b.ibuf,
                images: vec![b.tex],
            });
            let u = Uniforms { mvp };
            ctx.apply_uniforms(UniformsSource::table(&u));
            ctx.draw(0, b.count, 1);
        }
        ctx.end_render_pass();
    }

    /// Luigi.s joint matrices for a Sunshine clip (`wait`, `run1`, `jump`...).
    pub fn pose(&mut self, clip_name: &str, frame: f32) -> Option<Vec<Mat4>> {
        if !self.clips.contains_key(clip_name) {
            let path = self.bck_dir.join(format!("ma_{clip_name}.bck"));
            let clip = sms::Clip::from_file(&path).ok()?;
            self.clips.insert(clip_name.to_string(), clip);
        }
        let clip = self.clips.get(clip_name)?;
        Some(self.retarget.pose(&self.model, &self.rig, clip, frame))
    }

    pub fn bind_world(&self) -> Vec<Mat4> {
        self.model.bind_world()
    }

    pub fn joint_count(&self) -> usize {
        self.model.joints.len()
    }
}

pub fn melee_dir() -> std::path::PathBuf {
    std::env::var("SSBM_ASSETS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("../libssbm/assets"))
}









