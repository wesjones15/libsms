//! Mario's life meter, the Shine Sprite sun in the HUD's top right, drawn as `TGCConsole2` draws
//! it (GC2D/GCConsole2.cpp) into one small RGBA image for a host to put on screen or on a
//! billboard.
//!
//! The disc draws it from pieces (`game_6/scrn/standard_1.blo`, the `'\0l_0'` pane): a hub (the
//! spiral, `lm0x`) and eight rays (`lm1x..lm8x`), each a picture of its own in two layers: `_2`
//! the grey slot behind and `_1` the lit piece in front (`game_6/timg/life_icon_<n>_<layer>.bti`).
//! The slots stay; a ray is lit while Mario's health reaches its number (`startInsertLife`,
//! `resetLife`: `mHealth + 1 > i`, the hub always), and the hub dims to half grey under four
//! (`updateLifeMeterColors`). Behind them is the red splash (`l_ba`, `water_back.bti`) and the
//! "LIFE" text (`l_tx`).
//!
//! The eight rays sit around the hub 33 to 39 degrees apart with a hundred degrees left empty
//! on top, where the text goes. A host that wants the sun without the text can have it whole:
//! `segments` 10 fills the gap with two more rays, each the nearest game ray turned about the
//! hub. Not drawn: the meter's slide in and out and the pulse of the rays when health is low
//! (`updateLifeMeterBlink`); the underwater air meter is the same pieces in green, and a host's
//! shield here takes those colours (`updateLifeMeterColors`' air mode).

use std::path::Path;

use libsms_assets::Image;
use libsms_assets::blo::{Screen, tag};

use crate::hud::{rgba, texel};

/// One picture at its place on the HUD's screen.
#[derive(Debug, Clone)]
struct Pic {
    image: Image,
    at: [i32; 2],
    black: u32,
    white: u32,
}

impl Pic {
    fn center(&self) -> [f32; 2] {
        [
            self.at[0] as f32 + self.image.width as f32 * 0.5,
            self.at[1] as f32 + self.image.height as f32 * 0.5,
        ]
    }
}

/// A ray in the ring: which game ray it is made from and how far it is turned about the hub.
#[derive(Debug, Clone, Copy)]
struct Slot {
    from: usize,
    turn: f32,
}

/// What the meter shows: shares of the whole meter, health first and the shield after it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Life {
    /// 0..1: rays lit in the game's colours (at least one while above 0).
    pub health: f32,
    /// 0..1: rays after those in the air meter's cyan; health + shield is clamped to 1.
    pub shield: f32,
}

impl Life {
    pub const FULL: Life = Life {
        health: 1.0,
        shield: 0.0,
    };
}

/// The air meter's pieces' colours (`startInsertLife(1)`): the texture's 0 and 1 (RGBA).
const SHIELD_BLACK: u32 = 0x003C_FF00;
const SHIELD_WHITE: u32 = 0x00FF_FFFF;
/// The hub and its slot when health is under half (`updateLifeMeterColors`).
const DIM_WHITE: u32 = 0x7F7F_7FFF;
/// The splash behind the sun (`startInsertLife(0)`).
const BACK_WHITE: u32 = 0xFF4C_00C8;
const BACK_BLACK: u32 = 0xFF4C_0000;

pub struct LifeMeter {
    hub: [Pic; 2],
    /// The game's eight rays, `[slot, lit]`.
    rays: [[Pic; 2]; 8],
    /// The ring, in the order the rays go out.
    ring: Vec<Slot>,
    back: Option<Pic>,
    text: Option<Pic>,
    /// Where the image's top-left is on the HUD's screen.
    pub origin: [i32; 2],
    /// The meter, straight (not premultiplied) RGBA, top row first.
    pub image: Image,
    /// Bumped whenever `image` changes.
    pub version: u32,
    shown: Option<(u8, u8)>,
    accum: Vec<[f32; 4]>,
}

impl LifeMeter {
    /// From `assets/game_6/`; `segments` 8 is the game's ring and anything else the filled one
    /// of 10. `with_hud` adds the red splash and the "LIFE" text. Err (with why) if the files
    /// aren't there.
    pub fn load(dir: &Path, segments: u8, with_hud: bool) -> Result<LifeMeter, String> {
        let root = dir.join("game_6");
        let blo = std::fs::read(root.join("scrn/standard_1.blo"))
            .map_err(|e| format!("game_6/scrn/standard_1.blo: {e}"))?;
        let screen = Screen::parse(&blo).map_err(|e| e.to_string())?;
        let pic = |name: &str| -> Result<Pic, String> {
            let p = screen
                .find(tag(name))
                .ok_or_else(|| format!("no pane {name}"))?;
            let file = p
                .texture
                .as_deref()
                .ok_or_else(|| format!("pane {name} has no picture"))?;
            let d = std::fs::read(root.join("timg").join(file))
                .map_err(|e| format!("game_6/timg/{file}: {e}"))?;
            let t = libsms_assets::texture::read_bti(&d, file.to_owned())
                .map_err(|e| format!("{file}: {e}"))?;
            Ok(Pic {
                image: t.image,
                at: p.at,
                black: p.black,
                white: p.white,
            })
        };
        let hub = [pic("lm02")?, pic("lm01")?];
        let ray = |n: usize| -> Result<[Pic; 2], String> {
            Ok([pic(&format!("lm{n}2"))?, pic(&format!("lm{n}1"))?])
        };
        let rays = [
            ray(1)?,
            ray(2)?,
            ray(3)?,
            ray(4)?,
            ray(5)?,
            ray(6)?,
            ray(7)?,
            ray(8)?,
        ];
        let back = if with_hud {
            let mut p = pic("l_ba")?;
            p.black = BACK_BLACK;
            p.white = BACK_WHITE;
            Some(p)
        } else {
            None
        };
        let text = if with_hud { Some(pic("l_tx")?) } else { None };

        let c = hub[1].center();
        let angle = |p: &Pic| {
            let m = p.center();
            (m[1] - c[1]).atan2(m[0] - c[0])
        };
        let mut ring: Vec<(f32, Slot)> = (0..8)
            .map(|i| (angle(&rays[i][1]), Slot { from: i, turn: 0.0 }))
            .collect();
        if segments != 8 {
            // The empty arc on top (angles run clockwise on the screen, up is -90 degrees):
            // the biggest step between neighbours.
            let mut sorted: Vec<f32> = ring.iter().map(|r| r.0).collect();
            sorted.sort_by(f32::total_cmp);
            let (mut gap_from, mut gap) = (0.0f32, 0.0f32);
            for k in 0..sorted.len() {
                let (a, b) = (sorted[k], sorted[(k + 1) % sorted.len()]);
                let d = (b - a).rem_euclid(std::f32::consts::TAU);
                if d > gap {
                    gap = d;
                    gap_from = a;
                }
            }
            // Two more rays at the spacing the others keep, one in from each end.
            let step = gap / 3.0;
            for k in 1..=2 {
                let a = gap_from + step * k as f32;
                let near = ring
                    .iter()
                    .map(|r| (r.1.from, ang_diff(a, r.0)))
                    .min_by(|x, y| x.1.abs().total_cmp(&y.1.abs()))
                    .unwrap();
                ring.push((
                    a,
                    Slot {
                        from: near.0,
                        turn: near.1,
                    },
                ));
            }
        }
        // Lit from the top going clockwise, so the last to go is the one beside the gap.
        let top = -std::f32::consts::FRAC_PI_2;
        ring.sort_by(|a, b| {
            (a.0 - top)
                .rem_euclid(std::f32::consts::TAU)
                .total_cmp(&(b.0 - top).rem_euclid(std::f32::consts::TAU))
        });
        let ring: Vec<Slot> = ring.into_iter().map(|r| r.1).collect();

        // One image over everything the meter can cover, the turned rays' corners included.
        let mut lo = [f32::MAX; 2];
        let mut hi = [f32::MIN; 2];
        let mut cover = |x: f32, y: f32| {
            lo[0] = lo[0].min(x);
            lo[1] = lo[1].min(y);
            hi[0] = hi[0].max(x);
            hi[1] = hi[1].max(y);
        };
        let mut corners = |p: &Pic, turn: f32| {
            let (w, h) = (p.image.width as f32, p.image.height as f32);
            for (dx, dy) in [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)] {
                let (x, y) = turned(
                    [p.at[0] as f32 + dx, p.at[1] as f32 + dy],
                    c,
                    turn,
                );
                cover(x, y);
            }
        };
        corners(&hub[0], 0.0);
        for s in &ring {
            corners(&rays[s.from][0], s.turn);
        }
        for p in back.iter().chain(text.iter()) {
            corners(p, 0.0);
        }
        let origin = [lo[0].floor() as i32 - 1, lo[1].floor() as i32 - 1];
        let (w, h) = (
            (hi[0].ceil() as i32 + 1 - origin[0]) as u32,
            (hi[1].ceil() as i32 + 1 - origin[1]) as u32,
        );
        Ok(LifeMeter {
            hub,
            rays,
            ring,
            back,
            text,
            origin,
            image: Image {
                width: w,
                height: h,
                rgba: vec![0; (w * h * 4) as usize],
            },
            version: 0,
            shown: None,
            accum: vec![[0.0; 4]; (w * h) as usize],
        })
    }

    /// How many rays the ring has.
    pub fn segments(&self) -> usize {
        self.ring.len()
    }

    /// The rays lit for a share of health and the ones after them for a share of shield.
    pub fn lit(&self, life: Life) -> (u8, u8) {
        let n = self.ring.len() as f32;
        let health = life.health.clamp(0.0, 1.0);
        let both = (life.health.max(0.0) + life.shield.max(0.0)).clamp(0.0, 1.0);
        let lit = if health > 0.0 {
            ((health * n - 1e-3).ceil().max(1.0)) as u8
        } else {
            0
        };
        let all = ((both * n - 1e-3).ceil() as u8).max(lit);
        (lit, all - lit)
    }

    /// Redraws the meter for `life` if it shows something else now; true when it did.
    pub fn update(&mut self, life: Life) -> bool {
        let key = self.lit(life);
        if self.shown == Some(key) {
            return false;
        }
        self.shown = Some(key);
        self.draw(key.0 as usize, key.1 as usize);
        self.version = self.version.wrapping_add(1);
        true
    }

    fn draw(&mut self, lit: usize, shield: usize) {
        self.accum.fill([0.0; 4]);
        let c = self.hub[1].center();
        if let Some(back) = self.back.clone() {
            self.put(&back, c, 0.0, back.black, back.white);
        }
        // Under half the rays, the hub dims (the game counts the eight).
        let dim = lit * 8 < 4 * self.ring.len() && lit > 0 || lit == 0;
        let hub_white = if dim { DIM_WHITE } else { self.hub[1].white };
        let hub_black = self.hub[1].black;
        let slots = self.ring.clone();
        let (back, front) = (self.hub[0].clone(), self.hub[1].clone());
        self.put(&back, c, 0.0, hub_black, hub_white);
        for s in &slots {
            let p = self.rays[s.from][0].clone();
            self.put(&p, c, s.turn, p.black, p.white);
        }
        if lit > 0 || shield > 0 {
            self.put(&front, c, 0.0, hub_black, hub_white);
        }
        for (i, s) in slots.iter().enumerate() {
            let p = self.rays[s.from][1].clone();
            if i < lit {
                self.put(&p, c, s.turn, p.black, p.white);
            } else if i < lit + shield {
                self.put(&p, c, s.turn, SHIELD_BLACK, SHIELD_WHITE);
            }
        }
        if let Some(t) = self.text.clone() {
            self.put(&t, c, 0.0, t.black, t.white);
        }
        for (px, out) in self.accum.iter().zip(self.image.rgba.chunks_exact_mut(4)) {
            let a = px[3];
            for k in 0..3 {
                out[k] = if a > 0.0 {
                    (px[k] / a * 255.0).round().clamp(0.0, 255.0) as u8
                } else {
                    0
                };
            }
            out[3] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }

    /// Lays a straight colour `rgb` (0..1) at `alpha` over screen pixel `(x, y)`.
    fn blend(&mut self, x: i32, y: i32, rgb: [f32; 3], alpha: f32) {
        let (x, y) = (x - self.origin[0], y - self.origin[1]);
        let (w, h) = (self.image.width as i32, self.image.height as i32);
        if alpha <= 0.0 || x < 0 || y < 0 || x >= w || y >= h {
            return;
        }
        let d = &mut self.accum[(y * w + x) as usize];
        let keep = 1.0 - alpha;
        for k in 0..3 {
            d[k] = rgb[k] * alpha + d[k] * keep;
        }
        d[3] = alpha + d[3] * keep;
    }

    /// `J2DPicture::draw`: the texture between `black` and `white`, the picture turned `turn`
    /// radians about `pivot` (screen coordinates, clockwise).
    fn put(&mut self, p: &Pic, pivot: [f32; 2], turn: f32, black: u32, white: u32) {
        let (b, w) = (rgba(black), rgba(white));
        let shade = |t: [f32; 4]| {
            (
                [0, 1, 2].map(|k| b[k] + (w[k] - b[k]) * t[k]),
                b[3] + (w[3] - b[3]) * t[3],
            )
        };
        if turn == 0.0 {
            for y in 0..p.image.height {
                for x in 0..p.image.width {
                    let (rgb, a) = shade(texel(&p.image, x, y));
                    self.blend(p.at[0] + x as i32, p.at[1] + y as i32, rgb, a);
                }
            }
            return;
        }
        // Each pixel of the turned box looks back at where it came from in the picture.
        let (w_, h_) = (p.image.width as f32, p.image.height as f32);
        let corners = [(0.0, 0.0), (w_, 0.0), (0.0, h_), (w_, h_)]
            .map(|(dx, dy)| turned([p.at[0] as f32 + dx, p.at[1] as f32 + dy], pivot, turn));
        let x0 = corners.iter().map(|c| c.0).fold(f32::MAX, f32::min).floor() as i32;
        let x1 = corners.iter().map(|c| c.0).fold(f32::MIN, f32::max).ceil() as i32;
        let y0 = corners.iter().map(|c| c.1).fold(f32::MAX, f32::min).floor() as i32;
        let y1 = corners.iter().map(|c| c.1).fold(f32::MIN, f32::max).ceil() as i32;
        for y in y0..y1 {
            for x in x0..x1 {
                let (sx, sy) = turned([x as f32 + 0.5, y as f32 + 0.5], pivot, -turn);
                let t = sample(&p.image, sx - p.at[0] as f32, sy - p.at[1] as f32);
                if t[3] <= 0.0 {
                    continue;
                }
                let (rgb, a) = shade(t);
                self.blend(x, y, rgb, a);
            }
        }
    }
}

/// `at` turned `by` radians clockwise (y down) about `pivot`.
fn turned(at: [f32; 2], pivot: [f32; 2], by: f32) -> (f32, f32) {
    let (s, c) = by.sin_cos();
    let (dx, dy) = (at[0] - pivot[0], at[1] - pivot[1]);
    (pivot[0] + dx * c - dy * s, pivot[1] + dx * s + dy * c)
}

/// `to - from` as a turn within half a circle.
fn ang_diff(to: f32, from: f32) -> f32 {
    (to - from + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

/// The picture at `(x, y)` in its own pixels, bilinear, straight colour (zero outside).
fn sample(img: &Image, x: f32, y: f32) -> [f32; 4] {
    let (w, h) = (img.width as i32, img.height as i32);
    let (px, py) = (x - 0.5, y - 0.5);
    let (x0, y0) = (px.floor(), py.floor());
    let (fx, fy) = (px - x0, py - y0);
    let at = |ix: i32, iy: i32| -> [f32; 4] {
        if ix < 0 || iy < 0 || ix >= w || iy >= h {
            return [0.0; 4];
        }
        let t = texel(img, ix as u32, iy as u32);
        [t[0] * t[3], t[1] * t[3], t[2] * t[3], t[3]]
    };
    let mut out = [0.0; 4];
    for (dx, dy, wt) in [
        (0, 0, (1.0 - fx) * (1.0 - fy)),
        (1, 0, fx * (1.0 - fy)),
        (0, 1, (1.0 - fx) * fy),
        (1, 1, fx * fy),
    ] {
        let s = at(x0 as i32 + dx, y0 as i32 + dy);
        for k in 0..4 {
            out[k] += s[k] * wt;
        }
    }
    if out[3] > 0.0 {
        for k in 0..3 {
            out[k] /= out[3];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meter(segments: u8, hud: bool) -> Option<LifeMeter> {
        let dir = libsms::assets::find(None)?;
        LifeMeter::load(&dir, segments, hud).ok()
    }

    fn opaque(m: &LifeMeter) -> usize {
        m.image.rgba.chunks_exact(4).filter(|p| p[3] > 200).count()
    }

    #[test]
    fn rays_light_one_by_one_and_the_game_has_eight() {
        let Some(mut m) = meter(8, false) else { return };
        assert_eq!(m.segments(), 8);
        let mut last = 0;
        for h in [0.0, 0.125, 0.5, 1.0] {
            m.update(Life {
                health: h,
                shield: 0.0,
            });
            let lit: Vec<&[u8]> = m.image.rgba.chunks_exact(4).collect();
            // Lit pieces are saturated yellow-orange (red high, blue low); slots are grey.
            let warm = lit
                .iter()
                .filter(|p| p[3] > 200 && p[0] > 200 && p[2] < 80)
                .count();
            assert!(warm >= last, "{h}: {warm} after {last}");
            last = warm;
        }
        assert!(last > 300, "{last}");
    }

    #[test]
    fn a_share_of_health_is_a_whole_number_of_rays() {
        let Some(m) = meter(8, false) else { return };
        let l = |health, shield| m.lit(Life { health, shield });
        assert_eq!(l(1.0, 0.0), (8, 0));
        assert_eq!(l(0.0, 0.0), (0, 0));
        // A sliver is still one ray.
        assert_eq!(l(0.01, 0.0), (1, 0));
        assert_eq!(l(0.5, 0.0), (4, 0));
        assert_eq!(l(0.51, 0.0), (5, 0));
        // The shield takes the rays after health's and the two never pass the ring.
        assert_eq!(l(0.5, 0.25), (4, 2));
        assert_eq!(l(0.9, 0.9), (8, 0));
    }

    #[test]
    fn filling_the_gap_adds_two_rays_on_top() {
        let (Some(game), Some(full)) = (meter(8, false), meter(10, false)) else {
            return;
        };
        assert_eq!(full.segments(), 10);
        let mut game = game;
        let mut full = full;
        game.update(Life::FULL);
        full.update(Life::FULL);
        // The new rays sit above the hub's middle, in the gap.
        let c = game.hub[1].center();
        let top_rows = |m: &LifeMeter| {
            let cy = (c[1] - m.origin[1] as f32) as u32 - 30;
            (0..cy.min(m.image.height))
                .flat_map(|y| {
                    (0..m.image.width).map(move |x| (x, y))
                })
                .filter(|&(x, y)| m.image.rgba[((y * m.image.width + x) * 4 + 3) as usize] > 200)
                .count()
        };
        println!("above the hub: game {}, filled {}", top_rows(&game), top_rows(&full));
        assert!(top_rows(&full) > top_rows(&game) + 100);
        assert!(opaque(&full) > opaque(&game));
    }

    #[test]
    fn the_hud_look_adds_the_splash_and_the_text() {
        let (Some(plain), Some(hud)) = (meter(8, false), meter(8, true)) else {
            return;
        };
        let (mut plain, mut hud) = (plain, hud);
        plain.update(Life::FULL);
        hud.update(Life::FULL);
        assert!(hud.image.width * hud.image.height > plain.image.width * plain.image.height);
        assert!(opaque(&hud) > opaque(&plain));
    }

    #[test]
    fn redraws_only_on_change() {
        let Some(mut m) = meter(8, false) else { return };
        assert!(m.update(Life::FULL));
        let v = m.version;
        assert!(!m.update(Life::FULL));
        // The same rays from a different share.
        assert!(!m.update(Life {
            health: 0.99,
            shield: 0.0
        }));
        assert_eq!(m.version, v);
        assert!(m.update(Life {
            health: 0.5,
            shield: 0.0
        }));
    }
}
