//! Exact nozzle pickup: MarioReceiveMsg.cpp::getNozzle -> MarioParticle.cpp::emitGetEffect.
//! JPA particle emitter 0x0E / ms_itemget1_a.jpa; sound Item.cpp::touchPlayer 0x484E.
//! Uses the live viewer's particle uploader and shader, with no asset overrides.
//! cargo run --release -p viewer --example nozzle_pickup -- assets output/nozzle-pickup.png
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
        window_title: "Sunshine nozzle pickup | JPA 0x0E".into(),
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
    let out = args.get(1).map_or("nozzle-pickup.png", String::as_str);
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
    let mut particles = Particles::load(dir, &["ms_itemget1_a"]);
    let jpa = libsms_assets::jpa::Jpa::parse(
        &std::fs::read(dir.join("particle/ms_itemget1_a.jpa")).unwrap(),
    )
    .unwrap();
    eprintln!(
        "0x0E disc duration: emitter {} frames, particle lifetime {}",
        jpa.emitter.max_frame, jpa.emitter.lifetime
    );
    assert_eq!(particles.names().count(), 1, "missing exact 0x0E asset");
    particles.emit("ms_itemget1_a", Mat4::IDENTITY, 1.0);
    let times = [0, 2, 5, 10, 20, 35];
    let mut snapshots = Vec::new();
    for frame in 0..=600 {
        particles.advance(frame * 4);
        particles.build(V3::X, V3::Y);
        if times.contains(&frame) {
            eprintln!("0x0E frame {frame}: {} corners", particles.vertices.len());
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
        "one pickup must expire without another pickup"
    );
    for capture_frame in 0..3 {
        clear_background(Color::new(0.06, 0.15, 0.11, 1.));
        draw_text(
            "Nozzle pickup sparkle | JPA emitter 0x0E | ms_itemget1_a.jpa",
            18.,
            30.,
            27.,
            WHITE,
        );
        draw_text(
            "Single pickup, original disc scale / colour / texture / lifetime. 30 fps particle frames.",
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
            "Source: MarioReceiveMsg.cpp::getNozzle -> MarioParticle.cpp::emitGetEffect; MarDirectorLoadResource.cpp:303",
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
    eprintln!("exact nozzle pickup GPU capture: {out}");
}
