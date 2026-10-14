//! FLUDD's water for hosts that draw it themselves: [`libsms::water`]'s droplets as camera-facing
//! quads and `TSplashManager`'s splashes, in world space (sim units), with their two textures.
//!
//! The shapes are the viewer's (`crates/viewer/src/water.rs`): a flying drop is
//! `TModelWaterManager::calcDrawVtx`'s square of half-diagonal `size * 0.707` facing the camera,
//! pulled out along its screen-space velocity by `mExtension * 4` (`unk5D18`); a drop stuck to
//! the ground or a wall lies flat on it; a splash is a 16-frame sprite of `timg/splash.bti` that
//! rises, grows from 50 to 150 and fades (SplashManager.cpp). The game then melts overlapping
//! drops into one refracting body (`drawSilhouette`, `drawRefracAndSpec`); hosts without that
//! pass draw the drops see-through in [`WaterLook::tint`].

use std::path::Path;

use glam::Vec3;
use libsms::water::{SLOTS, State, WaterEvent, WaterManager};
use libsms_assets::texture::read_bti;

/// `unk5D18`: how far the stretched quad reaches along the velocity.
const STRETCH: f32 = 4.0;

/// One corner of a water quad. Quads are 4 corners in order (a fan from the first).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WaterVertex {
    pub pos: Vec3,
    pub uv: [f32; 2],
    /// The splashes fade; drops are 1.
    pub alpha: f32,
}

/// An RGBA8 image, rows top first.
#[derive(Debug, Clone, Default)]
pub struct WaterTexture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
struct Splash {
    pos: Vec3,
    speed: f32,
    life: u8,
}

#[derive(Debug, Clone, Default)]
pub struct WaterLook {
    /// The drop's coverage: white, with `timg/watermask.bti`'s soft round falloff as alpha.
    pub drop: WaterTexture,
    /// `timg/splash.bti` coloured as the game does.
    pub splash: WaterTexture,
    splashes: Vec<Splash>,
    /// This frame's quads: drops, splashes, then ground wetness (texture 0, dark alpha blend).
    pub vertices: Vec<WaterVertex>,
    pub drop_quads: u32,
    pub splash_quads: u32,
    pub wet_quads: u32,
    /// Colour and opacity to draw the drops in (`gModelWaterManagerWaterColor`; juice when
    /// Yoshi spits it).
    pub tint: [f32; 4],
}

fn bti(dir: &Path, rel: &str) -> Option<libsms_assets::texture::Texture> {
    let data = std::fs::read(dir.join(rel)).ok()?;
    read_bti(&data, rel.to_owned()).ok()
}

impl WaterLook {
    /// The textures from the extracted disc; missing ones stay empty (hosts draw untextured).
    pub fn load(dir: &Path) -> WaterLook {
        let mut look = WaterLook {
            tint: [0.60, 0.76, 0.94, 0.35],
            ..WaterLook::default()
        };
        if let Some(mask) = bti(dir, "timg/watermask.bti") {
            look.drop = WaterTexture {
                width: mask.image.width,
                height: mask.image.height,
                rgba: mask
                    .image
                    .rgba
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .flat_map(|m| [255, 255, 255, m[0]])
                    .collect(),
            };
        }
        if let Some(sp) = bti(dir, "timg/splash.bti") {
            // `GXSetTevColorIn(C0, ONE, TEXC, ZERO)`: (168, 203, 227) to white by the texture.
            let mut rgba = Vec::with_capacity(sp.image.rgba.len());
            for p in sp.image.rgba.as_chunks::<4>().0 {
                let t = p[0] as f32 / 255.0;
                let mix = |c: f32| ((c + (1.0 - c) * t) * 255.0) as u8;
                rgba.extend_from_slice(&[
                    mix(168.0 / 255.0),
                    mix(203.0 / 255.0),
                    mix(227.0 / 255.0),
                    p[3],
                ]);
            }
            look.splash = WaterTexture {
                width: sp.image.width,
                height: sp.image.height,
                rgba,
            };
        }
        look
    }

    /// One 30 Hz water frame: new splashes from the manager's events, then
    /// `TSplashManager::move`.
    pub fn step(&mut self, events: &[WaterEvent]) {
        for s in &mut self.splashes {
            // `unk638`: -0.5 * 2 * 2 per frame.
            s.speed -= 2.0;
            s.pos.y += s.speed;
            s.life = s.life.saturating_sub(1);
        }
        self.splashes.retain(|s| s.life > 0);
        for e in events {
            if let WaterEvent::Splash(p) = e
                && self.splashes.len() < 64
            {
                self.splashes.push(Splash {
                    pos: *p,
                    speed: 5.0,
                    life: 16,
                });
            }
        }
    }

    /// Builds this frame's quads facing a camera at `eye` with `right` and `up` (sim axes).
    /// `frac` is how far into the next 30 Hz water frame the display is (-1..1; below 0 when
    /// the host draws behind the sim): flying drops are drawn that far along their velocity. `juice` is [`libsms::Mario::juice_color`].
    pub fn build(
        &mut self,
        water: &WaterManager,
        frac: f32,
        eye: Vec3,
        right: Vec3,
        up: Vec3,
        juice: u8,
    ) {
        self.vertices.clear();
        self.tint = match juice {
            1..=3 => {
                let c = libsms::yoshi::JUICE_COLORS[juice as usize];
                [
                    c[0] as f32 / 255.0,
                    c[1] as f32 / 255.0,
                    c[2] as f32 / 255.0,
                    0.75,
                ]
            }
            _ => [0.60, 0.76, 0.94, 0.35],
        };
        let frac = frac.clamp(-1.0, 1.0);
        let uvs = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let push = |corners: [Vec3; 4], alpha: f32, out: &mut Vec<WaterVertex>| {
            for (pos, uv) in corners.into_iter().zip(uvs) {
                out.push(WaterVertex { pos, uv, alpha });
            }
        };

        for p in water.particles.iter().take(SLOTS) {
            let half = 1.414 * 0.5 * p.size;
            let corners = match p.state {
                State::Flying => {
                    if !water.drawn(p) {
                        continue;
                    }
                    let c = p.pos + p.vel * frac;
                    let at = |x: f32, y: f32| c + right * x + up * y;
                    let v = p.vel * water.extension(p);
                    let (vx, vy) = (v.dot(right), v.dot(up));
                    let len = (vx * vx + vy * vy).sqrt();
                    if len > 1.0 {
                        let (nx, ny) = (vx / len * half, vy / len * half);
                        [
                            at(nx + vx * STRETCH, ny + vy * STRETCH),
                            at(ny, -nx),
                            at(-nx - vx * STRETCH, -ny - vy * STRETCH),
                            at(-ny, nx),
                        ]
                    } else {
                        [
                            at(-half, half),
                            at(half, half),
                            at(half, -half),
                            at(-half, -half),
                        ]
                    }
                }
                // A flat drop on the surface it clings to.
                State::Ground | State::Wall => {
                    let c = p.pos;
                    let n = if p.state == State::Ground {
                        Vec3::Y
                    } else {
                        let d = (eye - c).normalize_or_zero();
                        Vec3::new(d.x, 0.0, d.z).normalize_or(Vec3::Z)
                    };
                    let a = n.any_orthonormal_vector();
                    let b = n.cross(a);
                    let r = half * 1.2;
                    let lift = n;
                    [
                        c + lift + (-a + b) * r,
                        c + lift + (a + b) * r,
                        c + lift + (a - b) * r,
                        c + lift + (-a - b) * r,
                    ]
                }
            };
            push(corners, 1.0, &mut self.vertices);
        }
        self.drop_quads = (self.vertices.len() / 4) as u32;

        for s in &self.splashes {
            let age = (16 - s.life) as f32 / 16.0;
            let half = age * 100.0 + 50.0;
            let at = |x: f32, y: f32| s.pos + right * x + up * y;
            push(
                [
                    at(-half, half),
                    at(half, half),
                    at(half, -half),
                    at(-half, -half),
                ],
                s.life as f32 / 16.0,
                &mut self.vertices,
            );
        }
        self.splash_quads = (self.vertices.len() / 4) as u32 - self.drop_quads;
        // Sunshine writes water coverage to the ground's pollution map. Hosts without that
        // map receive soft dark marks on sampled terrain, with the slide's exact footprint.
        let before = self.vertices.len();
        for patch in &water.wet_patches {
            let alpha = (patch.life / patch.duration.min(60.0)).clamp(0.0, 1.0);
            for y in 0..2 {
                for x in 0..2 {
                    let Some(corners) = patch.cell(x, y) else {
                        continue;
                    };
                    for (pos, uv) in corners.into_iter().zip([
                        [x as f32 * 0.5, y as f32 * 0.5],
                        [(x + 1) as f32 * 0.5, y as f32 * 0.5],
                        [(x + 1) as f32 * 0.5, (y + 1) as f32 * 0.5],
                        [x as f32 * 0.5, (y + 1) as f32 * 0.5],
                    ]) {
                        self.vertices.push(WaterVertex { pos, uv, alpha });
                    }
                }
            }
        }
        self.wet_quads = ((self.vertices.len() - before) / 4) as u32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libsms::{FlatHost, NozzleKind, Params, WaterEmit};

    #[test]
    fn a_spray_is_drawn_then_splashes() {
        let mut water = WaterManager::new(&Params::default());
        water.emit(&WaterEmit {
            nozzle: NozzleKind::Spray,
            pos: Vec3::new(0.0, 150.0, 0.0),
            dir: Vec3::Z,
            power: 40.0,
            // Each landing splashes half the time.
            count: 8,
            attack: 1,
            hit_radius: 50.0,
            hit_height: 80.0,
            inherit: Vec3::ZERO,
            size: 50.0,
            size_tremble: 0.0,
            dir_tremble: 0.2,
            pow_tremble: 0.0,
            particle_type: 0,
            burst: false,
        });
        let mut look = WaterLook::default();
        let (eye, right, up) = (Vec3::new(500.0, 150.0, 0.0), Vec3::Z, Vec3::Y);
        let mut drawn = false;
        let mut splashed = false;
        let host = FlatHost::default();
        for _ in 0..200 {
            water.step(&host, Vec3::ZERO);
            look.step(&water.events);
            water.events.clear();
            look.build(&water, 0.5, eye, right, up, 0);
            drawn |= look.drop_quads > 0;
            splashed |= look.splash_quads > 0;
            assert_eq!(
                look.vertices.len() as u32,
                4 * (look.drop_quads + look.splash_quads + look.wet_quads)
            );
        }
        assert!(drawn && splashed, "drawn {drawn} splashed {splashed}");
    }

    #[test]
    fn wet_terrain_is_camera_independent_and_fades_with_gameplay_wetness() {
        let mut water = WaterManager::new(&Params::default());
        water.emit(&WaterEmit {
            nozzle: NozzleKind::Spray,
            pos: Vec3::Y * 5.0,
            dir: -Vec3::Y,
            power: 40.0,
            count: 1,
            attack: 1,
            hit_radius: 50.0,
            hit_height: 80.0,
            inherit: Vec3::ZERO,
            size: 50.0,
            size_tremble: 0.0,
            dir_tremble: 0.0,
            pow_tremble: 0.0,
            particle_type: 0,
            burst: true,
        });
        water.step(&FlatHost::default(), Vec3::ZERO);
        water.particles.clear();
        let mut look = WaterLook::default();
        look.build(&water, 0.0, Vec3::Y * 500.0, Vec3::X, Vec3::Z, 0);
        assert_eq!(
            (look.drop_quads, look.splash_quads, look.wet_quads),
            (0, 0, 4)
        );
        let surface = look.vertices.clone();
        assert!(surface.iter().all(|v| v.pos.y == 1.0 && v.alpha == 1.0));
        look.build(&water, 0.0, Vec3::X * 500.0, Vec3::Z, Vec3::Y, 0);
        assert_eq!(look.vertices, surface);
        water.wet_patches[0].life = 30.0;
        look.build(&water, 0.0, Vec3::Y * 500.0, Vec3::X, Vec3::Z, 0);
        assert!(look.vertices.iter().all(|v| (v.alpha - 0.5).abs() < 0.001));
        water.wet_patches[0].life = 1.0;
        water.step(&FlatHost::default(), Vec3::ZERO);
        look.build(&water, 0.0, Vec3::Y * 500.0, Vec3::X, Vec3::Z, 0);
        assert_eq!(look.wet_quads, 0);
        assert!(!water.on_ground_near(Vec3::ZERO));
    }
}
