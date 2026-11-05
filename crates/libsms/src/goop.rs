//! The goop (Sunshine's pollution, `落書き`: Map/PollutionManager.cpp `TPollutionManager`,
//! PollutionLayer.cpp `TPollutionLayer`, PollutionCount.cpp `TPollutionCounterLayer`).
//!
//! In the game each stage has a few goop *layers*: flat or wall planes, each with an 8-bit
//! density texture (`mPollutionMap`, one texel per `mTexelSize` world units) projected onto the
//! map. Everything that paints or cleans draws a *stamp* into those textures on the GPU
//! (`pushStampTask` -> `drawTexStamp`): `stamp(1, ..)` is the pollute stamp
//! (`/common/map/pollute.bti`), `clean` is `stamp(0, ..)` with `/common/map/clean.bti`, both blended
//! source-alpha over the density (`GX_BL_SRCALPHA`/`GX_BL_INVSRCALPHA`): a pollute stamp moves
//! each texel toward 255 by the stamp's alpha, a clean stamp toward 0. A point is goop when its
//! texel is over the layer's threshold (`mPollutedThreshold`: 30 for ordinary goop, 128 fire,
//! 200 the `UNK7` kind; `initLayerInfo`) and the point is at the layer's depth (`isSame`).
//!
//! A host like Risk of Rain 2 has no goop layers, so this is one field over the whole world: a
//! sparse grid of [`TEXEL`]-unit texels in 16x16 chunks, each texel with its density and the
//! ground height it was painted at (probed through [`Host::ground`] when stamped). Stamps blend
//! like the game's; [`GoopField::is_polluted`] tests a point like `TPollutionLayer::isPolluted`.
//! What paints and what cleans call in: Shadow Mario's brush ([`crate::koopajr`]), the goop
//! enemies ([`crate::goop_critters`]) and FLUDD's water (`TModelWaterManager::splashGround` /
//! `splashWall` clean at the particle with `mCleanSize * 10` / `* 32`; the FFI routes
//! [`crate::water::WaterManager`]'s ground impacts here).
//!
//! Not the game's: the texel size (32 units, the step `TMario::checkGraffito` probes goop
//! with), the height band, the bounded chunk budget (oldest painted chunks go first) and the
//! optional drying ([`GoopField::dry_ticks`]); all are host choices for an open world.

use std::collections::HashMap;

use glam::Vec3;

use crate::host::{Enemy, Host, SurfaceFlags};

/// World units per goop texel.
pub const TEXEL: f32 = 32.0;
/// Texels per chunk side.
pub const CHUNK: usize = 16;
const CHUNK_TEXELS: usize = CHUNK * CHUNK;
/// `mPollutedThreshold` of an ordinary goop layer.
pub const POLLUTED: u8 = 30;
/// `TPollutionLayer::cleaned`: only texels denser than this hiss and steam when cleaned.
pub const CLEAN_FX: u8 = 100;
/// How far above or below a texel's painted height a point still counts as on it.
pub const HEIGHT_BAND: f32 = 100.0;
/// A stamp skips ground this far from the stamp's own height (ledges, cliffs).
const STEP_BAND: f32 = 150.0;
/// Probes per stamp side: heights between them are interpolated.
const PROBES: usize = 5;

/// `TPollutionLayer::getPollutionType` (PollutionLayer.hpp `POLLUTION_TYPE_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
#[repr(u8)]
pub enum GoopKind {
    /// Brown goop Mario sinks into (`POLLUTION_TYPE_SINK`): `TMario::checkSink`.
    #[default]
    Sink = 0,
    /// Burning goop (`POLLUTION_TYPE_FIRE`): `checkGraffitoFire`.
    Fire = 1,
    /// Oil Mario slides on (`POLLUTION_TYPE_SLIP`): `checkGraffitoSlip`.
    Slip = 2,
    /// Electric goop (`POLLUTION_TYPE_ELECTRIC`): `checkGraffitoElec`.
    Electric = 4,
}

impl GoopKind {
    /// `mPollutedThreshold` for the kind (`TPollutionLayer::initLayerInfo`).
    pub fn threshold(self) -> u8 {
        match self {
            GoopKind::Fire => 128,
            _ => POLLUTED,
        }
    }

    pub fn from_u8(v: u8) -> GoopKind {
        match v {
            1 => GoopKind::Fire,
            2 => GoopKind::Slip,
            4 => GoopKind::Electric,
            _ => GoopKind::Sink,
        }
    }
}

/// A stamp's alpha (`ResTIMG` of `pollute.bti` / `clean.bti`, or a model's top-down
/// projection for `stampModel`), square, row-major from -X/-Z.
#[derive(Debug, Clone, PartialEq)]
pub struct Mask {
    pub size: usize,
    pub alpha: Vec<u8>,
}

impl Mask {
    /// A soft disc: solid to `0.7` of the radius, fading to the rim. Stands in for the disc's
    /// stamp textures when a host hasn't loaded them.
    pub fn disc(size: usize) -> Mask {
        let size = size.max(2);
        let c = (size as f32 - 1.0) * 0.5;
        let alpha = (0..size * size)
            .map(|i| {
                let (x, y) = ((i % size) as f32 - c, (i / size) as f32 - c);
                let r = (x * x + y * y).sqrt() / (c + 0.5);
                let a = ((1.0 - r) / 0.3).clamp(0.0, 1.0);
                (a * 255.0).round() as u8
            })
            .collect();
        Mask { size, alpha }
    }

    /// Bilinear alpha at `(u, v)` in 0..1 (0 outside).
    pub fn sample(&self, u: f32, v: f32) -> f32 {
        if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) || self.size == 0 {
            return 0.0;
        }
        let n = self.size;
        let fx = u * (n - 1) as f32;
        let fy = v * (n - 1) as f32;
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(n - 1), (y0 + 1).min(n - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let a = |x: usize, y: usize| self.alpha[y * n + x] as f32 / 255.0;
        let top = a(x0, y0) + (a(x1, y0) - a(x0, y0)) * tx;
        let bottom = a(x0, y1) + (a(x1, y1) - a(x0, y1)) * tx;
        top + (bottom - top) * ty
    }
}

/// One 16x16 block of texels.
#[derive(Debug, Clone)]
pub struct Chunk {
    pub density: [u8; CHUNK_TEXELS],
    /// The ground height each texel was painted at.
    pub height: [f32; CHUNK_TEXELS],
    pub kind: GoopKind,
    /// Bumped on every change, for hosts that cache meshes.
    pub version: u32,
    /// [`GoopField::ticks`] when last painted (the budget drops the oldest first).
    painted: u64,
    /// Ticks since the last change, for drying.
    idle: u32,
}

impl Chunk {
    fn new(kind: GoopKind) -> Chunk {
        Chunk {
            density: [0; CHUNK_TEXELS],
            height: [0.0; CHUNK_TEXELS],
            kind,
            version: 0,
            painted: 0,
            idle: 0,
        }
    }

    fn empty(&self) -> bool {
        self.density.iter().all(|&d| d == 0)
    }
}

/// What the field did that a host may show or play.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GoopEvent {
    /// A stamp went down (`Painted` sounds are the painter's own).
    Painted { pos: Vec3, radius: f32 },
    /// Dense goop wiped off (`TPollutionLayer::cleaned`): `MSD_SE_ERASE_SCRAWL` at a volume of
    /// the stamp's size and the `ms_raku_kie` steam (`PARTICLE_MS_RAKU_KIE`).
    Erased { pos: Vec3, size: f32 },
}

/// The world's goop.
#[derive(Debug, Clone)]
pub struct GoopField {
    chunks: HashMap<(i32, i32), Chunk>,
    /// Chunks kept at most; painting past it drops the oldest painted.
    pub max_chunks: usize,
    /// Host option: an untouched chunk loses one density step every this many ticks (0: goop
    /// stays until cleaned, as in the game).
    pub dry_ticks: u32,
    pub pollute: Mask,
    pub clean: Mask,
    pub events: Vec<GoopEvent>,
    pub ticks: u64,
    /// `TPollutionLayer::cleaned`'s `effect_counter` and offset tables' cursors.
    erase_counter: u32,
    erase_cursor: (usize, usize),
    /// Per-enemy ticks left before goop hurts it again ([`GoopField::contacts`]).
    cooldowns: HashMap<u64, u32>,
}

impl Default for GoopField {
    fn default() -> Self {
        GoopField::new()
    }
}

fn chunk_of(x: f32, z: f32) -> ((i32, i32), usize) {
    let tx = (x / TEXEL).floor() as i32;
    let tz = (z / TEXEL).floor() as i32;
    let n = CHUNK as i32;
    let key = (tx.div_euclid(n), tz.div_euclid(n));
    let (lx, lz) = (tx.rem_euclid(n) as usize, tz.rem_euclid(n) as usize);
    (key, lz * CHUNK + lx)
}

/// The world-space centre of texel `i` of chunk `key`.
pub fn texel_center(key: (i32, i32), i: usize) -> (f32, f32) {
    let tx = key.0 * CHUNK as i32 + (i % CHUNK) as i32;
    let tz = key.1 * CHUNK as i32 + (i / CHUNK) as i32;
    ((tx as f32 + 0.5) * TEXEL, (tz as f32 + 0.5) * TEXEL)
}

impl GoopField {
    pub fn new() -> GoopField {
        GoopField {
            chunks: HashMap::new(),
            max_chunks: 1024,
            dry_ticks: 0,
            pollute: Mask::disc(32),
            clean: Mask::disc(32),
            events: Vec::new(),
            ticks: 0,
            erase_counter: 1,
            erase_cursor: (0, 0),
            cooldowns: HashMap::new(),
        }
    }

    /// Density at `pos`'s texel (0 when none), ignoring height.
    pub fn density(&self, pos: Vec3) -> u8 {
        let (key, i) = chunk_of(pos.x, pos.z);
        self.chunks.get(&key).map_or(0, |c| c.density[i])
    }

    /// `TPollutionManager::isPolluted`: is the point on goop (its texel over the threshold and
    /// the point at the texel's height)?
    pub fn is_polluted(&self, pos: Vec3) -> bool {
        self.kind_at(pos).is_some()
    }

    /// `getPollutionType` for a point that [`Self::is_polluted`].
    pub fn kind_at(&self, pos: Vec3) -> Option<GoopKind> {
        let (key, i) = chunk_of(pos.x, pos.z);
        let c = self.chunks.get(&key)?;
        (c.density[i] > c.kind.threshold() && (pos.y - c.height[i]).abs() < HEIGHT_BAND)
            .then_some(c.kind)
    }

    /// `TMario::checkGraffito`'s test for ordinary goop: the point and the eight around it a
    /// texel away all on goop (a lone texel or the very rim doesn't catch Mario).
    pub fn is_polluted_under(&self, pos: Vec3) -> bool {
        (-1..=1).all(|dz| {
            (-1..=1).all(|dx| {
                self.is_polluted(pos + Vec3::new(dx as f32 * TEXEL, 0.0, dz as f32 * TEXEL))
            })
        })
    }

    /// Sum of every texel's density (`getPollutionDegree`, unscaled).
    pub fn degree(&self) -> u64 {
        self.chunks
            .values()
            .map(|c| c.density.iter().map(|&d| d as u64).sum::<u64>())
            .sum()
    }

    /// The chunks, for drawing: key, kind, densities, heights, version.
    pub fn chunks(&self) -> impl Iterator<Item = ((i32, i32), &Chunk)> {
        self.chunks.iter().map(|(k, c)| (*k, c))
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    pub fn clear(&mut self) {
        self.chunks.clear();
    }

    /// `stamp(1, x, y, z, size)`: paints a disc of goop of `radius` at `pos` with the pollute
    /// stamp. `pos.y` is the ground the painter stands on.
    pub fn stamp(&mut self, host: &dyn Host, pos: Vec3, radius: f32, kind: GoopKind) {
        let mask = std::mem::replace(&mut self.pollute, Mask { size: 0, alpha: Vec::new() });
        self.stamp_mask(host, pos, radius, 0.0, &mask, kind);
        self.pollute = mask;
        self.events.push(GoopEvent::Painted { pos, radius });
    }

    /// `stampModel`'s effect with a host-made mask (a model's top-down silhouette): `mask`
    /// covers a square of half-side `half` centred on `pos`, turned `yaw` (radians) about Y.
    pub fn stamp_mask(
        &mut self,
        host: &dyn Host,
        pos: Vec3,
        half: f32,
        yaw: f32,
        mask: &Mask,
        kind: GoopKind,
    ) {
        if half <= 0.0 {
            return;
        }
        let heights = Heights::probe(host, pos, half * std::f32::consts::SQRT_2);
        let (s, c) = yaw.sin_cos();
        let reach = half * std::f32::consts::SQRT_2;
        let tick = self.ticks;
        self.for_texels(pos, reach, true, kind, |x, z, chunk, i| {
            let (dx, dz) = (x - pos.x, z - pos.z);
            // Into the mask's own axes.
            let (mx, mz) = (dx * c - dz * s, dx * s + dz * c);
            let a = mask.sample(mx / (2.0 * half) + 0.5, mz / (2.0 * half) + 0.5);
            if a <= 0.0 {
                return;
            }
            let Some(h) = heights.at(x, z) else { return };
            let old = chunk.density[i] as f32;
            if old > 0.0 && (chunk.height[i] - h).abs() > HEIGHT_BAND {
                // Goop on another floor here: leave it.
                return;
            }
            let new = old + (255.0 - old) * a;
            chunk.density[i] = new.round().clamp(0.0, 255.0) as u8;
            chunk.height[i] = h;
            chunk.painted = tick;
        });
        self.enforce_budget();
    }

    /// `TPollutionManager::clean` (`stamp(0, ..)`): wipes goop off a disc of `radius` around
    /// `pos` with the clean stamp. Returns how many texels lost goop.
    pub fn clean(&mut self, pos: Vec3, radius: f32) -> u32 {
        let mask = std::mem::replace(&mut self.clean, Mask { size: 0, alpha: Vec::new() });
        let mut dense = false;
        let mut changed = 0u32;
        self.for_texels(pos, radius, false, GoopKind::Sink, |x, z, chunk, i| {
            let d = chunk.density[i];
            if d == 0 || (chunk.height[i] - pos.y).abs() > HEIGHT_BAND + radius {
                return;
            }
            let a = mask.sample(
                (x - pos.x) / (2.0 * radius) + 0.5,
                (z - pos.z) / (2.0 * radius) + 0.5,
            );
            if a <= 0.0 {
                return;
            }
            dense |= d > CLEAN_FX;
            let new = (d as f32 * (1.0 - a)).round() as u8;
            if new != d {
                chunk.density[i] = new;
                changed += 1;
            }
        });
        self.clean = mask;
        if changed > 0 {
            self.chunks.retain(|_, c| !c.empty());
        }
        if dense {
            self.cleaned(pos, radius);
        }
        changed
    }

    /// `TPollutionLayer::cleaned`: one cleaning call in six, at a spot stepped around the
    /// cleaned point by the game's offset tables, hisses and steams if that spot is dense goop.
    fn cleaned(&mut self, pos: Vec3, size: f32) {
        const X: [i32; 10] = [-1, 0, 2, 4, 1, -1, -2, 0, 3, -3];
        const Z: [i32; 10] = [-1, -1, 0, 2, -2, -3, 0, 3, 0, 1];
        self.erase_counter += 1;
        if self.erase_counter <= 5 {
            return;
        }
        self.erase_counter = 0;
        let (cx, cz) = self.erase_cursor;
        let at = pos + Vec3::new(X[cx] as f32 * TEXEL, 0.0, Z[cz] as f32 * TEXEL);
        self.erase_cursor = ((cx + 1) % 10, (cz + 3) % 10);
        // The spot was just cleaned too; the game reads the map before the GPU stamp lands,
        // so any spot of goop near the cleaning counts.
        self.events.push(GoopEvent::Erased { pos: at, size });
    }

    /// Visits the texels whose centres are within `reach` of `pos` (XZ). With `create`, missing
    /// chunks are made (of `kind`); without, only existing ones are visited. Bumps the version
    /// of every chunk visited.
    fn for_texels(
        &mut self,
        pos: Vec3,
        reach: f32,
        create: bool,
        kind: GoopKind,
        mut f: impl FnMut(f32, f32, &mut Chunk, usize),
    ) {
        let t0x = ((pos.x - reach) / TEXEL).floor() as i32;
        let t1x = ((pos.x + reach) / TEXEL).floor() as i32;
        let t0z = ((pos.z - reach) / TEXEL).floor() as i32;
        let t1z = ((pos.z + reach) / TEXEL).floor() as i32;
        let n = CHUNK as i32;
        for kz in t0z.div_euclid(n)..=t1z.div_euclid(n) {
            for kx in t0x.div_euclid(n)..=t1x.div_euclid(n) {
                let key = (kx, kz);
                let chunk = if create {
                    self.chunks.entry(key).or_insert_with(|| Chunk::new(kind))
                } else {
                    match self.chunks.get_mut(&key) {
                        Some(c) => c,
                        None => continue,
                    }
                };
                let mut touched = false;
                for lz in 0..CHUNK {
                    let tz = kz * n + lz as i32;
                    if tz < t0z || tz > t1z {
                        continue;
                    }
                    for lx in 0..CHUNK {
                        let tx = kx * n + lx as i32;
                        if tx < t0x || tx > t1x {
                            continue;
                        }
                        let (x, z) = ((tx as f32 + 0.5) * TEXEL, (tz as f32 + 0.5) * TEXEL);
                        if (x - pos.x).powi(2) + (z - pos.z).powi(2) > reach * reach {
                            continue;
                        }
                        f(x, z, chunk, lz * CHUNK + lx);
                        touched = true;
                    }
                }
                if touched {
                    chunk.version = chunk.version.wrapping_add(1);
                    chunk.idle = 0;
                }
            }
        }
        if create {
            self.chunks.retain(|_, c| !c.empty());
        }
    }

    fn enforce_budget(&mut self) {
        while self.chunks.len() > self.max_chunks.max(1) {
            let Some(oldest) = self.chunks.iter().min_by_key(|(_, c)| c.painted).map(|(k, _)| *k)
            else {
                break;
            };
            self.chunks.remove(&oldest);
        }
    }

    /// One tick: drying (if on) and the contact cooldowns.
    pub fn step(&mut self) {
        self.ticks += 1;
        self.cooldowns.retain(|_, t| {
            *t = t.saturating_sub(1);
            *t > 0
        });
        if self.dry_ticks == 0 {
            return;
        }
        let dry = self.dry_ticks;
        for c in self.chunks.values_mut() {
            c.idle += 1;
            if c.idle % dry == 0 {
                for d in c.density.iter_mut() {
                    *d = d.saturating_sub(1);
                }
                c.version = c.version.wrapping_add(1);
            }
        }
        self.chunks.retain(|_, c| !c.empty());
    }

    /// Host adaptation of `TMario::checkSink`'s floor damage for anything standing in goop:
    /// which of `enemies` have their feet on goop now and aren't still cooling down from the
    /// last time. Each one returned waits `period` ticks (`mSinkDmgTime`) before it can be
    /// returned again. What the goop does to them is the host's.
    pub fn contacts(&mut self, enemies: &[Enemy], period: u32) -> Vec<u64> {
        let mut out = Vec::new();
        for e in enemies {
            if self.cooldowns.contains_key(&e.id) {
                continue;
            }
            if self.is_polluted(e.pos) {
                self.cooldowns.insert(e.id, period.max(1));
                out.push(e.id);
            }
        }
        out
    }

    /// A random point on goop within `radius` of `near` (XZ), from `rand` in 0..1, for things
    /// that grow out of goop. `None` when there is none.
    pub fn random_spot(&self, near: Vec3, radius: f32, mut rand: impl FnMut() -> f32) -> Option<Vec3> {
        let mut spots: Vec<Vec3> = Vec::new();
        for (key, c) in &self.chunks {
            let (ox, oz) = texel_center(*key, 0);
            let half = CHUNK as f32 * TEXEL;
            if (ox + half * 0.5 - near.x).abs() > radius + half
                || (oz + half * 0.5 - near.z).abs() > radius + half
            {
                continue;
            }
            // Every fourth texel each way is plenty to choose from.
            for i in (0..CHUNK_TEXELS).filter(|i| (i % CHUNK) % 4 == 2 && (i / CHUNK) % 4 == 2) {
                if c.density[i] <= c.kind.threshold() {
                    continue;
                }
                let (x, z) = texel_center(*key, i);
                let p = Vec3::new(x, c.height[i], z);
                if (p.x - near.x).powi(2) + (p.z - near.z).powi(2) <= radius * radius
                    && (p.y - near.y).abs() < 1000.0
                {
                    spots.push(p);
                }
            }
        }
        if spots.is_empty() {
            return None;
        }
        let i = ((rand() * spots.len() as f32) as usize).min(spots.len() - 1);
        Some(spots[i])
    }
}

/// Ground heights over a stamp, probed on a [`PROBES`] grid and interpolated.
struct Heights {
    origin: Vec3,
    half: f32,
    h: [[Option<f32>; PROBES]; PROBES],
}

impl Heights {
    fn probe(host: &dyn Host, pos: Vec3, half: f32) -> Heights {
        let mut h = [[None; PROBES]; PROBES];
        for (j, row) in h.iter_mut().enumerate() {
            for (i, cell) in row.iter_mut().enumerate() {
                let f = |k: usize| (k as f32 / (PROBES - 1) as f32) * 2.0 - 1.0;
                let q = Vec3::new(pos.x + f(i) * half, pos.y + 60.0, pos.z + f(j) * half);
                *cell = host
                    .ground(q)
                    .filter(|g| {
                        !g.flags.has(SurfaceFlags::ILLEGAL | SurfaceFlags::WATER_SURFACE)
                            && (g.height - pos.y).abs() < STEP_BAND
                    })
                    .map(|g| g.height);
            }
        }
        Heights { origin: pos, half, h }
    }

    /// Bilinear height at `(x, z)`; probes that found no ground drop out (their neighbours'
    /// share is used), and a texel with none of its four is skipped.
    fn at(&self, x: f32, z: f32) -> Option<f32> {
        let g = |v: f32, o: f32| ((v - o) / self.half * 0.5 + 0.5).clamp(0.0, 1.0) * (PROBES - 1) as f32;
        let (fx, fz) = (g(x, self.origin.x), g(z, self.origin.z));
        let (i0, j0) = (fx.floor() as usize, fz.floor() as usize);
        let (i1, j1) = ((i0 + 1).min(PROBES - 1), (j0 + 1).min(PROBES - 1));
        let (tx, tz) = (fx - i0 as f32, fz - j0 as f32);
        let mut sum = 0.0;
        let mut weight = 0.0;
        for (i, j, w) in [
            (i0, j0, (1.0 - tx) * (1.0 - tz)),
            (i1, j0, tx * (1.0 - tz)),
            (i0, j1, (1.0 - tx) * tz),
            (i1, j1, tx * tz),
        ] {
            if let Some(h) = self.h[j][i] {
                sum += h * w;
                weight += w;
            }
        }
        (weight > 1e-4).then(|| sum / weight)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::FlatHost;

    #[test]
    fn a_stamp_makes_goop_and_clean_wipes_it() {
        let mut g = GoopField::new();
        let host = FlatHost::default();
        g.stamp(&host, Vec3::ZERO, 384.0, GoopKind::Sink);
        assert!(g.is_polluted(Vec3::ZERO));
        assert!(g.is_polluted_under(Vec3::new(100.0, 0.0, 0.0)));
        assert!(!g.is_polluted(Vec3::new(600.0, 0.0, 0.0)), "outside the disc");
        assert!(!g.is_polluted(Vec3::new(0.0, 500.0, 0.0)), "far above the goop");
        let before = g.degree();
        assert!(g.clean(Vec3::ZERO, 200.0) > 0);
        assert!(g.degree() < before);
        assert!(!g.is_polluted(Vec3::ZERO));
        assert!(g.is_polluted(Vec3::new(300.0, 0.0, 0.0)), "the ring left");
        for _ in 0..8 {
            g.clean(Vec3::ZERO, 500.0);
        }
        assert_eq!(g.degree(), 0);
        assert_eq!(g.chunk_count(), 0);
        // One cleaning call in six over dense goop hisses (`cleaned`'s counter).
        g.stamp(&host, Vec3::ZERO, 384.0, GoopKind::Sink);
        g.events.clear();
        for i in 0..12 {
            g.clean(Vec3::new(i as f32 * 40.0 - 200.0, 0.0, 0.0), 20.0);
        }
        let erased = g.events.iter().filter(|e| matches!(e, GoopEvent::Erased { .. })).count();
        assert_eq!(erased, 2);
    }

    #[test]
    fn goop_follows_the_ground_and_skips_cliffs() {
        struct Step;
        impl Host for Step {
            fn ground(&self, p: Vec3) -> Option<crate::host::GroundHit> {
                let height = if p.x > 0.0 { 1000.0 } else { 0.0 };
                (p.y >= height).then_some(crate::host::GroundHit {
                    height,
                    normal: Vec3::Y,
                    flags: SurfaceFlags::NONE,
                })
            }
        }
        let mut g = GoopField::new();
        g.stamp(&Step, Vec3::new(-150.0, 0.0, 0.0), 120.0, GoopKind::Sink);
        assert!(g.is_polluted(Vec3::new(-150.0, 0.0, 0.0)));
        // Under the cliff top the probe finds the high floor (out of the band): no goop there.
        assert!(!g.is_polluted(Vec3::new(-20.0, 1000.0, 0.0)));
    }

    #[test]
    fn contacts_wait_out_their_period() {
        let mut g = GoopField::new();
        g.stamp(&FlatHost::default(), Vec3::ZERO, 300.0, GoopKind::Sink);
        let e = [Enemy { id: 7, pos: Vec3::ZERO, radius: 30.0, height: 100.0 }];
        assert_eq!(g.contacts(&e, 90), vec![7]);
        assert!(g.contacts(&e, 90).is_empty());
        for _ in 0..90 {
            g.step();
        }
        assert_eq!(g.contacts(&e, 90), vec![7]);
    }

    #[test]
    fn the_budget_drops_the_oldest_paint() {
        let mut g = GoopField::new();
        g.max_chunks = 4;
        let host = FlatHost::default();
        for i in 0..10 {
            g.stamp(&host, Vec3::new(i as f32 * 5000.0, 0.0, 0.0), 50.0, GoopKind::Sink);
            g.step();
        }
        assert!(g.chunk_count() <= 4);
        assert!(g.is_polluted(Vec3::new(45000.0, 0.0, 0.0)));
        assert!(!g.is_polluted(Vec3::ZERO));
    }

    #[test]
    fn drying_is_off_unless_asked() {
        let mut g = GoopField::new();
        g.stamp(&FlatHost::default(), Vec3::ZERO, 200.0, GoopKind::Sink);
        for _ in 0..1000 {
            g.step();
        }
        assert!(g.is_polluted(Vec3::ZERO));
        g.dry_ticks = 1;
        for _ in 0..300 {
            g.step();
        }
        assert_eq!(g.chunk_count(), 0);
    }

    #[test]
    fn random_spots_are_on_goop() {
        let mut g = GoopField::new();
        g.stamp(&FlatHost::default(), Vec3::new(1000.0, 0.0, 0.0), 300.0, GoopKind::Sink);
        let mut s = 0.1f32;
        for _ in 0..20 {
            let p = g
                .random_spot(Vec3::ZERO, 2000.0, || {
                    s = (s * 7.31 + 0.13).fract();
                    s
                })
                .expect("goop in reach");
            assert!(g.is_polluted(p));
        }
        assert!(g.random_spot(Vec3::new(-9000.0, 0.0, 0.0), 500.0, || 0.5).is_none());
    }
}
