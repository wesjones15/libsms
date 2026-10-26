//! Placement proof for RoR2 SurvivorPod's exact Flames mesh emitter, not a JPA substitute.
//! Local inputs: donut5Mesh OBJ and matEntryBurn textures exported from the installed game.
//! Cloud shader preview is approximate; the adapter retains the original Unity material.
use macroquad::{miniquad::*, prelude::*};
use std::path::Path;
pub struct Fire {
    positions: Vec<Vec3>,
    uvs: Vec<Vec2>,
    indices: Vec<u16>,
    material: Material,
    _textures: Vec<Texture2D>,
}
impl Fire {
    pub async fn load(dir: &Path) -> Self {
        let text = std::fs::read_to_string(dir.join("flame.obj")).unwrap();
        let mut positions = Vec::new();
        let mut uvs = Vec::new();
        let mut indices = Vec::new();
        for line in text.lines() {
            let mut words = line.split_whitespace();
            match words.next() {
                Some("v") => {
                    let v: Vec<f32> = words.map(|w| w.parse().unwrap()).collect();
                    positions.push(vec3(v[0], v[1], v[2]));
                }
                Some("vt") => {
                    let v: Vec<f32> = words.map(|w| w.parse().unwrap()).collect();
                    uvs.push(vec2(v[0], 1. - v[1]));
                }
                Some("f") => indices.extend(
                    words.map(|w| w.split('/').next().unwrap().parse::<u16>().unwrap() - 1),
                ),
                _ => {}
            }
        }
        let material=load_material(ShaderSource::Glsl {
            vertex:"#version 100\nattribute vec3 position;attribute vec2 texcoord;attribute vec4 color0;varying mediump vec2 uv;varying lowp vec4 color;uniform mat4 Model;uniform mat4 Projection;void main(){gl_Position=Projection*Model*vec4(position,1.);uv=texcoord;color=color0/255.;}",
            fragment:"#version 100\nprecision mediump float;varying mediump vec2 uv;varying lowp vec4 color;uniform sampler2D Main;uniform sampler2D Cloud;uniform sampler2D Mask;uniform sampler2D Ramp;uniform float Time;void main(){float m=texture2D(Mask,fract(uv+vec2(Time*0.25,0.))).r;float n=texture2D(Main,fract(uv*vec2(5.,1.)+vec2(0.,-Time*1.5))).r*texture2D(Cloud,fract(uv)).r;float a=clamp((n+0.4)*m*color.a*2.,0.,1.);vec3 c=texture2D(Ramp,vec2(a,0.5)).rgb;gl_FragColor=vec4(c*a*1.63,a);}"
        },MaterialParams {textures:vec!["Main".into(),"Cloud".into(),"Mask".into(),"Ramp".into()],uniforms:vec![UniformDesc::new("Time",UniformType::Float1)],pipeline_params:PipelineParams {color_blend:Some(BlendState::new(Equation::Add,BlendFactor::One,BlendFactor::One)),depth_test:Comparison::LessOrEqual,depth_write:false,..Default::default()},..Default::default()}).unwrap();
        let mut textures = Vec::new();
        for (name, file) in [
            ("Main", "_MainTex.png"),
            ("Cloud", "_Cloud2Tex.png"),
            ("Mask", "_Cloud1Tex.png"),
            ("Ramp", "_RemapTex.png"),
        ] {
            let tex = load_texture(dir.join(file).to_str().unwrap())
                .await
                .unwrap();
            material.set_texture(name, tex.clone());
            textures.push(tex);
        }
        Self {
            positions,
            uvs,
            indices,
            material,
            _textures: textures,
        }
    }
    pub fn draw(&self, time: f32, center: Vec3) {
        self.material.set_uniform("Time", time);
        gl_use_material(&self.material);
        // Source emitter: 8/sec, lifetime .7..1, start sizes xy 1..1.3 and z 3..5.
        // Renderer pivot z=-1.2 and source rotation map local +Z down the fall axis.
        for i in 0..=(time * 8.) as u32 {
            let age = time - i as f32 / 8.;
            let random = ((i * 17 + 3) % 31) as f32 / 31.;
            let life = 0.7 + 0.3 * random;
            if age > life {
                continue;
            }
            let u = age / life;
            let smooth = u * u * (3. - 2. * u);
            let xy = (1. + 0.3 * random) * (0.82784 + 0.17216 * smooth);
            let z = (3. + 2. * random) * (1. - u);
            let alpha = if u < 0.2912 {
                u / 0.2912
            } else {
                (1. - u) / (1. - 0.2912)
            };
            let vertices = self
                .positions
                .iter()
                .zip(&self.uvs)
                .map(|(p, uv)| {
                    let v = vec3(p.x * xy, -(p.z - 1.2) * z, -p.y * xy) * 3.5 + center;
                    Vertex::new(v.x, v.y, v.z, uv.x, uv.y, Color::new(1., 1., 1., alpha))
                })
                .collect();
            draw_mesh(&Mesh {
                vertices,
                indices: self.indices.clone(),
                texture: None,
            });
        }
        gl_use_default_material();
    }
}
