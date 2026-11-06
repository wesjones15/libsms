//! Bowser Jr. as a player: Shadow Mario's form and his own, on Mario's body and acrobatics.
//!
//! * **Shadow Mario** (`TEnemyMario : TMario`, Enemy/enemyMario.cpp): Mario's rig, statuses and
//!   physics with no FLUDD (`offFlag(MARIO_FLAG_HAS_FLUDD)`) and goop never catching him
//!   (`checkGraffito`'s `PLAYER_TYPE_SHADOW_MARIO` return). He carries the magic brush
//!   (`kagemario_brush.bmd` on `jnt_hand_L`, [`BRUSH_JOINT`]) and paints with it:
//!   - **the brush** (in place of FLUDD, while R is held on the ground): a goop trail at his
//!     feet, `mPolluteSize` (160) wide (`emReplay`'s `pollute` while `mPolluteFlag`), with
//!     `MSD_SE_MA_KAGE_PAINTS`;
//!   - **landings**: a jump's landing splats 384 of goop (`emJumping`'s `stamp(1, .., 384)`);
//!   - **the sign** (`emDrawStamp`): Bowser Jr.'s mark (`stamp_koopa_sign`, the actor drawn at
//!     3x and stamped with `stampModel`) in front of him, with `MSD_SE_MA_KAGE_PAINTS` and
//!     `MSD_SE_MA_KAGE_PAINTINGWIND`;
//!   - **hiding in goop** (`emHide`): standing on goop he sinks into it: no collision until
//!     the goop under him is gone or [`JrParams::hide_max`] (7200) ticks pass, then he pops
//!     out (`emAppear`) flashing for 120 ticks. The dive in and out is `ms_kgm_change` with
//!     `MSD_SE_MA_KAGE_FIELD_AWAY` / `_APPEAR` (`emDisappear`).
//!   - **water** (`receiveMessage`'s spray): each hit takes one of `mWaterCounter`'s 64; at 0
//!     he's drenched ([`JrEvent::Drenched`]: the game puts him down to sleep,
//!     `EM_DOING_PRE_DOWN_ANIMATION`); 600 ticks without a hit and the counter is full again
//!     (`mWaterHitTimer`).
//! * **Bowser Jr.** (`TLimitKoopaJr`, Enemy/limitkoopajr.cpp; the disc's `koopajr_model.bmd`):
//!   [`Form::Jr`]. The disc has no Bowser Jr. walk: here he moves on Mario's physics and the
//!   host shows his four clips by [`KoopaJr::jr_clip`] (`koopajr_wait` standing and moving,
//!   `_yahoo` jumping, as `TNerveLimitKoopaJrYahoo` cheers Mario's jumps, `_shoot` shooting,
//!   `_damage` hurt). His shot is the goop ball (`TNerveLimitKoopaJrLaunch`, every
//!   `mSLShotDoodlePeriod` = 600 in the game; [`JrParams::shot_period`] here) with
//!   `ms_koopajr_killer` at the mouth, and he spits Piranha seeds that grow goop Piranhas
//!   ([`crate::goop_critters`]).
//! * **Changing form** ([`KoopaJr::change_form`]): `ms_kgm_change` and
//!   `MSD_SE_MA_KAGE_FIELD_AWAY` (the game changes form in THP movies; this is the in-world
//!   disappearance used for it).
//!
//! * **Brush attacks** (host moves, not the game's: the disc's Shadow Mario only paints at his
//!   feet and draws on gates): the brush button tapped swings it ([`Brush::Swing`], Mario's
//!   `ma_draw` stroke, `ANIM_DRAW` as `startGateDrawing` plays it, over his upper body) and a
//!   blob of goop flies from the brush to where the player aims ([`KoopaJr::aim_at`]); held, he
//!   brandishes it forward ([`Brush::Spray`]) and sprays a stream of small blobs. Each blob
//!   ([`crate::goop_critters::ShotKind::Paint`]) splats his paint kind where it lands. The feet
//!   trail below still runs while the button is held on the ground.
//!
//! The host owns the [`GoopField`] and [`Critters`]; [`KoopaJr::step`] runs after
//! [`Mario::tick`] each tick. The kinds he paints are [`KoopaJr::paint_kind`] (brown goop by
//! default; the host may cycle them). Reconstructed: the sign's world size (`stamp_koopa_sign`'s
//! extent at 3x is the host's mask), the brush's stamp interval, the shot's speed and arc.

use glam::Vec3;

use crate::goop::{GoopField, GoopKind, Mask};
use crate::goop_critters::Critters;
use crate::host::Host;
use crate::mario::Mario;
use crate::math::{cos, sin};

/// The joint the brush hangs from (`kagemario_brush.bmd`, `TEnemyMario::initValues`).
pub const BRUSH_JOINT: &str = "jnt_hand_L";

/// Mario's gate-drawing clip (`ANIM_DRAW`, `ma_draw.bck`), the brush strokes' pose.
pub const DRAW_CLIP: &str = "draw";

/// Shadow Mario's brush attack this tick.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Brush {
    #[default]
    Idle,
    /// The stroke, `tick` 120 Hz ticks in; `shot` once its blob has gone.
    Swing { tick: u32, shot: bool },
    /// The brush held out forward, spraying.
    Spray { tick: u32 },
}

/// Which of the two he is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum Form {
    #[default]
    Shadow = 0,
    Jr = 1,
}

/// Bowser Jr.'s clips by `TLimitKoopaJr`'s index (`koopajr_*.bck` in file order).
pub mod jr_clip {
    pub const DAMAGE: u8 = 0;
    pub const SHOOT: u8 = 1;
    pub const WAIT: u8 = 2;
    pub const YAHOO: u8 = 3;
    pub const NAMES: [&str; 4] = ["damage", "shoot", "wait", "yahoo"];
}

#[derive(Debug, Clone, PartialEq)]
pub struct JrParams {
    /// `mPolluteSize`: the brush trail's radius.
    pub brush_size: f32,
    /// Ticks between brush stamps.
    pub brush_every: u32,
    /// `emJumping`'s landing splat radius.
    pub land_size: f32,
    /// Air ticks a landing needs to splat (a step down doesn't).
    pub land_air_ticks: u32,
    /// Half-side of the sign stamp (the actor at 3x).
    pub sign_half: f32,
    /// How far in front of him the sign goes.
    pub sign_ahead: f32,
    /// Ticks the sign takes, during which he can't paint another.
    pub sign_ticks: u32,
    pub hide_max: u32,
    pub appear_invincible: i16,
    /// `mWaterCtMax`.
    pub water_max: u32,
    /// `mWaterHitTimer`'s refill.
    pub water_refill: u32,
    /// Ticks between form changes.
    pub change_cooldown: u32,
    pub shot_period: u32,
    pub shot_speed: f32,
    pub shot_gravity: f32,
    /// The shot's goop splat radius (`mSLStampRange * 32 * scale`).
    pub shot_splat: f32,
    pub seed_period: u32,
    pub seed_speed: f32,
    /// The swing's frames of `ma_draw` (the hand rises forward from `swing_from`, is furthest
    /// ahead at `swing_release` and has swept down and back by `swing_to`) and its rate.
    pub swing_from: f32,
    pub swing_release: f32,
    pub swing_to: f32,
    pub swing_rate: f32,
    /// The swing's blob: speed (units a tick), gravity, splat radius, damage share.
    pub swing_speed: f32,
    pub swing_gravity: f32,
    pub swing_splat: f32,
    pub swing_damage: f32,
    /// The spray's pose: `ma_draw` swayed between these frames (the brush out in front).
    pub spray_from: f32,
    pub spray_to: f32,
    /// Ticks between spray blobs, and theirs.
    pub spray_every: u32,
    pub spray_speed: f32,
    pub spray_gravity: f32,
    pub spray_splat: f32,
    pub spray_damage: f32,
    /// The spray's scatter, radians either way.
    pub spray_spread: f32,
}

impl Default for JrParams {
    fn default() -> Self {
        JrParams {
            brush_size: 160.0,
            brush_every: 6,
            land_size: 384.0,
            land_air_ticks: 20,
            sign_half: 450.0,
            sign_ahead: 350.0,
            sign_ticks: 120,
            hide_max: 7200,
            appear_invincible: 120,
            water_max: 64,
            water_refill: 600,
            change_cooldown: 60,
            shot_period: 600,
            shot_speed: 20.0,
            shot_gravity: 0.15,
            shot_splat: 200.0,
            seed_period: 900,
            seed_speed: 15.0,
            swing_from: 28.0,
            swing_release: 44.0,
            swing_to: 72.0,
            swing_rate: 2.0,
            swing_speed: 40.0,
            swing_gravity: 0.15,
            swing_splat: 220.0,
            swing_damage: 1.0,
            spray_from: 40.0,
            spray_to: 52.0,
            spray_every: 8,
            spray_speed: 30.0,
            spray_gravity: 0.3,
            spray_splat: 90.0,
            spray_damage: 0.15,
            spray_spread: 0.06,
        }
    }
}

/// What he did, for the host to show and play.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum JrEvent {
    FormChanged { form: Form, pos: Vec3 },
    /// Sank into the goop (`emHide`) / came out (`emAppear`).
    Hidden { pos: Vec3 },
    Appeared { pos: Vec3 },
    /// Out of water hits (`mWaterCounter` at 0).
    Drenched { pos: Vec3 },
    /// He painted goop.
    Painted { pos: Vec3, radius: f32, kind: GoopKind },
    /// The sign went down, turned `yaw` (radians).
    Sign { pos: Vec3, yaw: f32 },
    Sound { name: &'static str, pos: Vec3 },
    Effect { name: &'static str, pos: Vec3, scale: f32 },
}

#[derive(Debug, Clone)]
pub struct KoopaJr {
    pub form: Form,
    pub params: JrParams,
    pub paint_kind: GoopKind,
    /// Under the goop: not drawn, no collision.
    pub hidden: bool,
    /// The brush is on the goop this tick (the host shows the stroke).
    pub painting: bool,
    /// `mWaterCounter`.
    pub water: u32,
    /// Bowser Jr.'s mark seen from above (the host's projection of `stamp_koopa_sign`; a disc
    /// stands in).
    pub sign_mask: Mask,
    /// Ticks into the sign, while painting it.
    pub sign_timer: u32,
    /// Ticks into the shot clip, while shooting.
    pub shot_timer: u32,
    pub events: Vec<JrEvent>,
    /// The host's id for him (his shots don't hit him).
    pub id: u64,
    /// The brush attack under way.
    pub brush: Brush,
    /// What the player aims at (the crosshair's point), for the brush's blobs; straight ahead
    /// along Mario's aim when None.
    pub aim_at: Option<Vec3>,
    brush_was: bool,
    spray_seed: u32,
    hide_ticks: u32,
    shot_cooldown: u32,
    seed_cooldown: u32,
    change_cooldown: u32,
    water_idle: u32,
    brush_tick: u32,
    air: u32,
    hurt_timer: u32,
    last_health: i16,
}

impl Default for KoopaJr {
    fn default() -> Self {
        KoopaJr::new(JrParams::default())
    }
}

impl KoopaJr {
    pub fn new(params: JrParams) -> KoopaJr {
        KoopaJr {
            form: Form::Shadow,
            water: params.water_max,
            params,
            paint_kind: GoopKind::Sink,
            hidden: false,
            painting: false,
            sign_mask: Mask::disc(32),
            sign_timer: 0,
            shot_timer: 0,
            events: Vec::new(),
            id: 0,
            brush: Brush::Idle,
            aim_at: None,
            brush_was: false,
            spray_seed: 0x5EED,
            hide_ticks: 0,
            shot_cooldown: 0,
            seed_cooldown: 0,
            change_cooldown: 0,
            water_idle: 0,
            brush_tick: 0,
            air: 0,
            hurt_timer: 0,
            last_health: i16::MAX,
        }
    }

    /// Makes `m` his body: no FLUDD, and goop never catches him.
    pub fn attach(&self, m: &mut Mario) {
        m.remove_fludd();
        m.goop.immune = true;
    }

    fn feet(m: &Mario) -> Vec3 {
        Vec3::new(m.pos.x, m.floor_y, m.pos.z)
    }

    fn forward(m: &Mario) -> Vec3 {
        Vec3::new(sin(m.face_yaw), 0.0, cos(m.face_yaw))
    }

    fn sound(&mut self, name: &'static str, pos: Vec3) {
        self.events.push(JrEvent::Sound { name, pos });
    }

    fn effect(&mut self, name: &'static str, pos: Vec3, scale: f32) {
        self.events.push(JrEvent::Effect { name, pos, scale });
    }

    /// Whether he's drawn and can be hit.
    pub fn collides(&self) -> bool {
        !self.hidden
    }

    /// Shadow Mario <-> Bowser Jr. Returns whether he changed.
    pub fn change_form(&mut self, m: &Mario) -> bool {
        if self.change_cooldown > 0 || self.hidden || m.is_down() {
            return false;
        }
        self.form = match self.form {
            Form::Shadow => Form::Jr,
            Form::Jr => Form::Shadow,
        };
        self.change_cooldown = self.params.change_cooldown;
        let pos = m.pos + Vec3::Y * 80.0;
        self.effect("ms_kgm_change", pos, 1.0);
        self.sound("MSD_SE_MA_KAGE_FIELD_AWAY", m.pos);
        self.events.push(JrEvent::FormChanged { form: self.form, pos });
        true
    }

    /// Sinks into the goop under him (`emHide`). Only Shadow Mario, only on goop.
    pub fn hide(&mut self, m: &Mario, goop: &GoopField) -> bool {
        if self.form != Form::Shadow || self.hidden || !m.on_ground() || m.is_down() {
            return false;
        }
        if !goop.is_polluted(Self::feet(m)) {
            return false;
        }
        self.hidden = true;
        self.hide_ticks = 0;
        let pos = m.pos + Vec3::Y * 80.0;
        self.effect("ms_kgm_change", pos, 1.0);
        self.sound("MSD_SE_MA_KAGE_FIELD_AWAY", m.pos);
        self.events.push(JrEvent::Hidden { pos: m.pos });
        true
    }

    /// Comes out of the goop (`emAppear`): flashing for a while.
    pub fn appear(&mut self, m: &mut Mario) {
        if !self.hidden {
            return;
        }
        self.hidden = false;
        self.hide_ticks = 0;
        m.invincible = m.invincible.max(self.params.appear_invincible as _);
        let pos = m.pos + Vec3::Y * 80.0;
        self.effect("ms_kgm_change", pos, 1.0);
        self.sound("MSD_SE_MA_KAGE_FIELD_APPEAR", m.pos);
        self.events.push(JrEvent::Appeared { pos: m.pos });
    }

    /// Paints Bowser Jr.'s sign in front of him (`emDrawStamp`).
    pub fn paint_sign(&mut self, m: &Mario, host: &dyn Host, goop: &mut GoopField) -> bool {
        if self.form != Form::Shadow || self.hidden || self.sign_timer > 0 || !m.on_ground() {
            return false;
        }
        let yaw = (m.face_yaw as f32) * std::f32::consts::PI / 32768.0;
        let at = Self::feet(m) + Self::forward(m) * self.params.sign_ahead;
        let mask = std::mem::replace(&mut self.sign_mask, Mask { size: 0, alpha: Vec::new() });
        goop.stamp_mask(host, at, self.params.sign_half, yaw, &mask, self.paint_kind);
        self.sign_mask = mask;
        self.sign_timer = 1;
        self.sound("MSD_SE_MA_KAGE_PAINTS", m.pos);
        self.sound("MSD_SE_MA_KAGE_PAINTINGWIND", m.pos);
        self.events.push(JrEvent::Sign { pos: at, yaw });
        self.events.push(JrEvent::Painted { pos: at, radius: self.params.sign_half, kind: self.paint_kind });
        true
    }

    fn mouth(m: &Mario) -> Vec3 {
        m.pos + Vec3::Y * 100.0 + Self::forward(m) * 60.0
    }

    /// Bowser Jr.'s goop ball (`TNerveLimitKoopaJrLaunch`). Returns the shot's id.
    pub fn shoot(&mut self, m: &Mario, critters: &mut Critters) -> Option<u64> {
        if self.form != Form::Jr || self.shot_cooldown > 0 || m.is_down() {
            return None;
        }
        self.shot_cooldown = self.params.shot_period;
        self.shot_timer = 1;
        let from = Self::mouth(m);
        let vel = Self::forward(m) * self.params.shot_speed + Vec3::Y * 4.0;
        self.effect("ms_koopajr_killer", from, 1.0);
        let id = critters.shoot_goop(from, vel, self.params.shot_gravity, self.params.shot_splat, self.id);
        Some(id)
    }

    /// Spits a Piranha seed: a goop Piranha grows where it lands.
    pub fn spit_seed(&mut self, m: &Mario, critters: &mut Critters) -> Option<u64> {
        if self.form != Form::Jr || self.seed_cooldown > 0 || m.is_down() {
            return None;
        }
        self.seed_cooldown = self.params.seed_period;
        self.shot_timer = 1;
        let from = Self::mouth(m);
        let vel = Self::forward(m) * self.params.seed_speed * 0.6 + Vec3::Y * self.params.seed_speed * 0.6;
        Some(critters.throw_seed(from, vel, self.id))
    }

    /// FLUDD's water hit him. Returns true when it drenched him (the counter ran out).
    pub fn water_hit(&mut self, m: &Mario) -> bool {
        self.water_idle = 0;
        self.effect("ms_enm_wathit", m.pos + Vec3::Y * 80.0, 1.0);
        self.sound("MSD_SE_EN_COMMON_W_HIT_OK", m.pos);
        if self.water > 0 {
            self.water -= 1;
            return false;
        }
        self.events.push(JrEvent::Drenched { pos: m.pos });
        self.water = self.params.water_max;
        true
    }

    /// The brush's pose this tick: `ma_draw`'s frame over his upper body, or None.
    pub fn brush_frame(&self) -> Option<f32> {
        let p = &self.params;
        match self.brush {
            Brush::Idle => None,
            Brush::Swing { tick, .. } => Some((p.swing_from + tick as f32 * p.swing_rate * 0.5).min(p.swing_to)),
            Brush::Spray { tick } => {
                // Swayed back and forth over the spray's frames, a sway a second.
                let t = (tick % 120) as f32 / 120.0;
                let s = 0.5 - 0.5 * (t * std::f32::consts::TAU).cos();
                Some(p.spray_from + (p.spray_to - p.spray_from) * s)
            }
        }
    }

    /// Where the brush's blobs leave from: the brush in his left hand, out in front.
    fn brush_tip(m: &Mario, yaw: f32) -> Vec3 {
        let (s, c) = yaw.sin_cos();
        let forward = Vec3::new(s, 0.0, c);
        let left = Vec3::new(c, 0.0, -s);
        m.pos + Vec3::Y * 95.0 + forward * 70.0 + left * 25.0
    }

    /// The heading (radians) the brush points: at the aim point, else his aim or facing.
    fn brush_yaw(&self, m: &Mario) -> f32 {
        if let Some(t) = self.aim_at {
            let d = t - m.pos;
            if d.x * d.x + d.z * d.z > 1.0 {
                return d.x.atan2(d.z);
            }
        }
        let a = m.aim.map_or(m.face_yaw, |a| a.yaw);
        a as f32 * std::f32::consts::PI / 32768.0
    }

    /// A blob's start velocity toward the aim point (arced for its gravity), else along his
    /// aim's pitch. `spread` turns it by that many radians.
    fn blob_velocity(&self, m: &Mario, from: Vec3, speed: f32, gravity: f32, spread: f32) -> Vec3 {
        let speed = speed.max(1.0);
        let mut v = match self.aim_at {
            Some(t) if (t - from).length() > 50.0 => {
                let d = t - from;
                let ticks = (d.length() / speed).clamp(1.0, 240.0);
                // Each tick the shot falls `gravity` more before it moves.
                d / ticks + Vec3::Y * (gravity * (ticks + 1.0) * 0.5)
            }
            _ => {
                let yaw = self.brush_yaw(m);
                let pitch = m.aim.map_or(0.0, |a| a.pitch as f32 * std::f32::consts::PI / 32768.0);
                Vec3::new(yaw.sin() * pitch.cos(), pitch.sin(), yaw.cos() * pitch.cos()) * speed
            }
        };
        if spread != 0.0 {
            let (s, c) = spread.sin_cos();
            v = Vec3::new(v.x * c + v.z * s, v.y, -v.x * s + v.z * c);
        }
        v
    }

    fn spray_rand(&mut self) -> f32 {
        // xorshift: a scatter that is the same on every replay of the same ticks.
        let mut x = self.spray_seed;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.spray_seed = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// Shadow Mario's brush attacks, one tick after [`Self::step`] (`brush` is its button held).
    /// Puts his upper body in the stroke's pose and returns the ids of the blobs it threw.
    pub fn brush_step(&mut self, m: &mut Mario, critters: &mut Critters, brush: bool) -> Vec<u64> {
        let mut shots = Vec::new();
        let pressed = brush && !self.brush_was;
        self.brush_was = brush;
        if self.form != Form::Shadow || self.hidden || m.is_down() {
            self.brush = Brush::Idle;
            return shots;
        }
        let p = self.params.clone();
        self.brush = match self.brush {
            Brush::Idle if pressed => {
                self.sound("MSD_SE_MA_KAGE_PAINTINGWIND", m.pos);
                Brush::Swing { tick: 0, shot: false }
            }
            Brush::Idle => Brush::Idle,
            Brush::Swing { tick, shot } => {
                let tick = tick + 1;
                let frame = p.swing_from + tick as f32 * p.swing_rate * 0.5;
                let mut shot = shot;
                if !shot && frame >= p.swing_release {
                    shot = true;
                    let yaw = self.brush_yaw(m);
                    let from = Self::brush_tip(m, yaw);
                    let vel = self.blob_velocity(m, from, p.swing_speed, p.swing_gravity, 0.0);
                    shots.push(critters.shoot_paint(from, vel, p.swing_gravity, p.swing_splat, self.paint_kind, p.swing_damage, self.id));
                    self.sound("MSD_SE_MA_KAGE_PAINTS", from);
                }
                if frame < p.swing_to {
                    Brush::Swing { tick, shot }
                } else if brush {
                    // Still held after the stroke: out in front, spraying.
                    Brush::Spray { tick: 0 }
                } else {
                    Brush::Idle
                }
            }
            Brush::Spray { .. } if !brush => Brush::Idle,
            Brush::Spray { tick } => {
                let tick = tick + 1;
                if tick % p.spray_every.max(1) == 0 {
                    let yaw = self.brush_yaw(m);
                    let from = Self::brush_tip(m, yaw);
                    let spread = self.spray_rand() * p.spray_spread;
                    let mut vel = self.blob_velocity(m, from, p.spray_speed, p.spray_gravity, spread);
                    vel.y += self.spray_rand() * p.spray_spread * p.spray_speed * 0.5;
                    shots.push(critters.shoot_paint(from, vel, p.spray_gravity, p.spray_splat, self.paint_kind, p.spray_damage, self.id));
                }
                if tick % 30 == 1 {
                    self.sound("MSD_SE_MA_KAGE_PAINTINGWIND", m.pos);
                }
                Brush::Spray { tick }
            }
        };
        if let Some(frame) = self.brush_frame() {
            pose_brush(m, frame);
            // He faces where he paints (his model; his run keeps its own heading).
            let turns = self.brush_yaw(m) / std::f32::consts::TAU;
            m.model_yaw = ((turns - turns.round()) * 65536.0) as i32 as i16;
        }
        shots
    }

    /// Which Bowser Jr. clip shows ([`jr_clip`]).
    pub fn jr_clip(&self, m: &Mario) -> u8 {
        if self.hurt_timer > 0 {
            jr_clip::DAMAGE
        } else if self.shot_timer > 0 {
            jr_clip::SHOOT
        } else if !m.on_ground() && m.vel.y > 0.0 {
            jr_clip::YAHOO
        } else {
            jr_clip::WAIT
        }
    }

    /// One tick, after `m.tick`. `brush` is the brush button (R) held.
    pub fn step(&mut self, m: &mut Mario, host: &dyn Host, goop: &mut GoopField, brush: bool) {
        let p = self.params.clone();
        self.shot_cooldown = self.shot_cooldown.saturating_sub(1);
        self.seed_cooldown = self.seed_cooldown.saturating_sub(1);
        self.change_cooldown = self.change_cooldown.saturating_sub(1);
        self.hurt_timer = self.hurt_timer.saturating_sub(1);
        if m.health < self.last_health && self.last_health != i16::MAX {
            self.hurt_timer = 60;
        }
        self.last_health = m.health;
        if self.shot_timer > 0 {
            self.shot_timer += 1;
            if self.shot_timer > 90 {
                self.shot_timer = 0;
            }
        }
        if self.sign_timer > 0 {
            self.sign_timer += 1;
            if self.sign_timer > p.sign_ticks {
                self.sign_timer = 0;
            }
        }
        self.water_idle += 1;
        if self.water_idle >= p.water_refill {
            self.water = p.water_max;
        }
        let feet = Self::feet(m);
        if self.hidden {
            self.hide_ticks += 1;
            if !goop.is_polluted(feet) || self.hide_ticks > p.hide_max || m.is_down() {
                self.appear(m);
            }
            self.painting = false;
            return;
        }
        // Landings splat goop.
        if m.on_ground() {
            if self.air >= p.land_air_ticks && self.form == Form::Shadow {
                goop.stamp(host, feet, p.land_size, self.paint_kind);
                self.events.push(JrEvent::Painted { pos: feet, radius: p.land_size, kind: self.paint_kind });
            }
            self.air = 0;
        } else {
            self.air += 1;
        }
        // The brush.
        self.painting = brush && self.form == Form::Shadow && m.on_ground() && !m.is_down();
        if self.painting {
            if self.brush_tick % p.brush_every.max(1) == 0 {
                goop.stamp(host, feet, p.brush_size, self.paint_kind);
                self.events.push(JrEvent::Painted { pos: feet, radius: p.brush_size, kind: self.paint_kind });
            }
            if self.brush_tick % 30 == 0 {
                self.sound("MSD_SE_MA_KAGE_PAINTS", m.pos);
            }
            self.brush_tick += 1;
        } else {
            self.brush_tick = 0;
        }
    }
}

/// Mario's upper body (from the chest, the pumping layer FLUDD uses) in `ma_draw` at `frame`:
/// the brush stroke's pose, for his own sim and a host's copy of another player's.
pub fn pose_brush(m: &mut Mario, frame: f32) {
    m.upper_on = true;
    m.upper_clip = Some(DRAW_CLIP);
    m.upper_ctrl.frame = frame;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::FlatHost;
    use crate::input::Pad;
    use crate::{AnimTable, Params};
    use glam::Vec2;

    fn setup() -> (KoopaJr, Mario, GoopField, FlatHost) {
        let mut m = Mario::new(Params::default(), AnimTable::default(), Vec3::ZERO, 0);
        let jr = KoopaJr::default();
        jr.attach(&mut m);
        (jr, m, GoopField::new(), FlatHost::default())
    }

    fn tick(jr: &mut KoopaJr, m: &mut Mario, host: &mut FlatHost, goop: &mut GoopField, pad: &Pad, brush: bool) {
        m.tick(host, pad, 0);
        m.events.clear();
        jr.step(m, host, goop, brush);
        goop.step();
    }

    #[test]
    fn he_has_no_fludd_and_goop_never_catches_him() {
        let (mut jr, mut m, mut goop, mut host) = setup();
        assert!(!m.has_fludd());
        goop.stamp(&host, Vec3::ZERO, 2000.0, GoopKind::Sink);
        for _ in 0..300 {
            tick(&mut jr, &mut m, &mut host, &mut goop, &Pad::default(), false);
        }
        assert!(!m.is_sinking());
        assert_eq!(m.goop.dirty, 0.0);
    }

    #[test]
    fn the_brush_leaves_a_trail_while_held() {
        let (mut jr, mut m, mut goop, mut host) = setup();
        let pad = Pad { stick: Vec2::new(0.0, 1.0), ..Pad::default() };
        for _ in 0..240 {
            tick(&mut jr, &mut m, &mut host, &mut goop, &pad, true);
        }
        assert!(goop.is_polluted(Vec3::ZERO), "where he started");
        assert!(goop.is_polluted(Vec3::new(m.pos.x, 0.0, m.pos.z)), "where he is");
        let before = goop.texel_count();
        for _ in 0..60 {
            tick(&mut jr, &mut m, &mut host, &mut goop, &pad, false);
        }
        assert!(goop.texel_count() <= before, "no paint without the brush");
    }

    #[test]
    fn a_landing_splats_goop() {
        let (mut jr, mut m, mut goop, mut host) = setup();
        let jump = Pad { a: true, ..Pad::default() };
        tick(&mut jr, &mut m, &mut host, &mut goop, &jump, false);
        for _ in 0..240 {
            tick(&mut jr, &mut m, &mut host, &mut goop, &Pad::default(), false);
        }
        assert!(goop.texel_count() > 0, "a jump's landing splat");
        assert!(jr.events.iter().any(|e| matches!(e, JrEvent::Painted { radius, .. } if *radius == 384.0)));
    }

    #[test]
    fn he_hides_in_goop_and_comes_out_when_its_cleaned() {
        let (mut jr, mut m, mut goop, mut host) = setup();
        for _ in 0..10 {
            tick(&mut jr, &mut m, &mut host, &mut goop, &Pad::default(), false);
        }
        assert!(!jr.hide(&m, &goop), "no goop, nowhere to hide");
        goop.stamp(&host, Vec3::ZERO, 500.0, GoopKind::Sink);
        assert!(jr.hide(&m, &goop));
        assert!(!jr.collides());
        for _ in 0..100 {
            tick(&mut jr, &mut m, &mut host, &mut goop, &Pad::default(), false);
        }
        assert!(jr.hidden);
        for _ in 0..10 {
            goop.clean(Vec3::ZERO, 600.0);
        }
        tick(&mut jr, &mut m, &mut host, &mut goop, &Pad::default(), false);
        assert!(!jr.hidden);
        assert!(m.is_invincible());
    }

    #[test]
    fn the_sign_is_painted_ahead_of_him() {
        let (mut jr, mut m, mut goop, mut host) = setup();
        for _ in 0..10 {
            tick(&mut jr, &mut m, &mut host, &mut goop, &Pad::default(), false);
        }
        assert!(jr.paint_sign(&m, &host, &mut goop));
        assert!(!jr.paint_sign(&m, &host, &mut goop), "one at a time");
        let ahead = Vec3::new(sin(m.face_yaw), 0.0, cos(m.face_yaw)) * jr.params.sign_ahead;
        assert!(goop.is_polluted(ahead));
    }

    #[test]
    fn bowser_jr_shoots_goop_and_spits_seeds() {
        let (mut jr, mut m, mut goop, mut host) = setup();
        let mut critters = Critters::default();
        assert!(jr.shoot(&m, &mut critters).is_none(), "Shadow Mario doesn't shoot");
        assert!(jr.change_form(&m));
        assert_eq!(jr.form, Form::Jr);
        assert!(jr.events.iter().any(|e| matches!(e, JrEvent::Effect { name: "ms_kgm_change", .. })));
        assert!(jr.shoot(&m, &mut critters).is_some());
        assert!(jr.shoot(&m, &mut critters).is_none(), "the shot's period");
        assert_eq!(jr.jr_clip(&m), jr_clip::SHOOT);
        assert!(jr.spit_seed(&m, &mut critters).is_some());
        for _ in 0..400 {
            tick(&mut jr, &mut m, &mut host, &mut goop, &Pad::default(), false);
            critters.step(&host, &mut goop, None, &[]);
        }
        assert_eq!(critters.piranhas.len(), 1, "the seed grew");
        assert!(goop.texel_count() > 0, "the goop ball splatted");
    }

    #[test]
    fn a_brush_swing_throws_goop_to_where_he_aims() {
        let (mut jr, mut m, mut goop, mut host) = setup();
        let mut critters = Critters::default();
        let aim = Vec3::new(600.0, 0.0, 1200.0);
        jr.aim_at = Some(aim);
        let mut thrown = Vec::new();
        let mut posed = false;
        for i in 0..400 {
            tick(&mut jr, &mut m, &mut host, &mut goop, &Pad::default(), false);
            // A tap: held for one tick.
            thrown.extend(jr.brush_step(&mut m, &mut critters, i == 0));
            posed |= m.upper_layer().is_some_and(|(c, _)| c == DRAW_CLIP);
            critters.step(&host, &mut goop, None, &[]);
        }
        assert!(posed, "the stroke shows on his upper body");
        assert_eq!(thrown.len(), 1, "one blob a swing");
        assert_eq!(jr.brush, Brush::Idle, "a tap is one swing");
        assert!(goop.is_polluted(aim), "the blob splats where he aimed");
        assert!(!goop.is_polluted(Vec3::ZERO), "not at his feet");
    }

    #[test]
    fn holding_the_brush_sprays_a_stream_until_let_go() {
        let (mut jr, mut m, mut goop, mut host) = setup();
        let mut critters = Critters::default();
        jr.aim_at = Some(Vec3::new(0.0, 0.0, 900.0));
        let mut thrown = 0;
        for _ in 0..360 {
            tick(&mut jr, &mut m, &mut host, &mut goop, &Pad::default(), true);
            thrown += jr.brush_step(&mut m, &mut critters, true).len();
            critters.step(&host, &mut goop, None, &[]);
        }
        assert!(matches!(jr.brush, Brush::Spray { .. }));
        assert!(thrown > 20, "a stream, not a shot: {thrown}");
        assert!(goop.is_polluted(Vec3::new(0.0, 0.0, 900.0)), "it lands ahead");
        tick(&mut jr, &mut m, &mut host, &mut goop, &Pad::default(), false);
        assert!(jr.brush_step(&mut m, &mut critters, false).is_empty());
        assert_eq!(jr.brush, Brush::Idle);
    }

    #[test]
    fn spray_blobs_hurt_less_than_the_swing() {
        let (mut jr, mut m, _, _) = setup();
        let mut critters = Critters::default();
        let mut goop = GoopField::new();
        let host = FlatHost::default();
        jr.aim_at = Some(Vec3::new(0.0, 100.0, 500.0));
        let target = crate::goop_critters::Target { id: 9, pos: Vec3::new(0.0, 0.0, 500.0), radius: 80.0, height: 200.0, mario: false };
        let mut scales = Vec::new();
        for i in 0..300 {
            let _ = jr.brush_step(&mut m, &mut critters, i < 200);
            critters.step(&host, &mut goop, None, &[target]);
            for e in critters.events.drain(..) {
                if let crate::goop_critters::CritterEvent::Hit { target: 9, scale, .. } = e {
                    scales.push(scale);
                }
            }
        }
        assert!(scales.contains(&1.0), "the swing's blob hit: {scales:?}");
        assert!(scales.iter().any(|s| *s < 0.5), "spray blobs hit for less: {scales:?}");
    }

    #[test]
    fn enough_water_drenches_him() {
        let (mut jr, m, _, _) = setup();
        let mut drenched = false;
        for _ in 0..=64 {
            drenched |= jr.water_hit(&m);
        }
        assert!(drenched);
    }
}
