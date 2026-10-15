//! The screen effects of losing: the "TOO BAD!" letters of a miss and the GAME OVER wipe, as lists
//! of pictures for a host to draw over its screen.
//!
//! "TOO BAD!" is `TConsoleStr::processMiss` (GC2D/ConsoleStr.cpp, `big_tx_1.blo`'s `ms00..ms06`
//! panes, `game_6/timg/big_tx_*.bti`, coloured letters): each letter, ten frames after the one
//! before, drops from above the screen turning once about itself while it fades in, jumps up,
//! settles back, waits, and falls out again. It plays over the scene, which opens from black
//! (`TMarDirector` state 7, `startWipe(10, 0, 2.2)`): [`Frame::fill`] is that black. Not drawn:
//! the star burst as each letter lands (`createEmitter(0x1F9)`).
//!
//! GAME OVER is `Hx_GameOver` (GC2D/hx_wiper.c): the picture `wipe_gameover.bti` (from the
//! disc's `common.szs` or `cmn2d_*.szs`; its intensity is its alpha) drawn in black through
//! `Hxs_GameOver` over the whole screen, so the screen goes black with the letters cut out of
//! it, the scene showing through them. Its texture coordinates are zoomed and turned: it spins
//! in from huge (`mag` growing by 0.074 a frame while `Hx_Motion` turns it two circles), pulls
//! back, and bounces. The black is one image per frame ([`Frame::overlay`]), worked out per pixel
//! as the GPU would with the texture clamped. At the end a red box (`Hx_GameOver` states 4 and 5:
//! inset 100 pixels, fading in over 32 frames, then solid for 100) comes in behind it, so the
//! cut-out letters fill with red ([`Frame::card`]).
//!
//! Both run at the game's 30 frames a second: [`ScreenFx::step`] takes seconds and runs the whole
//! frames that makes. Positions are on a 640 by 480 screen (the wipe's `hx.width` by
//! `hx.height`; the 600 wide HUD layout is centred on it).

use std::path::Path;

use libsms_assets::Image;
use libsms_assets::blo::{Screen, tag};

/// The screen the sprites' positions are on.
pub const SCREEN: [f32; 2] = [640.0, 480.0];
/// Game frames a second.
pub const FPS: f32 = 30.0;

/// Which effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    TooBad,
    GameOver,
}

/// One picture on the screen: `picture` index into [`ScreenFx::picture`], its middle and size in
/// screen pixels, turned `rotation` radians clockwise about its middle, drawn in `color` (the
/// picture's own colours times it; alpha included).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Sprite {
    pub picture: u32,
    pub center: [f32; 2],
    pub size: [f32; 2],
    pub rotation: f32,
    pub color: [f32; 4],
}

/// A flat rectangle on the screen: left, top, width, height, and its colour with alpha.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Card {
    pub rect: [f32; 4],
    pub color: [f32; 4],
}

/// What to draw this frame, back to front: the whole screen covered by black at `fill` (alpha),
/// the `card` (alpha 0 when there is none), the `overlay` (black pixels with their alpha, over the
/// middle 640 by 480 of the screen, and black at `edge` alpha where the screen goes past it),
/// then the sprites.
#[derive(Debug, Clone, Default)]
pub struct Frame {
    pub fill: f32,
    pub card: Card,
    pub overlay: Option<Image>,
    pub edge: f32,
    pub sprites: Vec<Sprite>,
}

struct Letter {
    picture: u32,
    /// Where it rests: its middle on the screen.
    at: [f32; 2],
    size: [f32; 2],
}

/// `img` times a gradient of four corner colours (RGBA; top left, top right, bottom left, bottom
/// right) over it, bilinear: a `J2DPicture` with `mCornerColor`.
fn tint(mut img: Image, corners: [u32; 4]) -> Image {
    let c = corners.map(|c| c.to_be_bytes().map(|v| v as f32 / 255.0));
    let (w, h) = (img.width as usize, img.height as usize);
    for y in 0..h {
        let t = (y as f32 + 0.5) / h as f32;
        for x in 0..w {
            let s = (x as f32 + 0.5) / w as f32;
            let px = &mut img.rgba[(y * w + x) * 4..][..4];
            for k in 0..4 {
                let m = c[0][k] * (1.0 - s) * (1.0 - t)
                    + c[1][k] * s * (1.0 - t)
                    + c[2][k] * (1.0 - s) * t
                    + c[3][k] * s * t;
                px[k] = (px[k] as f32 * m).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    img
}

/// `TBoundPane` motion: a quadratic Bezier from `p0` through `p1` to `p2`, `frames` updates long.
fn bezier(p0: f32, p1: f32, p2: f32, u: f32) -> f32 {
    p2 * u * u + p0 * (1.0 - u) * (1.0 - u) + p1 * (1.0 - u) * 2.0 * u
}

/// `Hx_MotionSet` / `Hx_MotionUpdate`: a distance covered over three stretches (speeding up,
/// steady, slowing down).
#[derive(Debug, Clone, Copy, Default)]
struct Motion {
    t1: f32,
    t2: f32,
    up: f32,
    down: f32,
    count: f32,
    speed: f32,
    pos: f32,
}

impl Motion {
    fn new(distance: f32, time1: f32, time2: f32, time3: f32) -> Motion {
        let velocity = 2.0 * distance / (time3 + (time2 + (time1 + time2)));
        Motion {
            t1: time1,
            t2: time1 + time2,
            up: if time1 != 0.0 { velocity / time1 } else { 0.0 },
            down: if time3 != 0.0 { -velocity / time3 } else { 0.0 },
            ..Motion::default()
        }
    }

    fn update(&mut self) -> f32 {
        if self.t1 > self.count {
            self.speed += self.up;
        } else if self.t2 <= self.count {
            self.speed += self.down;
        }
        self.count += 1.0;
        self.pos += self.speed;
        self.pos
    }
}

/// `Hx_GameOver`'s state.
#[derive(Debug, Clone, Default)]
struct GameOver {
    state: u8,
    timer: i32,
    mag: f32,
    rot: f32,
    fade: f32,
    motion: Motion,
    bound: usize,
    bound_timer: i32,
    bound_delta: f32,
    alpha: u8,
    /// Frames since the wipe ended.
    after: u32,
}

/// The overlay's size: the screen's half, which the GPU smooths up.
const OVERLAY: [u32; 2] = [320, 240];

/// `boundtable`: a change of `mag` a frame and for how many frames. The game's last pair
/// (`0.12f, -8.0f`) reads as no frames, ending the bounce there.
const BOUND: [f32; 8] = [-0.13, 6.0, 0.13, 6.0, -0.12, 8.0, 0.12, 0.0];

impl GameOver {
    fn new() -> GameOver {
        GameOver {
            mag: 0.3,
            timer: 50,
            state: 1,
            motion: Motion::new(-12.566_371, 10.0, 15.0, 25.0),
            ..GameOver::default()
        }
    }

    /// One game frame of the handler.
    fn tick(&mut self) {
        match self.state {
            1 => {
                if self.countdown() == 0 {
                    self.state = 2;
                    self.timer = 10;
                }
                self.mag += 0.074;
                self.rot = self.motion.update();
                self.fade += 5.1;
            }
            2 => {
                if self.countdown() == 0 {
                    self.state = 3;
                    self.bound_delta = BOUND[0];
                    self.bound_timer = BOUND[1] as i32;
                    self.bound = 2;
                }
                self.mag -= 0.1;
            }
            3 => {
                if self.bound_timer != 0 {
                    self.bound_timer -= 1;
                }
                if self.bound_timer == 0 {
                    self.bound_delta = BOUND[self.bound];
                    self.bound_timer = BOUND[self.bound + 1] as i32;
                    self.bound += 2;
                    if self.bound_timer == 0 {
                        self.state = 4;
                        self.timer = 32;
                        self.alpha = 255;
                        return;
                    }
                }
                self.mag += self.bound_delta;
            }
            4 => {
                if self.countdown() == 0 {
                    self.timer = 100;
                    self.state = 5;
                }
                self.alpha = self.alpha.wrapping_add(8);
            }
            5 => {
                if self.countdown() == 0 {
                    self.state = 6;
                }
            }
            _ => self.after += 1,
        }
    }

    fn countdown(&mut self) -> i32 {
        if self.timer != 0 {
            self.timer -= 1;
        }
        self.timer
    }
}

pub struct ScreenFx {
    pictures: Vec<Image>,
    letters: Vec<Letter>,
    /// `wipe_gameover.bti`'s picture index, with the disc's wipes.
    game_over_picture: Option<u32>,
    kind: Option<Kind>,
    /// Game frames since it started.
    frame: u32,
    /// Seconds not yet run as frames.
    spare: f32,
    game_over: GameOver,
    /// Bumped by every [`ScreenFx::start`] and whenever the frame changes.
    pub version: u32,
    now: Frame,
}

/// How long the black takes to open onto the scene (the wipe's 2.2 seconds).
const TOO_BAD_OPEN: u32 = 66;
/// Frames the whole "TOO BAD!" is shown for: the last letter settled (frame 200) and a moment,
/// fading over the last second.
const TOO_BAD_LENGTH: u32 = 215;
/// GAME OVER's black is held this long after its wipe ends, then fades.
const GAME_OVER_HOLD: u32 = 15;
const GAME_OVER_FADE: u32 = 20;

impl ScreenFx {
    /// From `assets/` (`game_6/` for the letters, `wipe/` for GAME OVER). The effects whose
    /// pictures aren't there are left out; Err only when neither is.
    pub fn load(dir: &Path) -> Result<ScreenFx, String> {
        let root = dir.join("game_6");
        let mut pictures: Vec<Image> = Vec::new();
        let mut letters = Vec::new();
        let read = |path: &Path, name: &str| -> Result<Image, String> {
            let d = std::fs::read(path).map_err(|e| format!("{name}: {e}"))?;
            Ok(libsms_assets::texture::read_bti(&d, name.to_owned())
                .map_err(|e| format!("{name}: {e}"))?
                .image)
        };
        let mut why = String::new();
        let blo = std::fs::read(root.join("scrn/big_tx_1.blo"))
            .map_err(|e| format!("game_6/scrn/big_tx_1.blo: {e}"))
            .and_then(|b| Screen::parse(&b).map_err(|e| e.to_string()));
        match blo {
            Ok(screen) => {
                for i in 0..7 {
                    let Some(p) = screen.find(tag(&format!("ms0{i}"))) else {
                        why = format!("no pane ms0{i}");
                        letters.clear();
                        break;
                    };
                    let Some(file) = p.texture.as_deref() else {
                        why = format!("pane ms0{i} has no picture");
                        letters.clear();
                        break;
                    };
                    // Each letter is its own picture: the texture times the pane's corner colours.
                    let index = match read(&root.join("timg").join(file), file) {
                        Ok(img) => {
                            pictures.push(tint(img, p.corners));
                            pictures.len() - 1
                        }
                        Err(e) => {
                            why = e;
                            letters.clear();
                            break;
                        }
                    };
                    // The HUD's 600 wide layout, centred on the 640 wide screen.
                    let (w, h) = (p.rect[2] as f32, p.rect[3] as f32);
                    letters.push(Letter {
                        picture: index as u32,
                        at: [p.at[0] as f32 + w * 0.5 + 20.0, p.at[1] as f32 + h * 0.5],
                        size: [w, h],
                    });
                }
            }
            Err(e) => why = e,
        }
        let game_over_picture = match read(&dir.join("wipe/wipe_gameover.bti"), "wipe/wipe_gameover.bti") {
            Ok(img) => {
                pictures.push(img);
                Some(pictures.len() as u32 - 1)
            }
            Err(e) => {
                if why.is_empty() {
                    why = e;
                }
                None
            }
        };
        if letters.is_empty() && game_over_picture.is_none() {
            return Err(why);
        }
        Ok(ScreenFx {
            pictures,
            letters,
            game_over_picture,
            kind: None,
            frame: 0,
            spare: 0.0,
            game_over: GameOver::new(),
            version: 0,
            now: Frame::default(),
        })
    }

    /// Whether `kind` has its pictures.
    pub fn has(&self, kind: Kind) -> bool {
        match kind {
            Kind::TooBad => !self.letters.is_empty(),
            Kind::GameOver => self.game_over_picture.is_some(),
        }
    }

    /// Picture `i` (straight RGBA, top row first).
    pub fn picture(&self, i: u32) -> Option<&Image> {
        self.pictures.get(i as usize)
    }

    pub fn pictures(&self) -> usize {
        self.pictures.len()
    }

    /// Starts `kind` over from its first frame. False if its pictures aren't there.
    pub fn start(&mut self, kind: Kind) -> bool {
        if !self.has(kind) {
            return false;
        }
        self.kind = Some(kind);
        self.frame = 0;
        self.spare = 0.0;
        self.game_over = GameOver::new();
        self.build();
        true
    }

    /// Stops what's showing.
    pub fn stop(&mut self) {
        if self.kind.take().is_some() {
            self.now = Frame::default();
            self.version = self.version.wrapping_add(1);
        }
    }

    /// What's running, if anything.
    pub fn kind(&self) -> Option<Kind> {
        self.kind
    }

    /// Whether the running effect has finished (or none runs).
    pub fn finished(&self) -> bool {
        match self.kind {
            None => true,
            Some(Kind::TooBad) => self.frame >= TOO_BAD_LENGTH,
            Some(Kind::GameOver) => {
                self.game_over.after >= GAME_OVER_HOLD + GAME_OVER_FADE
            }
        }
    }

    /// The running effect `seconds` on, in whole game frames.
    pub fn step(&mut self, seconds: f32) {
        if self.kind.is_none() || self.finished() {
            return;
        }
        self.spare += seconds.max(0.0) * FPS;
        let mut ran = false;
        while self.spare >= 1.0 && !self.finished() {
            self.spare -= 1.0;
            self.frame += 1;
            if self.kind == Some(Kind::GameOver) {
                self.game_over.tick();
            }
            ran = true;
        }
        if ran {
            self.build();
        }
    }

    /// This frame's pictures.
    pub fn frame(&self) -> &Frame {
        &self.now
    }

    fn build(&mut self) {
        self.version = self.version.wrapping_add(1);
        self.now = match self.kind {
            Some(Kind::TooBad) => self.too_bad(),
            Some(Kind::GameOver) => self.game_over_frame(),
            None => Frame::default(),
        };
    }

    fn too_bad(&self) -> Frame {
        let f = self.frame;
        let fill = (1.0 - f as f32 / TOO_BAD_OPEN as f32).max(0.0);
        // The letters leave with the frame count run out, fading over its last second.
        let leaving = ((TOO_BAD_LENGTH.saturating_sub(f)) as f32 / FPS).clamp(0.0, 1.0);
        let mut sprites = Vec::new();
        for (i, l) in self.letters.iter().enumerate() {
            let t = f as i32 - 10 * i as i32;
            if t < 0 {
                continue;
            }
            // processMiss: the stretches of each letter's own clock `t` (in frames).
            let (y, spin, alpha) = if t < 60 {
                let u = t as f32 / 60.0;
                // Turning once and fading in (12 of 255 a frame) while it drops.
                (
                    bezier(-270.0, -270.0, 30.0, u),
                    (-t as f32) * 6.0,
                    (12.0 * (t + 1) as f32 / 255.0).min(1.0),
                )
            } else if t < 100 {
                (bezier(30.0, -80.0, -80.0, (t - 60) as f32 / 40.0), 0.0, 1.0)
            } else if t < 140 {
                (bezier(-80.0, -80.0, 0.0, (t - 100) as f32 / 40.0), 0.0, 1.0)
            } else if t < 300 {
                (0.0, 0.0, 1.0)
            } else {
                (bezier(0.0, 0.0, 150.0, ((t - 300) as f32 / 30.0).min(1.0)), 0.0, 1.0)
            };
            sprites.push(Sprite {
                picture: l.picture,
                center: [l.at[0], l.at[1] + y],
                size: l.size,
                rotation: spin.to_radians(),
                color: [1.0, 1.0, 1.0, alpha * leaving],
            });
        }
        Frame {
            fill,
            card: Card::default(),
            overlay: None,
            edge: 0.0,
            sprites,
        }
    }

    fn game_over_frame(&self) -> Frame {
        let g = &self.game_over;
        let Some(picture) = self.game_over_picture else {
            return Frame::default();
        };
        let img = &self.pictures[picture as usize];
        // Let go at the end.
        let out = if g.state == 6 {
            let t = g.after.saturating_sub(GAME_OVER_HOLD) as f32 / GAME_OVER_FADE as f32;
            (1.0 - t).clamp(0.0, 1.0)
        } else {
            1.0
        };
        // `Hxs_GameOver(alpha, 2 mag, rot)`: alpha is the fade-in until the wipe has pulled back.
        let alpha = if g.state >= 2 { 1.0 } else { (g.fade / 255.0).clamp(0.0, 1.0) };
        let corners = game_over_uv(g.mag * 2.0, g.rot, img);
        let (w, h) = (OVERLAY[0] as usize, OVERLAY[1] as usize);
        let mut rgba = vec![0u8; w * h * 4];
        for y in 0..h {
            let t = (y as f32 + 0.5) / h as f32;
            for x in 0..w {
                let s = (x as f32 + 0.5) / w as f32;
                let mut uv = [0.0f32; 2];
                for k in 0..2 {
                    uv[k] = corners[0][k] * (1.0 - s) * (1.0 - t)
                        + corners[1][k] * s * (1.0 - t)
                        + corners[2][k] * s * t
                        + corners[3][k] * (1.0 - s) * t;
                }
                let a = sample_alpha(img, uv[0], uv[1]) * alpha * out;
                rgba[(y * w + x) * 4 + 3] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
        // The red box: 7, 15, ... 255 over state 4's 32 frames, then solid.
        let card = match g.state {
            4 | 5 | 6 => Card {
                rect: [100.0, 100.0, SCREEN[0] - 200.0, SCREEN[1] - 200.0],
                color: [
                    1.0,
                    0.0,
                    0.0,
                    if g.state == 4 { g.alpha as f32 / 255.0 } else { 1.0 } * out,
                ],
            },
            _ => Card::default(),
        };
        Frame {
            fill: 0.0,
            card,
            overlay: Some(Image {
                width: w as u32,
                height: h as u32,
                rgba,
            }),
            // Past the picture the texture is clamped to its border, which is black.
            edge: sample_alpha(img, -1.0, -1.0) * alpha * out,
            sprites: Vec::new(),
        }
    }
}

/// `Hxs_GameOver`'s texture coordinates for the screen's four corners (top left, top right,
/// bottom right, bottom left): a square about the middle of the picture turned `rot` radians and
/// `mag` times its half diagonal, its width stretched by `aspect` (the integer
/// `hx.width / hx.height` over the picture's own).
fn game_over_uv(mag: f32, rot: f32, img: &Image) -> [[f32; 2]; 4] {
    let aspect = (SCREEN[0] as i32 / SCREEN[1] as i32) as f32 / (img.width as f32 / img.height as f32);
    let d = std::f32::consts::FRAC_1_SQRT_2;
    let (s, c) = rot.sin_cos();
    let dir = [c * d - s * d, s * d + c * d];
    let len = mag * std::f32::consts::FRAC_1_SQRT_2;
    [
        [aspect * (-dir[0] * len) + 0.5, -dir[1] * len + 0.5],
        [aspect * (dir[1] * len) + 0.5, -dir[0] * len + 0.5],
        [aspect * (dir[0] * len) + 0.5, dir[1] * len + 0.5],
        [aspect * (-dir[1] * len) + 0.5, dir[0] * len + 0.5],
    ]
}

/// The picture's alpha at texture coordinates `(u, v)`, bilinear, clamped at its edges.
fn sample_alpha(img: &Image, u: f32, v: f32) -> f32 {
    let (w, h) = (img.width as i32, img.height as i32);
    let (px, py) = (u * w as f32 - 0.5, v * h as f32 - 0.5);
    let (x0, y0) = (px.floor(), py.floor());
    let (fx, fy) = (px - x0, py - y0);
    let at = |x: i32, y: i32| {
        let (x, y) = (x.clamp(0, w - 1) as usize, y.clamp(0, h - 1) as usize);
        img.rgba[(y * w as usize + x) * 4 + 3] as f32 / 255.0
    };
    let (x0, y0) = (x0 as i32, y0 as i32);
    at(x0, y0) * (1.0 - fx) * (1.0 - fy)
        + at(x0 + 1, y0) * fx * (1.0 - fy)
        + at(x0, y0 + 1) * (1.0 - fx) * fy
        + at(x0 + 1, y0 + 1) * fx * fy
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fx() -> Option<ScreenFx> {
        let dir = libsms::assets::find(None)?;
        ScreenFx::load(&dir).ok()
    }

    fn run(fx: &mut ScreenFx, frames: u32) {
        for _ in 0..frames {
            fx.step(1.0 / FPS);
        }
    }

    #[test]
    fn the_letters_drop_in_one_after_another_and_settle() {
        let Some(mut fx) = fx() else { return };
        if !fx.start(Kind::TooBad) {
            return;
        }
        assert_eq!(fx.frame().fill, 1.0);
        run(&mut fx, 12);
        // The first letter is in the air and the second has only begun.
        let early = fx.frame().sprites.len();
        assert!((1..=2).contains(&early), "{early}");
        run(&mut fx, 190);
        let f = fx.frame();
        assert_eq!(f.sprites.len(), 7);
        // At rest they stand on one line.
        let y0 = f.sprites[0].center[1];
        assert!(f.sprites.iter().all(|s| (s.center[1] - y0).abs() < 1.0 && s.rotation == 0.0));
        // The black has opened.
        assert_eq!(f.fill, 0.0);
        run(&mut fx, 30);
        assert!(fx.finished());
    }

    #[test]
    fn game_over_is_black_with_the_letters_cut_out() {
        let Some(mut fx) = fx() else { return };
        if !fx.start(Kind::GameOver) {
            return;
        }
        // Through the spin, the pull back and the bounce.
        run(&mut fx, 50 + 10 + 10);
        let f = fx.frame();
        let o = f.overlay.as_ref().expect("overlay");
        assert!(o.rgba.chunks_exact(4).all(|p| p[..3] == [0, 0, 0]));
        let alphas: Vec<u8> = o.rgba.chunks_exact(4).map(|p| p[3]).collect();
        let solid = alphas.iter().filter(|&&a| a > 240).count();
        let holes = alphas.iter().filter(|&&a| a < 16).count();
        assert!(solid > alphas.len() / 2, "{solid} solid");
        assert!(holes > alphas.len() / 100, "{holes} holes");
        assert!(f.edge > 0.9);
        // The red box comes at the end, fading in and then solid, and goes with the rest.
        assert_eq!(f.card.color[3], 0.0);
        run(&mut fx, 20 + 16);
        let half = fx.frame().card.color[3];
        assert!(half > 0.0 && half < 1.0, "{half}");
        run(&mut fx, 60);
        let card = fx.frame().card;
        assert_eq!(card.color, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(card.rect, [100.0, 100.0, 440.0, 280.0]);
        run(&mut fx, 400);
        assert!(fx.finished());
        assert_eq!(fx.frame().edge, 0.0);
    }

    #[test]
    fn the_motion_covers_its_distance() {
        let mut m = Motion::new(-12.566_371, 10.0, 15.0, 25.0);
        let mut last = 0.0;
        for _ in 0..50 {
            last = m.update();
        }
        assert!((last + 12.566_371).abs() < 0.5, "{last}");
    }
}
