//! `Map/Shimmer.cpp::TShimmer`: dolpic1's typed Shimmer record selects
//! ShimmerLow.bmd/.btk. Texture slot 0 is B_kagero_s; slot 1 is replaced by
//! TScreenTexture in loadAfter. This is an indirect scene-refraction sheet.
use glam::Vec2;
use libsms_assets::{Bmd, bmd::TexMtx, btk::TexSrt, texture::Texture};
use std::path::Path;

pub const FILES: [&str; 2] = ["heat_haze/shimmerlow.bmd", "heat_haze/shimmerlow.btk"];

pub struct HeatHaze {
    model: Bmd,
    animation: TexSrt,
    pub texture: Texture,
    pub packed: Vec<u8>,
    pub normal_gain: f32,
    /// Camera-space distance in Sunshine units, from the authored sheet.
    pub depth: f32,
    matrix: [[f32; 3]; 2],
}

impl HeatHaze {
    pub fn load(dir: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(dir.join(FILES[0])).map_err(|e| e.to_string())?;
        let model = Bmd::parse(&bytes).map_err(|e| e.to_string())?;
        let bytes = std::fs::read(dir.join(FILES[1])).map_err(|e| e.to_string())?;
        let animation = TexSrt::parse(&bytes).map_err(|e| e.to_string())?;
        let material = model.materials.first().ok_or("Shimmer material absent")?;
        let indirect = material
            .indirect
            .as_ref()
            .ok_or("Shimmer indirect stage absent")?;
        if indirect.orders.first() != Some(&[0, 0])
            || indirect.stages.first().map(|s| (s[2], s[3])) != Some((7, 1))
        {
            return Err("unsupported Shimmer indirect layout".into());
        }
        let texture = material.textures[0]
            .and_then(|id| model.textures.get(id as usize))
            .ok_or("Shimmer normal map absent")?
            .clone();
        let matrix = indirect.matrices[0];
        let batches = model.batches().map_err(|e| e.to_string())?;
        let depth = -batches
            .iter()
            .flat_map(|b| &b.vertices)
            .map(|v| v.position.z)
            .next()
            .ok_or("Shimmer sheet absent")?;
        if !depth.is_finite() || depth <= 0.0 {
            return Err("invalid Shimmer depth".into());
        }
        // GX indirect STU = alpha, blue, green, biased by 128 (bias mask 7).
        // Pack AG normals like particles.rs's existing host refraction maps.
        let mut offsets = Vec::new();
        for px in texture.image.rgba.chunks_exact(4) {
            let stu = [px[3], px[2], px[1]].map(|c| (c as f32 - 128.0) / 128.0);
            // GX offsets are texels of the half-resolution screen copy:
            // ScreenUtil.cpp / Resolution.cpp -> 320 x 224.
            let mut offset = matrix.map(|r| r.iter().zip(stu).map(|(a, b)| a * b).sum::<f32>());
            offset[0] *= 128.0 / 320.0;
            offset[1] *= 128.0 / 224.0;
            offsets.push(Vec2::from_array(offset));
        }
        // Preserve this very subtle source displacement while making the
        // vectors eligible for the host shader's near-flat-normal discard.
        // Bound the largest vector to 0.8, then compensate by 1/gain in the host.
        let peak = offsets.iter().map(|v| v.length()).fold(0.0, f32::max);
        if peak <= 1e-8 {
            return Err("Shimmer displacement map is empty".into());
        }
        let normal_gain = 0.8 / peak;
        let mut packed = Vec::with_capacity(texture.image.rgba.len());
        for offset in offsets {
            let pack = |v: f32| {
                ((v * normal_gain * 0.5 + 0.5) * 255.)
                    .round()
                    .clamp(0., 255.) as u8
            };
            packed.extend_from_slice(&[255, pack(-offset[1]), 255, pack(offset[0])]);
        }
        Ok(Self {
            model,
            animation,
            texture,
            packed,
            normal_gain,
            depth,
            matrix,
        })
    }

    fn texture_matrix(&mut self, seconds: f32) -> TexMtx {
        // The BTK itself supplies the 2x5 scale and vertical scroll; the host
        // samples it at 60 animation frames/s, like the other Sunshine presenters.
        self.animation
            .apply(&mut self.model, seconds.max(0.0) * 60.0);
        self.model.materials[0].tex_mtx[0].unwrap()
    }

    pub fn uv_rows(&mut self, seconds: f32) -> [[f32; 3]; 2] {
        self.texture_matrix(seconds).srt_rows()
    }

    /// Source displacement in screen UV units, for the headless viewer proof.
    pub fn offset(&mut self, uv: Vec2, seconds: f32) -> Vec2 {
        let rows = self.uv_rows(seconds);
        let uv = Vec2::new(
            rows[0][0] * uv.x + rows[0][1] * uv.y + rows[0][2],
            rows[1][0] * uv.x + rows[1][1] * uv.y + rows[1][2],
        );
        let image = &self.texture.image;
        let x = (uv.x.rem_euclid(1.) * image.width as f32) as usize % image.width as usize;
        let y = (uv.y.rem_euclid(1.) * image.height as f32) as usize % image.height as usize;
        let px = &image.rgba[(y * image.width as usize + x) * 4..][..4];
        let stu = [px[3], px[2], px[1]].map(|c| (c as f32 - 128.) / 128.);
        let v = self
            .matrix
            .map(|r| r.iter().zip(stu).map(|(a, b)| a * b).sum::<f32>());
        Vec2::new(v[0] * 128. / 320., v[1] * 128. / 224.)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disc_shimmer_uses_original_indirect_map_scroll_and_depth() {
        let Some(dir) = libsms::assets::find(None) else {
            eprintln!("SKIP: Sunshine assets absent");
            return;
        };
        if !FILES.iter().all(|f| dir.join(f).is_file()) {
            eprintln!("SKIP: Shimmer cache absent");
            return;
        }
        let mut h = HeatHaze::load(&dir).unwrap();
        assert_eq!(h.texture.name, "B_kagero_s");
        assert_eq!((h.texture.image.width, h.texture.image.height), (64, 64));
        assert!((h.depth - 2425.).abs() < 0.01);
        assert_eq!(h.animation.frames, 600.);
        let a = h.uv_rows(0.);
        let b = h.uv_rows(0.25);
        assert_eq!((a[0][0], a[1][1]), (2., 5.));
        assert!((a[1][2] - b[1][2]).abs() > 0.1);
        assert_eq!(a, h.uv_rows(10.));
        assert_eq!(h.packed.len(), 64 * 64 * 4);
        // Amplification must not amplify the actual scene warp after the
        // shader's reciprocal magnitude is applied.
        for (source, packed) in h
            .texture
            .image
            .rgba
            .chunks_exact(4)
            .zip(h.packed.chunks_exact(4))
        {
            let stu = [source[3], source[2], source[1]].map(|v| v as f32 - 128.);
            let source_offset = h
                .matrix
                .map(|r| r.iter().zip(stu).map(|(a, b)| a * b).sum::<f32>());
            let source_offset = Vec2::new(source_offset[0] / 320., source_offset[1] / 224.);
            let decoded = Vec2::new(
                packed[3] as f32 / 255. * 2. - 1.,
                -(packed[1] as f32 / 255. * 2. - 1.),
            ) / h.normal_gain;
            assert!((decoded - source_offset).abs().max_element() <= 1.01 / (255. * h.normal_gain));
        }
        assert!(
            h.packed
                .chunks_exact(4)
                .filter(|p| {
                    let x = p[3] as f32 / 255. * 2. - 1.;
                    let y = p[1] as f32 / 255. * 2. - 1.;
                    x * x + y * y > 0.02 // survive the host's near-flat-normal discard
                })
                .count()
                > 100
        );
    }
}
