//! Delfino Plaza's M gate, `MoveBG/ModelGate.cpp::loadAfter/perform`.
//! Model 05_gate01, BTK plus BRK, and JPA gatewind cues 0x131..0x134.
//! A replaceable still supplies the three movie Y/U/V planes; the authored TEV,
//! mask and indirect texture retain their own animation.
use crate::tev::{self, Baked, Inputs, Tex};
use libsms_assets::{Bmd, Error, Image, brk::Colors, btk::TexSrt};
use std::path::Path;

pub struct Portal {
    pub model: Bmd,
    texture: TexSrt,
    colors: Colors,
    textures: Vec<Option<Tex>>,
}
impl Portal {
    pub fn load(dir: &Path) -> Result<Self, Error> {
        let mut model = Bmd::parse(&std::fs::read(dir.join("portal/05_gate01.bmd"))?)?;
        // Keep the M upright independently of the replaceable interior's row order.
        // Slot 5 is the fixed mask; BTK animates slots 3 and 4 only.
        if let Some(mask) = &mut model.materials[0].tex_mtx[5] {
            mask.scale.y = -mask.scale.y;
        }
        let texture = TexSrt::parse(&std::fs::read(dir.join("portal/05_gate01.btk"))?)?;
        let colors = Colors::parse(&std::fs::read(dir.join("portal/05_gate01.brk"))?)?;
        let textures = model
            .textures
            .iter()
            .map(|t| Some(Tex::new(t.image.clone(), t.wrap_s, t.wrap_t)))
            .collect();
        Ok(Self {
            model,
            texture,
            colors,
            textures,
        })
    }
    /// Full-range YCbCr, matching the gate's first three TEV stages (Y/U/V THP planes).
    pub fn set_interior(&mut self, still: &Image) {
        for plane in 0..3 {
            let mut image = still.clone();
            for px in image.rgba.chunks_exact_mut(4) {
                let [r, g, b] = [px[0], px[1], px[2]].map(|v| v as f32);
                let v = match plane {
                    0 => 0.299 * r + 0.587 * g + 0.114 * b,
                    1 => 128. - 0.168736 * r - 0.331264 * g + 0.5 * b,
                    _ => 128. + 0.5 * r - 0.418688 * g - 0.081312 * b,
                };
                px.fill(v.round().clamp(0., 255.) as u8);
            }
            self.textures[plane] = Some(Tex::new(
                image,
                libsms_assets::texture::Wrap::Clamp,
                libsms_assets::texture::Wrap::Clamp,
            ));
        }
        // The original movie uses only a vertical slice. A still fills the M.
        for matrix in self.model.materials[0].tex_mtx[..3].iter_mut().flatten() {
            matrix.center = glam::Vec3::ZERO;
            matrix.scale = glam::Vec2::new(1., -1.);
            matrix.translation = glam::Vec2::new(0., 1.);
        }
    }
    /// Host entrance direction: 0 shows the swirling colours, 1 reveals the still.
    /// BRK follows the source's unkD0; BTK keeps running throughout.
    pub fn image(&mut self, frame: f32, activation: f32) -> Baked {
        self.texture.apply(&mut self.model, frame);
        self.colors.apply(
            &mut self.model,
            activation.clamp(0., 1.) * self.colors.frames,
        );
        let channels = [[
            self.model.materials[0].mat_colors[0].map(|v| v as f32 / 255.),
            [1.; 4],
        ]];
        let mut baked = tev::Source {
            bmd: &self.model,
            textures: &self.textures,
            channels: &channels,
        }
        .bake_preserving_alpha(0, &Inputs::default(), 512)
        .expect("gate is texture mapped");
        // The source material is GX_BL_SRCALPHA / GX_BL_ZERO. Its surviving
        // edge pixels replace the wall with C*A, leaving the dark M border.
        // Encode that result plus the alpha-test coverage for ordinary hosts.
        for p in baked.rgba.chunks_exact_mut(4) {
            let alpha = p[3] as u32;
            for c in &mut p[..3] {
                *c = ((*c as u32 * alpha + 127) / 255) as u8;
            }
            p[3] = if alpha == 0 { 0 } else { 255 };
        }
        baked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assets() -> Option<std::path::PathBuf> {
        let dir = libsms::assets::find(None)?;
        if !dir.join("portal/05_gate01.bmd").is_file() {
            eprintln!("SKIP: portal assets absent");
            return None;
        }
        Some(dir)
    }

    #[test]
    fn disc_gate_uses_model_distortion_and_animates_with_a_replaceable_still() {
        let Some(dir) = assets() else {
            eprintln!("SKIP: Sunshine assets absent");
            return;
        };
        let mut p = Portal::load(&dir).unwrap();
        let m = &p.model.materials[0];
        assert_eq!(m.tex_gens.len(), 6);
        assert_eq!(m.stages.len(), 7);
        let ind = m
            .indirect
            .as_ref()
            .expect("05_gate01 must retain its indirect stage");
        assert_eq!(ind.orders, [[4, 4]]);
        assert!((ind.matrices[0][0][0] - 0.6 / 32.).abs() < 1e-6);
        assert_eq!(ind.stages[4][2..4], [7, 1]);
        let still = crate::plaza::plaza_still(&dir, 128).unwrap();
        p.set_interior(&still);
        let idle = p.image(0., 0.);
        let active = p.image(0., 1.);
        let next = p.image(30., 1.);
        assert_ne!(idle.rgba, active.rgba, "spraying must reveal the interior");
        assert_ne!(
            active.rgba, next.rgba,
            "a still interior must not freeze the M distortion"
        );
        let original = p.model.materials[0].indirect.take();
        let flat = p.image(30., 1.);
        assert_ne!(
            next.rgba, flat.rgba,
            "indirect map must alter the M, beyond BTK scrolling alone"
        );
        p.model.materials[0].indirect = original;
        assert_eq!(
            idle.rgba,
            p.image(p.texture.frames, 0.).rgba,
            "BTK must loop without a seam"
        );
        let before = p.image(0., 1.).rgba;
        p.set_interior(&Image {
            width: 128,
            height: 128,
            rgba: [20, 60, 190, 255].repeat(128 * 128),
        });
        assert_ne!(
            before,
            p.image(0., 1.).rgba,
            "interior replacement must remain independent of the M"
        );
    }
}
