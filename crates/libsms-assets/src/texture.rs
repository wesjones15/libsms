//! GX textures to RGBA8.
//!
//! J3D stores images as `ResTIMG` headers (JUTTexture.hpp; the same 0x20-byte header is a whole
//! `.bti` file). Formats are `GXTexFmt`; the colour-indexed ones carry a palette in
//! `GXTlutFmt`. Pixel layouts are the standard GX tiled formats. Only the first image level is
//! decoded.

use crate::Error;
use crate::reader::Be;
use libcommon::texture::gx::{self, GxFormat, ia8, rgb565, rgb5a3};

/// A decoded image, row-major RGBA8, top row first.
#[derive(Debug, Clone)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Shared GX sampler modes; re-exported to preserve existing asset reader APIs.
pub use libcommon::texture::Wrap;

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

/// Decodes the first level of a `GXTexFmt` image. `palette` is used by C4/C8/C14X2.
pub fn decode(
    format: u8,
    width: u32,
    height: u32,
    data: &[u8],
    palette: &[[u8; 4]],
) -> Result<Image, Error> {
    let gx = GxFormat::from_gx(format as u32)
        .ok_or_else(|| Error::Bad(format!("texture format {format}")))?;
    let rgba = gx::decode(gx, width, height, data, palette).map_err(|e| Error::Bad(e.to_string()))?;
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

    fn fnv(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf29ce484222325, |h, &b| (h ^ b as u64).wrapping_mul(0x100000001b3))
    }

    /// Pixel-exact baseline over pseudo-random data, including partial edge tiles and
    /// palette indices past the palette's end, so a refactor cannot change color or alpha
    /// unnoticed.
    #[test]
    fn decoded_pixels_match_baseline() {
        let mut s = 0x1234_5678u32;
        let mut next = || {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            (s >> 24) as u8
        };
        let data: Vec<u8> = (0..4096).map(|_| next()).collect();
        let palette: Vec<[u8; 4]> = (0..40).map(|_| [next(), next(), next(), next()]).collect();
        let formats = [0u8, 1, 2, 3, 4, 5, 6, 8, 9, 10, 14];
        let got: Vec<u64> = formats
            .iter()
            .map(|&f| fnv(&decode(f, 13, 11, &data, &palette).unwrap().rgba))
            .collect();
        println!("{got:#x?}");
        assert_eq!(got, BASELINE);
    }

    // Recorded from the decoder before it moved to libcommon::texture::gx.
    const BASELINE: [u64; 11] = [
        0x8951b18c7a118b4d,
        0x46ebc824be401321,
        0x9824912f42098bdf,
        0x7ba54e580471227b,
        0xa7e5dc0259b842b2,
        0x433f0bf7c535a946,
        0xb51c3975d39f7dfc,
        0xb9bf7e9cd143e6cc,
        0x6214fbdb537f24b7,
        0x10eb3a1f96c15455,
        0xb9d7fb80fab30c30,
    ];

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
