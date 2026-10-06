//! Drawing FLUDD's water with the disc's textures.
//!
//! Flying particles are the game's stretched quads (`TModelWaterManager::calcDrawVtx`): a
//! square of half-diagonal `size * 0.707` turned to face the camera and pulled out along the
//! screen-space velocity by `mExtension * 4` (`unk5D18`). The game adds them into the frame's
//! alpha (`drawSilhouette`), so overlapping drops melt into one body, then refracts the screen
//! and adds a highlight wherever that alpha is set (`drawRefracAndSpec`). The same two passes
//! here: the quads add `timg/watermask.bti`'s soft falloff into a coverage target, and a
//! screen pass keeps what passes a threshold, shades it from the coverage slope (a tube's
//! normal) and tints it the water's blue. Particles stuck to the ground or a wall add flat
//! drops on the surface. Splashes are `TSplashManager`'s: 16-frame sprites
//! of `timg/splash.bti` that rise, grow from 50 to 150 and fade (SplashManager.cpp).

use std::path::Path;

use libsms::water::{State, WaterEvent, WaterManager};
use libsms_assets::texture::read_bti;
use macroquad::miniquad::{
    BlendFactor, BlendState, BlendValue, Equation, PipelineParams, UniformType,
};
use macroquad::prelude::*;

/// Decomp units to scene units (the viewer's `S`).
const S: f32 = 0.01;
/// `unk5D18`: how far the stretched quad reaches along the velocity.
const STRETCH: f32 = 4.0;

struct Splash {
    pos: libsms::Vec3,
    speed: f32,
    life: u8,
}

pub struct WaterView {
    drop: Texture2D,
    splash: Texture2D,
    splashes: Vec<Splash>,
    accum: Material,
    composite: Material,
    coverage: Option<RenderTarget>,
}

const ACCUM_VERTEX: &str = "#version 100
attribute vec3 position;
attribute vec2 texcoord;
attribute vec4 color0;
varying lowp vec2 uv;
varying lowp vec4 color;
uniform mat4 Model;
uniform mat4 Projection;
void main() {
    gl_Position = Projection * Model * vec4(position, 1);
    color = color0 / 255.0;
    uv = texcoord;
}";

const ACCUM_FRAGMENT: &str = "#version 100
precision mediump float;
varying lowp vec2 uv;
varying lowp vec4 color;
uniform sampler2D Texture;
void main() {
    float c = texture2D(Texture, uv).a * color.a;
    gl_FragColor = vec4(c, c, c, c);
}";

const COMPOSITE_VERTEX: &str = "#version 100
attribute vec3 position;
attribute vec2 texcoord;
varying lowp vec2 uv;
uniform mat4 Model;
uniform mat4 Projection;
void main() {
    gl_Position = Projection * Model * vec4(position, 1);
    uv = texcoord;
}";

// Coverage above the threshold is water. Its slope gives a normal across the stream; the
// edges brighten and a light from the upper left catches the top.
const COMPOSITE_FRAGMENT: &str = "#version 100
precision mediump float;
varying vec2 uv;
uniform sampler2D Texture;
uniform vec2 Texel;
void main() {
    float c = texture2D(Texture, uv).a;
    float m = smoothstep(0.30, 0.48, c);
    if (m <= 0.0) discard;
    float dx = texture2D(Texture, uv + vec2(Texel.x * 2.0, 0.0)).a
             - texture2D(Texture, uv - vec2(Texel.x * 2.0, 0.0)).a;
    float dy = texture2D(Texture, uv + vec2(0.0, Texel.y * 2.0)).a
             - texture2D(Texture, uv - vec2(0.0, Texel.y * 2.0)).a;
    vec3 n = normalize(vec3(-dx, -dy, 0.35));
    float rim = 1.0 - n.z;
    float spec = pow(max(dot(n, normalize(vec3(-0.45, 0.55, 0.7))), 0.0), 28.0);
    vec3 water = vec3(0.60, 0.76, 0.94);
    vec3 col = mix(water, vec3(1.0), clamp(spec * 1.4 + rim * 0.25, 0.0, 1.0));
    float a = m * clamp(0.28 + rim * 0.55 + spec, 0.0, 0.95);
    gl_FragColor = vec4(col, a);
}";

fn bti(dir: &Path, rel: &str) -> Option<libsms_assets::texture::Texture> {
    let data = std::fs::read(dir.join(rel)).ok()?;
    read_bti(&data, rel.to_owned()).ok()
}

fn upload(w: u32, h: u32, rgba: &[u8]) -> Texture2D {
    let tex = Texture2D::from_rgba8(w as u16, h as u16, rgba);
    tex.set_filter(FilterMode::Linear);
    tex
}

impl WaterView {
    pub fn load(dir: &Path) -> Option<WaterView> {
        // The droplet's coverage: `watermask`'s soft round falloff.
        let mask = bti(dir, "timg/watermask.bti")?;
        let (w, h) = (mask.image.width, mask.image.height);
        let rgba: Vec<u8> = mask
            .image
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|m| [255, 255, 255, m[0]])
            .collect();
        let drop = upload(w, h, &rgba);
        // `GXSetTevColorIn(C0, ONE, TEXC, ZERO)`: (168, 203, 227) to white by the texture.
        let sp = bti(dir, "timg/splash.bti")?;
        let mut rgba = Vec::with_capacity(sp.image.rgba.len());
        for p in sp.image.rgba.as_chunks::<4>().0 {
            let t = p[0] as f32 / 255.0;
            let mix = |c: f32| ((c + (1.0 - c) * t) * 255.0) as u8;
            rgba.extend_from_slice(&[
                mix(168.0 / 255.0),
                mix(203.0 / 255.0),
                mix(227.0 / 255.0),
                p[3],
            ]);
        }
        let splash = upload(sp.image.width, sp.image.height, &rgba);
        let add = BlendState::new(Equation::Add, BlendFactor::One, BlendFactor::One);
        let accum = load_material(
            ShaderSource::Glsl {
                vertex: ACCUM_VERTEX,
                fragment: ACCUM_FRAGMENT,
            },
            MaterialParams {
                pipeline_params: PipelineParams {
                    color_blend: Some(add),
                    alpha_blend: Some(add),
                    depth_write: false,
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .map_err(|e| eprintln!("water shader: {e:?}"))
        .ok()?;
        let over = BlendState::new(
            Equation::Add,
            BlendFactor::Value(BlendValue::SourceAlpha),
            BlendFactor::OneMinusValue(BlendValue::SourceAlpha),
        );
        let composite = load_material(
            ShaderSource::Glsl {
                vertex: COMPOSITE_VERTEX,
                fragment: COMPOSITE_FRAGMENT,
            },
            MaterialParams {
                pipeline_params: PipelineParams {
                    color_blend: Some(over),
                    alpha_blend: Some(over),
                    ..Default::default()
                },
                uniforms: vec![UniformDesc::new("Texel", UniformType::Float2)],
                ..Default::default()
            },
        )
        .map_err(|e| eprintln!("water shader: {e:?}"))
        .ok()?;
        Some(WaterView {
            drop,
            splash,
            splashes: Vec::new(),
            accum,
            composite,
            coverage: None,
        })
    }

    /// One 30 Hz frame: new splashes from the manager's events, then `TSplashManager::move`.
    pub fn step(&mut self, events: &[WaterEvent]) {
        for s in &mut self.splashes {
            // `unk638`: -0.5 * 2 * 2 per frame.
            s.speed -= 2.0;
            s.pos.y += s.speed;
            s.life = s.life.saturating_sub(1);
        }
        self.splashes.retain(|s| s.life > 0);
        for e in events {
            if let WaterEvent::Splash(p) = e
                && self.splashes.len() < 64
            {
                self.splashes.push(Splash {
                    pos: *p,
                    speed: 5.0,
                    life: 16,
                });
            }
        }
    }

    /// Draws the water through `camera` (scene units). Leaves `camera` set.
    pub fn draw(&mut self, water: &WaterManager, camera: &Camera3D) {
        let (eye, target) = (camera.position, camera.target);
        let fwd = (target - eye).normalize_or_zero();
        let right = fwd.cross(Vec3::Y).normalize_or_zero();
        let up = right.cross(fwd);
        let scene = |p: libsms::Vec3| vec3(p.x, p.y, p.z) * S;

        // Back to front, so the blended quads overlap correctly.
        let mut order: Vec<(f32, usize)> = water
            .particles
            .iter()
            .enumerate()
            .map(|(i, p)| ((scene(p.pos) - eye).length_squared(), i))
            .collect();
        order.sort_by(|a, b| b.0.total_cmp(&a.0));

        let mut drops = Mesh {
            vertices: Vec::new(),
            indices: Vec::new(),
            texture: Some(self.drop.clone()),
        };
        let white = Color::new(1.0, 1.0, 1.0, 1.0);
        for (_, i) in order {
            let p = &water.particles[i];
            let c = scene(p.pos);
            let half = 1.414 * 0.5 * p.size * S;
            let corners: [Vec3; 4] = match p.state {
                State::Flying => {
                    if !water.drawn(p) {
                        continue;
                    }
                    let v = vec3(p.vel.x, p.vel.y, p.vel.z) * S * water.extension(p);
                    let (vx, vy) = (v.dot(right), v.dot(up));
                    let len = (vx * vx + vy * vy).sqrt();
                    if len * 100.0 > 1.0 {
                        let (nx, ny) = (vx / len * half, vy / len * half);
                        let at = |x: f32, y: f32| c + right * x + up * y;
                        [
                            at(nx + vx * STRETCH, ny + vy * STRETCH),
                            at(ny, -nx),
                            at(-nx - vx * STRETCH, -ny - vy * STRETCH),
                            at(-ny, nx),
                        ]
                    } else {
                        let at = |x: f32, y: f32| c + right * x + up * y;
                        [
                            at(-half, half),
                            at(half, half),
                            at(half, -half),
                            at(-half, -half),
                        ]
                    }
                }
                // A flat drop on the surface it clings to.
                State::Ground | State::Wall => {
                    let n = if p.state == State::Ground {
                        Vec3::Y
                    } else {
                        let d = (eye - c).normalize_or_zero();
                        vec3(d.x, 0.0, d.z).normalize_or(Vec3::Z)
                    };
                    let a = n.any_orthonormal_vector();
                    let b = n.cross(a);
                    let r = half * 1.2;
                    let lift = n * 0.01;
                    [
                        c + lift + (-a + b) * r,
                        c + lift + (a + b) * r,
                        c + lift + (a - b) * r,
                        c + lift + (-a - b) * r,
                    ]
                }
            };
            let base = drops.vertices.len() as u16;
            for (k, uv) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
                .into_iter()
                .enumerate()
            {
                let v = corners[k];
                drops
                    .vertices
                    .push(Vertex::new(v.x, v.y, v.z, uv.0, uv.1, white));
            }
            drops
                .indices
                .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            if drops.vertices.len() > 60_000 {
                break;
            }
        }
        if !drops.indices.is_empty() {
            // Pass 1: coverage (`drawSilhouette`), at half resolution.
            let (w, h) = (
                (screen_width() / 2.0).max(1.0) as u32,
                (screen_height() / 2.0).max(1.0) as u32,
            );
            let stale = self
                .coverage
                .as_ref()
                .is_none_or(|rt| rt.texture.width() as u32 != w || rt.texture.height() as u32 != h);
            if stale {
                let rt = render_target(w, h);
                rt.texture.set_filter(FilterMode::Linear);
                self.coverage = Some(rt);
            }
            let rt = self.coverage.clone().unwrap();
            set_camera(&Camera3D {
                position: camera.position,
                target: camera.target,
                up: camera.up,
                fovy: camera.fovy,
                aspect: Some(screen_width() / screen_height()),
                render_target: Some(rt.clone()),
                ..Default::default()
            });
            clear_background(Color::new(0.0, 0.0, 0.0, 0.0));
            gl_use_material(&self.accum);
            draw_mesh(&drops);
            gl_use_default_material();
            // Pass 2: the water surface over the scene (`drawRefracAndSpec`).
            set_default_camera();
            self.composite
                .set_uniform("Texel", vec2(1.0 / w as f32, 1.0 / h as f32));
            gl_use_material(&self.composite);
            draw_texture_ex(
                &rt.texture,
                0.0,
                0.0,
                WHITE,
                DrawTextureParams {
                    dest_size: Some(vec2(screen_width(), screen_height())),
                    // Render targets come back upside down.
                    flip_y: true,
                    ..Default::default()
                },
            );
            gl_use_default_material();
            set_camera(camera);
        }

        let mut splashes = Mesh {
            vertices: Vec::new(),
            indices: Vec::new(),
            texture: Some(self.splash.clone()),
        };
        for s in &self.splashes {
            let c = scene(s.pos);
            let age = (16 - s.life) as f32 / 16.0;
            let half = (age * 100.0 + 50.0) * S;
            let col = Color::new(1.0, 1.0, 1.0, s.life as f32 / 16.0);
            let at = |x: f32, y: f32| c + right * x + up * y;
            let corners = [
                at(-half, half),
                at(half, half),
                at(half, -half),
                at(-half, -half),
            ];
            let base = splashes.vertices.len() as u16;
            for (k, uv) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
                .into_iter()
                .enumerate()
            {
                let v = corners[k];
                splashes
                    .vertices
                    .push(Vertex::new(v.x, v.y, v.z, uv.0, uv.1, col));
            }
            splashes.indices.extend_from_slice(&[
                base,
                base + 1,
                base + 2,
                base,
                base + 2,
                base + 3,
            ]);
        }
        if !splashes.indices.is_empty() {
            draw_mesh(&splashes);
        }
    }
}
