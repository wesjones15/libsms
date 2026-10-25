//! Interactive statue/M gate entrance preview. X replays, F full statue, C close,
//! arrows orbit, Space pauses. --capture <folder> saves a deterministic 30 fps sequence.
//! Source: MarioAutodemo.cpp::TMario::warpOut, successful-return branch (arg & 255 == 2).
//! Original BCKs, Bianco return position, and zero-speed fall from MarioPhysics::jumpProcess.
#![allow(dead_code)]
#[path = "../src/j3d.rs"]
mod j3d;
#[path = "../src/mario_model.rs"]
mod mario_model;

use ::glam::{Mat4 as GMat, Vec3 as G3};
use libsms::{Assets, Sim, Status};
use libsms_assets::{Bmd, Pose, jpa::Jpa};
use libsms_present::{
    particles::Particles,
    portal::Portal,
    tev::{Inputs, Source, Tex},
};
use macroquad::miniquad::{
    BlendFactor, BlendState, BlendValue, Comparison, Equation, PipelineParams, ShaderSource,
};
use macroquad::prelude::*;
use std::path::{Path, PathBuf};

const VERTEX: &str = r#"#version 100
attribute vec3 position; attribute vec2 texcoord; attribute vec4 color0;
varying lowp vec2 uv; varying lowp vec4 color;
uniform mat4 Model; uniform mat4 Projection;
void main(){gl_Position=Projection*Model*vec4(position,1.);uv=texcoord;color=color0/255.;}"#;
const FRAGMENT: &str = r#"#version 100
varying lowp vec2 uv; varying lowp vec4 color; uniform sampler2D Texture;
void main(){gl_FragColor=texture2D(Texture,uv)*color;}"#;
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
const ACTIVATE: f32 = 1.0;
// TMario::warpOut's ordinary gate delay is 180 ticks at 120 Hz. The original
// appear clip scales the centre from ~zero at frame 59 to full at frame 89.
const LAUNCH: f32 = ACTIVATE + 180. / 120.;
// dolpic1 scene.bin MarioPositionObj's Bianco return point (0,550,5250),
// relative to the gate's placement (0,300,4941.47).
const RETURN: G3 = libsms_present::view::ENTRANCE_RETURN;

fn fall_at(seconds: f32, gravity: f32) -> (f32, f32) {
    libsms_present::view::entrance_fall(seconds, gravity)
}

fn config() -> Conf {
    Conf {
        window_title: "Sunshine statue portal - Rust viewer".into(),
        window_width: 1100,
        window_height: 800,
        ..Default::default()
    }
}
fn material(add: bool, depth_write: bool) -> Material {
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
                    if add {
                        BlendFactor::One
                    } else {
                        BlendFactor::OneMinusValue(BlendValue::SourceAlpha)
                    },
                )),
                depth_test: Comparison::LessOrEqual,
                depth_write,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap()
}
fn texture(image: &libsms_assets::Image) -> Texture2D {
    let t = Texture2D::from_rgba8(image.width as u16, image.height as u16, &image.rgba);
    t.set_filter(FilterMode::Linear);
    t
}
fn load_fx(dir: &Path) -> Particles {
    let mut fx = Particles::default();
    for name in WINDS {
        let jpa =
            Jpa::parse(&std::fs::read(dir.join(format!("portal/{name}.jpa"))).unwrap()).unwrap();
        println!(
            "{name}: start {} lifetime {}",
            jpa.emitter.start_frame, jpa.emitter.lifetime
        );
        fx.add(name, jpa);
    }
    let jpa = Jpa::parse(&std::fs::read(dir.join("particle/ms_m_gout_gate.jpa")).unwrap()).unwrap();
    println!(
        "ms_m_gout_gate: start {} lifetime {}",
        jpa.emitter.start_frame, jpa.emitter.lifetime
    );
    fx.add("ms_m_gout_gate", jpa);
    // EmitterViewObj.cpp::TMarioParticleManager::perform runs one JPA calc for
    // each SMSGetAnmFrameRate unit: two at 30 fps, matching the 60 Hz BCK clock.
    // The shared particle scheduler currently supplies 30 Hz; use its existing
    // playback-rate control for this source-timed entrance effect.
    fx.set_speed("ms_m_gout_gate", 2.);
    fx
}
fn vertex(p: G3, uv: [f32; 2], color: Color) -> Vertex {
    Vertex::new(p.x, p.y, p.z, uv[0], uv[1], color)
}
fn statue(dir: &Path) -> Vec<Mesh> {
    let bmd = Bmd::parse(&std::fs::read(dir.join("portal/plaza.bmd")).unwrap()).unwrap();
    let pose = Pose::new(&bmd, GMat::IDENTITY, &bmd.bind_world());
    let textures: Vec<_> = bmd
        .textures
        .iter()
        .map(|t| Some(Tex::new(t.image.clone(), t.wrap_s, t.wrap_t)))
        .collect();
    let channels: Vec<_> = bmd
        .materials
        .iter()
        .map(|m| [m.mat_colors[0].map(|v| v as f32 / 255.), [0.; 4]])
        .collect();
    let source = Source {
        bmd: &bmd,
        textures: &textures,
        channels: &channels,
    };
    libsms_present::plaza::statue_batches(&bmd)
        .unwrap()
        .into_iter()
        .map(|b| {
            let baked = source.bake(b.material, &Inputs::default(), 512).unwrap();
            let tex = texture(&libsms_assets::Image {
                width: baked.width,
                height: baked.height,
                rgba: baked.rgba,
            });
            // Same sampler setup as the existing J3D viewer renderer.
            unsafe { macroquad::window::get_internal_gl() }
                .quad_context
                .texture_set_wrap(
                    tex.raw_miniquad_id(),
                    macroquad::miniquad::TextureWrap::Repeat,
                    macroquad::miniquad::TextureWrap::Repeat,
                );
            let verts = b
                .vertices
                .iter()
                .map(|v| {
                    let (p, n) = pose.skin(v);
                    let p = (p - G3::new(0., 300., 4941.47)) * 0.01;
                    let lighting = 0.72
                        + 0.28
                            * n.normalize_or_zero()
                                .dot(G3::new(-0.3, 0.8, 0.5).normalize())
                                .max(0.);
                    let c = v.colors[0].map(|v| v as f32 / 255.);
                    vertex(
                        p,
                        [v.uv[0].x, v.uv[0].y],
                        Color::new(c[0] * lighting, c[1] * lighting, c[2] * lighting, 1.),
                    )
                })
                .collect();
            Mesh {
                vertices: verts,
                indices: b.indices.iter().map(|&i| i as u16).collect(),
                texture: Some(tex),
            }
        })
        .collect()
}

/// MarioAutodemo::warpOut waits at the stage return point before falling and rolling.
fn pose_at(time: f32, _appear_frames: f32, gravity: f32) -> (&'static str, f32, G3, bool) {
    let (clip, frame, position, _) = libsms_present::view::entrance_pose(time - ACTIVATE, gravity);
    (clip, frame, position, time >= ACTIVATE)
}

#[macroquad::main(config)]
async fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let dir = PathBuf::from(
        args.first()
            .filter(|s| !s.starts_with('-'))
            .cloned()
            .unwrap_or_else(|| "assets".into()),
    );
    let capture = args
        .iter()
        .position(|a| a == "--capture")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from);
    if let Some(out) = &capture {
        std::fs::create_dir_all(out).unwrap();
    }
    let assets = Assets::load(&dir);
    let skeleton = assets.skeleton.as_ref().expect("Mario rig");
    for name in CLIPS {
        println!(
            "{name}: {} frames",
            assets.clips.get(name).expect("gate clip").frames
        );
    }
    let mut sim = Sim::new(
        assets.params.clone(),
        assets.anim_table.clone(),
        G3::ZERO,
        0,
    );
    sim.mario.give_fludd();
    let mut mario = mario_model::MarioModel::load(&dir).expect("Mario models");
    let statue = statue(&dir);
    let mut portal = Portal::load(&dir).unwrap();
    portal.set_interior(&libsms_present::plaza::plaza_still(&dir, 256).unwrap());
    let b = portal.model.batches().unwrap().remove(0);
    println!(
        "Portal quad: {:?}",
        b.vertices
            .iter()
            .map(|v| (v.position, v.uv[0]))
            .collect::<Vec<_>>()
    );
    let tex = texture(&libsms_assets::Image {
        width: 256,
        height: 256,
        rgba: vec![0; 256 * 256 * 4],
    });
    // Present the baked image with the same row convention as the approved
    // headless capture. The original quad's top has T=0, not T=1.
    let mut gate = Mesh {
        vertices: b
            .vertices
            .iter()
            .map(|v| {
                vertex(
                    v.position * 0.01 + G3::new(0., 0., 0.035),
                    [v.uv[0].x, 1. - v.uv[0].y],
                    WHITE,
                )
            })
            .collect(),
        indices: b.indices.iter().map(|&i| i as u16).collect(),
        texture: Some(tex.clone()),
    };
    let opaque = material(false, true);
    let alpha = material(false, false);
    let add = material(true, false);
    let mut fx = load_fx(&dir);
    let mut fxtextures: Vec<(u32, Texture2D)> = Vec::new();
    let mut time = 0f32;
    let mut tick = 0u64;
    let mut frame = 0u32;
    let mut paused = false;
    let mut full = true;
    let mut auto_camera = true;
    let mut orbit = 0f32;
    let mut launched = false;
    loop {
        if is_key_pressed(KeyCode::Escape) {
            break;
        }
        if is_key_pressed(KeyCode::Space) {
            paused = !paused
        }
        if is_key_pressed(KeyCode::F) {
            full = true;
            auto_camera = false;
        }
        if is_key_pressed(KeyCode::C) {
            full = false;
            auto_camera = false;
        }
        if is_key_pressed(KeyCode::X) || is_key_pressed(KeyCode::R) {
            time = ACTIVATE;
            tick = (ACTIVATE * 120.) as u64;
            launched = false;
            fx = load_fx(&dir);
            fxtextures.clear();
        }
        let dt = if paused {
            0.
        } else if capture.is_some() {
            1. / 30.
        } else {
            get_frame_time().min(0.05)
        };
        if is_key_down(KeyCode::Left) {
            orbit -= dt * 0.8
        }
        if is_key_down(KeyCode::Right) {
            orbit += dt * 0.8
        }
        let camera_blend = if auto_camera {
            ((time - 0.9) / 0.7).clamp(0., 1.)
        } else if full {
            0.
        } else {
            1.
        };
        let eye = G3::new(10., 10., 23.).lerp(G3::new(5.5, 3.8, 11.5), camera_blend);
        let target = G3::new(0., 7.3, -2.5).lerp(G3::new(0., 1.8, 1.4), camera_blend);
        let eye = target + GMat::from_rotation_y(orbit).transform_vector3(eye - target);
        let camera = Camera3D {
            position: vec3(eye.x, eye.y, eye.z),
            target: vec3(target.x, target.y, target.z),
            up: vec3(0., 1., 0.),
            fovy: 45f32.to_radians(),
            z_near: 0.05,
            z_far: 200.,
            ..Default::default()
        };
        let view = GMat::look_at_rh(eye, target, G3::Y);
        let proj = GMat::perspective_rh_gl(
            45f32.to_radians(),
            screen_width() / screen_height(),
            0.05,
            200.,
        );
        let activation = ((time - ACTIVATE) / 0.6).clamp(0., 1.);
        let image = portal.image(time * 60., activation);
        tex.update(&macroquad::texture::Image {
            bytes: image.rgba,
            width: image.width as u16,
            height: image.height as u16,
        });
        gate.texture = Some(tex.clone());
        while (tick as f32) / 120. < time {
            let tick_time = tick as f32 / 120.;
            if tick_time >= ACTIVATE
                && tick_time < LAUNCH + fall_at(10., assets.params.jump.gravity).1
            {
                for (i, name) in WINDS.iter().enumerate() {
                    fx.request(
                        i as u32,
                        name,
                        GMat::from_translation(G3::new(0., 200., 0.)),
                    );
                }
            }
            // TMario::warpOut calls warpOutEffect when the appear clip starts,
            // before its delay/rolling phase. Trigger the burst at activation.
            if !launched && tick_time >= ACTIVATE {
                fx.emit("ms_m_gout_gate", GMat::from_translation(RETURN), 1.);
                launched = true;
            }
            fx.advance(tick);
            tick += 1;
        }
        let right = view.inverse().x_axis.truncate();
        let up = view.inverse().y_axis.truncate();
        fx.build(right, up);
        clear_background(Color::new(0.08, 0.11, 0.16, 1.));
        set_camera(&camera);
        draw_plane(
            vec3(0., -0.025, 0.),
            vec2(50., 50.),
            None,
            Color::new(0.19, 0.22, 0.25, 1.),
        );
        draw_grid(
            40,
            1.,
            Color::new(0.27, 0.30, 0.33, 1.),
            Color::new(0.22, 0.25, 0.28, 1.),
        );
        gl_use_material(&opaque);
        for mesh in &statue {
            draw_mesh(mesh);
        }
        gl_use_default_material();
        gl_use_material(&alpha);
        draw_mesh(&gate);
        gl_use_default_material();
        let (clip, clipframe, position, visible) = pose_at(
            time,
            assets.clips.get(CLIPS[0]).unwrap().frames,
            assets.params.jump.gravity,
        );
        if visible {
            sim.mario
                .portrait_pose(Status::Jump, clip, clipframe, 0, false);
            let joints = skeleton.pose(assets.clips.get(clip).unwrap(), clipframe);
            mario.draw(
                &sim,
                &joints,
                mario_model::root_matrix(position * 0.01, 0., 0.01),
                view,
                proj,
                time,
            );
        }
        for (i, t) in fx.textures.iter().enumerate() {
            if i >= fxtextures.len() {
                fxtextures.push((
                    0,
                    texture(&libsms_assets::Image {
                        width: t.width,
                        height: t.height,
                        rgba: t.rgba.clone(),
                    }),
                ));
            }
            if fxtextures[i].0 != t.version {
                fxtextures[i].1.update(&macroquad::texture::Image {
                    width: t.width as u16,
                    height: t.height as u16,
                    bytes: t.rgba.clone(),
                });
                fxtextures[i].0 = t.version;
            }
        }
        for batch in &fx.batches {
            let verts = &fx.vertices
                [batch.first_vertex as usize..(batch.first_vertex + batch.vertex_count) as usize];
            let mesh = Mesh {
                vertices: verts
                    .iter()
                    .map(|v| {
                        vertex(
                            v.pos * 0.01,
                            [v.u, v.v],
                            Color::new(v.color[0], v.color[1], v.color[2], v.color[3]),
                        )
                    })
                    .collect(),
                indices: (0..verts.len() as u16 / 4)
                    .flat_map(|q| [q * 4, q * 4 + 1, q * 4 + 2, q * 4, q * 4 + 2, q * 4 + 3])
                    .collect(),
                texture: Some(fxtextures[batch.texture as usize].1.clone()),
            };
            gl_use_material(if batch.blend == 0 { &add } else { &alpha });
            draw_mesh(&mesh);
        }
        gl_use_default_material();
        set_default_camera();
        draw_rectangle(
            0.,
            0.,
            screen_width(),
            72.,
            Color::new(0.04, 0.05, 0.08, 0.94),
        );
        draw_text(
            "SUNSHINE STATUE ENTRANCE | Rust viewer",
            20.,
            28.,
            24.,
            WHITE,
        );
        draw_text(
            "X replay   F full statue   C close   arrows orbit   Space pause   Esc close",
            20.,
            56.,
            19.,
            LIGHTGRAY,
        );
        draw_rectangle(
            0.,
            screen_height() - 62.,
            screen_width(),
            62.,
            Color::new(0.04, 0.05, 0.08, 0.94),
        );
        let label = if visible {
            format!("{clip}  frame {clipframe:.0} | time {time:.2}s")
        } else {
            format!("05_gate01 M portal | activation {activation:.0} | time {time:.2}s")
        };
        draw_text(&label, 16., screen_height() - 36., 19., WHITE);
        draw_text(
            "Source: warpOut | ms_m_gout_gate (0x40) | Bianco return point | original BCK reveal",
            16.,
            screen_height() - 13.,
            16.,
            LIGHTGRAY,
        );
        if let Some(out) = &capture {
            get_screen_data()
                .export_png(out.join(format!("frame_{frame:04}.png")).to_str().unwrap());
            if frame >= 191 {
                break;
            }
        }
        frame += 1;
        time += dt;
        if capture.is_none() && time > 7. {
            time = 0.;
            tick = 0;
            launched = false;
            fx = load_fx(&dir);
            fxtextures.clear();
        }
        next_frame().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_reveal_finishes_at_the_return_point_before_falling() {
        let Some(dir) = libsms::assets::find(None) else {
            eprintln!("SKIP: Sunshine assets absent");
            return;
        };
        let assets = Assets::load(&dir);
        let clip = assets
            .clips
            .get(CLIPS[0])
            .expect("successful gate appearance clip");
        assert_eq!(clip.frames, 89.);
        assert!(clip.sample(1, 59.).unwrap().scale.max_element() < 0.001);
        assert!(clip.sample(1, 89.).unwrap().scale.min_element() > 0.99);
        let gravity = assets.params.jump.gravity;
        let start = pose_at(ACTIVATE, clip.frames, gravity);
        let end = pose_at(LAUNCH - 0.001, clip.frames, gravity);
        assert_eq!(start.0, CLIPS[0]);
        assert_eq!(end.0, CLIPS[0]);
        assert_eq!(
            start.2, end.2,
            "materialization must finish at the airborne return point"
        );
        assert_eq!(start.2, RETURN);
        assert_eq!(pose_at(LAUNCH + 0.05, clip.frames, gravity).0, CLIPS[1]);
        let landed = pose_at(LAUNCH + 1., clip.frames, gravity);
        assert_eq!(landed.0, CLIPS[2]);
        assert_eq!(landed.2, G3::new(RETURN.x, 0., RETURN.z));
        for name in CLIPS {
            assert!(assets.clips.get(name).is_some());
        }
    }
}
