//! JParticle effect files (`.jpa`, version `JEFFjpa1`), as Sunshine loads them
//! (`JPAEmitterLoader_v10::load`, JPAEmitterLoader.cpp): a 0x20-byte header, then blocks, each
//! a 4-byte tag and its size. `BEM1` is the emitter (`JPABaseEmitter::loadBaseEmitterBlock`),
//! `FLD1` a field (`JPABaseField::loadFieldBlock`), `BSP1` the particles' shape and colours
//! (`JPABaseShape`), `ESP1` their scale, alpha and spin over life (`JPAExtraShape`), `ETX1` the
//! extra textures (`JPAExTexShape`), `KFA1` emitter keyframes and `SSP1` child particles
//! (`JPASweepShape`)
//! and `TEX1` the textures (`JPATexture`: a name, then a `ResTIMG` at 0x20). Block offsets
//! below are from each block's start, as the game reads them.
//!
//! Fixed-point values are `JPAConvertFixToFloat` (JPAMath.cpp): `s16 / 32768`, rounded to
//! five places, 0x7fff exactly 1.

use crate::Error;
use libcommon::anim::hermite;
use crate::reader::Be;
use crate::texture::{Texture, read_timg};

/// `JPAConvertFixToFloat`.
pub fn fix(v: i16) -> f32 {
    if v == 0x7fff {
        return 1.0;
    }
    let f = v as f32 * (1.0 / 32768.0) * 100000.0;
    let mut r = f as i32;
    let d = (f as i32) % 10;
    if d >= 5 {
        r += 10 - d;
    } else {
        r -= d;
    }
    r as f32 * 1e-5
}

/// `BEM1`: how and where particles are made, and how they move.
#[derive(Debug, Clone, Default)]
pub struct Emitter {
    pub scale: [f32; 3],
    pub trans: [f32; 3],
    /// Degrees (times 182 into `s16` angles).
    pub rot: [i16; 3],
    /// `VOLUME_TYPE_*`: 0 cube, 1 sphere, 2 cylinder, 3 torus, 4 point, 5 circle, 6 line.
    pub volume_type: u8,
    /// Frames skipped between emissions (0: every frame).
    pub emit_interval: u8,
    /// Sphere layers, or particles on a circle or line.
    pub volume_subdivision: u16,
    /// Particles a frame (`mChildSpawnRate`, misnamed).
    pub rate: f32,
    pub rate_random: f32,
    /// Frames the emitter makes particles for; 0 forever.
    pub max_frame: i32,
    pub start_frame: i16,
    pub volume_size: u16,
    /// Share of a full turn the volume covers.
    pub volume_sweep: f32,
    pub volume_min_radius: f32,
    /// Frames.
    pub lifetime: i16,
    pub lifetime_random: f32,
    pub weight: f32,
    pub weight_random: f32,
    /// Random spread of the starting speed (`unk1C4`).
    pub speed_random: f32,
    /// Random spread of `moment` (`unk1CC`).
    pub moment_random: f32,
    pub air_resistance: f32,
    pub air_resistance_random: f32,
    /// Starting speed out from the volume's centre (`unk1FC`).
    pub vel_omni: f32,
    /// Starting speed out from the volume's axis (`unk200`).
    pub vel_axis: f32,
    /// Random starting speed (`unk204`).
    pub vel_random: f32,
    /// Starting speed along `direction` (`unk208`).
    pub vel_dir: f32,
    /// Push along the starting direction every frame (`unk1C8`).
    pub moment: f32,
    pub direction: [f32; 3],
    /// Cone spread around `direction` (`unk1E4`), 1 a full sphere.
    pub spread: f32,
    /// `EMIT_FLAG_*`: 1 fixed density, 2 fixed interval, 4 inherit scale, 8 follow the
    /// emitter, 0x10 children follow.
    pub flags: u32,
    pub key_anm_mask: u32,
}

/// `FLD1`.
#[derive(Debug, Clone, Default)]
pub struct Field {
    /// 0 gravity, 1 air, 2 magnet, 3 newton, 4 vortex, 5 random, 6 drag, 7 convection.
    pub kind: u8,
    /// 0 accelerates, 1 adds to the base velocity, 2 adds to this frame's velocity.
    pub add_type: u8,
    /// `STATUS_*` (JPAField.hpp).
    pub status: u16,
    pub magnitude: f32,
    pub magnitude_random: f32,
    pub max_distance: f32,
    pub pos: [f32; 3],
    pub dir: [f32; 3],
    pub param: [f32; 3],
    pub fade_in_end: f32,
    pub fade_out_start: f32,
    pub fade_in_start: f32,
    pub fade_out_end: f32,
}

/// A colour table key (`JPAColorRegAnmKey`).
#[derive(Debug, Clone, Copy, Default)]
pub struct ColorKey {
    pub frame: i16,
    pub color: [u8; 4],
}

/// `BSP1`.
#[derive(Debug, Clone, Default)]
pub struct BaseShape {
    pub size_x: f32,
    pub size_y: f32,
    pub loop_offset: i16,
    /// 0 point, 1 line, 2 billboard, 3 directional, 4 directional cross, 5 stripe, 6 stripe
    /// cross, 7 rotation, 8 rotation cross, 9 directional billboard, 10 Y billboard.
    pub kind: u8,
    /// 0 velocity, 1 position, 2 position reversed, 3 emitter direction, 4 previous particle.
    pub dir_type: u8,
    /// 0 Y, 1 X, 2 Z, 3 XYZ, 4 Y jiggle.
    pub rot_type: u8,
    pub col_loop: bool,
    pub tex_loop: bool,
    pub global_tex_anm: bool,
    pub global_col_anm: bool,
    /// TEV colour inputs (`data[0x30]`): 0 tex, 1 prm*tex, 2 prm+tex*(1-prm)... see
    /// [`BaseShape::tev`].
    pub color_mode: u8,
    /// `GXBlendMode` index (0 none, 1 blend, 2 logic, 3 subtract), source and destination
    /// factors (`stBlendFactor`).
    pub blend_mode: u8,
    pub blend_src: u8,
    pub blend_dst: u8,
    pub flags: u8,
    pub tex_anm_type: u8,
    pub tex_keys: Vec<u8>,
    pub tex_anm: bool,
    pub tex_empty: bool,
    pub texture: u8,
    pub col_anm_max: i16,
    pub col_anm_type: u8,
    pub prm_on: bool,
    pub prm_anm: bool,
    pub env_on: bool,
    pub env_anm: bool,
    pub prm: [u8; 4],
    pub env: [u8; 4],
    /// The colour tables, one entry per frame (`makeColorTable`).
    pub prm_table: Vec<[u8; 4]>,
    pub env_table: Vec<[u8; 4]>,
    pub tex_static_trans: [f32; 2],
    pub tex_static_scale: [f32; 2],
    pub tiling: [f32; 2],
    pub tex_scroll_trans: [f32; 2],
    pub tex_scroll_scale: [f32; 2],
    pub tex_scroll_rotate: f32,
    pub tex_scroll: bool,
    pub projection: bool,
}

/// `ESP1`.
#[derive(Debug, Clone, Default)]
pub struct ExtraShape {
    pub alpha_in_timing: f32,
    pub alpha_out_timing: f32,
    pub alpha_in_value: f32,
    pub alpha_base_value: f32,
    pub alpha_out_value: f32,
    pub alpha_flags: u8,
    pub alpha_wave_type: u8,
    pub alpha_wave: [f32; 3],
    pub alpha_wave_random: f32,
    pub alpha_increase: f32,
    pub alpha_decrease: f32,
    pub random_scale: f32,
    pub scale_in_timing: f32,
    pub scale_out_timing: f32,
    pub scale_in_y: f32,
    pub scale_out_y: f32,
    pub pivot_y: u8,
    pub anm_type_y: u8,
    pub anm_cycle_y: i16,
    pub scale_in_x: f32,
    pub scale_out_x: f32,
    pub pivot_x: u8,
    pub anm_type_x: u8,
    pub anm_cycle_x: i16,
    pub scale_flags: u8,
    pub increase_x: f32,
    pub increase_y: f32,
    pub decrease_x: f32,
    pub decrease_y: f32,
    pub rotate_angle: f32,
    pub rotate_speed: f32,
    pub rotate_random_speed: f32,
    pub rotate_random_angle: f32,
    pub rotate_direction: f32,
    pub rotate: bool,
}

impl ExtraShape {
    pub fn scale_on(&self) -> bool {
        self.scale_flags & 1 != 0
    }
    pub fn alpha_on(&self) -> bool {
        self.alpha_flags & 1 != 0
    }
    pub fn sin_wave(&self) -> bool {
        self.alpha_flags & 2 != 0
    }
    pub fn diff_xy(&self) -> bool {
        self.scale_flags & 2 != 0
    }
    pub fn scale_anm_x(&self) -> bool {
        self.scale_flags & 8 != 0
    }
    pub fn scale_anm_y(&self) -> bool {
        self.scale_flags & 4 != 0
    }
    pub fn scale_by_speed_x(&self) -> bool {
        self.scale_flags & 0x20 != 0
    }
    pub fn scale_by_speed_y(&self) -> bool {
        self.scale_flags & 0x10 != 0
    }
}

/// `ETX1`.
#[derive(Debug, Clone, Default)]
pub struct ExTexShape {
    /// 0 none, 1 indirect, 2 indirect with a sub texture.
    pub ind_mode: u8,
    /// GX indirect matrix, with its signed power-of-two exponent.
    pub ind_matrix: [[f32; 3]; 2],
    pub ind_exp: i8,
    pub ind_texture: u8,
    pub sub_texture: u8,
    pub flags: u8,
    pub second_texture: u8,
}

impl ExTexShape {
    pub fn second_on(&self) -> bool {
        self.flags & 1 != 0
    }
}

/// `KFA1`: time, value, incoming tangent, outgoing tangent, evaluated with cubic Hermite.
#[derive(Debug, Clone, Default)]
pub struct KeyFrame {
    pub kind: u8,
    pub looped: bool,
    pub keys: Vec<[f32; 4]>,
}

impl KeyFrame {
    /// `JPABaseEmitter::calcKeyFrameAnime` / `JPAGetKeyFrameValue`.
    pub fn value(&self, mut time: f32) -> Option<f32> {
        let first = self.keys.first()?;
        let last = self.keys.last()?;
        if self.looped && last[0] >= 0.0 {
            time = time.rem_euclid(last[0] + 1.0);
        }
        if time <= first[0] {
            return Some(first[1]);
        }
        if time >= last[0] {
            return Some(last[1]);
        }
        let pair = self.keys.windows(2).find(|p| time < p[1][0])?;
        let (a, b) = (pair[0], pair[1]);
        let span = b[0] - a[0];
        if span <= 0.0 {
            return Some(b[1]);
        }
        Some(hermite(time, a[0], a[1], a[3], b[0], b[1], b[2]))
    }
}

/// `SSP1`, read as `JPASweepShape::JPASweepShape`. Children have their own
/// geometry/texture and lifetime; TEV and blending come from the base shape.
#[derive(Debug, Clone, Default)]
pub struct ChildShape {
    pub kind: u8,
    pub dir_type: u8,
    pub rot_type: u8,
    pub lifetime: i16,
    pub rate: i16,
    pub timing: f32,
    pub step: u8,
    pub pos_random: f32,
    pub base_vel: f32,
    pub vel_inherit: f32,
    pub vel_random: f32,
    pub moment: f32,
    pub fields: bool,
    pub flags: u8,
    pub scale_out: bool,
    pub alpha_out: bool,
    pub texture: u8,
    pub scale_inherit: f32,
    pub alpha_inherit: f32,
    pub size_y: f32,
    pub size_x: f32,
    pub rotate_speed: f32,
    pub rotate: bool,
    pub inherit_flags: u8,
    pub prm: [u8; 4],
    pub env: [u8; 4],
    pub rgb_inherit: f32,
}

fn read_child(b: Be) -> Result<ChildShape, Error> {
    let rgba = |o| -> Result<[u8; 4], Error> { Ok(b.bytes(o, 4)?.try_into().unwrap()) };
    Ok(ChildShape {
        kind: b.u8(0x10)?,
        dir_type: b.u8(0x11)?,
        rot_type: b.u8(0x12)?,
        lifetime: b.i16(0x14)?,
        rate: b.i16(0x16)?,
        timing: fx(b, 0x18)?,
        step: b.u8(0x1A)?,
        pos_random: b.f32(0x28)?,
        base_vel: b.f32(0x2C)?,
        vel_inherit: fx(b, 0x30)?,
        vel_random: fx(b, 0x32)?,
        moment: fx(b, 0x34)?,
        fields: b.u8(0x36)? != 0,
        flags: b.u8(0x44)?,
        scale_out: b.u8(0x45)? != 0,
        alpha_out: b.u8(0x46)? != 0,
        texture: b.u8(0x47)?,
        scale_inherit: fx(b, 0x48)?,
        alpha_inherit: fx(b, 0x4A)?,
        size_y: b.f32(0x4C)?,
        size_x: b.f32(0x50)?,
        rotate_speed: fx(b, 0x54)?,
        rotate: b.u8(0x56)? != 0,
        inherit_flags: b.u8(0x57)?,
        prm: rgba(0x58)?,
        env: rgba(0x5C)?,
        rgb_inherit: fx(b, 0x60)?,
    })
}

/// One `.jpa` file.
#[derive(Debug, Clone)]
pub struct Jpa {
    pub emitter: Emitter,
    pub fields: Vec<Field>,
    pub base: BaseShape,
    pub extra: Option<ExtraShape>,
    pub ex_tex: Option<ExTexShape>,
    pub keys: Vec<KeyFrame>,
    pub child: Option<ChildShape>,
    pub textures: Vec<Texture>,
}

impl Jpa {
    pub fn parse(d: &[u8]) -> Result<Jpa, Error> {
        let r = Be(d);
        if r.bytes(0, 8)? != b"JEFFjpa1" {
            return Err(Error::Bad("not a JEFFjpa1 particle file".into()));
        }
        let blocks = r.u32(0xC)? as usize;
        let mut o = 0x20;
        let mut emitter = None;
        let mut base = None;
        let mut jpa = Jpa {
            emitter: Emitter::default(),
            fields: Vec::new(),
            base: BaseShape::default(),
            extra: None,
            ex_tex: None,
            keys: Vec::new(),
            child: None,
            textures: Vec::new(),
        };
        for _ in 0..blocks {
            let tag = r.bytes(o, 4)?;
            let size = r.u32(o + 4)? as usize;
            if size < 8 {
                return Err(Error::Bad(format!("block at {o:#x} has size {size}")));
            }
            let b = Be(r.bytes(o, size)?);
            match tag {
                b"BEM1" => emitter = Some(read_emitter(b)?),
                b"FLD1" => jpa.fields.push(read_field(b)?),
                b"BSP1" => base = Some(read_base(b)?),
                b"ESP1" => jpa.extra = Some(read_extra(b)?),
                b"ETX1" => jpa.ex_tex = Some(read_ex_tex(b)?),
                b"KFA1" => jpa.keys.push(read_key(b)?),
                b"SSP1" => jpa.child = Some(read_child(b)?),
                b"TEX1" => {
                    let name = b.cstr(0xC)?;
                    jpa.textures.push(read_timg(b.0, 0x20, name)?);
                }
                _ => {}
            }
            o += size;
        }
        jpa.emitter = emitter.ok_or_else(|| Error::Bad("no BEM1 block".into()))?;
        jpa.base = base.ok_or_else(|| Error::Bad("no BSP1 block".into()))?;
        Ok(jpa)
    }
}

fn fx(b: Be, o: usize) -> Result<f32, Error> {
    Ok(fix(b.i16(o)?))
}

/// `loadBaseEmitterBlock` reads these in order from 0xC.
fn read_emitter(b: Be) -> Result<Emitter, Error> {
    let v3 =
        |o: usize| -> Result<[f32; 3], Error> { Ok([b.f32(o)?, b.f32(o + 4)?, b.f32(o + 8)?]) };
    let dir = [fx(b, 0x64)?, fx(b, 0x66)?, fx(b, 0x68)?];
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    Ok(Emitter {
        scale: v3(0xC)?,
        trans: v3(0x18)?,
        rot: [b.i16(0x24)?, b.i16(0x26)?, b.i16(0x28)?],
        volume_type: b.u8(0x2A)?,
        emit_interval: b.u8(0x2B)?,
        volume_subdivision: b.u16(0x2E)?,
        rate: b.f32(0x30)?,
        rate_random: fx(b, 0x34)?,
        max_frame: b.i16(0x36)? as i32,
        start_frame: b.i16(0x38)?,
        volume_size: b.u16(0x3A)?,
        volume_sweep: fx(b, 0x3C)?,
        volume_min_radius: fx(b, 0x3E)?,
        lifetime: b.i16(0x40)?,
        lifetime_random: fx(b, 0x42)?,
        weight: fx(b, 0x44)?,
        weight_random: fx(b, 0x46)?,
        speed_random: fx(b, 0x48)?,
        moment_random: fx(b, 0x4A)?,
        air_resistance: fx(b, 0x4C)?,
        air_resistance_random: fx(b, 0x4E)?,
        vel_omni: b.f32(0x50)?,
        vel_axis: b.f32(0x54)?,
        vel_random: b.f32(0x58)?,
        vel_dir: b.f32(0x5C)?,
        moment: b.f32(0x60)?,
        direction: if len > 0.0 {
            dir.map(|c| c / len)
        } else {
            [0.0, 0.0, 1.0]
        },
        spread: fx(b, 0x6A)?,
        flags: b.u32(0x6C)?,
        key_anm_mask: b.u32(0x70)?,
    })
}

/// `loadFieldBlock`: four bytes from 0xC, the status, then floats from 0x14.
fn read_field(b: Be) -> Result<Field, Error> {
    let v3 =
        |o: usize| -> Result<[f32; 3], Error> { Ok([b.f32(o)?, b.f32(o + 4)?, b.f32(o + 8)?]) };
    Ok(Field {
        kind: b.u8(0xC)?,
        add_type: b.u8(0xE)?,
        status: b.u16(0x10)?,
        magnitude: b.f32(0x14)?,
        magnitude_random: b.f32(0x18)?,
        max_distance: b.f32(0x1C)?,
        pos: v3(0x20)?,
        dir: v3(0x2C)?,
        param: v3(0x38)?,
        fade_in_end: fx(b, 0x44)?,
        fade_out_start: fx(b, 0x46)?,
        fade_in_start: fx(b, 0x48)?,
        fade_out_end: fx(b, 0x4A)?,
    })
}

/// `makeColorTable`: `max + 1` entries, stepped linearly between the keys.
fn color_table(b: Be, at: usize, keys: usize, max: i16) -> Result<Vec<[u8; 4]>, Error> {
    let keys: Vec<ColorKey> = (0..keys)
        .map(|i| {
            let o = at + i * 6;
            Ok(ColorKey {
                frame: b.i16(o)?,
                color: [b.u8(o + 2)?, b.u8(o + 3)?, b.u8(o + 4)?, b.u8(o + 5)?],
            })
        })
        .collect::<Result<_, Error>>()?;
    let Some(first) = keys.first() else {
        return Ok(Vec::new());
    };
    let n = max.max(0) as usize + 1;
    let mut out = vec![[0u8; 4]; n];
    let mut cur = first.color.map(|c| c as f32);
    let mut step = [0.0f32; 4];
    let mut j = 0;
    for (i, slot) in out.iter_mut().enumerate() {
        if j < keys.len() && i as i32 == keys[j].frame as i32 {
            *slot = keys[j].color;
            cur = keys[j].color.map(|c| c as f32);
            j += 1;
            step = if j < keys.len() {
                let span = 1.0 / (keys[j].frame - keys[j - 1].frame) as f32;
                let next = keys[j].color.map(|c| c as f32);
                [0, 1, 2, 3].map(|k| span * (next[k] - cur[k]))
            } else {
                [0.0; 4]
            };
        } else {
            for k in 0..4 {
                cur[k] += step[k];
            }
            // GXColor stores the float truncated.
            *slot = cur.map(|c| c.clamp(0.0, 255.0) as u8);
        }
    }
    Ok(out)
}

/// `JPABaseShape::JPABaseShape`.
fn read_base(b: Be) -> Result<BaseShape, Error> {
    let kind = b.u8(0x24)?;
    let (r5, r6) = (b.u8(0x22)?, b.u8(0x23)?);
    let stripe = kind == 5 || kind == 6;
    let tex_keys = b.u8(0x4E)?;
    let tex_anm = b.u8(0x4C)? & 1 != 0 && tex_keys != 0;
    let col_anm_max = b.i16(0x5C)?;
    let (prm_flags, env_flags) = (b.u8(0x60)?, b.u8(0x61)?);
    let rgba = |o: usize| -> Result<[u8; 4], Error> {
        Ok([b.u8(o)?, b.u8(o + 1)?, b.u8(o + 2)?, b.u8(o + 3)?])
    };
    Ok(BaseShape {
        size_y: b.f32(0x18)?,
        size_x: b.f32(0x1C)?,
        loop_offset: b.i16(0x20)?,
        kind,
        dir_type: b.u8(0x25)?,
        rot_type: b.u8(0x26)?,
        col_loop: r5 & 1 != 0,
        tex_loop: r6 & 1 != 0,
        global_col_anm: r5 & 2 != 0 || stripe,
        global_tex_anm: r6 & 2 != 0 || stripe,
        color_mode: b.u8(0x30)?,
        blend_mode: b.u8(0x35)?,
        blend_src: b.u8(0x36)?,
        blend_dst: b.u8(0x37)?,
        flags: b.u8(0x44)?,
        tex_anm_type: b.u8(0x4D)?,
        tex_keys: if tex_anm {
            let at = b.i16(0x12)? as usize;
            b.bytes(at, tex_keys as usize)?.to_vec()
        } else {
            Vec::new()
        },
        tex_anm,
        tex_empty: b.u8(0x4C)? & 2 == 0,
        texture: b.u8(0x4F)?,
        col_anm_max,
        col_anm_type: b.u8(0x5E)?,
        prm_on: prm_flags & 1 != 0,
        prm_anm: prm_flags & 2 != 0,
        env_on: env_flags & 1 != 0,
        env_anm: env_flags & 2 != 0,
        prm: rgba(0x64)?,
        env: rgba(0x68)?,
        prm_table: if prm_flags & 2 != 0 {
            color_table(b, b.i16(0x14)? as usize, b.u8(0x62)? as usize, col_anm_max)?
        } else {
            Vec::new()
        },
        env_table: if env_flags & 2 != 0 {
            color_table(b, b.i16(0x16)? as usize, b.u8(0x63)? as usize, col_anm_max)?
        } else {
            Vec::new()
        },
        tex_static_trans: [fx(b, 0x80)? * 10.0, fx(b, 0x82)? * 10.0],
        tex_static_scale: [fx(b, 0x84)? * 10.0, fx(b, 0x86)? * 10.0],
        tiling: [fx(b, 0x88)? * 10.0, fx(b, 0x8A)? * 10.0],
        tex_scroll_trans: [fx(b, 0x8C)?, fx(b, 0x8E)?],
        tex_scroll_scale: [fx(b, 0x90)? * 0.1, fx(b, 0x92)? * 0.1],
        tex_scroll_rotate: fx(b, 0x94)?,
        tex_scroll: b.u8(0x96)? != 0,
        projection: b.u8(0x43)? != 0,
    })
}

impl BaseShape {
    /// The list is drawn first to last (`getListOrder`).
    pub fn list_order(&self) -> bool {
        self.flags & 1 == 0
    }

    /// The colour inputs of TEV stage 0 (`JPABaseShape` ctor: `unk48`), each 0 zero, 1 one,
    /// 2 the texture's colour, 3 C0 (the prm colour), 4 C1 (the env colour). The stage
    /// computes `d + a * (1 - c) + b * c`; its alpha is the texture's times prm's
    /// (`JPADrawSetupTev::setupTev`).
    pub fn tev(&self) -> [u8; 4] {
        match self.color_mode {
            0 => [0, 2, 1, 0],
            1 => [0, 3, 2, 0],
            2 => [3, 1, 2, 0],
            3 => [4, 3, 2, 0],
            4 => [0, 2, 3, 4],
            _ => [0, 0, 0, 3],
        }
    }
}

/// `JPAExtraShape::JPAExtraShape`.
fn read_extra(b: Be) -> Result<ExtraShape, Error> {
    let alpha_in_timing = fx(b, 0x14)?;
    let alpha_out_timing = fx(b, 0x16)?;
    let alpha_in_value = fx(b, 0x18)?;
    let alpha_base_value = fx(b, 0x1A)?;
    let alpha_out_value = fx(b, 0x1C)?;
    let scale_in_timing = fx(b, 0x36)?;
    let scale_out_timing = fx(b, 0x38)?;
    let scale_in_y = fx(b, 0x3A)? * 10.0;
    let scale_out_y = fx(b, 0x3E)? * 10.0;
    let scale_in_x = fx(b, 0x44)? * 10.0;
    let scale_out_x = fx(b, 0x48)? * 10.0;
    let (increase_x, increase_y) = if scale_in_timing != 0.0 {
        (
            (1.0 - scale_in_x) / scale_in_timing,
            (1.0 - scale_in_y) / scale_in_timing,
        )
    } else {
        (1.0, 1.0)
    };
    let (decrease_x, decrease_y) = if scale_out_timing != 1.0 {
        (
            (scale_out_x - 1.0) / (1.0 - scale_out_timing),
            (scale_out_y - 1.0) / (1.0 - scale_out_timing),
        )
    } else {
        (1.0, 1.0)
    };
    Ok(ExtraShape {
        alpha_in_timing,
        alpha_out_timing,
        alpha_in_value,
        alpha_base_value,
        alpha_out_value,
        alpha_flags: b.u8(0x1E)?,
        alpha_wave_type: b.u8(0x1F)?,
        alpha_wave: [fx(b, 0x20)?, fx(b, 0x22)?, fx(b, 0x24)?],
        alpha_wave_random: fx(b, 0x26)?,
        alpha_increase: if alpha_in_timing != 0.0 {
            (alpha_base_value - alpha_in_value) / alpha_in_timing
        } else {
            1.0
        },
        alpha_decrease: if alpha_out_timing != 1.0 {
            (alpha_out_value - alpha_base_value) / (1.0 - alpha_out_timing)
        } else {
            1.0
        },
        random_scale: fx(b, 0x34)?,
        scale_in_timing,
        scale_out_timing,
        scale_in_y,
        scale_out_y,
        pivot_y: b.u8(0x40)?,
        anm_type_y: b.u8(0x41)?,
        anm_cycle_y: b.i16(0x42)?,
        scale_in_x,
        scale_out_x,
        pivot_x: b.u8(0x4A)?,
        anm_type_x: b.u8(0x4B)?,
        anm_cycle_x: b.i16(0x4C)?,
        scale_flags: b.u8(0x4E)?,
        increase_x,
        increase_y,
        decrease_x,
        decrease_y,
        rotate_angle: fx(b, 0x5A)?,
        rotate_speed: fx(b, 0x5C)?,
        rotate_random_speed: fx(b, 0x5E)?,
        rotate_random_angle: fx(b, 0x60)?,
        rotate_direction: fx(b, 0x62)?,
        rotate: b.u8(0x64)? != 0,
    })
}

/// `JPAExTexShape::JPAExTexShape`.
fn read_ex_tex(b: Be) -> Result<ExTexShape, Error> {
    Ok(ExTexShape {
        ind_mode: b.u8(0x10)?,
        ind_matrix: [
            [fx(b, 0x12)?, fx(b, 0x14)?, fx(b, 0x16)?],
            [fx(b, 0x18)?, fx(b, 0x1A)?, fx(b, 0x1C)?],
        ],
        ind_exp: b.u8(0x1E)? as i8,
        ind_texture: b.u8(0x1F)?,
        sub_texture: b.u8(0x20)?,
        flags: b.u8(0x30)?,
        second_texture: b.u8(0x33)?,
    })
}

fn read_key(b: Be) -> Result<KeyFrame, Error> {
    let count = b.u8(0x10)? as usize;
    Ok(KeyFrame {
        kind: b.u8(0xC)?,
        looped: b.u8(0x12)? != 0,
        keys: (0..count)
            .map(|i| {
                let at = 0x20 + i * 16;
                Ok([b.f32(at)?, b.f32(at + 4)?, b.f32(at + 8)?, b.f32(at + 12)?])
            })
            .collect::<Result<_, Error>>()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emitter_scale_keys_use_tangents_clamping_and_looping() {
        let mut raw = vec![0; 0x20 + 2 * 16];
        raw[0xC] = 10;
        raw[0x10] = 2;
        raw[0x12] = 1;
        for (i, value) in [0.0f32, 1.0, 0.0, 0.5, 4.0, 3.0, 0.5, 0.0]
            .iter()
            .enumerate()
        {
            raw[0x20 + i * 4..0x24 + i * 4].copy_from_slice(&value.to_be_bytes());
        }
        let mut key = read_key(Be(&raw)).unwrap();
        assert_eq!(key.kind, 10);
        assert_eq!(key.value(0.0), Some(1.0));
        assert_eq!(key.value(2.0), Some(2.0));
        assert_eq!(key.value(5.0), Some(1.0));
        // Different outgoing/incoming slopes change the midpoint.
        key.keys[0][3] = 1.0;
        assert_eq!(key.value(2.0), Some(2.25));
        key.looped = false;
        assert_eq!(key.value(-1.0), Some(1.0));
        assert_eq!(key.value(99.0), Some(3.0));
        assert!(read_key(Be(&raw[..raw.len() - 1])).is_err());
    }

    #[test]
    fn wet_slide_ribbon_has_scene_projection_indirect_matrix_and_scale_curve() {
        let Some(dir) = libsms::assets::find(None) else {
            return;
        };
        let jpa =
            Jpa::parse(&std::fs::read(dir.join("particle/ms_m_watslide_a.jpa")).unwrap()).unwrap();
        assert_eq!(jpa.base.kind, 6);
        assert!(jpa.base.projection);
        let ext = jpa.ex_tex.as_ref().unwrap();
        assert_eq!(ext.ind_mode, 1);
        assert!(ext.second_on());
        assert_eq!(ext.ind_exp, -2);
        assert_eq!(ext.ind_matrix, [[0.5, 0.0, 0.0], [0.0, 0.5, 0.0]]);
        let key = &jpa.keys[0];
        assert_eq!(key.kind, 10);
        assert!(key.looped);
        assert!((key.value(0.0).unwrap() - 0.9).abs() < 1e-6);
        assert!((key.value(6.0).unwrap() - 1.3).abs() < 1e-6);
        assert!((key.value(13.0).unwrap() - 0.9).abs() < 1e-6);
        assert_eq!(
            jpa.textures[jpa.base.texture as usize].name,
            "P_ms_fb_8x8i4"
        );
        // This gets replaced by the current screen in TMarioParticleManager::emitTry.
        assert!(
            jpa.textures[jpa.base.texture as usize]
                .image
                .rgba
                .iter()
                .all(|&v| v == 0)
        );
    }

    #[test]
    fn fixed_point_rounds_like_the_game() {
        assert_eq!(fix(0x7fff), 1.0);
        assert_eq!(fix(0), 0.0);
        assert!((fix(0x4000) - 0.5).abs() < 1e-6);
        assert!((fix(0x0ccc) - 0.1).abs() < 1e-6);
        assert!((fix(-0x4000) + 0.5).abs() < 1e-6);
    }

    #[test]
    fn rejects_other_files() {
        assert!(Jpa::parse(b"JEFFjpa2\0\0\0\0\0\0\0\0").is_err());
        assert!(Jpa::parse(b"short").is_err());
    }

    /// The super ground pound's flames parse, from the user's extracted `assets/`.
    #[test]
    fn the_super_pound_flames_parse() {
        let Some(dir) = libsms::assets::find(None) else {
            eprintln!("skipping: no assets/");
            return;
        };
        for n in ["a", "b", "c", "d"] {
            let path = dir.join(format!("particle/ms_m_sphipd_{n}.jpa"));
            let Ok(data) = std::fs::read(&path) else {
                eprintln!("skipping: no {}", path.display());
                return;
            };
            let j = Jpa::parse(&data).unwrap();
            assert!(!j.textures.is_empty(), "{n}: no textures");
            assert!(
                j.emitter.lifetime > 0,
                "{n}: lifetime {}",
                j.emitter.lifetime
            );
            // All four add their light (blend, source alpha, one).
            assert_eq!(
                (j.base.blend_mode, j.base.blend_src, j.base.blend_dst),
                (1, 6, 1),
                "{n}"
            );
        }
    }
}

#[cfg(test)]
mod hermite_baseline {
    use super::*;

    /// Sampled bits over random keys and times outside the range, pinned before the curve
    /// moved to libcommon.
    #[test]
    fn key_frame_values_match_baseline() {
        let mut s = 0x2468_ACE1u32;
        let mut next = || {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            (s >> 8) as f32 / (1u32 << 24) as f32
        };
        let keys: Vec<[f32; 4]> = [0.0f32, 5.0, 11.0, 18.0, 26.0]
            .iter()
            .map(|&t| [t, next() * 8.0 - 4.0, next() * 2.0 - 1.0, next() * 2.0 - 1.0])
            .collect();
        let mut hash = 0xcbf29ce484222325u64;
        for looped in [false, true] {
            let kf = KeyFrame { kind: 0, looped, keys: keys.clone() };
            for step in 0..100 {
                let v = kf.value(-2.0 + step as f32 * 0.37).unwrap();
                hash = (hash ^ v.to_bits() as u64).wrapping_mul(0x100000001b3);
            }
        }
        println!("{hash:#x}");
        assert_eq!(hash, BASELINE);
    }

    // Recorded from the curve before it moved to libcommon::anim.
    const BASELINE: u64 = 0x3ff2612f7d9e1003;
}
