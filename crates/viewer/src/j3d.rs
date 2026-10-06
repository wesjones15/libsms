//! Draws J3D models the way the GameCube would: a TEV interpreter in GLSL fed by CPU skinning and
//! per-vertex lighting, on raw miniquad pipelines (macroquad's mesh vertex can't carry the three
//! texcoord sets and two colour channels Mario's materials use).
//!
//! GX semantics: colour channels follow `GXSetChanCtrl` (material/ambient source, light mask,
//! diffuse and attenuation functions; J3DColorChan); TEV stages are `GXSetTevColorIn/Op` incl.
//! the compare ops, konst selections and swap tables (J3DTevBlock::load, J3DMaterial.cpp);
//! texgens are `GXSetTexCoordGen` with the material's texture matrices (`J3DTexMtx::calc`,
//! J3DTevs.cpp), and `GX_TG_SRTG` reads a colour channel's red and green; the alpha test is
//! `GXSetAlphaCompare`.

use std::collections::HashMap;

use glam::{Mat3, Mat4, Vec3};
use libsms_assets::bmd::{Blend, ColorChan, Cull, Material, TevCombiner};
use libsms_assets::texture::Wrap;
use libsms_assets::{Bmd, Pose};
use macroquad::miniquad::{
    self, BlendFactor, BlendState, BlendValue, BufferId, BufferLayout, BufferSource, BufferType,
    BufferUsage, Comparison, CullFace, Equation, FrontFaceOrder, PassAction, Pipeline,
    PipelineParams, RenderingBackend, ShaderMeta, ShaderSource, TextureId, TextureWrap,
    UniformBlockLayout, UniformDesc, UniformType, UniformsSource, VertexAttribute, VertexFormat,
};

pub const MAX_STAGES: usize = 8;
const FLOATS: usize = 20;

const VERTEX: &str = r#"#version 100
attribute vec3 pos;
attribute vec3 nrm;
attribute vec4 uv01;
attribute vec2 uv2;
attribute vec4 ras0;
attribute vec4 ras1;
varying highp vec3 v_nrm;
varying highp vec4 v_uv01;
varying highp vec2 v_uv2;
varying lowp vec4 v_ras0;
varying lowp vec4 v_ras1;
uniform mat4 Mvp;
void main() {
    gl_Position = Mvp * vec4(pos, 1.0);
    v_nrm = nrm;
    v_uv01 = uv01;
    v_uv2 = uv2;
    v_ras0 = ras0;
    v_ras1 = ras1;
}"#;

const FRAGMENT: &str = r#"#version 100
precision highp float;
varying highp vec3 v_nrm;
varying highp vec4 v_uv01;
varying highp vec2 v_uv2;
varying lowp vec4 v_ras0;
varying lowp vec4 v_ras1;

uniform sampler2D Tex0;
uniform sampler2D Tex1;
uniform sampler2D Tex2;
uniform sampler2D Tex3;
uniform vec4 StageColor[8];
uniform vec4 StageColorOp[8];
uniform vec4 StageAlpha[8];
uniform vec4 StageAlphaOp[8];
uniform vec4 StageOrder[8];
uniform vec4 StageMisc[8];
uniform vec4 GenA[4];
uniform vec4 GenB[4];
uniform vec4 C0;
uniform vec4 C1;
uniform vec4 C2;
uniform vec4 K0;
uniform vec4 K1;
uniform vec4 K2;
uniform vec4 K3;
uniform vec4 Swap0;
uniform vec4 Swap1;
uniform vec4 Swap2;
uniform vec4 Swap3;
uniform vec4 Info;
uniform vec4 AlphaTest;

vec4 konst(float sel) {
    if (sel < 7.5) { return vec4(1.0 - sel / 8.0); }
    vec4 k = K0;
    float idx = mod(sel - 12.0, 4.0);
    if (idx > 2.5) { k = K3; } else if (idx > 1.5) { k = K2; } else if (idx > 0.5) { k = K1; }
    if (sel < 15.5) { return k; }
    if (sel < 19.5) { return vec4(k.r); }
    if (sel < 23.5) { return vec4(k.g); }
    if (sel < 27.5) { return vec4(k.b); }
    return vec4(k.a);
}

float pick(vec4 c, float i) {
    if (i < 0.5) { return c.r; }
    if (i < 1.5) { return c.g; }
    if (i < 2.5) { return c.b; }
    return c.a;
}

vec4 swizzle(vec4 c, float table) {
    vec4 s = Swap0;
    if (table > 2.5) { s = Swap3; } else if (table > 1.5) { s = Swap2; } else if (table > 0.5) { s = Swap1; }
    return vec4(pick(c, s.x), pick(c, s.y), pick(c, s.z), pick(c, s.w));
}

vec3 cin(float a, vec4 p, vec4 r0, vec4 r1, vec4 r2, vec4 t, vec4 ras, vec4 k) {
    if (a < 0.5) { return p.rgb; }
    if (a < 1.5) { return vec3(p.a); }
    if (a < 2.5) { return r0.rgb; }
    if (a < 3.5) { return vec3(r0.a); }
    if (a < 4.5) { return r1.rgb; }
    if (a < 5.5) { return vec3(r1.a); }
    if (a < 6.5) { return r2.rgb; }
    if (a < 7.5) { return vec3(r2.a); }
    if (a < 8.5) { return t.rgb; }
    if (a < 9.5) { return vec3(t.a); }
    if (a < 10.5) { return ras.rgb; }
    if (a < 11.5) { return vec3(ras.a); }
    if (a < 12.5) { return vec3(1.0); }
    if (a < 13.5) { return vec3(0.5); }
    if (a < 14.5) { return k.rgb; }
    return vec3(0.0);
}

float ain(float a, vec4 p, vec4 r0, vec4 r1, vec4 r2, vec4 t, vec4 ras, vec4 k) {
    if (a < 0.5) { return p.a; }
    if (a < 1.5) { return r0.a; }
    if (a < 2.5) { return r1.a; }
    if (a < 3.5) { return r2.a; }
    if (a < 4.5) { return t.a; }
    if (a < 5.5) { return ras.a; }
    if (a < 6.5) { return k.a; }
    return 0.0;
}

float bias(float b) { return b < 0.5 ? 0.0 : (b < 1.5 ? 0.5 : (b < 2.5 ? -0.5 : 0.0)); }
float scl(float s) { return s < 0.5 ? 1.0 : (s < 1.5 ? 2.0 : (s < 2.5 ? 4.0 : 0.5)); }

// Packed 8-bit channel values for the compare ops.
float b8(float v) { return floor(clamp(v, 0.0, 1.0) * 255.0 + 0.5); }
float gr16(vec3 v) { return b8(v.g) * 256.0 + b8(v.r); }
float bgr24(vec3 v) { return b8(v.b) * 65536.0 + b8(v.g) * 256.0 + b8(v.r); }

vec3 combine_color(vec3 a, vec3 b, vec3 c, vec3 d, vec4 op) {
    if (op.x < 1.5) {
        vec3 l = mix(a, b, c);
        vec3 r = (d + (op.x > 0.5 ? -l : l) + bias(op.y)) * scl(op.z);
        return op.w > 0.5 ? clamp(r, 0.0, 1.0) : clamp(r, -4.0, 4.0);
    }
    // GX_TEV_COMP_*: d + (a OP b ? c : 0).
    bool hit = false;
    if (op.x < 8.5) { hit = b8(a.r) > b8(b.r); }
    else if (op.x < 9.5) { hit = b8(a.r) == b8(b.r); }
    else if (op.x < 10.5) { hit = gr16(a) > gr16(b); }
    else if (op.x < 11.5) { hit = gr16(a) == gr16(b); }
    else if (op.x < 12.5) { hit = bgr24(a) > bgr24(b); }
    else if (op.x < 13.5) { hit = bgr24(a) == bgr24(b); }
    else {
        vec3 m;
        if (op.x < 14.5) {
            m = vec3(b8(a.r) > b8(b.r) ? 1.0 : 0.0, b8(a.g) > b8(b.g) ? 1.0 : 0.0, b8(a.b) > b8(b.b) ? 1.0 : 0.0);
        } else {
            m = vec3(b8(a.r) == b8(b.r) ? 1.0 : 0.0, b8(a.g) == b8(b.g) ? 1.0 : 0.0, b8(a.b) == b8(b.b) ? 1.0 : 0.0);
        }
        return clamp(d + c * m, 0.0, 1.0);
    }
    return clamp(d + (hit ? c : vec3(0.0)), 0.0, 1.0);
}

float combine_alpha(float a, float b, float c, float d, vec4 op) {
    if (op.x < 1.5) {
        float l = mix(a, b, c);
        float r = (d + (op.x > 0.5 ? -l : l) + bias(op.y)) * scl(op.z);
        return op.w > 0.5 ? clamp(r, 0.0, 1.0) : clamp(r, -4.0, 4.0);
    }
    bool hit = (mod(op.x, 2.0) < 0.5) ? b8(a) > b8(b) : b8(a) == b8(b);
    return clamp(d + (hit ? c : 0.0), 0.0, 1.0);
}

vec2 texgen(vec4 a, vec4 b) {
    // a.x: source (0..2 uv sets, 3 normal, 4 colour 0 r/g, 5 colour 1 r/g)
    vec2 src = v_uv01.xy;
    if (a.x > 4.5) { return v_ras1.rg; }
    if (a.x > 3.5) { return v_ras0.rg; }
    if (a.x > 2.5) { vec3 n = normalize(v_nrm); src = vec2(n.x, n.y); }
    else if (a.x > 1.5) { src = v_uv2; }
    else if (a.x > 0.5) { src = v_uv01.zw; }
    if (b.w < 0.5) { return src; }
    return vec2(a.y * src.x + a.z * src.y + a.w, b.x * src.x + b.y * src.y + b.z);
}

bool alpha_cmp(float comp, float a, float ref) {
    if (comp < 0.5) { return false; }
    if (comp < 1.5) { return a < ref; }
    if (comp < 2.5) { return a == ref; }
    if (comp < 3.5) { return a <= ref; }
    if (comp < 4.5) { return a > ref; }
    if (comp < 5.5) { return a != ref; }
    if (comp < 6.5) { return a >= ref; }
    return true;
}

void main() {
    vec2 tc0 = texgen(GenA[0], GenB[0]);
    vec2 tc1 = texgen(GenA[1], GenB[1]);
    vec2 tc2 = texgen(GenA[2], GenB[2]);
    vec2 tc3 = texgen(GenA[3], GenB[3]);

    vec4 prev = vec4(0.0);
    vec4 r0 = C0;
    vec4 r1 = C1;
    vec4 r2 = C2;
    for (int s = 0; s < 8; s++) {
        if (float(s) >= Info.x) { break; }
        vec4 order = StageOrder[s];
        vec2 tc = tc0;
        if (order.y > 2.5 && order.y < 3.5) { tc = tc3; }
        else if (order.y > 1.5 && order.y < 2.5) { tc = tc2; }
        else if (order.y > 0.5 && order.y < 1.5) { tc = tc1; }
        vec4 t = vec4(1.0);
        if (order.x < 0.5) { t = texture2D(Tex0, tc); }
        else if (order.x < 1.5) { t = texture2D(Tex1, tc); }
        else if (order.x < 2.5) { t = texture2D(Tex2, tc); }
        else if (order.x < 3.5) { t = texture2D(Tex3, tc); }
        vec4 ras = vec4(0.0);
        if (order.z > 3.5 && order.z < 4.5) { ras = v_ras0; }
        else if (order.z > 4.5 && order.z < 5.5) { ras = v_ras1; }
        else if (order.z < 0.5) { ras = vec4(v_ras0.rgb, 0.0); }
        else if (order.z < 1.5) { ras = vec4(v_ras1.rgb, 0.0); }
        vec4 misc = StageMisc[s];
        // misc.w: ras swap table * 4 + tex swap table
        t = swizzle(t, mod(misc.w, 4.0));
        ras = swizzle(ras, floor(misc.w / 4.0));
        vec4 kc = konst(misc.y);
        vec4 ka = konst(misc.z);

        vec4 ca = StageColor[s];
        vec3 a = clamp(cin(ca.x, prev, r0, r1, r2, t, ras, kc), 0.0, 1.0);
        vec3 b = clamp(cin(ca.y, prev, r0, r1, r2, t, ras, kc), 0.0, 1.0);
        vec3 c = clamp(cin(ca.z, prev, r0, r1, r2, t, ras, kc), 0.0, 1.0);
        vec3 d = cin(ca.w, prev, r0, r1, r2, t, ras, kc);
        vec3 rc = combine_color(a, b, c, d, StageColorOp[s]);

        vec4 aa = StageAlpha[s];
        float a1 = clamp(ain(aa.x, prev, r0, r1, r2, t, ras, ka), 0.0, 1.0);
        float b1 = clamp(ain(aa.y, prev, r0, r1, r2, t, ras, ka), 0.0, 1.0);
        float c1 = clamp(ain(aa.z, prev, r0, r1, r2, t, ras, ka), 0.0, 1.0);
        float d1 = ain(aa.w, prev, r0, r1, r2, t, ras, ka);
        float ra = combine_alpha(a1, b1, c1, d1, StageAlphaOp[s]);

        if (order.w < 0.5) { prev.rgb = rc; } else if (order.w < 1.5) { r0.rgb = rc; }
        else if (order.w < 2.5) { r1.rgb = rc; } else { r2.rgb = rc; }
        if (misc.x < 0.5) { prev.a = ra; } else if (misc.x < 1.5) { r0.a = ra; }
        else if (misc.x < 2.5) { r1.a = ra; } else { r2.a = ra; }
    }
    vec4 outc = clamp(prev, 0.0, 1.0);
    // GXSetAlphaCompare(comp0, ref0, op, comp1, ref1) on the 8-bit alpha.
    float a8 = b8(outc.a);
    bool p0 = alpha_cmp(AlphaTest.x, a8, AlphaTest.y);
    bool p1 = alpha_cmp(AlphaTest.w, a8, Info.w);
    bool pass = AlphaTest.z < 0.5 ? (p0 && p1) : (AlphaTest.z < 1.5 ? (p0 || p1) : (AlphaTest.z < 2.5 ? (p0 != p1) : (p0 == p1)));
    if (!pass) { discard; }
    gl_FragColor = vec4(outc.rgb * Info.y, outc.a * Info.z);
}"#;

#[repr(C)]
struct Uniforms {
    mvp: [f32; 16],
    stage_color: [[f32; 4]; MAX_STAGES],
    stage_color_op: [[f32; 4]; MAX_STAGES],
    stage_alpha: [[f32; 4]; MAX_STAGES],
    stage_alpha_op: [[f32; 4]; MAX_STAGES],
    stage_order: [[f32; 4]; MAX_STAGES],
    stage_misc: [[f32; 4]; MAX_STAGES],
    gen_a: [[f32; 4]; 4],
    gen_b: [[f32; 4]; 4],
    regs: [[f32; 4]; 3],
    konst: [[f32; 4]; 4],
    swap: [[f32; 4]; 4],
    /// stages, brightness, alpha, ref1
    info: [f32; 4],
    /// comp0, ref0, op, comp1
    alpha_test: [f32; 4],
}

fn uniform_layout() -> UniformBlockLayout {
    let mut u = vec![UniformDesc::new("Mvp", UniformType::Mat4)];
    for name in [
        "StageColor",
        "StageColorOp",
        "StageAlpha",
        "StageAlphaOp",
        "StageOrder",
        "StageMisc",
    ] {
        u.push(UniformDesc::new(name, UniformType::Float4).array(MAX_STAGES));
    }
    u.push(UniformDesc::new("GenA", UniformType::Float4).array(4));
    u.push(UniformDesc::new("GenB", UniformType::Float4).array(4));
    for name in [
        "C0",
        "C1",
        "C2",
        "K0",
        "K1",
        "K2",
        "K3",
        "Swap0",
        "Swap1",
        "Swap2",
        "Swap3",
        "Info",
        "AlphaTest",
    ] {
        u.push(UniformDesc::new(name, UniformType::Float4));
    }
    UniformBlockLayout { uniforms: u }
}

fn comb(c: &TevCombiner) -> ([f32; 4], [f32; 4]) {
    (
        [c.a as f32, c.b as f32, c.c as f32, c.d as f32],
        [
            c.op as f32,
            c.bias as f32,
            c.scale as f32,
            c.clamp as u8 as f32,
        ],
    )
}

/// A directional light in view space.
#[derive(Clone, Copy)]
pub struct Light {
    pub dir: Vec3,
    pub color: Vec3,
}

/// Lights 0 and 1 light colour channel 0 (Mario's materials use mask 3); light 2 is the
/// specular light of channel 1 (mask 4, `GX_AF_SPEC`).
pub struct Lights {
    pub lights: [Light; 3],
    pub shininess: f32,
}

impl Lights {
    /// A key light over the camera's shoulder, a dim fill from the other side, and the key as
    /// the specular light. The game sets its lights per scene; this is a neutral daylight.
    pub fn daylight(view: Mat4) -> Lights {
        let to_view = |d: Vec3| view.transform_vector3(d.normalize()).normalize();
        let key = to_view(Vec3::new(0.4, 1.0, 0.6));
        Lights {
            lights: [
                Light {
                    dir: key,
                    color: Vec3::splat(0.75),
                },
                Light {
                    dir: to_view(Vec3::new(-0.6, 0.3, -0.5)),
                    color: Vec3::splat(0.25),
                },
                Light {
                    dir: key,
                    color: Vec3::splat(1.0),
                },
            ],
            shininess: 24.0,
        }
    }
}

/// `GXSetChanCtrl` evaluated for one vertex: material source times clamped (ambient + lights).
fn channel(
    ch: &ColorChan,
    mat: [f32; 4],
    amb: [f32; 4],
    vtx: [f32; 4],
    n: Vec3,
    lights: &Lights,
    alpha: bool,
) -> [f32; 4] {
    let m = if ch.mat_from_vertex { vtx } else { mat };
    if !ch.lighting {
        return m;
    }
    let a = if ch.amb_from_vertex { vtx } else { amb };
    let mut sum = [a[0], a[1], a[2], a[3]];
    for (i, l) in lights.lights.iter().enumerate() {
        if ch.light_mask & (1 << i) == 0 {
            continue;
        }
        let ndl = n.dot(l.dir);
        let diffuse = match ch.diffuse_fn {
            0 => 1.0,
            1 => ndl,
            _ => ndl.max(0.0),
        };
        let atten = match ch.attn_fn {
            // `GX_AF_SPEC`: falls off with the half-angle; the eye looks down -Z in view space.
            0 => {
                if ndl <= 0.0 {
                    0.0
                } else {
                    let h = (l.dir + Vec3::Z).normalize();
                    n.dot(h).max(0.0).powf(lights.shininess)
                }
            }
            _ => 1.0,
        };
        let k = diffuse * atten;
        if alpha {
            sum[3] += k;
        } else {
            sum[0] += l.color.x * k;
            sum[1] += l.color.y * k;
            sum[2] += l.color.z * k;
        }
    }
    std::array::from_fn(|i| m[i] * sum[i].clamp(0.0, 1.0))
}

fn unit(c: [u8; 4]) -> [f32; 4] {
    c.map(|v| v as f32 / 255.0)
}

struct GpuBatch {
    material: usize,
    shape: usize,
    src: Vec<libsms_assets::BatchVertex>,
    vbuf: BufferId,
    ibuf: BufferId,
    count: i32,
    scratch: Vec<f32>,
}

pub struct GpuModel {
    pub bmd: Bmd,
    batches: Vec<GpuBatch>,
    pub textures: Vec<TextureId>,
}

/// Per-draw changes to a model's materials.
#[derive(Default, Clone)]
pub struct Look {
    /// Shapes not to draw (`J3DShpFlag_Visible` set).
    pub hidden: Vec<usize>,
    /// (material, texture map, texture) from texture pattern animations.
    pub tex_no: Vec<(usize, u8, u16)>,
    /// Replaces the model's own texture 0 (the caps and hands use the body's, `TMarioCap`).
    pub texture0: Option<TextureId>,
    /// Konst colour 0 alpha: Mario's dirt level (`TMario::addDirty`).
    pub k0_alpha: Option<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PipeKey {
    blend: u8,
    cull: u8,
    z_write: bool,
}

pub struct Renderer {
    shader: miniquad::ShaderId,
    pipes: HashMap<PipeKey, Pipeline>,
    white: TextureId,
}

/// Which of a frame's two passes a draw belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pass {
    /// Material modes 1 and 2 (`J3DDrawBuffer` 0).
    Opaque,
    /// Mode 4: translucent, drawn after everything opaque.
    Translucent,
}

fn ctx() -> &'static mut dyn RenderingBackend {
    // SAFETY: the viewer only renders from the main thread.
    unsafe { macroquad::window::get_internal_gl() }.quad_context
}

impl Renderer {
    pub fn new() -> Renderer {
        let ctx = ctx();
        let shader = ctx
            .new_shader(
                ShaderSource::Glsl {
                    vertex: VERTEX,
                    fragment: FRAGMENT,
                },
                ShaderMeta {
                    images: vec!["Tex0".into(), "Tex1".into(), "Tex2".into(), "Tex3".into()],
                    uniforms: uniform_layout(),
                },
            )
            .expect("TEV shader compiles");
        let white = ctx.new_texture_from_rgba8(1, 1, &[255; 4]);
        Renderer {
            shader,
            pipes: HashMap::new(),
            white,
        }
    }

    fn pipeline(&mut self, key: PipeKey) -> Pipeline {
        let shader = self.shader;
        *self.pipes.entry(key).or_insert_with(|| {
            let blend = match key.blend {
                1 => Some(BlendState::new(
                    Equation::Add,
                    BlendFactor::Value(BlendValue::SourceAlpha),
                    BlendFactor::OneMinusValue(BlendValue::SourceAlpha),
                )),
                2 => Some(BlendState::new(
                    Equation::Add,
                    BlendFactor::Value(BlendValue::SourceAlpha),
                    BlendFactor::One,
                )),
                _ => None,
            };
            ctx().new_pipeline(
                &[BufferLayout::default()],
                &[
                    VertexAttribute::new("pos", VertexFormat::Float3),
                    VertexAttribute::new("nrm", VertexFormat::Float3),
                    VertexAttribute::new("uv01", VertexFormat::Float4),
                    VertexAttribute::new("uv2", VertexFormat::Float2),
                    VertexAttribute::new("ras0", VertexFormat::Float4),
                    VertexAttribute::new("ras1", VertexFormat::Float4),
                ],
                shader,
                PipelineParams {
                    depth_test: Comparison::LessOrEqual,
                    // miniquad turns the depth *test* off for pipelines that don't write
                    // depth, so every pipeline "writes" and `draw` masks writes with GL.
                    depth_write: true,
                    color_blend: blend,
                    // Keep the framebuffer's alpha; miniquad wants this only with blending on.
                    alpha_blend: blend.map(|_| {
                        BlendState::new(Equation::Add, BlendFactor::Zero, BlendFactor::One)
                    }),
                    // GX faces are front when clockwise on screen.
                    front_face_order: FrontFaceOrder::Clockwise,
                    cull_face: match key.cull {
                        1 => CullFace::Front,
                        2 => CullFace::Back,
                        _ => CullFace::Nothing,
                    },
                    ..Default::default()
                },
            )
        })
    }

    /// Uploads a model's textures and allocates buffers for its batches.
    pub fn load(&mut self, bmd: Bmd) -> Result<GpuModel, libsms_assets::Error> {
        let ctx = ctx();
        let textures = bmd
            .textures
            .iter()
            .map(|t| {
                let id = ctx.new_texture_from_rgba8(
                    t.image.width as u16,
                    t.image.height as u16,
                    &t.image.rgba,
                );
                let wrap = |w: Wrap| match w {
                    Wrap::Clamp => TextureWrap::Clamp,
                    Wrap::Repeat => TextureWrap::Repeat,
                    Wrap::Mirror => TextureWrap::Mirror,
                };
                ctx.texture_set_wrap(id, wrap(t.wrap_s), wrap(t.wrap_t));
                id
            })
            .collect();
        let batches = bmd
            .batches()?
            .into_iter()
            .filter(|b| !b.indices.is_empty() && b.vertices.len() < u16::MAX as usize)
            .map(|b| {
                let indices: Vec<u16> = b.indices.iter().map(|&i| i as u16).collect();
                let vbuf = ctx.new_buffer(
                    BufferType::VertexBuffer,
                    BufferUsage::Stream,
                    BufferSource::empty::<f32>(b.vertices.len() * FLOATS),
                );
                let ibuf = ctx.new_buffer(
                    BufferType::IndexBuffer,
                    BufferUsage::Immutable,
                    BufferSource::slice(&indices),
                );
                GpuBatch {
                    material: b.material,
                    shape: b.shape,
                    scratch: vec![0.0; b.vertices.len() * FLOATS],
                    src: b.vertices,
                    vbuf,
                    ibuf,
                    count: indices.len() as i32,
                }
            })
            .collect();
        Ok(GpuModel {
            bmd,
            batches,
            textures,
        })
    }

    /// Draws one pass of a posed model. `world_to_scene` maps model space (decomp units) to the
    /// scene; `view` and `proj` are the camera's.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        model: &mut GpuModel,
        pose: &Pose,
        view: Mat4,
        proj: Mat4,
        lights: &Lights,
        look: &Look,
        pass: Pass,
    ) {
        // Flush macroquad's batched draws so the depth buffer has the scene.
        // SAFETY: main thread, between draws.
        unsafe { macroquad::window::get_internal_gl() }.flush();
        let mvp = proj * view;
        let normal_to_view = Mat3::from_mat4(view);
        let ctx = ctx();
        ctx.begin_default_pass(PassAction::Nothing);
        for b in &mut model.batches {
            if look.hidden.contains(&b.shape) {
                continue;
            }
            let mat: &Material = &model.bmd.materials[b.material];
            let translucent = mat.mode & 4 != 0;
            if translucent != (pass == Pass::Translucent) {
                continue;
            }
            // Skin and light every vertex.
            let mat_c = mat.mat_colors.map(unit);
            let amb_c = mat.amb_colors.map(unit);
            for (v, out) in b.src.iter().zip(b.scratch.chunks_exact_mut(FLOATS)) {
                let (p, n) = pose.skin(v);
                let nv = (normal_to_view * n).normalize_or_zero();
                let vc = v.colors.map(unit);
                let c0 = channel(&mat.chans[0], mat_c[0], amb_c[0], vc[0], nv, lights, false);
                let a0 = channel(&mat.chans[1], mat_c[0], amb_c[0], vc[0], nv, lights, true);
                let (c1, a1) = if mat.chan_num > 1 {
                    (
                        channel(&mat.chans[2], mat_c[1], amb_c[1], vc[1], nv, lights, false),
                        channel(&mat.chans[3], mat_c[1], amb_c[1], vc[1], nv, lights, true),
                    )
                } else {
                    ([0.0; 4], [0.0; 4])
                };
                out.copy_from_slice(&[
                    p.x, p.y, p.z, nv.x, nv.y, nv.z, v.uv[0].x, v.uv[0].y, v.uv[1].x, v.uv[1].y,
                    v.uv[2].x, v.uv[2].y, c0[0], c0[1], c0[2], a0[3], c1[0], c1[1], c1[2], a1[3],
                ]);
            }
            ctx.buffer_update(b.vbuf, BufferSource::slice(&b.scratch));

            let key = PipeKey {
                blend: match mat.blend() {
                    Blend::Opaque | Blend::AlphaTest => 0,
                    Blend::AlphaBlend => 1,
                    Blend::Additive => 2,
                },
                cull: match mat.cull {
                    Cull::None => 0,
                    Cull::Front => 1,
                    Cull::Back => 2,
                    Cull::All => continue,
                },
                z_write: mat.z_write && !translucent,
            };
            let pipe = self.pipeline(key);
            ctx.apply_pipeline(&pipe);

            // Textures for maps 0..3, after pattern animations and overrides.
            let mut tex_no = mat.textures;
            for &(m, map, t) in &look.tex_no {
                if m == b.material && (map as usize) < 8 {
                    tex_no[map as usize] = Some(t);
                }
            }
            let images: Vec<TextureId> = (0..4)
                .map(|i| match tex_no[i] {
                    Some(0) if look.texture0.is_some() => look.texture0.unwrap(),
                    Some(t) => model
                        .textures
                        .get(t as usize)
                        .copied()
                        .unwrap_or(self.white),
                    None => self.white,
                })
                .collect();
            ctx.apply_bindings(&miniquad::Bindings {
                vertex_buffers: vec![b.vbuf],
                index_buffer: b.ibuf,
                images,
            });

            let mut u = Uniforms {
                mvp: mvp.to_cols_array(),
                stage_color: [[0.0; 4]; MAX_STAGES],
                stage_color_op: [[0.0; 4]; MAX_STAGES],
                stage_alpha: [[0.0; 4]; MAX_STAGES],
                stage_alpha_op: [[0.0; 4]; MAX_STAGES],
                stage_order: [[0.0; 4]; MAX_STAGES],
                stage_misc: [[0.0; 4]; MAX_STAGES],
                gen_a: [[0.0; 4]; 4],
                gen_b: [[0.0; 4]; 4],
                regs: mat.tev_colors.map(|c| c.map(|v| v as f32 / 255.0)),
                konst: mat.konst.map(unit),
                swap: mat.swap_tables.map(|s| s.map(|v| v as f32)),
                info: [
                    mat.stages.len().min(MAX_STAGES) as f32,
                    1.0,
                    1.0,
                    mat.alpha_compare.ref1 as f32,
                ],
                alpha_test: [
                    mat.alpha_compare.comp0 as f32,
                    mat.alpha_compare.ref0 as f32,
                    mat.alpha_compare.op as f32,
                    mat.alpha_compare.comp1 as f32,
                ],
            };
            if let Some(a) = look.k0_alpha {
                u.konst[0][3] = a as f32 / 255.0;
            }
            for (i, s) in mat.stages.iter().take(MAX_STAGES).enumerate() {
                (u.stage_color[i], u.stage_color_op[i]) = comb(&s.color);
                (u.stage_alpha[i], u.stage_alpha_op[i]) = comb(&s.alpha);
                let map = if s.tex_map == 0xFF {
                    0xFF
                } else {
                    s.tex_map & 7
                };
                u.stage_order[i] = [
                    if map < 4 { map as f32 } else { 255.0 },
                    s.tex_coord as f32,
                    s.channel as f32,
                    s.color.out as f32,
                ];
                u.stage_misc[i] = [
                    s.alpha.out as f32,
                    s.kcolor_sel as f32,
                    s.kalpha_sel as f32,
                    (s.ras_swap * 4 + s.tex_swap) as f32,
                ];
            }
            for (i, g) in mat.tex_gens.iter().take(4).enumerate() {
                let src = match (g.kind, g.src) {
                    (10, 20) => 5.0,
                    (10, _) => 4.0,
                    (_, 1) => 3.0,
                    (_, s @ 4..=6) => (s - 4) as f32,
                    _ => 0.0,
                };
                u.gen_a[i][0] = src;
                let slot = g.matrix.wrapping_sub(30) / 3;
                if g.kind <= 1
                    && g.matrix != 60
                    && let Some(Some(m)) = mat.tex_mtx.get(slot as usize)
                {
                    let r = if src == 3.0 {
                        // Environment maps: the view normal mapped to [0, 1] (`fixupMtx1`)
                        // under the SRT.
                        let s = m.srt_rows();
                        [
                            [
                                s[0][0] * 0.5,
                                s[0][1] * -0.5,
                                s[0][0] * 0.5 + s[0][1] * 0.5 + s[0][2],
                            ],
                            [
                                s[1][0] * 0.5,
                                s[1][1] * -0.5,
                                s[1][0] * 0.5 + s[1][1] * 0.5 + s[1][2],
                            ],
                        ]
                    } else {
                        m.srt_rows()
                    };
                    u.gen_a[i] = [src, r[0][0], r[0][1], r[0][2]];
                    u.gen_b[i] = [r[1][0], r[1][1], r[1][2], 1.0];
                }
            }
            ctx.apply_uniforms(UniformsSource::table(&u));
            // SAFETY: GL calls on the main thread with a context current.
            if !key.z_write {
                unsafe { miniquad::gl::glDepthMask(miniquad::gl::GL_FALSE as _) };
            }
            ctx.draw(0, b.count, 1);
            if !key.z_write {
                unsafe { miniquad::gl::glDepthMask(miniquad::gl::GL_TRUE as _) };
            }
        }
        ctx.end_render_pass();
    }
}
