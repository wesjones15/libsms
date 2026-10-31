//! Exact HUD pictures nz01..04: GC2D/GCConsole2.cpp::loadAfter, standard_1.blo.
//! Layout proof, including the unchanged cooldown portrait plus its larger overlay.
use libsms_present::hud::Hud;
use macroquad::prelude::*;

fn config() -> Conf {
    Conf {
        window_title: "FLUDD radial | original HUD nozzle icons".into(),
        window_width: 1152,
        window_height: 896,
        high_dpi: false,
        ..Default::default()
    }
}
fn icon(tex: &Texture2D, center: Vec2, size: f32) {
    let fit = size / tex.width().max(tex.height());
    let dims = vec2(tex.width(), tex.height()) * fit;
    draw_texture_ex(
        tex,
        center.x - dims.x * 0.5,
        center.y - dims.y * 0.5,
        WHITE,
        DrawTextureParams {
            dest_size: Some(dims),
            ..Default::default()
        },
    );
}
fn text_center(text: &str, y: f32, size: u16) {
    text_at(text, 576.0, y, size);
}
fn text_at(text: &str, x: f32, y: f32, size: u16) {
    let m = measure_text(text, None, size, 1.);
    draw_text(text, x - m.width * 0.5, y, size as f32, WHITE);
}

#[macroquad::main(config)]
async fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let dir = std::path::Path::new(args.first().map_or("assets", String::as_str));
    let out = args
        .get(1)
        .map_or("output/nozzle-radial.png", String::as_str);
    let opening = args.get(2).is_some_and(|mode| mode == "opening");
    let mut hud = Hud::load(dir).unwrap();
    hud.update(
        &libsms_present::hud::Gauge {
            fill: 1.0,
            nozzle: libsms::NozzleKind::Spray,
            pressure: None,
        },
        0,
    );
    let icons: Vec<_> = (0..4)
        .map(|i| {
            let image = hud.nozzle_icon(i).unwrap();
            eprintln!("nz0{}: {}x{}", i + 1, image.width, image.height);
            let tex = Texture2D::from_rgba8(image.width as u16, image.height as u16, &image.rgba);
            tex.set_filter(FilterMode::Linear);
            tex
        })
        .collect();
    let portrait = load_texture("../live_icons/MarioAirFludd.png")
        .await
        .unwrap();
    let h = 896.;
    let outer = h * 0.18;
    let tank_scale = h / 480.0 * 0.8;
    let tank_size = vec2(hud.image.width as f32, hud.image.height as f32) * tank_scale;
    let tank_at = vec2(1152.0 - h * 0.02, 896.0 - h * 0.02) - tank_size;
    let tank = Texture2D::from_rgba8(
        hud.image.width as u16,
        hud.image.height as u16,
        &hud.image.rgba,
    );
    let [nx, ny, nw, nh] = hud.nozzle_rect();
    let head =
        tank_at + vec2(nx as f32 + nw as f32 * 0.5, ny as f32 + nh as f32 * 0.5) * tank_scale;
    let center = vec2(tank_at.x - outer - h * 0.035, head.y - outer * 0.6);
    let colors = [
        Color::from_rgba(33, 199, 219, 245),
        Color::from_rgba(150, 80, 25, 225),
        Color::from_rgba(88, 90, 101, 225),
        Color::from_rgba(150, 120, 20, 225),
    ];
    for frame in 0..if opening { 4 } else { 3 } {
        clear_background(Color::from_rgba(15, 27, 38, 255));
        text_center(
            "FLUDD radial layout | original HUD pictures nz01..04",
            32.,
            25,
        );
        text_center(
            "Source: GC2D/GCConsole2.cpp loadAfter | game_6/scrn/standard_1.blo",
            60.,
            18,
        );
        let (center, scale) = if opening {
            let progress = [0.0_f32, 0.30, 0.65, 1.0][frame];
            text_center(
                &format!("Opening: {:.0} ms / 280 ms", progress * 280.0),
                94.0,
                22,
            );
            let grow = 1.0 - (1.0 - progress).powi(3);
            let travel = progress * progress * (3.0 - 2.0 * progress);
            let position = head.lerp(center, travel);
            let scale = 0.08 + 0.92 * grow;
            (position, scale)
        } else {
            (center, 1.0)
        };
        let outer = outer * scale;
        text_at(
            "Select F.L.U.D.D. Nozzle with RS",
            center.x,
            center.y - outer - h * 0.052 * scale + 25. * scale,
            (24.0 * scale).max(2.0) as u16,
        );
        draw_circle(
            center.x,
            center.y,
            outer * 0.57,
            Color::from_rgba(7, 12, 20, 240),
        );
        for i in 0..4 {
            let a = (90. - i as f32 * 90.).to_radians();
            for j in 0..88 {
                let a0 = a + (-44. + j as f32).to_radians();
                let a1 = a + (-43. + j as f32).to_radians();
                let at = |r: f32, a: f32| center + vec2(a.cos(), -a.sin()) * r;
                let (a, b, c, d) = (
                    at(outer * 0.57, a0),
                    at(outer, a0),
                    at(outer, a1),
                    at(outer * 0.57, a1),
                );
                draw_triangle(a, b, c, colors[i]);
                draw_triangle(a, c, d, colors[i]);
            }
            // The left, right and bottom pictures sit slightly farther out, as the RoR2 radial
            // places them; the enlarged Hover picture already reads centred.
            let reach = if i == 0 { outer * 0.74 } else { outer * 0.79 };
            let at = center + vec2(a.cos(), -a.sin()) * reach;
            let size = (h * 0.08f32).clamp(40., 104.) * scale * if i == 0 { 1.35 } else { 1. };
            icon(&icons[[1, 2, 0, 3][i]], at, size);
            if i == 2 {
                draw_line(
                    at.x - size * 0.35,
                    at.y - size * 0.35,
                    at.x + size * 0.35,
                    at.y + size * 0.35,
                    6. * scale,
                    RED,
                );
            }
        }
        let hub = (h * 0.09f32).clamp(44., 116.) * scale;
        icon(&icons[1], center - vec2(0., h * 0.012 * scale), hub * 1.35);
        text_at(
            "HOVER",
            center.x,
            center.y + hub * 0.55 + 23. * scale,
            (27.0 * scale).max(2.0) as u16,
        );
        draw_texture_ex(
            &tank,
            tank_at.x,
            tank_at.y,
            WHITE,
            DrawTextureParams {
                dest_size: Some(tank_size),
                ..Default::default()
            },
        );
        text_at("Cooldown overlays: same Hover boost", 320., 635., 21);
        for (i, &idx) in [1usize, 2, 3].iter().enumerate() {
            let at = vec2(160. + i as f32 * 160., 725.);
            draw_texture_ex(
                &portrait,
                at.x - 48.,
                at.y - 48.,
                WHITE,
                DrawTextureParams {
                    dest_size: Some(vec2(96., 96.)),
                    ..Default::default()
                },
            );
            draw_rectangle_lines(at.x - 48., at.y - 48., 96., 96., 2., GRAY);
            icon(
                &icons[idx],
                at,
                96. * 0.95 * if idx == 1 { 1.35 } else { 1. },
            );
        }
        text_at("Generated cooldown portrait unchanged.", 320., 824., 19);
        if opening || frame == 2 {
            let filename = if opening {
                format!("{}-{frame}.png", out.trim_end_matches(".png"))
            } else {
                out.into()
            };
            get_screen_data().export_png(&filename);
        }
        next_frame().await;
    }
}
