//! Native fast entrance proof. --luigi selects the real HSD alt-skin and green palette.
//! --capture <folder>: 150 frames at 60 fps. Source: MarioParticle::warpOutEffect 0x40,
//! demo_gate_out_appear_get / rolling_get, MarioRun::jumpSlip (jmped).
use ::glam::{Mat4 as GMat, Vec3 as G3};
use libsms_present::{
    View,
    view::{Draw, Entrance},
};
use macroquad::{
    miniquad::{
        BlendFactor, BlendState, BlendValue, Comparison, Equation, PipelineParams, ShaderSource,
    },
    prelude::*,
};
use std::{collections::HashMap, path::PathBuf};
#[path = "support/pod_fire.rs"]
mod pod_fire;
fn conf() -> Conf {
    Conf {
        window_title: "Fast statue portal / native presenter".into(),
        window_width: 1100,
        window_height: 800,
        ..Default::default()
    }
}
fn material(add: bool, depth: bool) -> Material {
    load_material(ShaderSource::Glsl {
        vertex: "#version 100\nattribute vec3 position;attribute vec2 texcoord;attribute vec4 color0;varying lowp vec2 uv;varying lowp vec4 color;uniform mat4 Model;uniform mat4 Projection;void main(){gl_Position=Projection*Model*vec4(position,1.);uv=texcoord;color=color0/255.;}",
        fragment: "#version 100\nprecision mediump float;varying lowp vec2 uv;varying lowp vec4 color;uniform sampler2D Texture;void main(){vec4 c=texture2D(Texture,uv)*color;if(c.a<0.01)discard;gl_FragColor=c;}"
    }, MaterialParams { pipeline_params: PipelineParams { color_blend: Some(BlendState::new(Equation::Add,BlendFactor::Value(BlendValue::SourceAlpha), if add {BlendFactor::One} else {BlendFactor::OneMinusValue(BlendValue::SourceAlpha)})),depth_test:Comparison::LessOrEqual,depth_write:depth,..Default::default()}, ..Default::default() }).unwrap()
}
fn tex(t: &libsms_present::tev::Baked) -> Texture2D {
    let made = Texture2D::from_rgba8(t.width as u16, t.height as u16, &t.rgba);
    made.set_filter(FilterMode::Linear);
    made
}
fn meshes(view: &View, d: Draw, textures: &[Texture2D]) -> Vec<Mesh> {
    let mut out = Vec::new();
    let mut batch = |first: usize, count: usize, indices: Vec<u16>, bi: usize| {
        let color = view.colors[d.first_color as usize + bi];
        let id = view.texture_ids[d.first_color as usize + bi];
        let rotation = d.rot;
        let vertices = (0..count)
            .map(|i| {
                let vi = d.first_vertex as usize + first + i;
                let p = (rotation * view.positions[vi] + d.pos) * 0.01;
                let n = view.normals[vi];
                let light = if d.kind == 240 {
                    0.72 + 0.28 * n.dot(G3::new(-0.3, 0.8, 0.5).normalize()).max(0.)
                } else {
                    1.
                };
                let uv = view.uvs[vi];
                // Keep the native UVs. This camera and Unity's reflected geometry give
                // the same front view; undoing U here hid the host's backwards M.
                Vertex::new(
                    p.x,
                    p.y,
                    p.z,
                    uv[0],
                    uv[1],
                    Color::new(
                        color[0] * light,
                        color[1] * light,
                        color[2] * light,
                        color[3],
                    ),
                )
            })
            .collect();
        out.push(Mesh {
            vertices,
            indices,
            texture: usize::try_from(id)
                .ok()
                .and_then(|id| textures.get(id).cloned()),
        });
    };
    if let Some(m) = view.models.get(d.model as usize) {
        let mut first = 0;
        for (bi, b) in m.batches.iter().enumerate() {
            batch(
                first,
                b.vertices.len(),
                b.indices.iter().map(|&i| i as u16).collect(),
                bi,
            );
            first += b.vertices.len();
        }
    } else if let Some(l) = &view.luigi {
        for (bi, b) in l.batches.iter().enumerate() {
            let indices = l.indices
                [b.first_index as usize..(b.first_index + b.index_count) as usize]
                .iter()
                .map(|&i| (i - b.first_vertex) as u16)
                .collect();
            batch(
                b.first_vertex as usize,
                b.vertex_count as usize,
                indices,
                bi,
            );
        }
    }
    out
}
#[macroquad::main(conf)]
async fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let dir = PathBuf::from(args.first().cloned().unwrap_or_else(|| "assets".into()));
    let capture = args
        .iter()
        .position(|a| a == "--capture")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from);
    if let Some(p) = &capture {
        std::fs::create_dir_all(p).unwrap();
    }
    libsms_present::portal_cache::prepare(&dir).unwrap();
    let mut view = View::load(&dir).unwrap();
    let mut entrance = Entrance::load_fast(&dir, &mut view).unwrap();
    let (base_lo, base_hi) = entrance.pedestal_bounds(&view);
    let fire_center = (base_lo + base_hi) * 0.005 + G3::Y * ((base_lo.y - base_hi.y) * 0.005 + 5.5);
    let luigi = args.iter().any(|a| a == "--luigi");
    let late_luigi = args.iter().any(|a| a == "--late-luigi");
    let fire_dir = args
        .iter()
        .position(|a| a == "--descent-fire")
        .and_then(|i| args.get(i + 1));
    let fire = if let Some(path) = fire_dir {
        Some(pod_fire::Fire::load(&PathBuf::from(path)).await)
    } else {
        None
    };
    // ModelView fixes its model table at creation: preload the alternate mesh, then select
    // the rider later, reproducing a passenger/loadout arriving after pod preparation.
    if luigi || late_luigi {
        assert!(view.set_luigi(&dir.join("../../libssbm/assets")));
        if !late_luigi {
            view.set_skin(libsms_present::view::Skin::Luigi);
        }
    }
    let mut textures: Vec<_> = view.textures.list.iter().map(tex).collect();
    let mut versions: Vec<_> = (0..textures.len())
        .map(|i| view.textures.version(i as u32))
        .collect();
    let opaque = material(false, true);
    let alpha = material(false, false);
    let add = material(true, false);
    let mut rigid: HashMap<u64, Vec<Mesh>> = HashMap::new();
    let mut time = 0f32;
    let mut frame = 0;
    let mut fxtextures: Vec<Texture2D> = Vec::new();
    loop {
        if is_key_pressed(KeyCode::Escape) {
            break;
        }
        if late_luigi && time >= 0.25 {
            view.set_skin(libsms_present::view::Skin::Luigi);
        }
        let eye = if fire.is_some() {
            G3::new(22., 16., 31.)
        } else {
            G3::new(5.5, 3.8, 11.5)
        };
        let at = if fire.is_some() {
            G3::new(0., 8., -3.)
        } else {
            G3::new(0., 1.8, 1.4)
        };
        let matrix = GMat::look_at_rh(eye, at, G3::Y);
        let right = matrix.inverse().x_axis.truncate();
        let up = matrix.inverse().y_axis.truncate();
        entrance.update_fast(
            &mut view,
            (time - 0.35).max(0.),
            if time < 0.35 || fire.is_some() { 0 } else { 1 },
            G3::new(-right.x, right.y, right.z),
            G3::new(-up.x, up.y, up.z),
        );
        for (i, t) in view.textures.list.iter().enumerate() {
            if i >= textures.len() {
                textures.push(tex(t));
                versions.push(0);
            }
            let version = view.textures.version(i as u32);
            if version != versions[i] {
                textures[i].update(&Image {
                    bytes: t.rgba.clone(),
                    width: t.width as u16,
                    height: t.height as u16,
                });
                versions[i] = version;
            }
        }
        clear_background(Color::new(0.08, 0.11, 0.16, 1.));
        set_camera(&Camera3D {
            position: vec3(eye.x, eye.y, eye.z),
            target: vec3(at.x, at.y, at.z),
            up: vec3(0., 1., 0.),
            fovy: 45f32.to_radians(),
            ..Default::default()
        });
        draw_grid(40, 1., GRAY, DARKGRAY);
        gl_use_material(&opaque);
        for &d in &view.draws {
            gl_use_material(if d.kind == 241 { &alpha } else { &opaque });
            if d.flags & 4 == 0 {
                rigid.insert(d.key, meshes(&view, d, &textures));
            }
            if let Some(ms) = rigid.get(&d.key) {
                for m in ms {
                    draw_mesh(m);
                }
            }
        }
        if fxtextures.is_empty() {
            fxtextures = view
                .fx
                .textures
                .iter()
                .map(|t| Texture2D::from_rgba8(t.width as u16, t.height as u16, &t.rgba))
                .collect();
        }
        for b in &view.fx.batches {
            let vs = &view.fx.vertices
                [b.first_vertex as usize..(b.first_vertex + b.vertex_count) as usize];
            let vertices = vs
                .iter()
                .map(|v| {
                    Vertex::new(
                        -v.pos.x * 0.01,
                        v.pos.y * 0.01,
                        v.pos.z * 0.01,
                        v.u,
                        v.v,
                        Color::new(v.color[0], v.color[1], v.color[2], v.color[3]),
                    )
                })
                .collect();
            let indices = (0..vs.len() as u16 / 4)
                .flat_map(|q| [q * 4, q * 4 + 1, q * 4 + 2, q * 4, q * 4 + 2, q * 4 + 3])
                .collect();
            gl_use_material(if b.blend == 0 { &add } else { &alpha });
            draw_mesh(&Mesh {
                vertices,
                indices,
                texture: Some(fxtextures[b.texture as usize].clone()),
            });
        }
        gl_use_default_material();
        if let Some(fire) = &fire {
            fire.draw(time, vec3(fire_center.x, fire_center.y, fire_center.z));
        }
        set_default_camera();
        draw_rectangle(0., 0., 1100., 62., Color::new(0.04, 0.05, 0.08, 0.95));
        draw_text(
            if fire.is_some() {
                "SurvivorPod / Flames - exact donut5Mesh / matEntryBurn"
            } else if luigi || late_luigi {
                "LUIGI | Fast portal / green cap particles"
            } else {
                "MARIO | Fast portal / original particles"
            },
            18.,
            28.,
            24.,
            WHITE,
        );
        draw_text(
            if fire.is_some() {
                "3.5x wide / 1.75x tall | local particles / base +5.5m | Unity height needs in-game verification"
            } else {
                "Emitter 0x40 | 0.8s particles | normal-rate spin / gentle 0.8s fall | normal landing"
            },
            18.,
            52.,
            18.,
            LIGHTGRAY,
        );
        draw_text(
            &if fire.is_some() {
                "Flames PS -470276994286075641 | donut5Mesh -6476590250452040025".into()
            } else {
                format!(
                    "X + {:.2}s | original full Sunshine sequence retained in portal_entrance",
                    (time - 0.35).max(0.)
                )
            },
            18.,
            776.,
            19.,
            WHITE,
        );
        if let Some(p) = &capture {
            get_screen_data().export_png(p.join(format!("frame_{frame:04}.png")).to_str().unwrap());
            if frame >= 149 {
                break;
            }
        }
        time += if capture.is_some() {
            1. / 60.
        } else {
            get_frame_time().min(0.05)
        };
        frame += 1;
        if capture.is_none() && time > 4. {
            time = 0.;
            rigid.clear();
            fxtextures.clear();
            view = View::load(&dir).unwrap();
            entrance = Entrance::load_fast(&dir, &mut view).unwrap();
            if luigi || late_luigi {
                assert!(view.set_luigi(&dir.join("../../libssbm/assets")));
                view.set_skin(libsms_present::view::Skin::Luigi);
            }
            textures = view.textures.list.iter().map(tex).collect();
            versions = (0..textures.len())
                .map(|i| view.textures.version(i as u32))
                .collect();
        }
        next_frame().await;
    }
}
