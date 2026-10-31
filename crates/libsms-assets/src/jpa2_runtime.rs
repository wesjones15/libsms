//! JPAC2-10 -> shared JParticle data. Layouts/getters: Pikmin 2 JPABlock.h,
//! JPAShape.h; JPABaseShape.cpp, JPAExtraShape.cpp. No substitute emitters.
use crate::{Error, jpa::*, jpa2::Resource, reader::Be, texture::Texture};
fn vec3(b: Be, o: usize) -> Result<[f32; 3], Error> {
    Ok([b.f32(o)?, b.f32(o + 4)?, b.f32(o + 8)?])
}
fn rgba(b: Be, o: usize) -> Result<[u8; 4], Error> {
    Ok(b.bytes(o, 4)?.try_into().unwrap())
}
pub fn decode(res: &Resource, textures: &[Texture]) -> Result<Jpa, Error> {
    let mut p = Jpa {
        emitter: Emitter::default(),
        fields: Vec::new(),
        base: BaseShape::default(),
        extra: None,
        ex_tex: None,
        keys: Vec::new(),
        child: None,
        textures: Vec::new(),
    };
    let mut has_emitter = false;
    let mut has_base = false;
    for bytes in &res.blocks {
        let b = Be(bytes);
        match b.bytes(0, 4)? {
            b"BEM1" => {
                p.emitter = emitter(b)?;
                has_emitter = true;
            }
            b"BSP1" => {
                p.base = base(b)?;
                has_base = true;
            }
            b"ESP1" => p.extra = Some(extra(b)?),
            b"FLD1" => p.fields.push(field(b)?),
            b"SSP1" => p.child = Some(child(b)?),
            b"KFA1" => {
                let kind = b.u8(8)?;
                p.emitter.key_anm_mask |= 1u32.checked_shl(kind as u32).unwrap_or(0);
                p.keys.push(KeyFrame {
                    kind,
                    looped: b.u8(10)? != 0,
                    keys: (0..b.u8(9)? as usize)
                        .map(|i| {
                            let o = 12 + i * 16;
                            Ok([b.f32(o)?, b.f32(o + 4)?, b.f32(o + 8)?, b.f32(o + 12)?])
                        })
                        .collect::<Result<_, Error>>()?,
                });
            }
            b"ETX1" => {
                let f = b.u32(8)?;
                p.ex_tex = Some(ExTexShape {
                    ind_mode: (f & 3) as u8,
                    ind_matrix: [vec3(b, 12)?, vec3(b, 24)?],
                    ind_exp: b.u8(36)? as i8,
                    ind_texture: b.u8(37)?,
                    sub_texture: b.u8(38)?,
                    flags: if f & 0x100 != 0 { 1 } else { 0 },
                    second_texture: b.u8(38)?,
                });
            }
            _ => {}
        }
    }
    if !has_emitter || !has_base {
        return Err(Error::Bad(format!("incomplete JPA2 emitter {:#x}", res.id)));
    }
    for &index in &res.textures {
        p.textures.push(
            textures
                .get(index as usize)
                .ok_or_else(|| Error::Bad("JPA2 texture index".into()))?
                .clone(),
        );
    }
    if let Some(c) = p.child.as_mut() {
        c.flags = 1;
    }
    Ok(p)
}
fn emitter(b: Be) -> Result<Emitter, Error> {
    let flags = b.u32(8)?;
    // JPA2 stores binary angles; v1 stores degrees which the common runtime multiplies by 182.
    let rot = [b.i16(0x68)?, b.i16(0x6a)?, b.i16(0x6c)?].map(|a| (a as f32 / 182.0).round() as i16);
    Ok(Emitter {
        scale: vec3(b, 0x10)?,
        trans: vec3(b, 0x1c)?,
        direction: vec3(b, 0x28)?,
        rot,
        flags: flags & 31,
        volume_type: ((flags >> 8) & 7) as u8,
        emit_interval: b.u8(0x78)?,
        volume_subdivision: b.u16(0x76)?,
        vel_omni: b.f32(0x34)?,
        vel_axis: b.f32(0x38)?,
        vel_random: b.f32(0x3c)?,
        vel_dir: b.f32(0x40)?,
        spread: b.f32(0x44)?,
        speed_random: b.f32(0x48)?,
        rate: b.f32(0x4c)?,
        rate_random: b.f32(0x50)?,
        lifetime_random: b.f32(0x54)?,
        volume_sweep: b.f32(0x58)?,
        volume_min_radius: b.f32(0x5c)?,
        air_resistance: b.f32(0x60)?,
        moment_random: b.f32(0x64)?,
        max_frame: b.i16(0x6e)? as i32,
        start_frame: b.i16(0x70)?,
        lifetime: b.i16(0x72)?,
        volume_size: b.u16(0x74)?,
        weight: 1.0,
        moment: 0.0,
        ..Default::default()
    })
}
fn base(b: Be) -> Result<BaseShape, Error> {
    let f = b.u32(8)?;
    let blend = b.u16(0x18)?;
    let cf = b.u8(0x21)?;
    let tf = b.u8(0x1e)?;
    let max = b.i16(0x24)?;
    let tex_anm = tf & 1 != 0;
    let tex_keys = b.u8(0x1f)? as usize;
    let scroll = f & 0x1000000 != 0;
    let mut shape = BaseShape {
        size_x: b.f32(0x10)?,
        size_y: b.f32(0x14)?,
        kind: (f & 15) as u8,
        dir_type: ((f >> 4) & 7) as u8,
        rot_type: ((f >> 7) & 7) as u8,
        global_col_anm: f & 0x1000 != 0,
        global_tex_anm: f & 0x4000 != 0,
        color_mode: ((f >> 15) & 7) as u8,
        alpha_mode: ((f >> 18) & 1) as u8,
        blend_mode: (blend & 3) as u8,
        blend_src: ((blend >> 2) & 15) as u8,
        blend_dst: ((blend >> 6) & 15) as u8,
        prm: rgba(b, 0x26)?,
        env: rgba(b, 0x2a)?,
        prm_on: cf & 1 != 0,
        prm_anm: cf & 2 != 0,
        env_on: cf & 4 != 0,
        env_anm: cf & 8 != 0,
        col_anm_max: max,
        col_anm_type: (cf >> 4) & 7,
        tex_anm,
        tex_anm_type: (tf >> 2) & 7,
        texture: b.u8(0x20)?,
        tex_keys: if tex_anm {
            b.bytes(0x34 + if scroll { 0x28 } else { 0 }, tex_keys)?
                .to_vec()
        } else {
            Vec::new()
        },
        prm_table: if cf & 2 != 0 {
            color_table(b, b.u16(0x0c)? as usize, b.u8(0x22)? as usize, max)?
        } else {
            Vec::new()
        },
        env_table: if cf & 8 != 0 {
            color_table(b, b.u16(0x0e)? as usize, b.u8(0x23)? as usize, max)?
        } else {
            Vec::new()
        },
        tex_static_scale: [1.0; 2],
        tiling: [
            if f & (1 << 25) != 0 { 2.0 } else { 1.0 },
            if f & (1 << 26) != 0 { 2.0 } else { 1.0 },
        ],
        tex_scroll: scroll,
        flags: ((f >> 21) & 1) as u8 | if f & (1 << 22) == 0 { 2 } else { 0 },
        projection: f & (1 << 20) != 0,
        ..Default::default()
    };
    if scroll {
        shape.tex_static_trans = [b.f32(0x34)?, b.f32(0x38)?];
        shape.tex_static_scale = [b.f32(0x3c)?, b.f32(0x40)?];
        shape.tex_scroll_rotate = b.f32(0x44)?;
        shape.tex_scroll_trans = [b.f32(0x48)?, b.f32(0x4c)?];
        shape.tex_scroll_scale = [b.f32(0x50)?, b.f32(0x54)?];
    }
    Ok(shape)
}
fn extra(b: Be) -> Result<ExtraShape, Error> {
    let f = b.u32(8)?;
    let si = b.f32(0x0c)?;
    let so = b.f32(0x10)?;
    let ai = b.f32(0x2c)?;
    let ao = b.f32(0x30)?;
    let ix = b.f32(0x14)?;
    let ox = b.f32(0x18)?;
    let iy = b.f32(0x1c)?;
    let oy = b.f32(0x20)?;
    let av = b.f32(0x34)?;
    let ab = b.f32(0x38)?;
    let ov = b.f32(0x3c)?;
    let slope = |a: f32, d: f32| if d.abs() > 1e-6 { a / d } else { 1.0 };
    Ok(ExtraShape {
        scale_in_timing: si,
        scale_out_timing: so,
        scale_in_x: ix,
        scale_out_x: ox,
        scale_in_y: iy,
        scale_out_y: oy,
        random_scale: b.f32(0x24)?,
        increase_x: slope(1.0 - ix, si),
        increase_y: slope(1.0 - iy, si),
        decrease_x: slope(ox - 1.0, 1.0 - so),
        decrease_y: slope(oy - 1.0, 1.0 - so),
        anm_cycle_x: b.i16(0x28)?,
        anm_cycle_y: b.i16(0x2a)?,
        anm_type_x: ((f >> 8) & 3) as u8,
        anm_type_y: ((f >> 10) & 3) as u8,
        pivot_x: ((f >> 12) & 3) as u8,
        pivot_y: ((f >> 14) & 3) as u8,
        scale_flags: (f & 3) as u8,
        alpha_flags: ((f >> 16) & 3) as u8,
        alpha_in_timing: ai,
        alpha_out_timing: ao,
        alpha_in_value: av,
        alpha_base_value: ab,
        alpha_out_value: ov,
        alpha_increase: slope(ab - av, ai),
        alpha_decrease: slope(ov - ab, 1.0 - ao),
        alpha_wave: [b.f32(0x40)?, 0.0, b.f32(0x48)?],
        alpha_wave_random: b.f32(0x44)?,
        rotate: f & (1 << 24) != 0,
        rotate_angle: b.f32(0x4c)?,
        rotate_random_angle: b.f32(0x50)?,
        rotate_speed: b.f32(0x54)?,
        rotate_random_speed: b.f32(0x58)?,
        rotate_direction: b.f32(0x5c)?,
        ..Default::default()
    })
}
fn field(b: Be) -> Result<Field, Error> {
    let f = b.u32(8)?;
    Ok(Field {
        kind: (f & 15) as u8,
        add_type: ((f >> 8) & 3) as u8,
        status: (f >> 16) as u16,
        pos: vec3(b, 0x0c)?,
        dir: vec3(b, 0x18)?,
        magnitude: b.f32(0x24)?,
        magnitude_random: b.f32(0x28)?,
        max_distance: b.f32(0x2c)?,
        param: [b.f32(0x28)?, b.f32(0x2c)?, b.u8(0x40)? as f32],
        fade_in_end: b.f32(0x30)?,
        fade_out_start: b.f32(0x34)?,
        fade_in_start: b.f32(0x38)?,
        fade_out_end: b.f32(0x3c)?,
    })
}
fn child(b: Be) -> Result<ChildShape, Error> {
    let f = b.u32(8)?;
    Ok(ChildShape {
        kind: (f & 15) as u8,
        dir_type: ((f >> 4) & 7) as u8,
        rot_type: ((f >> 7) & 7) as u8,
        pos_random: b.f32(0x0c)?,
        base_vel: b.f32(0x10)?,
        vel_random: b.f32(0x14)?,
        vel_inherit: b.f32(0x18)?,
        moment: b.f32(0x1c)?,
        size_x: b.f32(0x20)?,
        size_y: b.f32(0x24)?,
        scale_inherit: b.f32(0x28)?,
        alpha_inherit: b.f32(0x2c)?,
        rgb_inherit: b.f32(0x30)?,
        prm: rgba(b, 0x34)?,
        env: rgba(b, 0x38)?,
        timing: b.f32(0x3c)?,
        lifetime: b.i16(0x40)?,
        rate: b.i16(0x42)?,
        step: b.u8(0x44)?,
        texture: b.u8(0x45)?,
        rotate_speed: b.i16(0x46)? as f32 / 65536.0,
        rotate: f & (1 << 24) != 0,
        fields: f & (1 << 21) != 0,
        scale_out: f & (1 << 22) != 0,
        alpha_out: f & (1 << 23) != 0,
        inherit_flags: ((f >> 16) & 1) as u8
            | (((f >> 17) & 1) as u8) << 1
            | (((f >> 18) & 1) as u8) << 2,
        ..Default::default()
    })
}
