//! Viewer-first proof of the exact type-0 model-water mask and shared cohesive mesh.
//! cargo run --release -p viewer --example water_stream -- assets output/water-stream.png
use ::glam::Vec3 as V3;
use libsms::water::WaterManager;
use libsms::{FlatHost, NozzleKind, Params, WaterEmit};
use libsms_present::water::WaterLook;
use macroquad::material::{
    MaterialParams, gl_use_default_material, gl_use_material, load_material,
};
use macroquad::miniquad::{
    BlendFactor, BlendState, BlendValue, Equation, PipelineParams, ShaderSource,
};
use macroquad::prelude::*;

fn config() -> Conf {
    Conf {
        window_title: "FLUDD | type 0 watermask | cohesive stream".into(),
        window_width: 1440,
        window_height: 660,
        high_dpi: false,
        ..Default::default()
    }
}
#[macroquad::main(config)]
async fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let dir = std::path::Path::new(args.first().map_or("assets", String::as_str));
    let out = args
        .get(1)
        .map_or("output/water-stream.png", String::as_str);
    let mut water = WaterManager::new(&Params::from_dir(&dir.join("params_mario")).0);
    water.reach = 2.0;
    water.spray_speed = 1.2;
    let emit = WaterEmit {
        nozzle: NozzleKind::Spray,
        pos: V3::new(0., 150., 0.),
        dir: V3::Z,
        power: 40.,
        count: 3,
        attack: 1,
        hit_radius: 50.,
        hit_height: 80.,
        inherit: V3::ZERO,
        size: 50.,
        size_tremble: 0.1,
        dir_tremble: 0.03,
        pow_tremble: 0.1,
        particle_type: 0,
        burst: false,
        spin: false,
        precise: false,
    };
    let mut snapshots = Vec::new();
    let mut look = WaterLook::load(dir);
    assert!(!look.drop.rgba.is_empty(), "exact watermask asset missing");
    let texture = Texture2D::from_rgba8(
        look.drop.width as u16,
        look.drop.height as u16,
        &look.drop.rgba,
    );
    texture.set_filter(FilterMode::Linear);
    for frame in 0..32 {
        water.emit(&emit);
        water.step(&FlatHost::default(), V3::ZERO);
        if [8, 18, 30].contains(&frame) {
            let mut airborne = water.clone();
            airborne
                .particles
                .retain(|p| p.state == libsms::water::State::Flying);
            let mut pair = Vec::new();
            for cohesive in [false, true] {
                look.cohesive = cohesive;
                look.nozzle = Some(emit.pos);
                let start = std::time::Instant::now();
                look.build(&airborne, -0.5, V3::new(500., 150., 0.), V3::Z, V3::Y, 0);
                eprintln!(
                    "cohesive={cohesive}: shared mesh build {:?}",
                    start.elapsed()
                );
                pair.push(look.vertices[..look.drop_quads as usize * 4].to_vec());
            }
            eprintln!(
                "frame {frame}: legacy {} / cohesive {} quads",
                pair[0].len() / 4,
                pair[1].len() / 4
            );
            snapshots.push((frame, pair));
        }
    }
    let material=load_material(ShaderSource::Glsl {vertex:"#version 100\nattribute vec3 position; attribute vec2 texcoord; attribute vec4 normal; uniform mat4 Model; uniform mat4 Projection; varying vec2 uv; varying vec4 color; void main(){gl_Position=Projection*Model*vec4(position,1.);uv=texcoord;color=vec4(1.,1.,1.,normal.x);}",
        fragment:"#version 100\nprecision mediump float; varying vec2 uv; varying vec4 color; uniform sampler2D Texture; void main(){float a=texture2D(Texture,uv).a*color.a; gl_FragColor=vec4(0.60,0.76,0.94,a*0.45);}"},
        MaterialParams {pipeline_params:PipelineParams { color_blend:Some(BlendState::new(Equation::Add,BlendFactor::Value(BlendValue::SourceAlpha),BlendFactor::OneMinusValue(BlendValue::SourceAlpha))),..Default::default()},..Default::default()}).unwrap();
    for n in 0..3 {
        clear_background(Color::from_rgba(18, 26, 35, 255));
        draw_text(
            "FLUDD Spray | model-water type 0 | timg/watermask.bti",
            20.,
            30.,
            27.,
            WHITE,
        );
        draw_text(
            "Source: Player/WaterGun.cpp TNozzleDeform::emit; ModelWaterManager.cpp calcDrawVtx / drawSilhouette",
            20.,
            56.,
            19.,
            LIGHTGRAY,
        );
        draw_text(
            "Same simulated drops / motion / disc mask. Tint-only comparison isolates segment seams; RoR2 refraction remains to be played.",
            20.,
            80.,
            18.,
            LIGHTGRAY,
        );
        for (col, (frame, pair)) in snapshots.iter().enumerate() {
            let x = col as f32 * 480.;
            draw_text(&format!("Water frame {frame}"), x + 18., 112., 23., WHITE);
            for (row, vertices) in pair.iter().enumerate() {
                let top = 130. + row as f32 * 255.;
                for gy in 0..10 {
                    for gx in 0..19 {
                        let c = if (gx + gy) % 2 == 0 {
                            Color::from_rgba(46, 61, 72, 255)
                        } else {
                            Color::from_rgba(39, 51, 63, 255)
                        };
                        draw_rectangle(
                            x + 12. + gx as f32 * 24.,
                            top + gy as f32 * 24.,
                            24.,
                            24.,
                            c,
                        );
                    }
                }
                draw_text(
                    if row == 0 {
                        "Separate segment overlays"
                    } else {
                        "Shared cohesive surface"
                    },
                    x + 22.,
                    top + 26.,
                    21.,
                    WHITE,
                );
                let mut mesh = Mesh {
                    vertices: Vec::new(),
                    indices: Vec::new(),
                    texture: if row == 0 {
                        Some(texture.clone())
                    } else {
                        None
                    },
                };
                gl_use_material(&material);
                for q in vertices.chunks_exact(4) {
                    if mesh.vertices.len() >= 2000 {
                        draw_mesh(&mesh);
                        mesh.vertices.clear();
                        mesh.indices.clear();
                    }
                    let b = mesh.vertices.len() as u16;
                    for v in q {
                        let mut vertex = Vertex::new(
                            x + 35. + v.pos.z * 0.16,
                            top + 150. - (v.pos.y - 150.) * 0.19,
                            0.,
                            v.uv[0],
                            v.uv[1],
                            WHITE,
                        );
                        vertex.normal.x = v.alpha;
                        mesh.vertices.push(vertex);
                    }
                    mesh.indices
                        .extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
                }
                draw_mesh(&mesh);
                gl_use_default_material();
            }
        }
        if n == 2 {
            get_screen_data().export_png(out);
        }
        next_frame().await;
    }
}
