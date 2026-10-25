//! The game's particle effects (the nozzle's mist, smoke, dust, bursts, the Rocket's exhaust),
//! run by `libsms-present`'s `View` exactly as the hosts do, and drawn here as textured quads
//! with macroquad, including projected framebuffer ribbons. The viewer's own
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
    refraction: Material,
    scene: Option<Texture2D>,
    wave_masks: Vec<Option<Texture2D>>,
}

impl FxLayer {
    pub fn capture_report(&self) {
        let fx = &self.view.fx;
        let refs: Vec<_> = fx.batches.iter().filter(|b| b.blend == blend::REFRACTION).collect();
        eprintln!("fx batches={} refraction={}", fx.batches.len(), refs.len());
        for b in refs {
            let tex = &fx.textures[b.texture as usize];
            eprintln!("ribbon mask peak={} covered={}", tex.rgba.chunks_exact(4).map(|p| p[2]).max().unwrap_or(0), tex.rgba.chunks_exact(4).filter(|p| p[2]>16).count());
            let mut lo = libsms::Vec3::splat(f32::INFINITY);
            let mut hi = libsms::Vec3::splat(f32::NEG_INFINITY);
            for v in &fx.vertices[b.first_vertex as usize..(b.first_vertex+b.vertex_count) as usize] { lo=lo.min(v.pos); hi=hi.max(v.pos); }
            eprintln!("ribbon vertices={} bounds {:?}..{:?}", b.vertex_count, lo, hi);
        }
    }
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
            refraction: load_material(ShaderSource::Glsl { vertex: VERTEX, fragment: r#"#version 100
precision mediump float;
varying lowp vec2 uv;
uniform sampler2D Texture;
uniform sampler2D Scene;
uniform vec2 Screen;
void main() {
    vec4 wave = texture2D(Texture, uv);
    float coverage = wave.b;
    if (coverage < 0.0039) discard;
    vec2 offset = (vec2(wave.a, wave.g) * 2.0 - 1.0) / 4.0;
    vec2 screenUV = gl_FragCoord.xy / Screen;
    vec3 behind = texture2D(Scene, screenUV + offset * 0.035).rgb;
    gl_FragColor = vec4(behind, coverage);
}"# }, MaterialParams {
                pipeline_params: PipelineParams {
                    color_blend: Some(BlendState::new(Equation::Add, BlendFactor::Value(BlendValue::SourceAlpha), BlendFactor::OneMinusValue(BlendValue::SourceAlpha))),
                    depth_test: Comparison::LessOrEqual, depth_write: false,
                    ..Default::default()
                },
                uniforms: vec![UniformDesc::new("Screen", UniformType::Float2)],
                textures: vec!["Scene".into()],
                ..Default::default()
            }).ok()?,
            scene: None,
            wave_masks: Vec::new(),
        })
    }

    /// One sim tick's events (the effects the sim asked for).
    pub fn events(&mut self, events: &[Event], mario: &Mario) {
        self.view.events(events, mario);
    }

    /// FLUDD's muzzles as this viewer drew them (an alt-skin's own pack), so the nozzle mist
    /// and the water leave the drawn nozzle.
    pub fn set_muzzles(&mut self, muzzles: &[::glam::Mat4]) {
        self.view.set_muzzles(muzzles);
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
        self.wave_masks.resize_with(fx.textures.len(), || None);
        for (i, (slot, t)) in self.textures.iter_mut().zip(&fx.textures).enumerate() {
            if slot.as_ref().is_some_and(|(v, _)| *v == t.version) {
                continue;
            }
            let tex = Texture2D::from_rgba8(t.width as u16, t.height as u16, &t.rgba);
            tex.set_filter(FilterMode::Linear);
            *slot = Some((t.version, tex));
            let rgba: Vec<u8> = t.rgba.chunks_exact(4).flat_map(|p| [255, 255, 255, p[2]]).collect();
            self.wave_masks[i] = Some(Texture2D::from_rgba8(t.width as u16, t.height as u16, &rgba));
        }
        for b in &fx.batches {
            if b.blend == blend::REFRACTION {
                // Capture the current scene: this authored ribbon reads the framebuffer.
                let shot = get_screen_data();
                if let Some(scene) = self.scene.as_ref() { scene.update(&shot); }
                else { self.scene = Some(Texture2D::from_image(&shot)); }
                self.refraction.set_texture("Scene", self.scene.as_ref().unwrap().clone());
                self.refraction.set_uniform("Screen", vec2(screen_width(), screen_height()));
            }
            let Some((_, tex)) = self.textures.get(b.texture as usize).and_then(|t| t.as_ref())
            else {
                continue;
            };
            let first = b.first_vertex as usize;
            let verts = &fx.vertices[first..first + b.vertex_count as usize];
            // `SMS_FX_OPACITY`: portrait captures dim the additive layers so the pose stays
            // readable (the Unity icon generator does the same with MarioFx.Opacity).
            let opacity = std::env::var("SMS_FX_OPACITY")
                .ok()
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(1.0f32)
                .clamp(0.0, 1.0);
            let vertices: Vec<Vertex> = verts
                .iter()
                .map(|v| {
                    Vertex::new(
                        v.pos.x * scale,
                        v.pos.y * scale,
                        v.pos.z * scale,
                        v.u,
                        v.v,
                        Color::new(v.color[0], v.color[1], v.color[2], v.color[3] * opacity),
                    )
                })
                .collect();
            let indices: Vec<u16> = (0..vertices.len() as u16 / 4)
                .flat_map(|q| {
                    let o = q * 4;
                    [o, o + 1, o + 2, o, o + 2, o + 3]
                })
                .collect();
            if b.blend == blend::REFRACTION {
                let mut tinted = vertices.clone();
                let opacity = std::env::var("SMS_WAVE_TINT").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.35);
                for v in &mut tinted { v.color = Color::new(0.75, 0.9, 1.0, opacity).into(); }
                gl_use_material(&self.alpha);
                draw_mesh(&Mesh { vertices: tinted, indices: indices.clone(), texture: self.wave_masks[b.texture as usize].clone() });
            }
            gl_use_material(if b.blend == blend::REFRACTION {
                &self.refraction
            } else if b.blend == blend::ADDITIVE {
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
