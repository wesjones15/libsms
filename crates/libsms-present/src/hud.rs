//! FLUDD's water gauge, the HUD's tank in the screen's bottom right, drawn as `TGCConsole2`
//! draws it (GC2D/GCConsole2.cpp) into one small RGBA image for the host to put on screen.
//!
//! The layout is the disc's (`game_6/scrn/standard_1.blo`, the `'\0w_0'` pane and its pictures,
//! placed as `TGCConsole2::loadAfter` detaches them) and so are the pictures (`game_6/timg/`).
//! Back to front, as `TGCConsole2::perform` draws them:
//! - `drawWaterBack`: the splash behind the tank (`w_ba`), blue; while a trigger nozzle charges
//!   (`TWaterGun::isPressureOn`) its lower part, as high as the charge, is red, flashing to
//!   yellow once it's full (`getPressureFlashColor`).
//! - the screen's own draw: the glass box (`w_t0`).
//! - `drawWater`: the water in the box's two faces (`w_t3` dark then `w_t2` light), each its
//!   face picture in a flat colour with the same picture as a mask slid down by the level
//!   (`unk2B8`, `updateWaterGaugeFill`): the water's top edge follows the face's slant. Then the
//!   water's top face (`w_t1`), lowered with the level and faint unless the tank is full or
//!   empty; the "WATER" text (`w_tx`) and the fitted nozzle's icon (`nz01..04`).
//!
//! Not drawn: the blinking X-button hint by the nozzle icon (`xb01..04`: hosts bind the swap
//! elsewhere), the gauge sliding in and out and the nozzle icon's swap animation
//! (`processMoveNozzle`), and Yoshi's juice gauge (riding, the water gauge shows his juice).

use std::path::Path;

use libsms::{Mario, NozzleKind};
use libsms_assets::Image;
use libsms_assets::blo::{Screen, tag};

/// The HUD's screen, in the game's 2D units (`J2DOrthoGraph` over the 600x480 screen).
pub const SCREEN: [i32; 2] = [600, 480];

/// What the gauge shows, read from the sim.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gauge {
    /// The tank, 0..1 (`updateWaterGaugeFill`; riding Yoshi, his juice).
    pub fill: f32,
    pub nozzle: NozzleKind,
    /// A trigger nozzle's charge and its maximum while it builds.
    pub pressure: Option<(f32, f32)>,
}

impl Gauge {
    /// None without FLUDD (the game shows the tank while Mario has it).
    pub fn of(m: &Mario) -> Option<Gauge> {
        let f = m.fludd.as_ref()?;
        let fill = if m.on_yoshi() {
            m.yoshi.juice as f32 / libsms::yoshi::JUICE_MAX as f32
        } else {
            f.level(&m.params)
        };
        Some(Gauge {
            fill: fill.clamp(0.0, 1.0),
            nozzle: f.nozzle,
            pressure: f.pressure(&m.params),
        })
    }
}

/// A picture at its place on the screen.
#[derive(Debug, Clone)]
struct Pic {
    image: Image,
    at: [i32; 2],
    black: u32,
    white: u32,
}

/// One of the water's two faces (`drawWater`'s layers).
#[derive(Debug, Clone)]
struct Face {
    pic: Pic,
    mask: Image,
    /// `0xRRGGBB`.
    color: u32,
    /// The share of the mask kept back at a full tank (`height`).
    height: f32,
    /// Its top below the glass's (`topDiff`).
    top: i32,
}

pub struct Hud {
    back: Pic,
    glass: Pic,
    surface: Pic,
    /// Drawn dark face first.
    faces: [Face; 2],
    text: Pic,
    /// Spray, Hover, Rocket, Turbo (`nz01..04`).
    nozzles: [Pic; 4],
    nozzle: usize,
    /// Where the image's top-left is on [`SCREEN`].
    pub origin: [i32; 2],
    /// The gauge, straight (not premultiplied) RGBA, top row first.
    pub image: Image,
    /// Bumped whenever `image` changes.
    pub version: u32,
    /// The charge flash's frame (`unk30C`), counting 30 Hz frames.
    flash: u8,
    shown: Option<(Gauge, usize, u8)>,
    accum: Vec<[f32; 4]>,
}

impl Hud {
    /// From `assets/game_6/`; None (with why) if it isn't there.
    pub fn load(dir: &Path) -> Result<Hud, String> {
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
        let glass = pic("w_t0")?;
        // `drawWater`'s two layers: 1 is `w_t2` (masked by `w_m2`), 2 is `w_t3`; the
        // colours are `unk9A`/`unk9E` (TGCConsole2's constructor), opaque.
        let face = |n: u8, color: u32, height: f32, top: i32| -> Result<Face, String> {
            Ok(Face {
                pic: pic(&format!("w_t{n}"))?,
                mask: pic(&format!("w_m{n}"))?.image,
                color,
                height,
                top: glass.at[1] + top,
            })
        };
        let faces = [face(3, 0x00B4F0, 0.13, 10)?, face(2, 0x64DCFF, 0.161, 7)?];
        let nozzles = [pic("nz01")?, pic("nz02")?, pic("nz03")?, pic("nz04")?];
        let back = pic("w_ba")?;
        let surface = pic("w_t1")?;
        let text = pic("w_tx")?;

        // One image over everything the gauge can cover (the top face moves down 47).
        let mut lo = [i32::MAX; 2];
        let mut hi = [i32::MIN; 2];
        let mut cover = |at: [i32; 2], img: &Image| {
            for k in 0..2 {
                lo[k] = lo[k].min(at[k]);
            }
            hi[0] = hi[0].max(at[0] + img.width as i32);
            hi[1] = hi[1].max(at[1] + img.height as i32);
        };
        for p in [&back, &glass, &text].into_iter().chain(nozzles.iter()) {
            cover(p.at, &p.image);
        }
        cover([surface.at[0], glass.at[1]], &surface.image);
        cover([surface.at[0], glass.at[1] + 48], &surface.image);
        for f in &faces {
            cover([f.pic.at[0], f.top], &f.pic.image);
        }
        let (w, h) = ((hi[0] - lo[0]) as u32, (hi[1] - lo[1]) as u32);
        Ok(Hud {
            back,
            glass,
            surface,
            faces,
            text,
            nozzles,
            nozzle: 0,
            origin: lo,
            image: Image {
                width: w,
                height: h,
                rgba: vec![0; (w * h * 4) as usize],
            },
            version: 0,
            flash: 0,
            shown: None,
            accum: vec![[0.0; 4]; (w * h) as usize],
        })
    }

    /// One of the HUD's four nozzle icons (`nz01..04`: Spray, Hover, Rocket, Turbo), coloured as
    /// the gauge draws it and on a transparent background. Hosts use these for a nozzle picker
    /// instead of loading the disc's pictures themselves.
    pub fn nozzle_icon(&self, index: usize) -> Option<Image> {
        let p = self.nozzles.get(index)?;
        let (b, w) = (rgba(p.black), rgba(p.white));
        let mut image = Image {
            width: p.image.width,
            height: p.image.height,
            rgba: vec![0; (p.image.width * p.image.height * 4) as usize],
        };
        for y in 0..p.image.height {
            for x in 0..p.image.width {
                let t = texel(&p.image, x, y);
                let i = ((y * p.image.width + x) * 4) as usize;
                for c in 0..3 {
                    let v = b[c] + (w[c] - b[c]) * t[c];
                    image.rgba[i + c] = (v * 255.0).round().clamp(0.0, 255.0) as u8;
                }
                let a = b[3] + (w[3] - b[3]) * t[3];
                image.rgba[i + 3] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
        Some(image)
    }

    /// Redraws the gauge for `g`, `frames` 30 Hz frames after the last call (the charge flash
    /// steps once a frame). Bumps `version` when the picture changed.
    pub fn update(&mut self, g: &Gauge, frames: u32) {
        // `updateWaterGaugeFill`: the icon for the fitted nozzle (Yoshi's leaves it).
        self.nozzle = match g.nozzle {
            NozzleKind::Spray => 0,
            NozzleKind::Hover | NozzleKind::Underwater => 1,
            NozzleKind::Rocket => 2,
            NozzleKind::Turbo => 3,
            NozzleKind::Yoshi => self.nozzle,
        };
        // `drawWaterBack`: the flash runs 0..24 while the charge is full, else rests at 0.
        match g.pressure {
            Some((p, max)) if p >= max => {
                for _ in 0..frames {
                    self.flash = if self.flash >= 24 { 0 } else { self.flash + 1 };
                }
            }
            _ => self.flash = 0,
        }
        let key = (*g, self.nozzle, self.flash);
        if self.shown == Some(key) {
            return;
        }
        self.shown = Some(key);
        self.draw(g);
        self.version = self.version.wrapping_add(1);
    }

    fn draw(&mut self, g: &Gauge) {
        self.accum.fill([0.0; 4]);
        self.draw_back(g);
        let glass = self.glass.clone();
        self.picture(&glass, glass.at, 1.0);
        for i in 0..2 {
            let f = self.faces[i].clone();
            self.face(&f, g.fill);
        }
        // The top face: `47 * (1 - fill) - 0.5` below the glass's top, plus one.
        let surface = self.surface.clone();
        let y = (self.glass.at[1] + (47.0 * (1.0 - g.fill) - 0.5) as i32 + 1).max(0);
        let alpha = if g.fill >= 1.0 {
            1.0
        } else if g.fill <= 0.0 {
            0.0
        } else {
            0x50 as f32 / 255.0
        };
        self.picture(&surface, [surface.at[0], y], alpha);
        let text = self.text.clone();
        self.picture(&text, text.at, 1.0);
        let nozzle = self.nozzles[self.nozzle].clone();
        self.picture(&nozzle, nozzle.at, 1.0);

        for (px, out) in self.accum.iter().zip(self.image.rgba.chunks_exact_mut(4)) {
            let a = px[3];
            for c in 0..3 {
                out[c] = if a > 0.0 {
                    (px[c] / a * 255.0).round().clamp(0.0, 255.0) as u8
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
        for c in 0..3 {
            d[c] = rgb[c] * alpha + d[c] * keep;
        }
        d[3] = alpha + d[3] * keep;
    }

    /// `J2DPicture::draw` at its texture's size: the texture between the pane's black and
    /// white colours, times `alpha`.
    fn picture(&mut self, p: &Pic, at: [i32; 2], alpha: f32) {
        let (b, w) = (rgba(p.black), rgba(p.white));
        for y in 0..p.image.height {
            for x in 0..p.image.width {
                let t = texel(&p.image, x, y);
                let rgb = [0, 1, 2].map(|c| b[c] + (w[c] - b[c]) * t[c]);
                let a = (b[3] + (w[3] - b[3]) * t[3]) * alpha;
                self.blend(at[0] + x as i32, at[1] + y as i32, rgb, a);
            }
        }
    }

    /// `drawWaterBack`: blue at 0x78 over the splash's alpha; below the charge's height,
    /// red (or the flash) at 0xC8.
    fn draw_back(&mut self, g: &Gauge) {
        let back = self.back.clone();
        let (w, h) = (back.image.width, back.image.height);
        let fill_top = g.pressure.map(|(p, max)| {
            let hidden = 1.0 - p / max.max(f32::EPSILON);
            (hidden * h as f32) as u32
        });
        let charged = rgba(pressure_color(self.flash));
        for y in 0..h {
            let lit = fill_top.is_some_and(|top| y >= top);
            let (rgb, a) = if lit {
                ([charged[0], charged[1], charged[2]], 0xC8 as f32 / 255.0)
            } else {
                ([0.0, 0.0, 1.0], 0x78 as f32 / 255.0)
            };
            for x in 0..w {
                let t = texel(&back.image, x, y);
                self.blend(back.at[0] + x as i32, back.at[1] + y as i32, rgb, a * t[3]);
            }
        }
    }

    /// One `drawWater` layer: the face's colour at `1 - mask` times the face's alpha, the mask
    /// slid `fill * (1 - height)` of its height down the face (texture matrix 0, clamped).
    fn face(&mut self, f: &Face, fill: f32) {
        let rgb = {
            let c = rgba(f.color << 8 | 0xFF);
            [c[0], c[1], c[2]]
        };
        let (w, h) = (f.pic.image.width, f.pic.image.height);
        let shift = fill * (1.0 - f.height);
        for y in 0..h {
            let v = (y as f32 + 0.5) / h as f32 + shift;
            for x in 0..w {
                let m = sample_v(&f.mask, x, v);
                let t = texel(&f.pic.image, x, y);
                self.blend(
                    f.pic.at[0] + x as i32,
                    f.top + y as i32,
                    rgb,
                    (1.0 - m) * t[3],
                );
            }
        }
    }
}

/// `getPressureFlashColor`: the charge's red, toward yellow over frames 0..9, yellow to 14,
/// back over 15..24 (RGBA).
fn pressure_color(frame: u8) -> u32 {
    let step = |n: f32| {
        let g = 0x3F + (n * 19.2) as i32;
        let b = 0x3F + (n * -6.3) as i32;
        0xFF00_0000 | (g.clamp(0, 255) as u32) << 16 | (b.clamp(0, 255) as u32) << 8
    };
    match frame {
        0..=9 => step(frame as f32),
        10..=14 => 0xFFFF_0000,
        15..=24 => step((25 - frame) as f32),
        _ => 0xFF3F_3F00,
    }
}

pub(crate) fn rgba(c: u32) -> [f32; 4] {
    c.to_be_bytes().map(|v| v as f32 / 255.0)
}

pub(crate) fn texel(img: &Image, x: u32, y: u32) -> [f32; 4] {
    let i = ((y * img.width + x) * 4) as usize;
    [0, 1, 2, 3].map(|c| img.rgba[i + c] as f32 / 255.0)
}

/// The alpha at column `x`, `v` (0..1 down the image) between rows, clamped at the edges.
fn sample_v(img: &Image, x: u32, v: f32) -> f32 {
    let h = img.height as i32;
    let py = v * h as f32 - 0.5;
    let y0 = py.floor();
    let t = py - y0;
    let row = |y: i32| texel(img, x, y.clamp(0, h - 1) as u32)[3];
    row(y0 as i32) * (1.0 - t) + row(y0 as i32 + 1) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hud() -> Option<Hud> {
        let dir = libsms::assets::find(None)?;
        Hud::load(&dir).ok()
    }

    fn gauge(fill: f32) -> Gauge {
        Gauge {
            fill,
            nozzle: NozzleKind::Spray,
            pressure: None,
        }
    }

    /// Opaque water-coloured pixels in the light face's column.
    fn water_rows(h: &Hud) -> usize {
        let f = &h.faces[1];
        let x = (f.pic.at[0] - h.origin[0]) as u32 + f.pic.image.width / 2;
        (0..h.image.height)
            .filter(|&y| {
                let i = ((y * h.image.width + x) * 4) as usize;
                let p = &h.image.rgba[i..i + 4];
                p[3] > 200 && p[2] > 200 && p[0] < 140 && p[1] > 180
            })
            .count()
    }

    #[test]
    fn the_water_rises_with_the_tank() {
        let Some(mut h) = hud() else { return };
        h.update(&gauge(0.0), 1);
        let empty = water_rows(&h);
        h.update(&gauge(0.5), 1);
        let half = water_rows(&h);
        h.update(&gauge(1.0), 1);
        let full = water_rows(&h);
        assert!(empty < half && half < full, "{empty} {half} {full}");
        assert!(full > 30, "{full}");
    }

    #[test]
    fn the_four_nozzle_icons_are_pictures_on_transparency() {
        let Some(h) = hud() else { return };
        for i in 0..4 {
            let icon = h.nozzle_icon(i).expect("HUD nozzle icon");
            assert!(icon.width > 0 && icon.height > 0);
            assert!(
                icon.rgba.chunks_exact(4).any(|p| p[3] > 0),
                "nozzle icon {i} is empty"
            );
            assert!(
                icon.rgba.chunks_exact(4).any(|p| p[3] == 0),
                "nozzle icon {i} has no transparent background"
            );
        }
        assert!(h.nozzle_icon(4).is_none());
    }

    #[test]
    fn redraws_only_on_change_and_flashes_a_full_charge() {
        let Some(mut h) = hud() else { return };
        h.update(&gauge(0.5), 1);
        let v = h.version;
        h.update(&gauge(0.5), 1);
        assert_eq!(h.version, v);
        let charged = Gauge {
            pressure: Some((10.0, 10.0)),
            ..gauge(0.5)
        };
        h.update(&charged, 1);
        h.update(&charged, 1);
        assert_eq!(h.version, v + 2);
        assert_eq!(pressure_color(12), 0xFFFF_0000);
        assert_eq!(pressure_color(0), 0xFF3F_3F00);
    }
}
