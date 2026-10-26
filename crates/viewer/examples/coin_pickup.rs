//! Exact pickup emitters 0x37/0x38, including B's SSP1 child texture 1.
//! Source: MarioParticle.cpp::emitGetCoinEffect; Particles.hpp:87-88;
//! JPASweepShape / JPADraw::initChild / JPABaseEmitter::doParticle.
//! `cargo run --release -p viewer --example coin_pickup -- assets out.png`
//! Uses the live viewer's shared shader and texture uploader. No host overrides.
#[allow(dead_code)]
#[path = "../src/fx.rs"]
mod fx;

use ::glam::{Mat4, Vec3 as V3};
use libsms_present::particles::{Particles, blend};
use macroquad::material::{
    MaterialParams, gl_use_default_material, gl_use_material, load_material,
};
use macroquad::miniquad::{
    BlendFactor, BlendState, BlendValue, Equation, PipelineParams, ShaderSource,
};
use macroquad::prelude::*;

fn config() -> Conf {
    Conf {
        window_title: "Sunshine coin pickup - exact disc particles".into(),
        window_width: 1320,
        window_height: 730,
        high_dpi: false,
        ..Default::default()
    }
}

#[macroquad::main(config)]
async fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let dir = std::path::Path::new(args.first().map_or("assets", String::as_str));
    let out = args.get(1).map_or("coin-pickup.png", String::as_str);
    let material = |dst| {
        load_material(
            ShaderSource::Glsl {
                vertex: fx::VERTEX,
                fragment: fx::FRAGMENT,
            },
            MaterialParams {
                pipeline_params: PipelineParams {
                    color_blend: Some(BlendState::new(
                        Equation::Add,
                        BlendFactor::Value(BlendValue::SourceAlpha),
                        dst,
                    )),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap()
    };
    let additive = material(BlendFactor::One);
    let alpha = material(BlendFactor::OneMinusValue(BlendValue::SourceAlpha));
    let times = [0, 2, 4, 7, 10, 16];
    let mut snapshots = Vec::new();
    for names in [&["ms_coinget_a", "ms_coinget_b"][..], &["ms_coinget_b"][..]] {
        let mut particles = Particles::load(dir, names);
        assert_eq!(
            particles.names().count(),
            names.len(),
            "missing coin assets"
        );
        for name in names {
            particles.emit(name, Mat4::IDENTITY, 1.0);
        }
        for frame in 0..=60 {
            particles.advance(frame * 4);
            particles.build(V3::X, V3::Y);
            if times.contains(&frame) {
                snapshots.push((
                    frame,
                    particles.vertices.clone(),
                    particles.batches.clone(),
                    particles
                        .textures
                        .iter()
                        .map(fx::upload_texture)
                        .collect::<Vec<_>>(),
                ));
            }
        }
        assert!(!particles.active(), "pickup must expire");
    }
    for capture_frame in 0..3 {
        clear_background(Color::new(0.04, 0.06, 0.09, 1.));
        draw_text(
            "Yellow coin pickup | original JPA emitters 0x37 + 0x38",
            18.,
            30.,
            27.,
            WHITE,
        );
        draw_text(
            "Mirror U/V preserved; 0x38 child sparkle: texture 1, 14 frames, authored RGBA. Same scale in every panel.",
            18.,
            55.,
            19.,
            LIGHTGRAY,
        );
        for (i, (frame, vertices, batches, textures)) in snapshots.iter().enumerate() {
            let row = i / 6;
            let x = (i % 6) as f32 * 220.;
            let y = 75. + row as f32 * 300.;
            for batch in batches {
                if batch.blend == blend::ADDITIVE {
                    gl_use_material(&additive);
                } else {
                    gl_use_material(&alpha);
                }
                let verts = &vertices[batch.first_vertex as usize
                    ..(batch.first_vertex + batch.vertex_count) as usize];
                let mut mesh = Mesh {
                    vertices: Vec::new(),
                    indices: Vec::new(),
                    texture: Some(textures[batch.texture as usize].clone()),
                };
                for q in verts.chunks_exact(4) {
                    let start = mesh.vertices.len() as u16;
                    for v in q {
                        mesh.vertices.push(Vertex::new(
                            x + 110. + v.pos.x * 0.35,
                            y + 205. - v.pos.y * 0.35,
                            0.,
                            v.u,
                            v.v,
                            Color::new(v.color[0], v.color[1], v.color[2], v.color[3]),
                        ));
                    }
                    mesh.indices
                        .extend([start, start + 1, start + 2, start, start + 2, start + 3]);
                }
                draw_mesh(&mesh);
            }
            gl_use_default_material();
            draw_text(
                &format!(
                    "{} | frame {frame}",
                    if row == 0 { "Combined" } else { "B + child" }
                ),
                x + 10.,
                y + 285.,
                19.,
                WHITE,
            );
        }
        draw_text(
            "Source: TMario::emitGetCoinEffect (MarioParticle.cpp:134), Particles.hpp:87-88; JPAEmitter.cpp / JPADraw.cpp",
            18.,
            710.,
            18.,
            LIGHTGRAY,
        );
        if capture_frame == 2 {
            get_screen_data().export_png(out);
        }
        next_frame().await;
    }
    eprintln!("coin pickup GPU capture: {out}");
}
