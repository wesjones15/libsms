//! The goop (Sunshine's pollution, `落書き`: Map/PollutionManager.cpp `TPollutionManager`,
//! PollutionLayer.cpp `TPollutionLayer`, PollutionAction.cpp, PollutionCount.cpp
//! `TPollutionCounterLayer`).
//!
//! In the game each stage has a few goop *layers*: flat or wall planes, each with an 8-bit
//! density texture (`mPollutionMap`, one texel per `mTexelSize` world units) projected onto the
//! map, and each of one *type* ([`GoopKind`]: brown sinking goop, burning goop, oil, electric
//! goop, ...). Everything that paints or cleans draws a *stamp* into those textures on the GPU
//! (`pushStampTask` -> `drawTexStamp`): `stamp(1, ..)` is the pollute stamp
//! (`/common/map/pollute.bti`), `clean` is `stamp(0, ..)` with `/common/map/clean.bti`, both blended
//! source-alpha over the density (`GX_BL_SRCALPHA`/`GX_BL_INVSRCALPHA`): a pollute stamp moves
//! each texel toward 255 by the stamp's alpha, a clean stamp toward 0. A point is goop when its
//! texel is over the type's threshold (`mPollutedThreshold`: 30 for most, 128 fire, 200 the
//! `UNK7` kind; `initLayerInfo`) and the point is at the layer's depth (`isSame`).
//!
//! A host like Risk of Rain 2 has no goop layers, so this is one field over the whole world: a
//! sparse grid of [`TEXEL`]-unit texels in 16x16 chunks, each texel with its density, its type
//! and the ground height it was painted at (probed through [`Host::ground`] when stamped).
//! Stamps blend like the game's; [`GoopField::is_polluted`] tests a point like
//! `TPollutionLayer::isPolluted`. What paints and what cleans call in: Shadow Mario's brush
//! ([`crate::koopajr`]), the goop enemies ([`crate::goop_critters`]), Mario running on oil
//! (`Event::GoopPaint`) and FLUDD's water (`TModelWaterManager::splashGround` / `splashWall`
//! clean at the particle with `mCleanSize * 10` / `* 32`; the FFI routes
//! [`crate::water::WaterManager`]'s ground impacts here).
//!
//! **The goop's look and sound** (`TPollutionLayer::action`): near Mario, a burning layer starts
//! `ms_newfire_b`/`_a` (at 1.5x) and `MSD_SE_EF_FIRE` at a random burning spot every
//! `mFireEffectWaitTime` (20) ticks within `mFireArea` (1600); an electric one `ms_thunder_s`
//! and `MSD_SE_EF_ELEC` every 16 ticks within `mThunderArea` (1000); every other kind ripples
//! (`ms_raku_indawa`) every 16 ticks within `mSpreadArea` (2000). [`GoopField::ambient`] does
//! that around the points a host passes (its players); [`GoopKind::surface`] names the layer
//! model whose material a host may draw each kind with.
//!
//! Not the game's (host choices for an open world, all tunable): the texel size (32 units, the
//! step `TMario::checkGraffito` probes goop with), the height band, the **lifetime** (goop
//! fades over [`GoopField::fade`] ticks and is gone [`GoopField::lifetime`] ticks after it was
//! last painted; 0 keeps it until cleaned, as in the game), and the **cap**
//! ([`GoopField::max_texels`] and [`GoopField::max_chunks`]: past either, the oldest goop goes
//! first). The two host kinds [`GoopKind::Poison`] and [`GoopKind::Lava`] use the disc's
//! `DmgGraffitoPoison`/`DmgGraffitoLava` tables, whose checks are empty in the game
//! (`checkGraffitoDamage`, `checkGraffitoLava`). Layer spreading (`FLAG_CAN_SPREAD`) isn't
//! ported.

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
/// The lifetime sweep visits each chunk once every this many ticks.
const SWEEP: u64 = 30;

/// `TPollutionLayer::getPollutionType` (PollutionLayer.hpp `POLLUTION_TYPE_*`), and two host
/// kinds past the game's range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
#[repr(u8)]
pub enum GoopKind {
    /// Brown goop Mario sinks into (`POLLUTION_TYPE_SINK`): `TMario::checkSink`.
    #[default]
    Sink = 0,
    /// Burning goop (`POLLUTION_TYPE_FIRE`): `checkGraffitoFire` sets his seat alight.
    Fire = 1,
    /// Oil Mario slides on (`POLLUTION_TYPE_SLIP`): `checkGraffitoSlip`, the `OIL_*` statuses.
    Slip = 2,
    /// Goop Mario can't walk into (`POLLUTION_TYPE_GLASS_WALL`): he's put back where he was.
    GlassWall = 3,
    /// Electric goop (`POLLUTION_TYPE_ELECTRIC`): `checkGraffitoElec`, `ELECTRIC_DAMAGE`.
    Electric = 4,
    /// Goop that swallows Mario at once (`POLLUTION_TYPE_INSTAKILL`): `SINK_LOSER`.
    Instakill = 5,
    /// Goop that only dirties (`POLLUTION_TYPE_SAFE`).
    Safe = 6,
    /// Dense burning goop (`POLLUTION_TYPE_UNK7`): burns like [`GoopKind::Fire`], threshold 200.
    Scorch = 7,
    /// Host kind: poison goop, a stumble from `DmgGraffitoPoison.prm` every `mSinkDmgTime`.
    Poison = 16,
    /// Host kind: lava goop, a burn with `DmgGraffitoLava.prm`.
    Lava = 17,
}

/// What a host might show for a kind.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GoopLook {
    /// Surface tint (linear RGB), for hosts not drawing the layer model.
    pub tint: [f32; 3],
    /// The ambient effects ([`GoopField::ambient`]) and their sound.
    pub effects: &'static [&'static str],
    pub effect_scale: f32,
    pub sound: Option<&'static str>,
    /// Ticks between ambient effects near a player, and how far round them.
    pub period: u32,
    pub area: f32,
}

impl GoopKind {
    pub const ALL: [GoopKind; 10] = [
        GoopKind::Sink,
        GoopKind::Fire,
        GoopKind::Slip,
        GoopKind::GlassWall,
        GoopKind::Electric,
        GoopKind::Instakill,
        GoopKind::Safe,
        GoopKind::Scorch,
        GoopKind::Poison,
        GoopKind::Lava,
    ];

    /// `mPollutedThreshold` for the kind (`TPollutionLayer::initLayerInfo`).
    pub fn threshold(self) -> u8 {
        match self {
            GoopKind::Scorch => 200,
            GoopKind::Fire | GoopKind::Lava => 128,
            _ => POLLUTED,
        }
    }

    pub fn from_u8(v: u8) -> GoopKind {
        GoopKind::ALL.into_iter().find(|k| *k as u8 == v).unwrap_or(GoopKind::Sink)
    }

    /// Burns what stands in it (`checkGraffitoFire`'s kinds).
    pub fn burns(self) -> bool {
        matches!(self, GoopKind::Fire | GoopKind::Scorch | GoopKind::Lava)
    }

    /// `checkGraffito`'s probe pattern: the oil kinds test his body's point and the eight
    /// around it; the sinking kinds his feet on the floor and the four beside them; electric
    /// goop just the point under his feet.
    pub fn probe(self) -> Probe {
        match self {
            GoopKind::Slip | GoopKind::Instakill | GoopKind::Safe => Probe::Square,
            GoopKind::Electric => Probe::Point,
            _ => Probe::Plus,
        }
    }

    /// The stage layer model (`/scene/map/pollution/<name>.bmd`, with its `.btk` scroll) a
    /// host may draw the kind with. The stages pair models and types per layer (scene .prm);
    /// this pairing is a host default.
    pub fn surface(self) -> &'static str {
        match self {
            GoopKind::Sink | GoopKind::Instakill => "pollution00",
            GoopKind::Fire | GoopKind::Scorch | GoopKind::Lava => "pollution03",
            GoopKind::Slip => "pollution02",
            GoopKind::Electric => "pollution04",
            GoopKind::GlassWall => "pollution05",
            GoopKind::Safe => "pollution01",
            GoopKind::Poison => "pollution06",
        }
    }

    /// The ambient effects and a tint (`TPollutionLayer::action`, `fire`, `electric`).
    pub fn look(self) -> GoopLook {
        const RIPPLE: &[&str] = &["ms_raku_indawa"];
        const FIRE: &[&str] = &["ms_newfire_b", "ms_newfire_a"];
        const THUNDER: &[&str] = &["ms_thunder_s"];
        let ripple = |tint| GoopLook {
            tint,
            effects: RIPPLE,
            effect_scale: 1.0,
            sound: None,
            period: 16,
            area: 2000.0,
        };
        let fire = |tint| GoopLook {
            tint,
            effects: FIRE,
            effect_scale: 1.5,
            sound: Some("MSD_SE_EF_FIRE"),
            period: 21,
            area: 1600.0,
        };
        match self {
            GoopKind::Sink => ripple([0.28, 0.16, 0.08]),
            GoopKind::Slip => ripple([0.05, 0.04, 0.03]),
            GoopKind::GlassWall => ripple([0.35, 0.55, 0.6]),
            GoopKind::Instakill => ripple([0.06, 0.02, 0.05]),
            GoopKind::Safe => ripple([0.4, 0.3, 0.2]),
            GoopKind::Poison => ripple([0.35, 0.05, 0.45]),
            GoopKind::Fire => fire([0.7, 0.18, 0.02]),
            GoopKind::Scorch => fire([0.55, 0.08, 0.02]),
            GoopKind::Lava => fire([0.9, 0.3, 0.03]),
            GoopKind::Electric => GoopLook {
                tint: [0.85, 0.75, 0.1],
                effects: THUNDER,
                effect_scale: 1.0,
                sound: Some("MSD_SE_EF_ELEC"),
                period: 16,
                area: 1000.0,
            },
        }
    }
}

/// Which points around Mario's feet `checkGraffito` tests for each kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    Point,
    Plus,
    Square,
}

impl Probe {
    /// Offsets in texels.
    pub fn offsets(self) -> &'static [(i8, i8)] {
        match self {
            Probe::Point => &[(0, 0)],
            Probe::Plus => &[(0, -1), (1, 0), (0, 0), (-1, 0), (0, 1)],
            Probe::Square => &[
                (-1, -1), (0, -1), (1, -1),
                (-1, 0), (0, 0), (1, 0),
                (-1, 1), (0, 1), (1, 1),
            ],
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
    /// Each texel's kind (the last paint's).
    pub kind: [GoopKind; CHUNK_TEXELS],
    /// [`GoopField::ticks`] (low 32 bits) when each texel was last painted.
    pub born: [u32; CHUNK_TEXELS],
    /// Bumped on every change, for hosts that cache meshes.
    pub version: u32,
    /// No live texel here is older than this (a lower bound; exact after a sweep).
    oldest: u32,
    /// Which tick of the [`SWEEP`] cycle visits it.
    slot: u8,
}

impl Chunk {
    fn new(slot: u8) -> Chunk {
        Chunk {
            density: [0; CHUNK_TEXELS],
            height: [0.0; CHUNK_TEXELS],
            kind: [GoopKind::Sink; CHUNK_TEXELS],
            born: [0; CHUNK_TEXELS],
            version: 0,
            oldest: u32::MAX,
            slot,
        }
    }

    fn empty(&self) -> bool {
        self.density.iter().all(|&d| d == 0)
    }

    fn live(&self) -> usize {
        self.density.iter().filter(|&&d| d > 0).count()
    }

    fn recompute_oldest(&mut self) {
        self.oldest = (0..CHUNK_TEXELS)
            .filter(|&i| self.density[i] > 0)
            .map(|i| self.born[i])
            .min()
            .unwrap_or(u32::MAX);
    }

    /// Clears texels painted at or before `upto`; returns how many.
    fn drop_older(&mut self, upto: u32) -> usize {
        let mut n = 0;
        for i in 0..CHUNK_TEXELS {
            if self.density[i] > 0 && self.born[i] <= upto {
                self.density[i] = 0;
                n += 1;
            }
        }
        if n > 0 {
            self.version = self.version.wrapping_add(1);
        }
        self.recompute_oldest();
        n
    }
}

/// What the field did that a host may show or play.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GoopEvent {
    /// A stamp went down (`Painted` sounds are the painter's own).
    Painted { pos: Vec3, radius: f32, kind: GoopKind },
    /// Dense goop wiped off (`TPollutionLayer::cleaned`): `MSD_SE_ERASE_SCRAWL` at a volume of
    /// the stamp's size and the `ms_raku_kie` steam (`PARTICLE_MS_RAKU_KIE`).
    Erased { pos: Vec3, size: f32 },
    /// A kind's ambient effect near a player ([`GoopField::ambient`]): start each of the
    /// look's `effects` at `pos`, scaled, and its sound.
    Ambient { pos: Vec3, kind: GoopKind },
}

/// The world's goop.
#[derive(Debug, Clone)]
pub struct GoopField {
    chunks: HashMap<(i32, i32), Chunk>,
    /// Chunks kept at most; painting past it drops the chunks with the oldest goop.
    pub max_chunks: usize,
    /// Goop texels kept at most; painting past it clears the oldest goop first.
    pub max_texels: usize,
    /// Ticks goop lasts after it was last painted (0: until cleaned, as in the game).
    pub lifetime: u32,
    /// Ticks at the end of its life that goop thins out over.
    pub fade: u32,
    pub pollute: Mask,
    pub clean: Mask,
    pub events: Vec<GoopEvent>,
    pub ticks: u64,
    live: usize,
    next_slot: u8,
    /// `TPollutionLayer::cleaned`'s `effect_counter` and offset tables' cursors.
    erase_counter: u32,
    erase_cursor: (usize, usize),
    /// Per-enemy ticks left before goop hurts it again ([`GoopField::contacts`]).
    cooldowns: HashMap<u64, u32>,
    /// `mEffectTimer` per kind, per player slot.
    ambient_timers: HashMap<(usize, GoopKind), u32>,
    seed: u32,
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

/// Default lifetime: 90 seconds at 120 Hz.
pub const DEFAULT_LIFETIME: u32 = 90 * 120;
/// Default fade: the last 5 seconds.
pub const DEFAULT_FADE: u32 = 5 * 120;
/// Default cap: 48k texels (about 49 million square units, a 7000-unit square).
pub const DEFAULT_MAX_TEXELS: usize = 48 * 1024;

impl GoopField {
    pub fn new() -> GoopField {
        GoopField {
            chunks: HashMap::new(),
            max_chunks: 768,
            max_texels: DEFAULT_MAX_TEXELS,
            lifetime: DEFAULT_LIFETIME,
            fade: DEFAULT_FADE,
            pollute: Mask::disc(32),
            clean: Mask::disc(32),
            events: Vec::new(),
            ticks: 0,
            live: 0,
            next_slot: 0,
            erase_counter: 1,
            erase_cursor: (0, 0),
            cooldowns: HashMap::new(),
            ambient_timers: HashMap::new(),
            seed: 0x2545_F491,
        }
    }

    fn rand(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Density at `pos`'s texel (0 when none), ignoring height.
    pub fn density(&self, pos: Vec3) -> u8 {
        let (key, i) = chunk_of(pos.x, pos.z);
        self.chunks.get(&key).map_or(0, |c| c.density[i])
    }

    /// `TPollutionManager::isPolluted`: is the point on goop (its texel over its kind's
    /// threshold and the point at the texel's height)?
    pub fn is_polluted(&self, pos: Vec3) -> bool {
        self.kind_at(pos).is_some()
    }

    /// `getPollutionType` for a point that [`Self::is_polluted`].
    pub fn kind_at(&self, pos: Vec3) -> Option<GoopKind> {
        let (key, i) = chunk_of(pos.x, pos.z);
        let c = self.chunks.get(&key)?;
        let k = c.kind[i];
        (c.density[i] > k.threshold() && (pos.y - c.height[i]).abs() < HEIGHT_BAND).then_some(k)
    }

    /// `TMario::checkGraffito`'s test for ordinary goop: the point and the eight around it a
    /// texel away all on goop (a lone texel or the very rim doesn't catch Mario).
    pub fn is_polluted_under(&self, pos: Vec3) -> bool {
        Probe::Square.offsets().iter().all(|&(dx, dz)| {
            self.is_polluted(pos + Vec3::new(dx as f32 * TEXEL, 0.0, dz as f32 * TEXEL))
        })
    }

    /// Sum of every texel's density (`getPollutionDegree`, unscaled).
    pub fn degree(&self) -> u64 {
        self.chunks
            .values()
            .map(|c| c.density.iter().map(|&d| d as u64).sum::<u64>())
            .sum()
    }

    /// The chunks, for drawing: key, kinds, densities, heights, version.
    pub fn chunks(&self) -> impl Iterator<Item = ((i32, i32), &Chunk)> {
        self.chunks.iter().map(|(k, c)| (*k, c))
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// Goop texels in the world.
    pub fn texel_count(&self) -> usize {
        self.live
    }

    pub fn clear(&mut self) {
        self.chunks.clear();
        self.live = 0;
    }

    /// `stamp(1, x, y, z, size)`: paints a disc of goop of `radius` at `pos` with the pollute
    /// stamp. `pos.y` is the ground the painter stands on.
    pub fn stamp(&mut self, host: &dyn Host, pos: Vec3, radius: f32, kind: GoopKind) {
        let mask = std::mem::replace(&mut self.pollute, Mask { size: 0, alpha: Vec::new() });
        self.stamp_mask(host, pos, radius, 0.0, &mask, kind);
        self.pollute = mask;
        self.events.push(GoopEvent::Painted { pos, radius, kind });
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
        let tick = self.ticks as u32;
        let mut added = 0usize;
        self.for_texels(pos, reach, true, |x, z, chunk, i| {
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
            let new = (old + (255.0 - old) * a).round().clamp(0.0, 255.0) as u8;
            if new == 0 {
                return;
            }
            if old == 0.0 {
                added += 1;
            }
            chunk.density[i] = new;
            chunk.height[i] = h;
            chunk.kind[i] = kind;
            chunk.born[i] = tick;
            chunk.oldest = chunk.oldest.min(tick);
        });
        self.live += added;
        self.enforce_budget();
    }

    /// `TPollutionManager::clean` (`stamp(0, ..)`): wipes goop off a disc of `radius` around
    /// `pos` with the clean stamp. Returns how many texels lost goop.
    pub fn clean(&mut self, pos: Vec3, radius: f32) -> u32 {
        let mask = std::mem::replace(&mut self.clean, Mask { size: 0, alpha: Vec::new() });
        let mut dense = false;
        let mut changed = 0u32;
        let mut gone = 0usize;
        self.for_texels(pos, radius, false, |x, z, chunk, i| {
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
                if new == 0 {
                    gone += 1;
                }
            }
        });
        self.live = self.live.saturating_sub(gone);
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
    /// chunks are made; without, only existing ones are visited. Bumps the version of every
    /// chunk visited.
    fn for_texels(
        &mut self,
        pos: Vec3,
        reach: f32,
        create: bool,
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
                    let slot = &mut self.next_slot;
                    self.chunks.entry(key).or_insert_with(|| {
                        let c = Chunk::new(*slot);
                        *slot = ((*slot as u64 + 1) % SWEEP) as u8;
                        c
                    })
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
                }
            }
        }
        if create {
            self.chunks.retain(|_, c| !c.empty());
        }
    }

    /// The cap: past [`Self::max_chunks`] the chunks holding the oldest goop go whole; past
    /// [`Self::max_texels`] the oldest goop is cleared, a tick's worth of paint at a time.
    fn enforce_budget(&mut self) {
        while self.chunks.len() > self.max_chunks.max(1) {
            let Some(oldest) = self.chunks.iter().min_by_key(|(_, c)| c.oldest).map(|(k, _)| *k)
            else {
                break;
            };
            if let Some(c) = self.chunks.remove(&oldest) {
                self.live = self.live.saturating_sub(c.live());
            }
        }
        let cap = self.max_texels.max(1);
        let mut guard = 0;
        while self.live > cap && guard < 4096 {
            guard += 1;
            let Some((key, upto)) =
                self.chunks.iter().min_by_key(|(_, c)| c.oldest).map(|(k, c)| (*k, c.oldest))
            else {
                break;
            };
            let c = self.chunks.get_mut(&key).unwrap();
            // `oldest` may be a stale lower bound: tighten it before clearing.
            c.recompute_oldest();
            if c.oldest > upto {
                continue;
            }
            let n = c.drop_older(upto);
            self.live = self.live.saturating_sub(n);
            if c.empty() {
                self.chunks.remove(&key);
            }
        }
    }

    /// One tick: the lifetime sweep and the contact cooldowns.
    pub fn step(&mut self) {
        self.ticks += 1;
        self.cooldowns.retain(|_, t| {
            *t = t.saturating_sub(1);
            *t > 0
        });
        if self.lifetime == 0 {
            return;
        }
        let now = self.ticks as u32;
        let slot = (self.ticks % SWEEP) as u8;
        let (life, fade) = (self.lifetime, self.fade.min(self.lifetime));
        let mut gone = 0usize;
        for c in self.chunks.values_mut().filter(|c| c.slot == slot) {
            if c.oldest == u32::MAX || now.wrapping_sub(c.oldest) < life - fade {
                continue;
            }
            let mut changed = false;
            for i in 0..CHUNK_TEXELS {
                let d = c.density[i];
                if d == 0 {
                    continue;
                }
                let age = now.wrapping_sub(c.born[i]);
                let new = if age >= life {
                    0
                } else if fade > 0 && age + fade > life {
                    let left = (life - age) as f32 / fade as f32;
                    d.min((255.0 * left) as u8)
                } else {
                    d
                };
                if new != d {
                    c.density[i] = new;
                    changed = true;
                    if new == 0 {
                        gone += 1;
                    }
                }
            }
            if changed {
                c.version = c.version.wrapping_add(1);
                c.recompute_oldest();
            }
        }
        if gone > 0 {
            self.live = self.live.saturating_sub(gone);
            self.chunks.retain(|_, c| !c.empty());
        }
    }

    /// `TPollutionLayer::action`'s effects round each of `around` (the players): a random spot
    /// on goop near the player (`getPollutedPos`, five tries) advances that kind's timer, and
    /// every [`GoopLook::period`] ticks it gets the kind's effects ([`GoopEvent::Ambient`]).
    pub fn ambient(&mut self, around: &[Vec3]) {
        if self.chunks.is_empty() {
            return;
        }
        for (slot, &p) in around.iter().enumerate() {
            for _ in 0..5 {
                let area = 2000.0;
                let q = Vec3::new(
                    p.x + area * (self.rand() - 0.5),
                    p.y,
                    p.z + area * (self.rand() - 0.5),
                );
                let (key, i) = chunk_of(q.x, q.z);
                let Some(c) = self.chunks.get(&key) else { continue };
                let k = c.kind[i];
                let look = k.look();
                let at = Vec3::new(q.x, c.height[i], q.z);
                let d = (at - p).with_y(0.0).abs().max_element();
                if c.density[i] <= k.threshold() || d > look.area * 0.5 || at.y > p.y + HEIGHT_BAND {
                    continue;
                }
                let t = self.ambient_timers.entry((slot, k)).or_insert(0);
                *t += 1;
                if *t >= look.period {
                    *t = 0;
                    self.events.push(GoopEvent::Ambient { pos: at, kind: k });
                }
                break;
            }
        }
    }

    /// Host adaptation of `TMario::checkSink`'s floor damage for anything standing in goop:
    /// which of `enemies` have their feet on goop now, and its kind, for those not still
    /// cooling down from the last time. Each one returned waits `period` ticks
    /// (`mSinkDmgTime`) before it can be returned again. What the goop does to them is the
    /// host's.
    pub fn contacts(&mut self, enemies: &[Enemy], period: u32) -> Vec<(u64, GoopKind)> {
        let mut out = Vec::new();
        for e in enemies {
            if self.cooldowns.contains_key(&e.id) {
                continue;
            }
            if let Some(k) = self.kind_at(e.pos) {
                self.cooldowns.insert(e.id, period.max(1));
                out.push((e.id, k));
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
                if c.density[i] <= c.kind[i].threshold() {
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
        assert_eq!(g.contacts(&e, 90), vec![(7, GoopKind::Sink)]);
        assert!(g.contacts(&e, 90).is_empty());
        for _ in 0..90 {
            g.step();
        }
        assert_eq!(g.contacts(&e, 90), vec![(7, GoopKind::Sink)]);
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
    fn goop_fades_and_expires_after_its_lifetime() {
        let mut g = GoopField::new();
        g.lifetime = 600;
        g.fade = 120;
        g.stamp(&FlatHost::default(), Vec3::ZERO, 200.0, GoopKind::Sink);
        let full = g.density(Vec3::ZERO);
        for _ in 0..540 {
            g.step();
        }
        let thin = g.density(Vec3::ZERO);
        assert!(thin > 0 && thin < full, "fading: {full} -> {thin}");
        for _ in 0..90 {
            g.step();
        }
        assert_eq!(g.chunk_count(), 0);
        assert_eq!(g.texel_count(), 0);
        // Lifetime 0 keeps it, as in the game.
        g.lifetime = 0;
        g.stamp(&FlatHost::default(), Vec3::ZERO, 200.0, GoopKind::Sink);
        for _ in 0..20_000 {
            g.step();
        }
        assert!(g.is_polluted(Vec3::ZERO));
    }

    #[test]
    fn repainting_renews_the_lifetime() {
        let mut g = GoopField::new();
        g.lifetime = 600;
        g.fade = 0;
        let host = FlatHost::default();
        g.stamp(&host, Vec3::ZERO, 200.0, GoopKind::Sink);
        for _ in 0..500 {
            g.step();
        }
        g.stamp(&host, Vec3::ZERO, 200.0, GoopKind::Sink);
        for _ in 0..500 {
            g.step();
        }
        assert!(g.is_polluted(Vec3::ZERO));
    }

    #[test]
    fn over_the_cap_the_oldest_goop_makes_way() {
        let mut g = GoopField::new();
        let host = FlatHost::default();
        g.stamp(&host, Vec3::ZERO, 200.0, GoopKind::Sink);
        let one = g.texel_count();
        g.max_texels = one;
        g.step();
        g.stamp(&host, Vec3::new(3000.0, 0.0, 0.0), 200.0, GoopKind::Fire);
        assert!(g.texel_count() <= g.max_texels);
        assert!(!g.is_polluted(Vec3::ZERO), "the first stamp went");
        assert_eq!(g.kind_at(Vec3::new(3000.0, 0.0, 0.0)), Some(GoopKind::Fire));
    }

    #[test]
    fn each_texel_keeps_its_own_kind() {
        let mut g = GoopField::new();
        let host = FlatHost::default();
        g.stamp(&host, Vec3::ZERO, 300.0, GoopKind::Sink);
        g.stamp(&host, Vec3::new(64.0, 0.0, 0.0), 100.0, GoopKind::Electric);
        assert_eq!(g.kind_at(Vec3::new(64.0, 0.0, 0.0)), Some(GoopKind::Electric));
        assert_eq!(g.kind_at(Vec3::new(-200.0, 0.0, 0.0)), Some(GoopKind::Sink));
        for k in GoopKind::ALL {
            assert_eq!(GoopKind::from_u8(k as u8), k);
        }
        // Fire's threshold is higher: a thin coat isn't burning goop yet.
        assert!(GoopKind::Scorch.threshold() > GoopKind::Fire.threshold());
    }

    #[test]
    fn burning_and_electric_goop_crackle_near_a_player() {
        let mut g = GoopField::new();
        let host = FlatHost::default();
        g.stamp(&host, Vec3::ZERO, 900.0, GoopKind::Fire);
        g.stamp(&host, Vec3::new(4000.0, 0.0, 0.0), 900.0, GoopKind::Electric);
        for _ in 0..400 {
            g.ambient(&[Vec3::ZERO, Vec3::new(4000.0, 0.0, 0.0)]);
        }
        let n = |k| g.events.iter().filter(|e| matches!(e, GoopEvent::Ambient { kind, .. } if *kind == k)).count();
        assert!(n(GoopKind::Fire) >= 5 && n(GoopKind::Electric) >= 5);
        assert_eq!(GoopKind::Fire.look().sound, Some("MSD_SE_EF_FIRE"));
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
