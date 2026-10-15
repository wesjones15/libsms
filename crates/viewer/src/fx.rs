//! The game's particle effects (the nozzle's mist, smoke, dust, bursts, the Rocket's exhaust),
//! run by `libsms-present`'s `View` exactly as the hosts do, and drawn here as textured quads
//! with macroquad. Refraction batches (the water's heat haze) aren't drawn. The viewer's own
//! Mario and water don't come from the view; only the effects do, so the models it also poses are
//! unused here.

use libsms::{Event, Mario, Sim};
use libsms_present::View;
use libsms_present::particles::blend;
use macroquad::material::{MaterialParams, gl_use_default_material, gl_use_material, load_material};
use macroquad::miniquad::{
    BlendFactor, BlendState, BlendValue, Comparison, Equation, PipelineParams, ShaderSource,
};
use macroquad::models::{Mesh, Vertex, draw_mesh};
use macroquad::prelude::*;
use std::path::Path;

const VERTEX: &str = r#"#version 100
attribute vec3 position;
attribute vec2 texcoord;
attribute vec4 color0;
varying lowp vec2 uv;
varying lowp vec4 color;
uniform mat4 Model;
uniform mat4 Projection;
void main() {
    gl_Position = Projection * Model * vec4(position, 1.0);
    color = color0 / 255.0;
    uv = texcoord;
}"#;

const FRAGMENT: &str = r#"#version 100
varying lowp vec4 color;
varying lowp vec2 uv;
uniform sampler2D Texture;
void main() {
    gl_FragColor = color * texture2D(Texture, uv);
}"#;

pub struct FxLayer {
    view: View,
    /// Per effect texture: the version uploaded, and the texture.
    textures: Vec<Option<(u32, Texture2D)>>,
    additive: Material,
    alpha: Material,
}

impl FxLayer {
    pub fn load(dir: &Path) -> Option<FxLayer> {
        let view = View::load(dir).ok()?;
        let material = |dst: BlendFactor| {
            load_material(
                ShaderSource::Glsl {
                    vertex: VERTEX,
                    fragment: FRAGMENT,
                },
                MaterialParams {
                    pipeline_params: PipelineParams {
                        color_blend: Some(BlendState::new(
                            Equation::Add,
                            BlendFactor::Value(BlendValue::SourceAlpha),
                            dst,
                        )),
                        depth_test: Comparison::LessOrEqual,
                        depth_write: false,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )
            .ok()
        };
        Some(FxLayer {
            view,
            textures: Vec::new(),
            additive: material(BlendFactor::One)?,
            alpha: material(BlendFactor::OneMinusValue(BlendValue::SourceAlpha))?,
        })
    }

    /// One sim tick's events (the effects the sim asked for).
    pub fn events(&mut self, events: &[Event], mario: &Mario) {
        self.view.events(events, mario);
    }

    /// Runs the effects up to the sim's state; `right` and `up` are the camera's axes.
    pub fn update(&mut self, sim: &mut Sim, dt: f32, right: Vec3, up: Vec3) {
        self.view.set_camera(
            libsms::Vec3::new(right.x, right.y, right.z),
            libsms::Vec3::new(up.x, up.y, up.z),
        );
        self.view.update(sim, dt);
    }

    /// Draws this frame's effects in the current 3D camera; `scale` is metres per sim unit.
    pub fn draw(&mut self, scale: f32) {
        let fx = &self.view.fx;
        self.textures.resize_with(fx.textures.len(), || None);
        for (slot, t) in self.textures.iter_mut().zip(&fx.textures) {
            if slot.as_ref().is_some_and(|(v, _)| *v == t.version) {
                continue;
            }
            let tex = Texture2D::from_rgba8(t.width as u16, t.height as u16, &t.rgba);
            tex.set_filter(FilterMode::Linear);
            *slot = Some((t.version, tex));
        }
        for b in &fx.batches {
            if b.blend > blend::ALPHA {
                continue;
            }
            let Some((_, tex)) = self.textures.get(b.texture as usize).and_then(|t| t.as_ref())
            else {
                continue;
            };
            let first = b.first_vertex as usize;
            let verts = &fx.vertices[first..first + b.vertex_count as usize];
            let vertices: Vec<Vertex> = verts
                .iter()
                .map(|v| {
                    Vertex::new(
                        v.pos.x * scale,
                        v.pos.y * scale,
                        v.pos.z * scale,
                        v.u,
                        v.v,
                        Color::new(v.color[0], v.color[1], v.color[2], v.color[3]),
                    )
                })
                .collect();
            let indices: Vec<u16> = (0..vertices.len() as u16 / 4)
                .flat_map(|q| {
                    let o = q * 4;
                    [o, o + 1, o + 2, o, o + 2, o + 3]
                })
                .collect();
            gl_use_material(if b.blend == blend::ADDITIVE {
                &self.additive
            } else {
                &self.alpha
            });
            draw_mesh(&Mesh {
                vertices,
                indices,
                texture: Some(tex.clone()),
            });
        }
        gl_use_default_material();
    }
}
