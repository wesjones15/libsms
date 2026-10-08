//! Drawing FLUDD's water with the disc's textures.
//!
//! Flying particles are the game's stretched quads (`TModelWaterManager::calcDrawVtx`): a
//! square of half-diagonal `size * 0.707` turned to face the camera and pulled out along the
//! screen-space velocity by `mExtension * 4` (`unk5D18`). The game draws them into the
//! framebuffer's alpha with the depth test on (`drawSilhouette`), so overlapping drops melt into
//! one body and anything in front of the water hides it; it then copies the frame and draws it
//! back refracted, with a highlight, wherever that alpha is set (`drawRefracAndSpec`). The same
//! here: the quads add `timg/watermask.bti`'s soft falloff into the window's alpha against the
//! scene's depth, the frame is copied to a texture, and a screen pass keeps what passes a
//! threshold, bends the scene behind it by the coverage slope (a tube's normal), tints it and
//! adds the highlight. Particles stuck to the ground or a wall add flat drops on the surface.
//! Splashes are `TSplashManager`'s: 16-frame sprites of `timg/splash.bti` that rise, grow from
//! 50 to 150 and fade (SplashManager.cpp).

use std::path::Path;

use libsms::water::{SLOTS, State, WaterEvent, WaterManager};
use libsms_assets::texture::read_bti;
use macroquad::miniquad::{
    self, BlendFactor, BlendState, BlendValue, BufferLayout, BufferSource, BufferType, BufferUsage,
    Comparison, Equation, PassAction, Pipeline, PipelineParams, RenderingBackend, ShaderMeta,
    TextureId, TextureParams, TextureWrap, UniformBlockLayout, UniformDesc, UniformType,
    UniformsSource, VertexAttribute, VertexFormat,
};
use macroquad::prelude::*;

/// Decomp units to scene units (the viewer's `S`).
const S: f32 = 0.01;
/// `unk5D18`: how far the stretched quad reaches along the velocity.
const STRETCH: f32 = 4.0;
/// Floats per drop vertex: position, uv.
const FLOATS: usize = 5;

struct Splash {
    pos: libsms::Vec3,
    speed: f32,
    life: u8,
}

/// The frame copy the screen pass reads (`GXCopyTex` in `drawRefracAndSpec`).
struct FrameCopy {
    size: (i32, i32),
    texture: TextureId,
    pass: miniquad::RenderPass,
}

pub struct WaterView {
    drop: TextureId,
    splash: Texture2D,
    splashes: Vec<Splash>,
    accum: Pipeline,
    composite: Pipeline,
    drops: miniquad::BufferId,
    drop_indices: miniquad::BufferId,
    screen: miniquad::BufferId,
    screen_indices: miniquad::BufferId,
    scratch: Vec<f32>,
    copy: Option<FrameCopy>,
    /// The window's framebuffer, asked for once: `glGetIntegerv` waits for the GPU.
    window_fb: Option<u32>,
}

const ACCUM_VERTEX: &str = "#version 100
attribute vec3 pos;
attribute vec2 uv;
varying lowp vec2 tc;
uniform mat4 mvp;
void main() {
    gl_Position = mvp * vec4(pos, 1);
    tc = uv;
}";

const ACCUM_FRAGMENT: &str = "#version 100
precision mediump float;
varying lowp vec2 tc;
uniform sampler2D Mask;
void main() {
    gl_FragColor = vec4(0.0, 0.0, 0.0, texture2D(Mask, tc).a);
}";

const COMPOSITE_VERTEX: &str = "#version 100
attribute vec2 pos;
varying vec2 uv;
void main() {
    gl_Position = vec4(pos, 0, 1);
    uv = pos * 0.5 + 0.5;
}";

// Coverage above the threshold is water. Its slope gives a normal across the stream, which
// bends the scene seen through it; the edges brighten and a light from the upper left catches
// the top.
const COMPOSITE_FRAGMENT: &str = "#version 100
precision mediump float;
varying vec2 uv;
uniform sampler2D Frame;
uniform vec2 texel;
uniform vec4 tint;
void main() {
    float c = texture2D(Frame, uv).a;
    float m = smoothstep(0.30, 0.48, c);
    if (m <= 0.0) discard;
    vec2 dx = vec2(texel.x * 4.0, 0.0);
    vec2 dy = vec2(0.0, texel.y * 4.0);
    float gx = texture2D(Frame, uv + dx).a - texture2D(Frame, uv - dx).a;
    float gy = texture2D(Frame, uv + dy).a - texture2D(Frame, uv - dy).a;
    vec3 n = normalize(vec3(-gx, -gy, 0.35));
    float rim = 1.0 - n.z;
    float spec = pow(max(dot(n, normalize(vec3(-0.45, 0.55, 0.7))), 0.0), 28.0);
    vec3 behind = texture2D(Frame, uv - n.xy * texel * 24.0).rgb;
    vec3 water = mix(behind, tint.rgb, tint.a);
    vec3 col = mix(water, vec3(1.0), clamp(spec * 1.4 + rim * 0.25, 0.0, 1.0));
    gl_FragColor = vec4(col, m * clamp(0.75 + rim * 0.25 + spec, 0.0, 1.0));
}";

fn ctx() -> &'static mut dyn RenderingBackend {
    // SAFETY: the viewer only renders from the main thread.
    unsafe { macroquad::window::get_internal_gl() }.quad_context
}

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
        let ctx = ctx();
        // The droplet's coverage: `watermask`'s soft round falloff.
        let mask = bti(dir, "timg/watermask.bti")?;
        let rgba: Vec<u8> = mask
            .image
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|m| [255, 255, 255, m[0]])
            .collect();
        let drop =
            ctx.new_texture_from_rgba8(mask.image.width as u16, mask.image.height as u16, &rgba);
        ctx.texture_set_wrap(drop, TextureWrap::Clamp, TextureWrap::Clamp);
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

        let shader = |vertex, fragment, images: &[&str], uniforms| {
            self::ctx()
                .new_shader(
                    miniquad::ShaderSource::Glsl { vertex, fragment },
                    ShaderMeta {
                        images: images.iter().map(|s| s.to_string()).collect(),
                        uniforms: UniformBlockLayout { uniforms },
                    },
                )
                .map_err(|e| eprintln!("water shader: {e:?}"))
                .ok()
        };
        let accum_shader = shader(
            ACCUM_VERTEX,
            ACCUM_FRAGMENT,
            &["Mask"],
            vec![UniformDesc::new("mvp", UniformType::Mat4)],
        )?;
        let composite_shader = shader(
            COMPOSITE_VERTEX,
            COMPOSITE_FRAGMENT,
            &["Frame"],
            vec![
                UniformDesc::new("texel", UniformType::Float2),
                UniformDesc::new("tint", UniformType::Float4),
            ],
        )?;
        let add = BlendState::new(Equation::Add, BlendFactor::One, BlendFactor::One);
        let accum = ctx.new_pipeline(
            &[BufferLayout::default()],
            &[
                VertexAttribute::new("pos", VertexFormat::Float3),
                VertexAttribute::new("uv", VertexFormat::Float2),
            ],
            accum_shader,
            PipelineParams {
                // Behind the scene's depth, hidden. miniquad drops the test without depth
                // writes, so the pipeline "writes" and `draw` masks them.
                depth_test: Comparison::LessOrEqual,
                depth_write: true,
                color_blend: Some(add),
                alpha_blend: Some(add),
                // Only the alpha channel, like `drawSilhouette`'s `GXSetColorUpdate(false)`.
                color_write: (false, false, false, true),
                ..Default::default()
            },
        );
        let over = BlendState::new(
            Equation::Add,
            BlendFactor::Value(BlendValue::SourceAlpha),
            BlendFactor::OneMinusValue(BlendValue::SourceAlpha),
        );
        let composite = ctx.new_pipeline(
            &[BufferLayout::default()],
            &[VertexAttribute::new("pos", VertexFormat::Float2)],
            composite_shader,
            PipelineParams {
                color_blend: Some(over),
                alpha_blend: Some(over),
                ..Default::default()
            },
        );
        let drops = ctx.new_buffer(
            BufferType::VertexBuffer,
            BufferUsage::Stream,
            BufferSource::empty::<f32>(SLOTS * 4 * FLOATS),
        );
        let indices: Vec<u16> = (0..SLOTS as u16)
            .flat_map(|q| {
                let b = q * 4;
                [b, b + 1, b + 2, b, b + 2, b + 3]
            })
            .collect();
        let drop_indices = ctx.new_buffer(
            BufferType::IndexBuffer,
            BufferUsage::Immutable,
            BufferSource::slice(&indices),
        );
        let screen = ctx.new_buffer(
            BufferType::VertexBuffer,
            BufferUsage::Immutable,
            BufferSource::slice(&[-1.0f32, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, 1.0]),
        );
        let screen_indices = ctx.new_buffer(
            BufferType::IndexBuffer,
            BufferUsage::Immutable,
            BufferSource::slice(&[0u16, 1, 2, 0, 2, 3]),
        );
        Some(WaterView {
            drop,
            splash,
            splashes: Vec::new(),
            accum,
            composite,
            drops,
            drop_indices,
            screen,
            screen_indices,
            scratch: Vec::with_capacity(SLOTS * 4 * FLOATS),
            copy: None,
            window_fb: None,
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

    /// The frame copy at the window's size, remade when that changes.
    fn copy_target(&mut self, size: (i32, i32)) -> (TextureId, miniquad::RenderPass) {
        let ctx = ctx();
        if let Some(c) = &self.copy
            && c.size != size
        {
            ctx.delete_render_pass(c.pass);
            ctx.delete_texture(c.texture);
            self.copy = None;
        }
        let c = self.copy.get_or_insert_with(|| {
            let texture = ctx.new_render_texture(TextureParams {
                width: size.0 as u32,
                height: size.1 as u32,
                ..Default::default()
            });
            ctx.texture_set_wrap(texture, TextureWrap::Clamp, TextureWrap::Clamp);
            let pass = ctx.new_render_pass(texture, None);
            FrameCopy {
                size,
                texture,
                pass,
            }
        });
        (c.texture, c.pass)
    }

    /// Draws the water through `camera` (scene units) over the scene already in the window,
    /// tinted by `juice` (`unk5D5F`: 0 water, else Yoshi's juice colour).
    pub fn draw(&mut self, water: &WaterManager, camera: &Camera3D, juice: u8) {
        let (eye, target) = (camera.position, camera.target);
        let fwd = (target - eye).normalize_or_zero();
        let right = fwd.cross(Vec3::Y).normalize_or_zero();
        let up = right.cross(fwd);
        let scene = |p: libsms::Vec3| vec3(p.x, p.y, p.z) * S;

        self.scratch.clear();
        for p in water.particles.iter().take(SLOTS) {
            let c = scene(p.pos);
            let half = 1.414 * 0.5 * p.size * S;
            let at = |x: f32, y: f32| c + right * x + up * y;
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
                        [
                            at(nx + vx * STRETCH, ny + vy * STRETCH),
                            at(ny, -nx),
                            at(-nx - vx * STRETCH, -ny - vy * STRETCH),
                            at(-ny, nx),
                        ]
                    } else {
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
            for (v, uv) in corners
                .into_iter()
                .zip([(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)])
            {
                self.scratch.extend_from_slice(&[v.x, v.y, v.z, uv.0, uv.1]);
            }
        }
        let quads = self.scratch.len() / (4 * FLOATS);
        if quads > 0 {
            self.draw_drops(quads, camera, juice);
        }
        self.draw_splashes(right, up);
    }

    fn draw_drops(&mut self, quads: usize, camera: &Camera3D, juice: u8) {
        use miniquad::gl;
        // macroquad batches; put the scene in the window first.
        // SAFETY: main thread, between draws.
        unsafe { macroquad::window::get_internal_gl() }.flush();
        let (w, h) = miniquad::window::screen_size();
        let size = (w as i32, h as i32);
        let (copy, copy_pass) = self.copy_target(size);
        let ctx = ctx();
        ctx.buffer_update(self.drops, BufferSource::slice(&self.scratch));

        // Pass 1: coverage into the window's alpha, tested against the scene's depth
        // (`drawSilhouette`).
        ctx.begin_default_pass(PassAction::Nothing);
        ctx.apply_pipeline(&self.accum);
        let window_fb = *self.window_fb.get_or_insert_with(|| {
            let mut fb = 0;
            // SAFETY: GL call on the main thread with the context current.
            unsafe { gl::glGetIntegerv(gl::GL_FRAMEBUFFER_BINDING, &mut fb) };
            fb as u32
        });
        // SAFETY: GL calls on the main thread with the context current. The pipeline has set
        // the colour mask to alpha only, so the clear touches nothing else.
        unsafe {
            gl::glClearColor(0.0, 0.0, 0.0, 0.0);
            gl::glClear(gl::GL_COLOR_BUFFER_BIT);
            gl::glDepthMask(gl::GL_FALSE as _);
        }
        ctx.apply_bindings(&miniquad::Bindings {
            vertex_buffers: vec![self.drops],
            index_buffer: self.drop_indices,
            images: vec![self.drop],
        });
        ctx.apply_uniforms(UniformsSource::table(&camera.matrix()));
        ctx.draw(0, (quads * 6) as i32, 1);
        // SAFETY: as above.
        unsafe { gl::glDepthMask(gl::GL_TRUE as _) };
        ctx.end_render_pass();

        // Copy the frame, coverage and all.
        ctx.begin_pass(Some(copy_pass), PassAction::Nothing);
        // SAFETY: as above; the copy pass's framebuffer is bound for drawing.
        unsafe {
            gl::glBindFramebuffer(gl::GL_READ_FRAMEBUFFER, window_fb);
            gl::glBlitFramebuffer(
                0,
                0,
                size.0,
                size.1,
                0,
                0,
                size.0,
                size.1,
                gl::GL_COLOR_BUFFER_BIT,
                gl::GL_NEAREST,
            );
        }
        ctx.end_render_pass();

        // Pass 2: the water surface (`drawRefracAndSpec`).
        ctx.begin_default_pass(PassAction::Nothing);
        ctx.apply_pipeline(&self.composite);
        ctx.apply_bindings(&miniquad::Bindings {
            vertex_buffers: vec![self.screen],
            index_buffer: self.screen_indices,
            images: vec![copy],
        });
        // `gModelWaterManagerWaterColor`: water barely tints what's behind it; juice is
        // coloured through.
        let tint = match juice {
            1..=3 => {
                let c = libsms::yoshi::JUICE_COLORS[juice as usize];
                [
                    c[0] as f32 / 255.0,
                    c[1] as f32 / 255.0,
                    c[2] as f32 / 255.0,
                    0.75,
                ]
            }
            _ => [0.60, 0.76, 0.94, 0.35],
        };
        // miniquad packs uniforms back to back: `texel` then `tint`.
        ctx.apply_uniforms(UniformsSource::table(&[
            1.0 / w,
            1.0 / h,
            tint[0],
            tint[1],
            tint[2],
            tint[3],
        ]));
        ctx.draw(0, 6, 1);
        // The window's alpha back to opaque (screenshots keep it). The composite pipeline
        // writes every channel, which is what miniquad's cached mask says afterwards.
        // SAFETY: as above.
        unsafe {
            gl::glColorMask(gl::GL_FALSE as _, gl::GL_FALSE as _, gl::GL_FALSE as _, 1);
            gl::glClearColor(0.0, 0.0, 0.0, 1.0);
            gl::glClear(gl::GL_COLOR_BUFFER_BIT);
            gl::glColorMask(1, 1, 1, 1);
        }
        ctx.end_render_pass();
    }

    fn draw_splashes(&self, right: Vec3, up: Vec3) {
        let mut splashes = Mesh {
            vertices: Vec::new(),
            indices: Vec::new(),
            texture: Some(self.splash.clone()),
        };
        for s in &self.splashes {
            let c = vec3(s.pos.x, s.pos.y, s.pos.z) * S;
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
            for (v, uv) in corners
                .into_iter()
                .zip([(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)])
            {
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
