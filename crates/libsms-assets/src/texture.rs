//! GX textures to RGBA8.
//!
//! J3D stores images as `ResTIMG` headers (JUTTexture.hpp; the same 0x20-byte header is a whole
//! `.bti` file). Formats are `GXTexFmt`; the colour-indexed ones carry a palette in
//! `GXTlutFmt`. Pixel layouts are the standard GX tiled formats. Only the first image level is
//! decoded.

use crate::Error;
use crate::reader::Be;

/// A decoded image, row-major RGBA8, top row first.
#[derive(Debug, Clone)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// `GXTexWrapMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wrap {
    Clamp,
    Repeat,
    Mirror,
}

impl Wrap {
    fn from_gx(v: u8) -> Wrap {
        match v {
            1 => Wrap::Repeat,
            2 => Wrap::Mirror,
            _ => Wrap::Clamp,
        }
    }
}

/// A `ResTIMG` texture: the image plus its sampler settings.
#[derive(Debug, Clone)]
pub struct Texture {
    pub name: String,
    pub format: u8,
    pub image: Image,
    pub wrap_s: Wrap,
    pub wrap_t: Wrap,
    /// `GXTexFilter`: 0 nearest, 1 linear, 2.. mipmapped variants.
    pub min_filter: u8,
    pub mag_filter: u8,
}

/// Parses a `ResTIMG` at `o` in `d`. Image and palette offsets are relative to the header.
pub fn read_timg(d: &[u8], o: usize, name: String) -> Result<Texture, Error> {
    let r = Be(d);
    let format = r.u8(o)?;
    let width = r.u16(o + 2)? as u32;
    let height = r.u16(o + 4)? as u32;
    let wrap_s = Wrap::from_gx(r.u8(o + 6)?);
    let wrap_t = Wrap::from_gx(r.u8(o + 7)?);
    let tlut_format = r.u8(o + 9)?;
    let tlut_count = r.u16(o + 0xA)? as usize;
    let tlut_offset = r.u32(o + 0xC)? as usize;
    let min_filter = r.u8(o + 0x14)?;
    let mag_filter = r.u8(o + 0x15)?;
    let image_offset = r.u32(o + 0x1C)? as usize;
    let palette = if tlut_count > 0 {
        let p = r.bytes(o + tlut_offset, tlut_count * 2)?;
        (0..tlut_count)
            .map(|i| {
                let c = u16::from_be_bytes([p[i * 2], p[i * 2 + 1]]);
                match tlut_format {
                    0 => ia8(c),
                    1 => rgb565(c),
                    _ => rgb5a3(c),
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    let image = decode(
        format,
        width,
        height,
        d.get(o + image_offset..).unwrap_or(&[]),
        &palette,
    )?;
    Ok(Texture {
        name,
        format,
        image,
        wrap_s,
        wrap_t,
        min_filter,
        mag_filter,
    })
}

/// A whole `.bti` file.
pub fn read_bti(d: &[u8], name: String) -> Result<Texture, Error> {
    read_timg(d, 0, name)
}

fn rgb565(c: u16) -> [u8; 4] {
    let r = ((c >> 11) & 0x1F) as u8;
    let g = ((c >> 5) & 0x3F) as u8;
    let b = (c & 0x1F) as u8;
    [
        (r << 3) | (r >> 2),
        (g << 2) | (g >> 4),
        (b << 3) | (b >> 2),
        255,
    ]
}

fn rgb5a3(c: u16) -> [u8; 4] {
    if c & 0x8000 != 0 {
        let r = ((c >> 10) & 0x1F) as u8;
        let g = ((c >> 5) & 0x1F) as u8;
        let b = (c & 0x1F) as u8;
        [
            (r << 3) | (r >> 2),
            (g << 3) | (g >> 2),
            (b << 3) | (b >> 2),
            255,
        ]
    } else {
        let a = ((c >> 12) & 0x7) as u8;
        let r = ((c >> 8) & 0xF) as u8;
        let g = ((c >> 4) & 0xF) as u8;
        let b = (c & 0xF) as u8;
        [r * 0x11, g * 0x11, b * 0x11, (a << 5) | (a << 2) | (a >> 1)]
    }
}

/// IA8 as stored in palettes and textures: alpha in the high byte, intensity in the low.
fn ia8(c: u16) -> [u8; 4] {
    let (a, i) = ((c >> 8) as u8, c as u8);
    [i, i, i, a]
}

/// Tile size in pixels and bytes per tile.
fn tile(format: u8) -> Option<((usize, usize), usize)> {
    Some(match format {
        0 | 8 | 14 => ((8, 8), 32), // I4, C4, CMPR
        1 | 2 | 9 => ((8, 4), 32),  // I8, IA4, C8
        3..=5 | 10 => ((4, 4), 32), // IA8, RGB565, RGB5A3, C14X2
        6 => ((4, 4), 64),          // RGBA8
        _ => return None,
    })
}

/// Decodes the first level of a `GXTexFmt` image. `palette` is used by C4/C8/C14X2.
pub fn decode(
    format: u8,
    width: u32,
    height: u32,
    data: &[u8],
    palette: &[[u8; 4]],
) -> Result<Image, Error> {
    let (w, h) = (width as usize, height as usize);
    let ((tw, th), tile_bytes) =
        tile(format).ok_or_else(|| Error::Bad(format!("texture format {format}")))?;
    let tiles_x = w.div_ceil(tw);
    let tiles_y = h.div_ceil(th);
    let need = tiles_x * tiles_y * tile_bytes;
    if data.len() < need {
        return Err(Error::Bad(format!(
            "format {format} {w}x{h} needs {need} bytes, have {}",
            data.len()
        )));
    }

    let mut rgba = vec![0u8; w * h * 4];
    let mut put = |x: usize, y: usize, px: [u8; 4]| {
        if x < w && y < h {
            rgba[(y * w + x) * 4..][..4].copy_from_slice(&px);
        }
    };
    let be16 = |o: usize| u16::from_be_bytes([data[o], data[o + 1]]);
    let pal = |i: usize| palette.get(i).copied().unwrap_or([0, 0, 0, 0]);

    for ty in 0..tiles_y {
        for tx in 0..tiles_x {
            let base = (ty * tiles_x + tx) * tile_bytes;
            let (x0, y0) = (tx * tw, ty * th);
            match format {
                14 => {
                    // CMPR: four DXT1 blocks per 8x8 tile.
                    for sub in 0..4 {
                        let b = base + sub * 8;
                        let (sx, sy) = (x0 + (sub % 2) * 4, y0 + (sub / 2) * 4);
                        let (c0, c1) = (be16(b), be16(b + 2));
                        let (p0, p1) = (rgb565(c0), rgb565(c1));
                        let mix = |a: u8, b: u8, wa: u16, wb: u16| {
                            ((a as u16 * wa + b as u16 * wb) / (wa + wb)) as u8
                        };
                        let lerp = |wa, wb| -> [u8; 4] {
                            std::array::from_fn(|i| {
                                if i == 3 {
                                    255
                                } else {
                                    mix(p0[i], p1[i], wa, wb)
                                }
                            })
                        };
                        let palette = if c0 > c1 {
                            [p0, p1, lerp(2, 1), lerp(1, 2)]
                        } else {
                            [p0, p1, lerp(1, 1), [0, 0, 0, 0]]
                        };
                        for row in 0..4 {
                            let bits = data[b + 4 + row];
                            for col in 0..4 {
                                let idx = (bits >> (6 - col * 2)) & 3;
                                put(sx + col, sy + row, palette[idx as usize]);
                            }
                        }
                    }
                }
                6 => {
                    // RGBA8: AR pairs for the 16 pixels, then GB pairs.
                    for i in 0..16 {
                        let (a, r) = (data[base + i * 2], data[base + i * 2 + 1]);
                        let (g, b) = (data[base + 32 + i * 2], data[base + 32 + i * 2 + 1]);
                        put(x0 + i % 4, y0 + i / 4, [r, g, b, a]);
                    }
                }
                _ => {
                    for i in 0..tw * th {
                        let (x, y) = (x0 + i % tw, y0 + i / tw);
                        let nibble = |i: usize| {
                            let byte = data[base + i / 2];
                            if i.is_multiple_of(2) { byte >> 4 } else { byte & 0xF }
                        };
                        let px = match format {
                            0 => {
                                let v = nibble(i) * 0x11;
                                [v, v, v, v]
                            }
                            1 => {
                                let v = data[base + i];
                                [v, v, v, v]
                            }
                            2 => {
                                let byte = data[base + i];
                                let (a, v) = ((byte >> 4) * 0x11, (byte & 0xF) * 0x11);
                                [v, v, v, a]
                            }
                            3 => ia8(be16(base + i * 2)),
                            4 => rgb565(be16(base + i * 2)),
                            5 => rgb5a3(be16(base + i * 2)),
                            8 => pal(nibble(i) as usize),
                            9 => pal(data[base + i] as usize),
                            10 => pal((be16(base + i * 2) & 0x3FFF) as usize),
                            _ => unreachable!(),
                        };
                        put(x, y, px);
                    }
                }
            }
        }
    }
    Ok(Image {
        width,
        height,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb5a3_opaque_and_translucent() {
        assert_eq!(rgb5a3(0xFFFF), [255, 255, 255, 255]);
        assert_eq!(rgb5a3(0x0F00), [255, 0, 0, 0]);
    }

    #[test]
    fn i8_tiles_are_8x4() {
        // A 16x4 I8 image is two tiles side by side: the second tile's first byte is (8, 0).
        let mut data = vec![0u8; 64];
        data[32] = 200;
        let img = decode(1, 16, 4, &data, &[]).unwrap();
        assert_eq!(img.rgba[8 * 4], 200);
    }

    #[test]
    fn c8_reads_the_palette() {
        let data = vec![1u8; 32];
        let img = decode(9, 8, 4, &data, &[[0; 4], [9, 8, 7, 6]]).unwrap();
        assert_eq!(&img.rgba[..4], &[9, 8, 7, 6]);
    }
}
