//! The counters of Sunshine's HUD, the yellow coin's (`c_*` panes), the blue coin's (`d_*`) and
//! Mario's lives (`m_*`, with the word "MARIO" over them), drawn as `TGCConsole2` lays them out (GC2D/GCConsole2.cpp, `standard_1.blo`) into one
//! small RGBA image each, for a host to put on screen: the coin's icon, the "x", and the number
//! in the HUD's own digit pictures (`game_6/timg/coin_number_<n>.bti`).
//!
//! The game counts coins to 999: two digits (with a leading zero) below 100, three from there, set
//! left to right on a line that climbs five pixels a digit (`setCounterDigits`,
//! `setBlueCoinDigits`); lives go to 99 (`setTwoDigits`). A host with bigger coin counts (a
//! roguelike's gold) gets more digits on the same line, up to [`MAX_DIGITS`]; the image is
//! always wide enough for all of them, with the number set from the left, so a host can pin its
//! left edge. Not drawn: the strip behind the
//! panes (`c_ba`, which the game slides in and out) and the counters' pulse as they change.

use std::path::Path;

use libsms_assets::Image;
use libsms_assets::blo::{Screen, tag};

use crate::hud::{rgba, texel};

/// Digits the counter can show.
pub const MAX_DIGITS: usize = 7;
/// The game's own digit count below which a zero pads the number (`value < 100`: two).
const MIN_DIGITS: usize = 2;

/// Which counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Yellow,
    Blue,
    /// Mario's lives: two digits, with "MARIO" over them.
    Mario,
}

impl Kind {
    fn prefix(self) -> &'static str {
        match self {
            Kind::Yellow => "c",
            Kind::Blue => "d",
            Kind::Mario => "m",
        }
    }

    fn digits(self) -> usize {
        match self {
            Kind::Mario => 2,
            _ => MAX_DIGITS,
        }
    }
}

/// One picture at its place on the HUD's screen.
#[derive(Debug, Clone)]
struct Pic {
    image: Image,
    at: [i32; 2],
    black: u32,
    white: u32,
}

pub struct CoinMeter {
    icon: Pic,
    x: Pic,
    /// The lives banner's "MARIO".
    text: Option<Pic>,
    digit_count: usize,
    digits: Vec<Image>,
    /// The first digit's top-left, and how far each next one sits from the one before.
    first: [i32; 2],
    step: [i32; 2],
    /// Where the image's top-left is on the HUD's 600x480 screen.
    pub origin: [i32; 2],
    /// The counter, straight (not premultiplied) RGBA, top row first.
    pub image: Image,
    /// Bumped whenever `image` changes.
    pub version: u32,
    shown: Option<u32>,
    accum: Vec<[f32; 4]>,
}

impl CoinMeter {
    /// From `assets/game_6/`. Err (with why) if the files aren't there.
    pub fn load(dir: &Path, kind: Kind) -> Result<CoinMeter, String> {
        let root = dir.join("game_6");
        let blo = std::fs::read(root.join("scrn/standard_1.blo"))
            .map_err(|e| format!("game_6/scrn/standard_1.blo: {e}"))?;
        let screen = Screen::parse(&blo).map_err(|e| e.to_string())?;
        let image = |file: &str| -> Result<Image, String> {
            let d = std::fs::read(root.join("timg").join(file))
                .map_err(|e| format!("game_6/timg/{file}: {e}"))?;
            Ok(libsms_assets::texture::read_bti(&d, file.to_owned())
                .map_err(|e| format!("{file}: {e}"))?
                .image)
        };
        let p = kind.prefix();
        let pic = |name: String| -> Result<(Pic, [i32; 2]), String> {
            let pane = screen
                .find(tag(&name))
                .ok_or_else(|| format!("no pane {}", name.trim_start_matches('\0')))?;
            let file = pane
                .texture
                .as_deref()
                .ok_or_else(|| format!("pane {} has no picture", name.trim_start_matches('\0')))?;
            Ok((
                Pic {
                    image: image(file)?,
                    at: pane.at,
                    black: pane.black,
                    white: pane.white,
                },
                pane.at,
            ))
        };
        let text = if kind == Kind::Mario {
            Some(pic("m_tx".to_owned())?.0)
        } else {
            None
        };
        let digit_count = kind.digits();
        let (icon, _) = pic(format!("{p}_ic"))?;
        let (x, _) = pic(format!("\0{p}_x"))?;
        let (_, first) = pic(format!("{p}_n1"))?;
        let (_, second) = pic(format!("{p}_n2"))?;
        let step = [second[0] - first[0], second[1] - first[1]];
        let digits = (0..10)
            .map(|n| image(&format!("coin_number_{n}.bti")))
            .collect::<Result<Vec<_>, _>>()?;
        let (dw, dh) = (digits[0].width as i32, digits[0].height as i32);

        // Every place a picture can land.
        let mut lo = [i32::MAX; 2];
        let mut hi = [i32::MIN; 2];
        let mut cover = |at: [i32; 2], w: i32, h: i32| {
            for k in 0..2 {
                lo[k] = lo[k].min(at[k]);
                hi[k] = hi[k].max(at[k] + if k == 0 { w } else { h });
            }
        };
        cover(icon.at, icon.image.width as i32, icon.image.height as i32);
        cover(x.at, x.image.width as i32, x.image.height as i32);
        for i in 0..digit_count as i32 {
            cover([first[0] + step[0] * i, first[1] + step[1] * i], dw, dh);
        }
        if let Some(t) = &text {
            cover(t.at, t.image.width as i32, t.image.height as i32);
        }
        let origin = [lo[0] - 1, lo[1] - 1];
        let (w, h) = ((hi[0] + 1 - origin[0]) as u32, (hi[1] + 1 - origin[1]) as u32);
        Ok(CoinMeter {
            icon,
            x,
            text,
            digit_count,
            digits,
            first,
            step,
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

    /// Redraws the counter for `value` (the most it can show clamped) if it shows something
    /// else now; true when it did.
    pub fn update(&mut self, value: u32) -> bool {
        let value = value.min(10u32.pow(self.digit_count as u32) - 1);
        if self.shown == Some(value) {
            return false;
        }
        self.shown = Some(value);
        self.draw(value);
        self.version = self.version.wrapping_add(1);
        true
    }

    fn draw(&mut self, value: u32) {
        self.accum.fill([0.0; 4]);
        if let Some(t) = self.text.clone() {
            self.put(&t.image, t.at, t.black, t.white);
        }
        let icon = self.icon.clone();
        self.put(&icon.image, icon.at, icon.black, icon.white);
        let x = self.x.clone();
        self.put(&x.image, x.at, x.black, x.white);
        let text = value.to_string();
        let count = text.len().max(MIN_DIGITS);
        let text = format!("{text:0>count$}");
        for (i, c) in text.bytes().enumerate() {
            let at = [
                self.first[0] + self.step[0] * i as i32,
                self.first[1] + self.step[1] * i as i32,
            ];
            let digit = self.digits[(c - b'0') as usize].clone();
            self.put(&digit, at, 0x0000_0000, 0xFFFF_FFFF);
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

    /// `J2DPicture::draw`: the texture between `black` and `white`, over what is there.
    fn put(&mut self, img: &Image, at: [i32; 2], black: u32, white: u32) {
        let (b, w) = (rgba(black), rgba(white));
        let (iw, ih) = (self.image.width as i32, self.image.height as i32);
        for y in 0..img.height {
            for x in 0..img.width {
                let t = texel(img, x, y);
                let a = b[3] + (w[3] - b[3]) * t[3];
                let (px, py) = (at[0] + x as i32 - self.origin[0], at[1] + y as i32 - self.origin[1]);
                if a <= 0.0 || px < 0 || py < 0 || px >= iw || py >= ih {
                    continue;
                }
                let d = &mut self.accum[(py * iw + px) as usize];
                let keep = 1.0 - a;
                for k in 0..3 {
                    d[k] = (b[k] + (w[k] - b[k]) * t[k]) * a + d[k] * keep;
                }
                d[3] = a + d[3] * keep;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meter(kind: Kind) -> Option<CoinMeter> {
        let dir = libsms::assets::find(None)?;
        CoinMeter::load(&dir, kind).ok()
    }

    fn lit(m: &CoinMeter) -> usize {
        m.image.rgba.chunks_exact(4).filter(|p| p[3] > 0).count()
    }

    #[test]
    fn digits_follow_the_value() {
        for kind in [Kind::Yellow, Kind::Blue] {
            let Some(mut m) = meter(kind) else { return };
            assert!(m.update(0));
            let zero = (lit(&m), m.image.rgba.clone());
            assert!(zero.0 > 0, "nothing drawn");
            assert!(!m.update(0), "same value redrew");
            assert!(m.update(7));
            assert_ne!(m.image.rgba, zero.1);
            // Three digits cover more than two, and the image doesn't grow.
            let (w, h) = (m.image.width, m.image.height);
            let two = lit(&m);
            m.update(123);
            assert!(lit(&m) > two);
            m.update(1_234_567);
            assert_eq!((m.image.width, m.image.height), (w, h));
            // Past what it can show, it holds the most.
            m.update(9_999_999);
            let most = m.image.rgba.clone();
            m.update(u32::MAX);
            assert!(m.image.rgba == most, "not clamped");
        }
    }

    #[test]
    fn lives_show_two_digits_and_the_word() {
        let (Some(mut lives), Some(mut yellow)) = (meter(Kind::Mario), meter(Kind::Yellow)) else {
            return;
        };
        assert!(lives.update(3));
        let three = lives.image.rgba.clone();
        lives.update(2);
        assert!(lives.image.rgba != three, "the number didn't change the picture");
        // Past 99 it holds 99.
        lives.update(99);
        let most = lives.image.rgba.clone();
        lives.update(1000);
        assert!(lives.image.rgba == most);
        // The word makes it wider than a yellow counter's icon and digits alone would be.
        yellow.update(3);
        assert_ne!((lives.image.width, lives.image.height), (0, 0));
    }

    #[test]
    fn the_blue_counter_is_blue_and_the_yellow_yellow() {
        let (Some(mut y), Some(mut b)) = (meter(Kind::Yellow), meter(Kind::Blue)) else {
            return;
        };
        y.update(5);
        b.update(5);
        // The icons differ, the digits (the same pictures) don't need to.
        assert_ne!(y.image.rgba, b.image.rgba);
        let tint = |m: &CoinMeter| {
            let (mut r, mut bl, mut n) = (0u64, 0u64, 0u64);
            for p in m.image.rgba.chunks_exact(4).filter(|p| p[3] > 200) {
                r += p[0] as u64;
                bl += p[2] as u64;
                n += 1;
            }
            (r / n.max(1), bl / n.max(1))
        };
        let (yr, yb) = tint(&y);
        let (br, bb) = tint(&b);
        eprintln!("yellow r{yr} b{yb}; blue r{br} b{bb}");
        assert!(yr > yb, "yellow isn't yellow");
        assert!(bb > br || bb > yb, "blue isn't blue");
    }
}
