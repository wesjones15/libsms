//! Fire damage proof: particle emitter 6, ms_firedmg_a.jpa.
//! Source: MarDirectorLoadResource.cpp (load 6), MarioReceiveMsg.cpp::receiveMessage,
//! MarioMove.cpp::checkGraffitoFire (emitAndBindToPosPtr at mPosition).
//! `cargo run --release -p viewer --example fire_panic -- out.png [seconds]`.

use std::io::Write;

use glam::Vec3;
use libsms::{Event, Host, Pad, Sim};
use libsms_present::View;
use libsms_present::particles::blend;

const W: usize = 512;
const H: usize = 512;

/// Flat ground, and the sim's events kept for the view.
struct Collect(Vec<Event>, Option<f32>);

impl Host for Collect {
    fn ground(&self, p: Vec3) -> Option<libsms::GroundHit> {
        Host::ground(&libsms::FlatHost::default(), p)
    }
    fn event(&mut self, e: Event) {
        self.0.push(e);
    }
    fn water_level(&self, _: Vec3) -> Option<f32> { self.1 }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().unwrap_or_else(|| "fire-panic.png".into());
    let secs: f32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(0.3);
    let dir = libsms::assets::find(None).expect("no assets/");
    let a = libsms::Assets::load(&dir);
    let mut view = View::load(&dir).expect("Mario's models");
    assert!(view.fx.has("ms_firedmg_a"), "emitter 6 missing");
    let mut sim = Sim::new(a.params, a.anim_table, Vec3::ZERO, 0);
    sim.mario.options.host_fire_reaction = true;
    let wet_demo = std::env::args().any(|a| a == "--wet");
    let mut water = libsms::water::WaterManager::new(&sim.mario.params);
    let mut water_view = libsms_present::water::WaterLook::load(&dir);
    let mut host = Collect(Vec::new(), wet_demo.then_some(10.0));
    let dt = 1.0 / 120.0;
    let pad = if wet_demo { Pad { stick: glam::Vec2::new(0.55, 0.0), ..Default::default() } } else { Pad::default() };
    sim.advance(&mut host, &pad, 0, dt);
    host.1 = None;
    if !wet_demo { sim.mario.ignite(); }
    let mut ended_at = None;
    for tick in 0..(secs * 120.0) as u32 {
        sim.advance_observed(&mut host, &pad, 0, dt, |mario, h| {
            let events = std::mem::take(&mut h.0);
            for event in &events {
                if let Event::BodyDrip { pos, velocity } = event {
                    water.emit_body_drip(*pos, *velocity);
                }
            }
            view.events(&events, mario);
        });
        if tick % 4 == 3 {
            water.step(&host, sim.mario.pos);
            water_view.step(&water.events);
            water.events.clear();
        }
        view.set_camera(Vec3::X, Vec3::Y);
        view.update(&mut sim, dt);
        if !sim.mario.on_fire() && ended_at.is_none() { ended_at = Some(sim.mario.ticks); }
    }
    water_view.build(&water, 0.0, sim.mario.pos + Vec3::Z * 1000.0, Vec3::X, Vec3::Y, 0);
    if wet_demo && secs > 0.1 && secs < 0.9 {
        assert!(water_view.drop_quads > 0, "vanilla body runoff was not drawn");
        assert!(water.particles.iter().all(|p| p.body_drip && p.kind == 1));
        assert!(water.contacts.is_empty(), "runoff must not keep re-wetting its emitter");
    }
    if std::env::args().any(|a| a == "--only-fire") {
        view.fx.build_filtered(Vec3::X, Vec3::Y, |name| name == "ms_firedmg_a");
        if ended_at.is_some_and(|t| sim.mario.ticks > t + 44) {
            assert!(view.fx.vertices.is_empty(), "fire tail outlived the 0.35-second fade");
        }
    }
    eprintln!("Emitter 6 ms_firedmg_a / {:?} / {} frame {}", sim.mario.status, sim.mario.anim.clip, sim.mario.anim.frame);
    let fx = &view.fx;
    eprintln!(
        "Fire after {secs} s: {} batches, {} corners, {} textures; Mario at {:?}",
        fx.batches.len(),
        fx.vertices.len(),
        fx.textures.len(),
        sim.mario.pos
    );
    // Side view: x right, y up, centred a metre above Mario's feet; 4 units a pixel.
    let centre = if std::env::args().any(|a| a == "--fixed-camera") {
        Vec3::new(0.0, 180.0, 0.0)
    } else { sim.mario.pos + Vec3::new(0.0, 100.0, 0.0) };
    let scale: f32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(0.9);
    let to_px = |p: Vec3| -> (f32, f32) {
        (
            W as f32 / 2.0 + (p.x - centre.x) * scale,
            H as f32 / 2.0 - (p.y - centre.y) * scale,
        )
    };
    let mut img = vec![[0.05f32, 0.06, 0.1]; W * H];
    let mut depth = vec![f32::NEG_INFINITY; W * H];
    let textures: Vec<_> = view.textures.list.iter().map(|t| libsms_present::particles::FxTexture {
        width: t.width, height: t.height, rgba: t.rgba.clone(), version: 0,
        wrap: t.wrap.map(|w| match w {
            0 => libsms_assets::texture::Wrap::Repeat,
            2 => libsms_assets::texture::Wrap::Mirror,
            _ => libsms_assets::texture::Wrap::Clamp,
        }),
    }).collect();
    let white = libsms_present::particles::FxTexture { width: 1, height: 1, rgba: vec![255; 4],
        wrap: [libsms_assets::texture::Wrap::Clamp; 2], version: 0 };
    for d in &view.draws {
        let mut first = d.first_vertex as usize;
        for (bi, batch) in view.models[d.model as usize].batches.iter().enumerate() {
            let color_index = d.first_color as usize + bi;
            let tex = usize::try_from(view.texture_ids[color_index]).ok()
                .and_then(|i| textures.get(i)).unwrap_or(&white);
            for tri in batch.indices.chunks_exact(3) {
                let verts = [tri[0], tri[1], tri[2]].map(|i| {
                    let i = first + i as usize;
                    libsms_present::particles::FxVertex {
                        pos: d.pos + d.rot * view.positions[i],
                        u: view.uvs[i][0], v: view.uvs[i][1], color: view.colors[color_index],
                    }
                });
                raster(&mut img, verts, tex, blend::ALPHA, &to_px, &mut depth, true);
            }
            first += batch.vertices.len();
        }
    }
    for b in &fx.batches {
        let tex = &fx.textures[b.texture as usize];
        let verts =
            &fx.vertices[b.first_vertex as usize..(b.first_vertex + b.vertex_count) as usize];
        for q in verts.chunks_exact(4) {
            for tri in [[0, 1, 2], [0, 2, 3]] {
                raster(&mut img, [q[tri[0]], q[tri[1]], q[tri[2]]], tex, b.blend, &to_px, &mut depth, false);
            }
        }
    }
    for (i, quad) in water_view.vertices.chunks_exact(4).enumerate() {
        let source = if i < water_view.drop_quads as usize { &water_view.drop } else { &water_view.splash };
        let tex = libsms_present::particles::FxTexture {
            width: source.width, height: source.height, rgba: source.rgba.clone(),
            wrap: [libsms_assets::texture::Wrap::Clamp; 2], version: 0,
        };
        let q: [_; 4] = std::array::from_fn(|i| { let v = quad[i]; libsms_present::particles::FxVertex {
            pos: v.pos, u: v.uv[0], v: v.uv[1], color: [water_view.tint[0], water_view.tint[1], water_view.tint[2], v.alpha * water_view.tint[3]],
        }});
        for tri in [[0, 1, 2], [0, 2, 3]] {
            raster(&mut img, [q[tri[0]], q[tri[1]], q[tri[2]]], &tex, blend::ALPHA, &to_px, &mut depth, false);
        }
    }
    let rgba: Vec<u8> = img
        .iter()
        .flat_map(|c| {
            [c[0], c[1], c[2]]
                .map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8)
                .into_iter()
                .chain([255])
        })
        .collect();
    std::fs::write(&out, png(W as u32, H as u32, &rgba)).unwrap();
    if wet_demo {
        std::fs::write(format!("{out}.txt"), format!("Vanilla wet runoff: model-water type 1 (WTP01_Wide), WetWaterEmit.prm; no JPA cue/model. {secs}s, {} droplets. Source: MarioInit.cpp::initValues WetWaterEmit allocation; MarioMove.cpp::thinkWaterSurface (120 ticks), checkWet (each 8 ticks, feet+5, velocity*0.3); ModelWaterManager.cpp::makeEmit. Disc: 3 droplets, life128, power5 +/-1, size15 +/-10, direction tremble0.5.\n", water_view.drop_quads)).unwrap();
        return;
    }
    std::fs::write(format!("{out}.txt"), format!(
        "Particle emitter 6 / ms_firedmg_a.jpa / {secs} seconds / {:?}\nAnimation: {} frame {}. firejmp = ANIM_FIREJMP 0x29, file table 0x0B; fjpend = ANIM_FJPEND 0x28, file table 0x0A. Source: MarioMove.cpp::setStatusToJumping (disc mFireDownForce=50); MarioJump.cpp::fireDowning (40% rebound, two bounces); MarioWait.cpp::fireJumpEnd. No panic run.\nSource: System/MarDirectorLoadResource.cpp load 6; Player/MarioReceiveMsg.cpp::receiveMessage; Player/MarioMove.cpp::checkGraffitoFire. Bound to mPosition, not the head. Textured Mario mesh and original particle batches rendered in Rust on the CPU.\n", sim.mario.status, sim.mario.anim.clip, sim.mario.anim.frame)).unwrap();
    eprintln!("wrote {out}");
}

fn raster(
    img: &mut [[f32; 3]],
    t: [libsms_present::particles::FxVertex; 3],
    tex: &libsms_present::particles::FxTexture,
    mode: u32,
    to_px: &dyn Fn(Vec3) -> (f32, f32),
    depth: &mut [f32],
    write_depth: bool,
) {
    let p: Vec<(f32, f32)> = t.iter().map(|v| to_px(v.pos)).collect();
    let area = (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[2].0 - p[0].0) * (p[1].1 - p[0].1);
    if area.abs() < 1e-6 {
        return;
    }
    let (x0, x1) = (
        p.iter().map(|q| q.0).fold(f32::MAX, f32::min).max(0.0) as usize,
        p.iter()
            .map(|q| q.0)
            .fold(f32::MIN, f32::max)
            .min(W as f32 - 1.0) as usize,
    );
    let (y0, y1) = (
        p.iter().map(|q| q.1).fold(f32::MAX, f32::min).max(0.0) as usize,
        p.iter()
            .map(|q| q.1)
            .fold(f32::MIN, f32::max)
            .min(H as f32 - 1.0) as usize,
    );
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let w0 = ((p[1].0 - px) * (p[2].1 - py) - (p[2].0 - px) * (p[1].1 - py)) / area;
            let w1 = ((p[2].0 - px) * (p[0].1 - py) - (p[0].0 - px) * (p[2].1 - py)) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            let u = w0 * t[0].u + w1 * t[1].u + w2 * t[2].u;
            let v = w0 * t[0].v + w1 * t[1].v + w2 * t[2].v;
            let c: Vec<f32> = (0..4)
                .map(|k| w0 * t[0].color[k] + w1 * t[1].color[k] + w2 * t[2].color[k])
                .collect();
            let s = tex.sample_nearest(u, v);
            let a = s[3] as f32 / 255.0 * c[3];
            if a < 0.01 { continue; }
            let z = w0 * t[0].pos.z + w1 * t[1].pos.z + w2 * t[2].pos.z;
            let index = y * W + x;
            if z < depth[index] { continue; }
            if write_depth { depth[index] = z; }
            let px = &mut img[y * W + x];
            for k in 0..3 {
                let src = s[k] as f32 / 255.0 * c[k];
                px[k] = if mode == blend::ADDITIVE {
                    px[k] + src * a
                } else {
                    px[k] * (1.0 - a) + src * a
                };
            }
        }
    }
}

/// A minimal PNG: RGBA8, zlib "stored" blocks (no compression).
fn png(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let mut raw = Vec::new();
    for row in rgba.chunks_exact(w as usize * 4) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (i, b) in blocks.iter().enumerate() {
        z.push((i + 1 == blocks.len()) as u8);
        z.extend_from_slice(&(b.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(b.len() as u16)).to_le_bytes());
        z.extend_from_slice(b);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&(b << 16 | a).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut chunk = |tag: &[u8], data: &[u8]| {
        out.write_all(&(data.len() as u32).to_be_bytes()).unwrap();
        let mut c = tag.to_vec();
        c.extend_from_slice(data);
        out.write_all(&c).unwrap();
        out.write_all(&crc(&c).to_be_bytes()).unwrap();
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    out
}

fn crc(d: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in d {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}
