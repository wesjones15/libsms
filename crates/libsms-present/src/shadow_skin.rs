//! Shadow Mario's own body (`kagemario/default.bmd`) and its swirling material, for
//! [`crate::view::Skin::Shadow`].
//!
//! The game (`TEMario::init`, emario.cpp) loads the model on Mario's 29 joints, starts
//! `kagemario_scroll.btk` on it and points its 8x8 `H_kagemario_dummy` at the frame's screen
//! copy (`gpScreenTexture->replace`). Every material runs the same two TEV stages:
//!
//! 1. the screen copy, read where the body is on screen but pushed about by an indirect warp
//!    (`H_kagemario_indwp4n_ia`, read through an environment-mapped texture matrix that the btk
//!    scrolls), goes through a ramp: `lerp(REG0, REG1, screen)`, dark blue to pale blue;
//! 2. `lerp(REG2, that, H_kagemario_main_i8)` plus the lit colour: the I8 picture keeps his
//!    features (eyes, cap brim, overalls) in REG2's navy where it is dark.
//!
//! So he is a moving, see-through-looking warp of whatever is behind him. A host that can read
//! its own screen may do exactly that with [`ShadowSwirl::params`]; for the rest, [`ShadowSwirl`]
//! bakes the same two stages into one texture over the body's own coordinates each game frame
//! (30 a second), with the disc's warp standing in for the screen: the warp read twice, the
//! second read pushed by the first and both scrolled by the btk, gives the same drifting
//! light-and-dark swirl in the ramp's colours. The host sees it as a texture whose version
//! changes ([`crate::view::Textures::version`]).
use std::path::Path;

use glam::Vec2;
use libcommon::texture::{Wrap, sample_rgba8_bilinear};
use libsms_assets::Bmd;
use libsms_assets::btk::TexSrt;

use crate::tev::Baked;

/// The model's file and its scroll (`TEMario::init`).
pub const MODEL: &str = "enemy/kagemario/default.bmd";
pub const SCROLL: &str = "enemy/kagemario/kagemario_scroll.btk";
/// The brush he holds in his left hand (`TEnemyMario::init`, enemyMario.cpp:
/// `/scene/kagemario/kagemario_brush.bmd`, `setBaseTRMtx(getAnmMtx(mJointIdHandL))`).
pub const BRUSH: &str = "enemy/kagemario/kagemario_brush.bmd";
/// `_mat_head`'s maps: the screen stand-in, the I8 picture and the warp.
const MAP_MAIN: usize = 1;
const MAP_WARP: usize = 2;
/// The warp's texture matrix (`texmtx 2`, the one `kagemario_scroll.btk` animates).
const WARP_MTX: usize = 2;
/// Side of the baked texture (the I8 picture's own size).
const SIZE: u32 = 128;
/// How far the warp pushes the second read, in texture widths (a stand-in for the indirect
/// matrix's reach across the screen, which has no meaning over the body's coordinates).
const PUSH: f32 = 0.35;

/// What a host shader needs to draw him as the game does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwirlParams {
    /// REG0, REG1: the ramp the warped screen goes through, dark to light.
    pub dark: [f32; 3],
    pub light: [f32; 3],
    /// REG2: where the I8 picture is dark.
    pub base: [f32; 3],
    /// The warp's texture matrix this frame: scale and translation (environment mapped: the
    /// view-space normal's x and y, halved and biased, through these).
    pub warp_scale: Vec2,
    pub warp_translation: Vec2,
}

pub struct ShadowSwirl {
    main: Option<(u32, u32, Vec<u8>)>,
    warp: (u32, u32, Vec<u8>),
    params: SwirlParams,
    base_translation: Vec2,
    scroll: Option<TexSrt>,
    /// The material the btk scrolls (its matrix is read back each frame).
    material: usize,
    /// The game frame last baked.
    frame: Option<u64>,
}

impl ShadowSwirl {
    /// From the loaded model (its maps and TEV registers) and the btk beside it. None when the
    /// model has no `_mat_head` with a warp.
    pub fn load(dir: &Path, bmd: &Bmd) -> Option<ShadowSwirl> {
        let material = bmd.material("_mat_head")?;
        let m = &bmd.materials[material];
        let image = |map: usize| {
            let t = &bmd.textures[*m.textures.get(map)?.as_ref()? as usize];
            Some((t.image.width, t.image.height, t.image.rgba.clone()))
        };
        let warp = image(MAP_WARP)?;
        let reg = |i: usize| [0, 1, 2].map(|c| (m.tev_colors[i][c] as f32 / 255.0).clamp(0.0, 1.0));
        let mtx = m.tex_mtx[WARP_MTX].as_ref();
        let scroll = std::fs::read(dir.join(SCROLL)).ok().and_then(|d| TexSrt::parse(&d).ok());
        Some(ShadowSwirl {
            main: image(MAP_MAIN),
            warp,
            params: SwirlParams {
                dark: reg(0),
                light: reg(1),
                base: reg(2),
                warp_scale: mtx.map_or(Vec2::ONE, |t| t.scale),
                warp_translation: mtx.map_or(Vec2::ZERO, |t| t.translation),
            },
            base_translation: mtx.map_or(Vec2::ZERO, |t| t.translation),
            scroll,
            material,
            frame: None,
        })
    }

    pub fn params(&self) -> SwirlParams {
        self.params
    }

    /// Steps the scroll to `time` seconds (the btk on `bmd`, the game's 30 frames a second)
    /// and returns the new bake when the game frame changed.
    pub fn step(&mut self, bmd: &mut Bmd, time: f32) -> Option<Baked> {
        let frame = (time.max(0.0) * 30.0) as u64;
        if self.frame == Some(frame) {
            return None;
        }
        self.frame = Some(frame);
        if let Some(s) = &self.scroll {
            s.apply(bmd, frame as f32);
            if let Some(Some(t)) = bmd.materials.get(self.material).map(|m| m.tex_mtx[WARP_MTX].as_ref()) {
                self.params.warp_scale = t.scale;
                self.params.warp_translation = t.translation;
            }
        } else {
            // No btk: drift as it would, about a texture a second.
            self.params.warp_translation = self.base_translation + Vec2::splat(frame as f32 / 30.0);
        }
        Some(self.bake())
    }

    /// The game frame last stepped to (0 before the first step).
    pub fn frame(&self) -> u64 {
        self.frame.unwrap_or(0)
    }

    /// The warp as a refraction map for a host's screen-grab shader, over the body's own
    /// coordinates at the current scroll: the warp's IA pair is the indirect offset
    /// (`GX_ITF_4`, a half bias; `_mat_head`'s indirect stage pushes the screen copy by it),
    /// packed as RoR2's distortion normals want them (`[255, -y, 255, x]`, a half bias, as
    /// `heat_haze` packs the shimmer) and scaled so the largest is 0.8. Returns the side
    /// ([`SIZE`]), the pixels and the gain the host divides its strength by.
    pub fn refraction(&self) -> (u32, Vec<u8>, f32) {
        let repeat = [Wrap::Repeat, Wrap::Repeat];
        let (ww, wh, wp) = (&self.warp.0, &self.warp.1, &self.warp.2);
        let t = self.params.warp_translation;
        let mut offsets = Vec::with_capacity((SIZE * SIZE) as usize);
        for y in 0..SIZE {
            for x in 0..SIZE {
                let (u, v) = ((x as f32 + 0.5) / SIZE as f32, (y as f32 + 0.5) / SIZE as f32);
                let w = sample_rgba8_bilinear(*ww, *wh, wp, repeat, u * 2.0 + t.x, v * 2.0 + t.y).unwrap_or([0.5; 4]);
                offsets.push(Vec2::new(w[0] - 0.5, w[3] - 0.5));
            }
        }
        let peak = offsets.iter().map(|o| o.length()).fold(0.0, f32::max);
        let gain = if peak > 1e-6 { 0.8 / peak } else { 1.0 };
        let pack = |v: f32| ((v * gain * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8;
        let mut rgba = Vec::with_capacity(offsets.len() * 4);
        for o in offsets {
            rgba.extend_from_slice(&[255, pack(-o.y), 255, pack(o.x)]);
        }
        (SIZE, rgba, gain)
    }

    /// The two stages over the body's coordinates at the current scroll.
    pub fn bake(&self) -> Baked {
        let repeat = [Wrap::Repeat, Wrap::Repeat];
        let (ww, wh, wp) = (&self.warp.0, &self.warp.1, &self.warp.2);
        let warp = |u: f32, v: f32| sample_rgba8_bilinear(*ww, *wh, wp, repeat, u, v).unwrap_or([0.5; 4]);
        let t = self.params.warp_translation;
        let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
        for y in 0..SIZE {
            for x in 0..SIZE {
                let (u, v) = ((x as f32 + 0.5) / SIZE as f32, (y as f32 + 0.5) / SIZE as f32);
                // The warp's IA pair is an offset (`GX_ITF_4`, biased by a half).
                let w = warp(u * 2.0 + t.x, v * 2.0 + t.y);
                let (du, dv) = ((w[0] - 0.5) * PUSH, (w[3] - 0.5) * PUSH);
                // The "screen": the warp's intensity, read again where the first read pushes.
                let s = warp(u * 1.3 - t.y * 0.5 + du, v * 1.3 + t.x * 0.5 + dv)[0];
                let s = s.clamp(0.0, 1.0);
                let ramp = |i: usize| self.params.dark[i] + (self.params.light[i] - self.params.dark[i]) * s;
                let main = self.main.as_ref().map_or(1.0, |(mw, mh, mp)| {
                    sample_rgba8_bilinear(*mw, *mh, mp, [Wrap::Clamp, Wrap::Clamp], u, v)
                        .map_or(1.0, |c| c[0])
                });
                for i in 0..3 {
                    let c = self.params.base[i] + (ramp(i) - self.params.base[i]) * main;
                    rgba.push((c.clamp(0.0, 1.0) * 255.0).round() as u8);
                }
                rgba.push(255);
            }
        }
        Baked {
            width: SIZE,
            height: SIZE,
            wrap: [1, 1],
            rgba,
            env: None,
        }
    }
}
