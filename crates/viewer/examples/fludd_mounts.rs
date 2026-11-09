//! Reference capture for the exact FLUDD tank and nozzle joints from WaterGun.cpp.
use libsms::{Assets, NozzleKind, Sim, Status};
use libsms_present::{View, view::models};
use macroquad::prelude::*;

fn point(p: libsms::Vec3) -> Vec3 { vec3(p.x, p.y, p.z) * 0.01 }

#[macroquad::main("Original FLUDD nozzle mounts")]
async fn main() {
    let out = std::env::args().nth(1).expect("output directory");
    std::fs::create_dir_all(&out).unwrap();
    let Some(dir) = libsms::assets::find(None) else {
        eprintln!("SKIP: no Sunshine cache");
        return;
    };
    let assets = Assets::load(&dir);
    let mut view = View::load(&dir).expect("original Mario and FLUDD models");
    for (name, kind, file, sliding) in [
        ("spray", NozzleKind::Spray, "normal_wg/normal_wg.bmd", false),
        ("wet-slide", NozzleKind::Spray, "normal_wg/normal_wg.bmd", true),
        ("hover", NozzleKind::Hover, "hover_wg/hover_wg.bmd", false),
        ("rocket", NozzleKind::Rocket, "rocket_wg/rocket_wg.bmd", false),
        ("turbo", NozzleKind::Turbo, "back_wg/back_wg.bmd", false),
    ] {
        let (status, clip, pose) = if sliding {
            (Status::Catch, libsms::anim::clip::DIVE, "Wet slide | SLDCT (0x88)")
        } else {
            (Status::Wait, libsms::anim::clip::WAIT, "Standing | WAIT (0xC3)")
        };
        for frame in if sliding { [20, 40, 60] } else { [0, 4, 8] } {
            let mut sim = Sim::new(assets.params.clone(), assets.anim_table.clone(), libsms::Vec3::ZERO, 0);
            sim.mario.give_fludd();
            sim.mario.fludd.as_mut().unwrap().change_nozzle(kind, false, &sim.mario.params);
            sim.mario.portrait_pose(status, clip, frame as f32, u32::from(sliding), sliding);
            view.update(&mut sim, 0.0);
            clear_background(Color::from_rgba(24, 31, 42, 255));
            set_camera(&Camera3D { position: vec3(3.3, 2.0, -1.0), target: vec3(0., 0.8, 0.), up: Vec3::Y, ..Default::default() });
            draw_grid(10, 0.5, GRAY, DARKGRAY);
            for draw in &view.draws {
                let color = if draw.model == models::FLUDD { YELLOW }
                    else if models::NOZZLES.contains(&draw.model) { SKYBLUE } else { RED };
                let mut offset = draw.first_vertex as usize;
                let mut lo = libsms::Vec3::splat(f32::INFINITY);
                let mut hi = libsms::Vec3::splat(f32::NEG_INFINITY);
                for batch in &view.models[draw.model as usize].batches {
                    let vertices: Vec<_> = view.positions[offset..offset + batch.vertices.len()].iter().map(|p| {
                        let p = draw.pos + draw.rot * libsms::Vec3::new(-p.x, p.y, p.z);
                        lo = lo.min(p);
                        hi = hi.max(p);
                        let p = point(p);
                        Vertex::new(p.x, p.y, p.z, 0.0, 0.0, color)
                    }).collect();
                    for chunk in batch.indices.chunks(1500) {
                        draw_mesh(&Mesh {
                            vertices: chunk.iter().map(|i| vertices[*i as usize]).collect(),
                            indices: (0..chunk.len() as u16).collect(),
                            texture: None,
                        });
                    }
                    offset += batch.vertices.len();
                }
                if draw.model == models::FLUDD { println!("{name} {frame}: tank bounds {lo:?} to {hi:?}"); }
            }
            let count = if kind == NozzleKind::Hover { 2 } else { 1 };
            for index in 0..count {
                let muzzle = sim.mario.muzzle_world(index).expect("posed nozzle joint");
                println!("{name} {frame}: muzzle {index} {muzzle:?}");
                draw_sphere(point(muzzle), 0.05, None, LIME);
            }
            set_default_camera();
            draw_text(&format!("{pose} | frame {frame}"), 20., 30., 24., WHITE);
            draw_text(&format!("{file} | null_G_muzzle / null_G_muzzle2"), 20., 58., 19., LIGHTGRAY);
            draw_text("Yellow tank: wg_mdl1.bmd | WaterGun.cpp::nozzleBmdData / TWaterGun::init", 20., 84., 18., LIGHTGRAY);
            get_screen_data().export_png(&format!("{out}/{name}-{frame}.png"));
            next_frame().await;
        }
    }
}
