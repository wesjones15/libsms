//! Viewer-first check of the shared type-0 water surface from the gameplay views that broke:
//! side-on, head-on (the spray coming at the camera) and while the nozzle moves.
//! cargo run --release -p viewer --example water_stream_views -- assets output/water-views.png
use ::glam::Vec3 as V3;
use libsms::water::WaterManager;
use libsms::{FlatHost, NozzleKind, Params, WaterEmit};
use libsms_present::water::{WaterLook, WaterVertex};
use macroquad::material::{
    MaterialParams, gl_use_default_material, gl_use_material, load_material,
};
use macroquad::miniquad::{
    BlendFactor, BlendState, BlendValue, Comparison, Equation, PipelineParams, ShaderSource,
};
use macroquad::prelude::*;

fn config() -> Conf {
    Conf {
        window_title: "FLUDD | type 0 watermask | side / head-on / moving".into(),
        window_width: 1440,
        window_height: 900,
        high_dpi: false,
        ..Default::default()
    }
}

fn material() -> Material {
    load_material(
        ShaderSource::Glsl {
            vertex: "#version 100\nattribute vec3 position; attribute vec2 texcoord; attribute vec4 normal; uniform mat4 Model; uniform mat4 Projection; varying vec2 uv; varying vec4 color; void main(){gl_Position=Projection*Model*vec4(position,1.);uv=texcoord;color=vec4(1.,1.,1.,normal.x);}",
            fragment: "#version 100\nprecision mediump float; varying vec2 uv; varying vec4 color; uniform sampler2D Texture; void main(){float a=texture2D(Texture,uv).a*color.a; gl_FragColor=vec4(0.60,0.76,0.94,a*0.45);}",
        },
        MaterialParams {
            pipeline_params: PipelineParams {
                color_blend: Some(BlendState::new(
                    Equation::Add,
                    BlendFactor::Value(BlendValue::SourceAlpha),
                    BlendFactor::OneMinusValue(BlendValue::SourceAlpha),
                )),
                depth_write: false,
                depth_test: Comparison::LessOrEqual,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap()
}

/// The host's left-handed basis for a camera at `eye` looking at `at`.
fn basis(eye: V3, at: V3) -> (V3, V3) {
    let forward = (at - eye).normalize();
    let right = V3::Y.cross(forward).normalize();
    (right, forward.cross(right))
}

fn draw_water(vertices: &[WaterVertex], texture: Option<Texture2D>, mat: &Material) {
    // macroquad meshes index with u16 and batch at 10k vertices; the host uses 32-bit indices.
    gl_use_material(mat);
    for chunk in vertices.chunks(4 * 800) {
        let mut mesh = Mesh {
            vertices: Vec::new(),
            indices: Vec::new(),
            texture: texture.clone(),
        };
        for q in chunk.chunks_exact(4) {
            let b = mesh.vertices.len() as u16;
            for v in q {
                let mut vertex = Vertex::new(v.pos.x, v.pos.y, v.pos.z, v.uv[0], v.uv[1], WHITE);
                vertex.normal.x = v.alpha;
                mesh.vertices.push(vertex);
            }
            mesh.indices
                .extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
        }
        draw_mesh(&mesh);
    }
    gl_use_default_material();
}

#[derive(Clone, Copy)]
enum Shot {
    Side,
    HeadOn,
    Moving,
}

impl Shot {
    fn title(self) -> &'static str {
        match self {
            Shot::Side => "Side-on (static nozzle)",
            Shot::HeadOn => "Head-on (spray at the camera)",
            Shot::Moving => "While the nozzle moves",
        }
    }
    fn eye(self) -> V3 {
        match self {
            Shot::Side | Shot::Moving => V3::new(650., 150., 320.),
            Shot::HeadOn => V3::new(0., 150., 1200.),
        }
    }
    fn at(self) -> V3 {
        match self {
            Shot::HeadOn => V3::new(0., 150., 420.),
            _ => V3::new(0., 150., 320.),
        }
    }
}

#[macroquad::main(config)]
async fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let dir = std::path::Path::new(args.first().map_or("assets", String::as_str));
    let out = args
        .get(1)
        .map_or("output/water-views.png", String::as_str);
    let mut look = WaterLook::load(dir);
    assert!(!look.drop.rgba.is_empty(), "exact watermask asset missing");
    let drop_tex = Texture2D::from_rgba8(
        look.drop.width as u16,
        look.drop.height as u16,
        &look.drop.rgba,
    );
    drop_tex.set_filter(FilterMode::Linear);
    let mat = material();

    let emit = |pos: V3| WaterEmit {
        nozzle: NozzleKind::Spray,
        pos,
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
    };

    let mut water = WaterManager::new(&Params::from_dir(&dir.join("params_mario")).0);
    water.reach = 2.0;
    water.spray_speed = 1.2;
    let mut moving = WaterManager::new(&Params::from_dir(&dir.join("params_mario")).0);
    moving.reach = 2.0;
    moving.spray_speed = 1.2;

    // Both sprays are the same stream; only the second nozzle translates sideways.
    let mut sides = Vec::new();
    for frame in 0..20 {
        water.emit(&emit(V3::new(0., 150., 0.)));
        water.step(&FlatHost::default(), V3::ZERO);
        moving.emit(&emit(V3::new(frame as f32 * 6., 150., 0.)));
        moving.step(&FlatHost::default(), V3::ZERO);
    }

    for (manager, shot, cohesive) in [
        (&water, Shot::Side, false),
        (&water, Shot::Side, true),
        (&water, Shot::HeadOn, false),
        (&water, Shot::HeadOn, true),
        (&moving, Shot::Moving, false),
        (&moving, Shot::Moving, true),
    ] {
        let (right, up) = basis(shot.eye(), shot.at());
        look.cohesive = cohesive;
        look.nozzle = Some(V3::new(0., 150., 0.));
        look.build(manager, -0.5, shot.eye(), right, up, 0);
        let vertices = look.vertices[..look.drop_quads as usize * 4].to_vec();
        sides.push((shot, cohesive, look.stream_quads > 0, vertices));
    }

    let stem = out.trim_end_matches(".png");
    for (i, (shot, cohesive, merged, vertices)) in sides.iter().enumerate() {
        let label = if *merged {
            "cpu-merged surface (rejected)"
        } else if *cohesive {
            "cohesive requested -> droplets (head-on fallback)"
        } else {
            "exact droplets (shipped)"
        };
        for pass in 0..2 {
            clear_background(Color::from_rgba(18, 26, 35, 255));
            let camera = Camera3D {
                position: vec3(shot.eye().x, shot.eye().y, shot.eye().z),
                target: vec3(shot.at().x, shot.at().y, shot.at().z),
                up: vec3(0., 1., 0.),
                fovy: 45f32.to_radians(),
                ..Default::default()
            };
            set_camera(&camera);
            draw_grid(
                10,
                50.,
                Color::from_rgba(40, 55, 68, 255),
                Color::from_rgba(34, 46, 58, 255),
            );
            draw_water(
                vertices,
                if *merged { None } else { Some(drop_tex.clone()) },
                &mat,
            );
            set_default_camera();
            draw_text(
                "FLUDD Spray | type 0 watermask | tint-only, host camera basis",
                15.,
                28.,
                24.,
                WHITE,
            );
            draw_text(
                &format!("{} | {label}", shot.title()),
                15.,
                54.,
                25.,
                WHITE,
            );
            if pass == 1 {
                get_screen_data().export_png(&format!("{stem}-{i}.png"));
            }
            next_frame().await;
        }
    }
}
