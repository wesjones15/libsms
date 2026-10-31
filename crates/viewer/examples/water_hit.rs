//! Exact enemy-water hit: TChuuHana::receiveMessage (chuuhana.cpp:556),
//! TSmallEnemy::receiveMessage (smallEnemy.cpp:542), cue 0xE7 / ms_enm_wathit.jpa.
//! Uses the live viewer's particle uploader and shader, with no asset overrides.
//! cargo run --release -p viewer --example water_hit -- assets output/water-hit.png
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
        window_title: "Sunshine enemy water hit | JPA 0xE7".into(),
        window_width: 1440,
        window_height: 420,
        high_dpi: false,
        ..Default::default()
    }
}

#[macroquad::main(config)]
async fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let dir = std::path::Path::new(args.first().map_or("assets", String::as_str));
    let out = args.get(1).map_or("water-hit.png", String::as_str);
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
    let mut particles = Particles::load(dir, &["ms_enm_wathit"]);
    assert_eq!(particles.names().count(), 1, "missing exact 0xE7 asset");
    particles.emit("ms_enm_wathit", Mat4::IDENTITY, 1.0);
    let times = [0, 1, 2, 4, 7, 12];
    let mut snapshots = Vec::new();
    for frame in 0..=90 {
        particles.advance(frame * 4);
        particles.build(V3::X, V3::Y);
        if times.contains(&frame) {
            eprintln!("0xE7 frame {frame}: {} corners", particles.vertices.len());
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
    assert!(
        !particles.active(),
        "one hit must expire without further hits"
    );
    for capture_frame in 0..3 {
        clear_background(Color::new(0.06, 0.15, 0.11, 1.));
        draw_text(
            "Enemy water-hit flash | JPA emitter 0xE7 | ms_enm_wathit.jpa",
            18.,
            30.,
            27.,
            WHITE,
        );
        draw_text(
            "Single hit, original disc scale / colour / texture / lifetime. 30 fps particle frames.",
            18.,
            55.,
            19.,
            LIGHTGRAY,
        );
        for (i, (frame, vertices, batches, textures)) in snapshots.iter().enumerate() {
            let x = i as f32 * 240.;
            for batch in batches {
                gl_use_material(if batch.blend == blend::ADDITIVE {
                    &additive
                } else {
                    &alpha
                });
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
                            x + 120. + v.pos.x * 1.0,
                            225. - v.pos.y * 1.0,
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
            draw_text(&format!("frame {frame}"), x + 75., 350., 22., WHITE);
        }
        draw_text(
            "Source: chuuhana.cpp:556 TChuuHana::receiveMessage; smallEnemy.cpp:542; MarDirectorLoadResource.cpp:206",
            18.,
            398.,
            18.,
            LIGHTGRAY,
        );
        if capture_frame == 2 {
            get_screen_data().export_png(out);
        }
        next_frame().await;
    }
    eprintln!("exact enemy water-hit GPU capture: {out}");
}
