//! Procedural poses driven by live jump states. No original models or effects are loaded.
use libsms::{AnimTable, FlatHost, Pad, Params, Sim, Status};
use macroquad::prelude::*;

const HEIGHT: f32 = libsms::mario::STAND_IN_HEIGHT * 0.01;

fn circle(tick: usize) -> Pad {
    let angle = tick as f32 * std::f32::consts::TAU / 12.0;
    Pad { stick: libsms::Vec2::new(angle.cos(), angle.sin()), ..Pad::default() }
}

fn body(center: Vec3, rotation: Mat4, corkscrew: bool) {
    unsafe { get_internal_gl().quad_gl.push_model_matrix(Mat4::from_translation(center) * rotation) };
    draw_cylinder(vec3(0.0, -HEIGHT * 0.5, 0.0), 0.35, 0.35, HEIGHT, None, RED);
    draw_sphere(vec3(0.0, HEIGHT * 0.3, 0.35), 0.08, None, WHITE);
    draw_cylinder(vec3(0.0, 0.5 - HEIGHT * 0.5, -0.35), 0.175, 0.175, 0.8, None, YELLOW);
    if corkscrew {
        draw_cylinder(vec3(0.0, -HEIGHT * 0.2, 0.0), 0.7, 0.7, HEIGHT * 0.4, None, PINK);
    }
    unsafe { get_internal_gl().quad_gl.pop_model_matrix() };
}

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
    let mut corkscrew = Sim::new(Params::builtin(), AnimTable::builtin(), libsms::Vec3::Y * 450.0, 0);
    corkscrew.mario.change_status(Status::JumpCatch, 0, true);
    corkscrew.mario.flight_camera = Some(libsms::Aim { yaw: 0, pitch: 0 });
    for tick in 0..24 {
        corkscrew.advance(&mut floor, &circle(tick), 0, 1.0 / 120.0);
    }
    assert_ne!(corkscrew.mario.model_pitch, 0, "the stick circle did not start the corkscrew");
    corkscrew.mario.flight_camera = Some(libsms::Aim { yaw: 0, pitch: 8000 });
    for frame in 0..=64 {
        clear_background(Color::from_rgba(24, 31, 42, 255));
        set_camera(&Camera3D {
            position: vec3(3.8, 3.0, 8.0),
            target: vec3(0.0, 1.5, 0.0),
            up: Vec3::Y,
            ..Default::default()
        });
        draw_grid(14, 1.0, GRAY, DARKGRAY);
        for (sim, x) in jumps.iter_mut().zip([-2.4, 0.0]) {
            sim.advance(&mut floor, &Pad { a: true, ..Pad::default() }, 0, 1.0 / 120.0);
            let pitch = f32::from(sim.mario.stand_in_jump_pitch()) * std::f32::consts::PI / 32768.0;
            body(vec3(x, 1.5, 0.0), Mat4::from_rotation_x(pitch), false);
        }
        corkscrew.advance(&mut floor, &circle(frame + 24), 0, 1.0 / 120.0);
        let yaw = f32::from(corkscrew.mario.model_yaw) * (std::f32::consts::PI / 32768.0);
        let (offset, rot) = libsms::Mario::model_root_pose(
            libsms::Vec3::ZERO, ::glam::Quat::from_rotation_y(yaw), corkscrew.mario.model_pitch, corkscrew.mario.model_roll,
        );
        let rotation = Mat4::from_quat(Quat::from_array(rot.to_array()));
        let feet = vec3(2.4, 1.5 - 1.6 * 0.35, 0.0);
        let center = feet + vec3(offset.x, offset.y, offset.z) * 0.01 + rotation.transform_vector3(Vec3::Y * (HEIGHT * 0.5));
        body(center, rotation, true);
        set_default_camera();
        draw_text("Procedural jump poses from live native animation clocks", 20.0, 30.0, 25.0, WHITE);
        draw_text("Side jump: TJMP1 (0xBF)  |  Triple jump: ROLL (0x6F)", 20.0, 58.0, 21.0, LIGHTGRAY);
        draw_text("Corkscrew: SPIN_P (0xF4) with the native host-added pitch and roll", 20.0, 84.0, 20.0, LIGHTGRAY);
        draw_text(&format!("Native tick {}", frame + 1), 20.0, screen_height() - 22.0, 22.0, WHITE);
        if [0, 4, 8, 12, 29, 62].contains(&frame) {
            get_screen_data().export_png(&format!("{out}/flips-{frame}.png"));
        }
        next_frame().await;
    }
}
