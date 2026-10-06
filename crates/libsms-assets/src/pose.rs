//! Skinning a [`Bmd`] with joint matrices.
//!
//! Joint world matrices come from `libsms::Skeleton::pose` (the clip the sim is playing) or
//! [`Bmd::bind_world`]. Envelope matrices are `sum(weight * joint * inverse_bind)`
//! (`J3DModel::calcWeightEnvelopeMtx`, J3DModel.cpp).

use glam::{Mat4, Vec3, Vec4};

use crate::bmd::{BatchVertex, Bmd, Skin};

/// `J3DGetTranslateRotateMtx` plus scale: T * Rz * Ry * Rx * S.
pub fn srt_matrix(s: Vec3, r: Vec3, t: Vec3) -> Mat4 {
    let (sx, cx) = r.x.sin_cos();
    let (sy, cy) = r.y.sin_cos();
    let (sz, cz) = r.z.sin_cos();
    let c0 = Vec4::new(cy * cz, cy * sz, -sy, 0.0) * s.x;
    let c1 = Vec4::new(sx * sy * cz - cx * sz, sx * sy * sz + cx * cz, sx * cy, 0.0) * s.y;
    let c2 = Vec4::new(cx * cz * sy + sx * sz, cx * sz * sy - sx * cz, cx * cy, 0.0) * s.z;
    Mat4::from_cols(c0, c1, c2, t.extend(1.0))
}

/// World matrices for a model's joints and envelopes.
#[derive(Debug, Clone)]
pub struct Pose {
    pub joints: Vec<Mat4>,
    pub envelopes: Vec<Mat4>,
}

impl Pose {
    /// `root` places the model; `joints` are model-space joint matrices, one per joint.
    pub fn new(model: &Bmd, root: Mat4, joints: &[Mat4]) -> Pose {
        let joints: Vec<Mat4> = (0..model.joints.len())
            .map(|i| root * joints.get(i).copied().unwrap_or(Mat4::IDENTITY))
            .collect();
        let envelopes = model
            .envelopes
            .iter()
            .map(|weights| {
                let mut m = Mat4::ZERO;
                for &(j, w) in weights {
                    let j = j as usize;
                    if let (Some(jm), Some(ib)) = (joints.get(j), model.inverse_bind.get(j)) {
                        m += (*jm * *ib) * w;
                    }
                }
                m
            })
            .collect();
        Pose { joints, envelopes }
    }

    pub fn matrix(&self, skin: Skin) -> Mat4 {
        match skin {
            Skin::Joint(j) => self
                .joints
                .get(j as usize)
                .copied()
                .unwrap_or(Mat4::IDENTITY),
            Skin::Envelope(e) => self
                .envelopes
                .get(e as usize)
                .copied()
                .unwrap_or(Mat4::IDENTITY),
        }
    }

    /// World position and (unnormalised) normal of a batch vertex.
    pub fn skin(&self, v: &BatchVertex) -> (Vec3, Vec3) {
        let m = self.matrix(v.skin);
        (
            m.transform_point3(v.position),
            m.transform_vector3(v.normal),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srt_is_t_rz_ry_rx_s() {
        let r = Vec3::new(0.3, -0.7, 1.1);
        let m = srt_matrix(Vec3::splat(2.0), r, Vec3::new(1.0, 2.0, 3.0));
        let want = Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0))
            * Mat4::from_rotation_z(r.z)
            * Mat4::from_rotation_y(r.y)
            * Mat4::from_rotation_x(r.x)
            * Mat4::from_scale(Vec3::splat(2.0));
        assert!(m.abs_diff_eq(want, 1e-5));
    }
}
