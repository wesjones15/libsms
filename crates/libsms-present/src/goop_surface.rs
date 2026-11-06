//! The goop as a host draws it: each kind's surface from the disc's goop layer models, laid over
//! the field's chunks as one RGBA image per chunk, and the disc's stamp shapes for the field.
//!
//! In the game each stage has its own goop layers (`TPollutionLayer`, PollutionLayer.cpp): a model
//! of the floor (`/scene/map/pollution/pollution0N.bmd`) whose first map is the layer's density
//! texture, which `TPollutionManager::stamp` paints, and whose other maps give the goop its look
//! (the brown paint `B_RAKenogu_pink`, the burning `TestChoco2`, the slick `rak_tex_ex1_*`, with
//! the warp `B_ind_metal`). The alpha test keeps only texels whose density is high enough. Here
//! the field ([`libsms::goop::GoopField`]) is the density, sparse and world-wide, so:
//!
//! * [`GoopSurface::load`] bakes each layer's TEV with the density map read as full and the maps'
//!   own matrices left out (so the tile repeats seamlessly), one tile per kind
//!   ([`GoopKind::surface`] says which layer);
//! * [`GoopSurface::chunk_image`] lays that tile in world space over one chunk, with the density
//!   (smoothed across texels and across chunk edges) as the alpha, soft around the kind's
//!   threshold; the host draws it on a quad or a grid at the chunk's heights
//!   (`sms_goop_chunk`) and redraws when the chunk's `version` changes;
//! * [`stamp_masks`] reads `pollute.bti` / `clean.bti` (`TPollutionManager::load`) for the field's
//!   stamps, and [`sign_mask`] projects Bowser Jr.'s sign model from above, as `stampModel` does
//!   (`TEnemyMario` draws it at three times his scale, turned half round).
//!
//! Host choices: the tile's world size ([`TILE_PERIOD`]), the edge softness and no scroll
//! (the layers' btk scroll their look slowly; a host may scroll the tile itself).
use std::path::Path;

use glam::{Mat4, Vec3};
use libsms::goop::{CHUNK, GoopField, GoopKind, Mask, TEXEL};
use libsms_assets::Pose;

use crate::tev::{Baked, Tex};
use crate::view::{Textures, build, load_bmd};

/// World units one tile covers (a host choice; the stage layers map theirs per stage).
pub const TILE_PERIOD: f32 = 400.0;
/// Most pixels a chunk image has on a side.
pub const MAX_CHUNK_IMAGE: u32 = 128;
const LAYERS: &str = "scene/map/pollution";
/// The stamps (`TPollutionManager::load`).
pub const POLLUTE_MASK: &str = "scene/map/pollute.bti";
pub const CLEAN_MASK: &str = "scene/map/clean.bti";
/// Bowser Jr.'s sign and how it's drawn on (`stamp_koopa_sign_draw1`).
pub const SIGN_MODEL: &str = "enemy/kagemario/stamp_koopa_sign/stamp_koopa_sign_model1.bmd";
pub const SIGN_CLIP: &str = "enemy/kagemario/stamp_koopa_sign/stamp_koopa_sign_draw1.bck";
/// `mBrushScaleupDuringDrawing` (enemyMario.cpp): the sign's scale against Shadow Mario's.
pub const SIGN_SCALE: f32 = 3.0;

/// One kind's look at full density.
#[derive(Debug, Clone)]
pub struct Tile {
    pub layer: &'static str,
    pub image: Baked,
    /// The look through one loop of the layer's btk ([`SCROLL_FRAMES`] bakes; `image` is the
    /// first), and the loop's length in seconds (0: it doesn't move).
    pub frames: Vec<Baked>,
    pub loop_secs: f32,
}

impl Tile {
    /// The tile's colour at `(u, v)` (repeats) `time` seconds into its loop, blending the two
    /// nearest frames.
    pub fn color_at(&self, u: f32, v: f32, time: f32) -> [f32; 3] {
        if self.frames.len() < 2 || self.loop_secs <= 0.0 {
            return sample_tile(&self.image, u, v);
        }
        let n = self.frames.len();
        let p = (time / self.loop_secs).rem_euclid(1.0) * n as f32;
        let i0 = (p as usize).min(n - 1);
        let w = p - i0 as f32;
        let a = sample_tile(&self.frames[i0], u, v);
        let b = sample_tile(&self.frames[(i0 + 1) % n], u, v);
        [0, 1, 2].map(|c| a[c] + (b[c] - a[c]) * w)
    }
}

#[derive(Debug, Clone, Default)]
pub struct GoopSurface {
    /// By [`GoopKind::ALL`] order; None where the layer's file is missing.
    pub tiles: Vec<Option<Tile>>,
}

impl GoopSurface {
    pub fn load(dir: &Path) -> GoopSurface {
        let mut baked: Vec<(&'static str, Option<(Vec<Baked>, f32)>)> = Vec::new();
        let tiles = GoopKind::ALL
            .iter()
            .map(|k| {
                let layer = k.surface();
                let image = match baked.iter().find(|(l, _)| *l == layer) {
                    Some((_, b)) => b.clone(),
                    None => {
                        let b = bake_layer(dir, layer);
                        baked.push((layer, b.clone()));
                        b
                    }
                };
                image.map(|(frames, loop_secs)| Tile { layer, image: frames[0].clone(), frames, loop_secs })
            })
            .collect();
        GoopSurface { tiles }
    }

    pub fn tile(&self, kind: GoopKind) -> Option<&Tile> {
        let i = GoopKind::ALL.iter().position(|&k| k == kind)?;
        self.tiles.get(i)?.as_ref()
    }

    /// Chunk `key`'s goop as `res` x `res` RGBA, rows from -Z, columns from -X (the chunk's
    /// texel order). Colour from the kind's tile at the pixel's world position (the kind's tint
    /// without a tile); alpha from the field's density, bilinear between texel centres
    /// (neighbouring chunks included) and eased around the kind's threshold.
    pub fn chunk_image(&self, field: &GoopField, key: (i32, i32), res: u32) -> Vec<u8> {
        self.chunk_image_at(field, key, res, 0.0)
    }

    /// [`Self::chunk_image`] `time` seconds into each kind's loop ([`Tile::color_at`]); a host
    /// redraws its chunks every so often to show the paint swim.
    pub fn chunk_image_at(&self, field: &GoopField, key: (i32, i32), res: u32, time: f32) -> Vec<u8> {
        let res = res.clamp(1, MAX_CHUNK_IMAGE);
        let side = CHUNK as f32 * TEXEL;
        let origin = (key.0 as f32 * side, key.1 as f32 * side);
        let at = |x: f32, z: f32| Vec3::new(x, 0.0, z);
        let mut out = Vec::with_capacity((res * res * 4) as usize);
        for py in 0..res {
            for px in 0..res {
                let x = origin.0 + (px as f32 + 0.5) / res as f32 * side;
                let z = origin.1 + (py as f32 + 0.5) / res as f32 * side;
                // Texel-centre grid around the pixel.
                let (gx, gz) = (x / TEXEL - 0.5, z / TEXEL - 0.5);
                let (x0, z0) = (gx.floor(), gz.floor());
                let (tx, tz) = (gx - x0, gz - z0);
                let d = |ix: f32, iz: f32| field.density(at((ix + 0.5) * TEXEL, (iz + 0.5) * TEXEL)) as f32;
                let top = d(x0, z0) + (d(x0 + 1.0, z0) - d(x0, z0)) * tx;
                let bottom = d(x0, z0 + 1.0) + (d(x0 + 1.0, z0 + 1.0) - d(x0, z0 + 1.0)) * tx;
                let density = top + (bottom - top) * tz;
                // The kind of the nearest texel with goop.
                let kind = nearest_kind(field, x, z);
                let alpha = match kind {
                    Some(k) => {
                        let th = k.threshold() as f32;
                        smoothstep(th * 0.5, (th * 1.5).min(250.0), density)
                    }
                    None => 0.0,
                };
                let rgb = match kind.and_then(|k| self.tile(k).map(|t| (k, t))) {
                    Some((_, t)) => t.color_at(x / TILE_PERIOD, z / TILE_PERIOD, time),
                    None => kind.map_or([0.0; 3], |k| k.look().tint),
                };
                for c in rgb {
                    out.push((c.clamp(0.0, 1.0) * 255.0).round() as u8);
                }
                out.push((alpha * 255.0).round() as u8);
            }
        }
        out
    }
}

fn nearest_kind(field: &GoopField, x: f32, z: f32) -> Option<GoopKind> {
    let here = Vec3::new(x, 0.0, z);
    let mut best: Option<(u8, GoopKind)> = None;
    for (dx, dz) in [(0.0, 0.0), (-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
        let p = here + Vec3::new(dx * TEXEL * 0.5, 0.0, dz * TEXEL * 0.5);
        let d = field.density(p);
        if d == 0 {
            continue;
        }
        let k = field.kind_ignoring_height(p);
        if best.is_none_or(|(bd, _)| d > bd) {
            best = k.map(|k| (d, k));
        }
        if dx == 0.0 && dz == 0.0 && best.is_some() {
            break;
        }
    }
    best.map(|(_, k)| k)
}

fn smoothstep(lo: f32, hi: f32, v: f32) -> f32 {
    let t = ((v - lo) / (hi - lo).max(1e-3)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn sample_tile(b: &Baked, u: f32, v: f32) -> [f32; 3] {
    use libcommon::texture::{Wrap, sample_rgba8_bilinear};
    let c = sample_rgba8_bilinear(b.width, b.height, &b.rgba, [Wrap::Repeat, Wrap::Repeat], u, v)
        .unwrap_or([1.0; 4]);
    [c[0], c[1], c[2]]
}

/// Frames baked over one loop of a layer's btk.
pub const SCROLL_FRAMES: usize = 12;

/// One layer's look at full density, over its colour maps' own repeat: one bake, or
/// [`SCROLL_FRAMES`] over its btk's loop (the layers slide their warp, `B_ind_metal`, one repeat
/// a loop, so the paint swims in place), with the loop's length in seconds.
fn bake_layer(dir: &Path, layer: &str) -> Option<(Vec<Baked>, f32)> {
    let mut bmd = load_bmd(dir, &format!("{LAYERS}/{layer}.bmd")).ok()?;
    let srt = std::fs::read(dir.join(format!("{LAYERS}/{layer}.btk")))
        .ok()
        .and_then(|d| libsms_assets::btk::TexSrt::parse(&d).ok())
        .filter(|s| s.frames > 1.0);
    // The density map is each material's first when it has others (the slick layers have just
    // their colour map, alpha blended).
    let mut density: Vec<String> = Vec::new();
    let mut animated: Vec<(usize, usize)> = Vec::new();
    for (mi, m) in bmd.materials.iter_mut().enumerate() {
        if m.textures.iter().flatten().count() >= 2
            && let Some(t) = m.textures[0]
        {
            density.push(bmd.textures[t as usize].name.clone());
        }
        // No scale or turn: the tile is the maps' own repeat, seamless. A matrix the btk moves
        // keeps only its translation (whole repeats over a loop, so still seamless).
        for (slot, t) in m.tex_mtx.iter_mut().enumerate() {
            let moved = srt.as_ref().is_some_and(|s| s.tracks.iter().any(|tr| tr.material == m.name && tr.matrix == slot));
            match t.as_mut() {
                Some(t) if moved => {
                    t.scale = glam::Vec2::ONE;
                    t.rotation = 0.0;
                    t.translation = glam::Vec2::ZERO;
                    animated.push((mi, slot));
                }
                _ => *t = None,
            }
        }
        // Every map over the one coordinate set the bake runs over (the burning layer reads its
        // colour over a second).
        for g in &mut m.tex_gens {
            if g.kind == 1 && (4..=11).contains(&g.src) {
                g.src = 4;
            }
        }
    }
    let full = Tex::new(
        libsms_assets::Image { width: 8, height: 8, rgba: vec![255; 8 * 8 * 4] },
        libcommon::texture::Wrap::Repeat,
        libcommon::texture::Wrap::Repeat,
    );
    let mut textures = Textures::default();
    let mut model = build(
        bmd,
        &|_, name| density.iter().any(|d| d == name).then(|| full.clone()),
        crate::view::models::COUNT as u32,
        &mut textures,
    )
    .ok()?;
    let bi = model.batch_texture.iter().position(|&t| t >= 0)?;
    let first = textures.list.get(model.batch_texture[bi] as usize)?.clone();
    let tidy = |mut b: Baked| {
        // The slick layers are see-through paint over the floor: keep their alpha as coverage.
        b.wrap = [0, 0];
        b
    };
    let Some(srt) = srt.filter(|_| !animated.is_empty()) else {
        return Some((vec![tidy(first)], 0.0));
    };
    let mi = model.batches[bi].material;
    let mut frames = Vec::with_capacity(SCROLL_FRAMES);
    for i in 0..SCROLL_FRAMES {
        srt.apply(&mut model.bmd, srt.frames * i as f32 / SCROLL_FRAMES as f32);
        for &(m, slot) in &animated {
            if let Some(t) = model.bmd.materials[m].tex_mtx[slot].as_mut() {
                t.scale = glam::Vec2::ONE;
                t.rotation = 0.0;
            }
        }
        let b = model.source().bake(mi, &crate::tev::Inputs::default(), 512)?;
        frames.push(tidy(b));
    }
    Some((frames, srt.frames / 30.0))
}

/// The field's stamps from the disc, `(pollute, clean)`: each picture's alpha (its intensity
/// where it has none).
pub fn stamp_masks(dir: &Path) -> Option<(Mask, Mask)> {
    let read = |rel: &str| -> Option<Mask> {
        let data = std::fs::read(dir.join(rel)).ok()?;
        let t = libsms_assets::texture::read_bti(&data, rel.into()).ok()?;
        mask_of(&t.image)
    };
    Some((read(POLLUTE_MASK)?, read(CLEAN_MASK)?))
}

/// A square mask from an image (resampled to its larger side).
fn mask_of(img: &libsms_assets::Image) -> Option<Mask> {
    let (w, h) = (img.width as usize, img.height as usize);
    if w == 0 || h == 0 || img.rgba.len() < w * h * 4 {
        return None;
    }
    let px = img.rgba.as_chunks::<4>().0;
    // The channel that carries the shape: whichever of intensity and alpha varies most.
    let range = |c: usize| {
        let (lo, hi) = px.iter().fold((255u8, 0u8), |(lo, hi), p| (lo.min(p[c]), hi.max(p[c])));
        hi.saturating_sub(lo)
    };
    let ch = if range(3) >= range(0) { 3 } else { 0 };
    let n = w.max(h);
    let raw: Vec<u8> = (0..n * n)
        .map(|i| {
            let (x, y) = ((i % n) * w / n, (i / n) * h / n);
            px[y * w + x][ch]
        })
        .collect();
    // Stretched to full strength at its densest: the field's stamps blend by the mask alone
    // (the game's `stamp` also scales by the stamp's own strength argument).
    let hi = raw.iter().copied().max().unwrap_or(0).max(1) as u32;
    let alpha = raw.iter().map(|&a| (a as u32 * 255 / hi) as u8).collect();
    Some(Mask { size: n, alpha })
}

/// Bowser Jr.'s sign seen from above as `stampModel` paints it: the model posed at the end of
/// its drawing clip, turned half round, rasterised into a `size` square. Returns the mask and
/// its half-side in world units at [`SIGN_SCALE`].
pub fn sign_mask(dir: &Path, size: usize) -> Option<(Mask, f32)> {
    let size = size.clamp(8, 256);
    let data = std::fs::read(dir.join(SIGN_MODEL)).ok()?;
    let bmd = libsms_assets::Bmd::parse(&data).ok()?;
    // The model is three pens (`cyl*`) that the clip moves along the sign's strokes; the game
    // stamps where they are every frame it plays, so the sign is everywhere they pass.
    let clip = std::fs::read(dir.join(SIGN_CLIP)).ok().and_then(|d| libsms::Clip::parse(&d));
    let frames = clip.as_ref().map_or(0, |c| c.frames.max(0.0) as usize);
    let batches = bmd.batches().ok()?;
    let mut tris: Vec<[(f32, f32); 3]> = Vec::new();
    let mut half: f32 = 0.0;
    for f in 0..=frames {
        let joints = match &clip {
            Some(c) => bmd.pose(|j| c.sample(j, f as f32), |_| None),
            None => bmd.bind_world(),
        };
        let pose = Pose::new(&bmd, Mat4::IDENTITY, &joints);
        for b in &batches {
            let pts: Vec<(f32, f32)> = b
                .vertices
                .iter()
                .map(|v| {
                    let p = pose.skin(v).0;
                    // `MsMtxSetRotRPH(.., 0, 180, 0)`: half round about Y.
                    (-p.x, -p.z)
                })
                .collect();
            for &(x, z) in &pts {
                half = half.max(x.abs()).max(z.abs());
            }
            for t in b.indices.chunks_exact(3) {
                tris.push([pts[t[0] as usize], pts[t[1] as usize], pts[t[2] as usize]]);
            }
        }
    }
    if half <= 0.0 || tris.is_empty() {
        return None;
    }
    let mut alpha = vec![0u8; size * size];
    // Four samples a pixel, for soft edges.
    let sub = [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)];
    for y in 0..size {
        for x in 0..size {
            let mut hits = 0;
            for (sx, sy) in sub {
                let px = ((x as f32 + sx) / size as f32 * 2.0 - 1.0) * half;
                let pz = ((y as f32 + sy) / size as f32 * 2.0 - 1.0) * half;
                if tris.iter().any(|t| inside(t, px, pz)) {
                    hits += 1;
                }
            }
            alpha[y * size + x] = (hits * 255 / sub.len()) as u8;
        }
    }
    Some((Mask { size, alpha }, half * SIGN_SCALE))
}

fn inside(t: &[(f32, f32); 3], x: f32, z: f32) -> bool {
    let e = |a: (f32, f32), b: (f32, f32)| (b.0 - a.0) * (z - a.1) - (b.1 - a.1) * (x - a.0);
    let (d0, d1, d2) = (e(t[0], t[1]), e(t[1], t[2]), e(t[2], t[0]));
    (d0 >= 0.0 && d1 >= 0.0 && d2 >= 0.0) || (d0 <= 0.0 && d1 <= 0.0 && d2 <= 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use libsms::Host;

    struct Flat;
    impl Host for Flat {
        fn ground(&self, _p: libsms::Vec3) -> Option<libsms::host::GroundHit> {
            Some(libsms::host::GroundHit {
                height: 0.0,
                normal: libsms::Vec3::Y,
                flags: libsms::host::SurfaceFlags::NONE,
            })
        }
    }

    fn dir() -> Option<std::path::PathBuf> {
        let d = libsms::assets::find(None);
        if d.is_none() {
            eprintln!("SKIP: no assets/");
        }
        d
    }

    #[test]
    fn every_kind_gets_a_tile_from_its_layer_model() {
        let Some(dir) = dir() else { return };
        let s = GoopSurface::load(&dir);
        for k in GoopKind::ALL {
            let t = s.tile(k).unwrap_or_else(|| panic!("{k:?} has a tile"));
            assert!(t.image.width >= 8 && t.image.rgba.len() == (t.image.width * t.image.height * 4) as usize);
            // Not a blank: the colours vary across the tile.
            let px = t.image.rgba.as_chunks::<4>().0;
            assert!(px.iter().any(|p| p[..3] != px[0][..3]), "{k:?}'s tile varies");
        }
    }

    #[test]
    fn the_paint_drifts_with_its_layer_scroll() {
        let Some(dir) = dir() else { return };
        let s = GoopSurface::load(&dir);
        let moving: Vec<_> = GoopKind::ALL
            .iter()
            .filter_map(|&k| s.tile(k).map(|t| (k, t.loop_secs)))
            .filter(|(_, l)| *l > 0.0)
            .collect();
        eprintln!("scrolling kinds: {moving:?}");
        assert!(!moving.is_empty(), "some layer scrolls");
        let (k, _) = moving[0];
        let mut f = GoopField::new();
        f.stamp(&Flat, Vec3::new(256.0, 0.0, 256.0), 200.0, k);
        let a = s.chunk_image_at(&f, (0, 0), 32, 0.0);
        let b = s.chunk_image_at(&f, (0, 0), 32, 10.0);
        assert_ne!(a, b, "the picture drifts");
        // The coverage doesn't move, only the paint.
        let alpha = |img: &[u8]| img.chunks_exact(4).map(|p| p[3]).collect::<Vec<_>>();
        assert_eq!(alpha(&a), alpha(&b));
    }

    #[test]
    fn a_chunk_image_is_opaque_on_goop_and_clear_off_it() {
        let Some(dir) = dir() else { return };
        let s = GoopSurface::load(&dir);
        let mut f = GoopField::new();
        f.stamp(&Flat, Vec3::new(256.0, 0.0, 256.0), 120.0, GoopKind::Sink);
        let img = s.chunk_image(&f, (0, 0), 64);
        assert_eq!(img.len(), 64 * 64 * 4);
        let a = |x: usize, y: usize| img[(y * 64 + x) * 4 + 3];
        assert!(a(32, 32) > 200, "centre is goop: {}", a(32, 32));
        assert_eq!(a(0, 0), 0, "corner is clear");
    }

    #[test]
    fn the_disc_stamps_and_the_sign_load() {
        let Some(dir) = dir() else { return };
        let (pollute, clean) = stamp_masks(&dir).expect("stamps");
        for m in [&pollute, &clean] {
            assert!(m.size >= 8);
            assert!(m.alpha.iter().any(|&a| a > 200) && m.alpha.iter().any(|&a| a < 50));
        }
        let (sign, half) = sign_mask(&dir, 64).expect("sign");
        let covered = sign.alpha.iter().filter(|&&a| a > 128).count();
        assert!(covered > 64 * 64 / 20 && covered < 64 * 64 * 9 / 10, "a shape, not a blot: {covered}");
        assert!(half > 50.0, "half side {half}");
    }
}
