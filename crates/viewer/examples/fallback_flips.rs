//! Procedural poses driven by live jump states. No original models or effects are loaded.
use libsms::{AnimTable, FlatHost, Pad, Params, Sim, Status};
use macroquad::prelude::*;

#[macroquad::main("Stand-in jump flips")]
async fn main() {
    let out = std::env::args().nth(1).expect("output directory");
    std::fs::create_dir_all(&out).unwrap();
    let mut jumps = [Status::UTurnJump, Status::UltraJump].map(|status| {
        let mut sim = Sim::new(Params::builtin(), AnimTable::builtin(), libsms::Vec3::ZERO, 0);
        sim.mario.change_status(status, 0, true);
        sim
    });
    let mut floor = FlatHost::default();
    for frame in 0..=64 {
        clear_background(Color::from_rgba(24, 31, 42, 255));
        set_camera(&Camera3D {
            position: vec3(3.8, 3.0, 6.0),
            target: vec3(0.0, 1.5, 0.0),
            up: Vec3::Y,
            ..Default::default()
        });
        draw_grid(14, 1.0, GRAY, DARKGRAY);
        for (sim, x) in jumps.iter_mut().zip([-1.6, 1.6]) {
            sim.advance(&mut floor, &Pad { a: true, ..Pad::default() }, 0, 1.0 / 120.0);
            let pitch = f32::from(sim.mario.stand_in_jump_pitch()) * std::f32::consts::PI / 32768.0;
            let transform = Mat4::from_translation(vec3(x, 1.5, 0.0)) * Mat4::from_rotation_x(pitch);
            unsafe { get_internal_gl().quad_gl.push_model_matrix(transform) };
            draw_cylinder(vec3(0.0, -0.8, 0.0), 0.35, 0.35, 1.6, None, RED);
            draw_sphere(vec3(0.0, 0.48, 0.35), 0.08, None, WHITE);
            draw_cylinder(vec3(0.0, -0.35, -0.35), 0.175, 0.175, 0.8, None, YELLOW);
            unsafe { get_internal_gl().quad_gl.pop_model_matrix() };
        }
        set_default_camera();
        draw_text("Procedural jump poses from live native animation clocks", 20.0, 30.0, 25.0, WHITE);
        draw_text("Side jump: TJMP1 (0xBF)  |  Triple jump: ROLL (0x6F)", 20.0, 58.0, 21.0, LIGHTGRAY);
        draw_text("Reference: Player/MarioJump.cpp. Fixed display positions.", 20.0, 84.0, 20.0, LIGHTGRAY);
        draw_text(&format!("Native tick {}", frame + 1), 20.0, screen_height() - 22.0, 22.0, WHITE);
        if [14, 29, 44, 62].contains(&frame) {
            get_screen_data().export_png(&format!("{out}/flips-{frame}.png"));
        }
        next_frame().await;
    }
}
