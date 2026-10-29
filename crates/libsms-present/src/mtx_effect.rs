//! Sunshine's NPC waist callback: `NpcInitPrg.cpp::TBaseNPC::setMtxEffect_`
//! installs type 0 on `koshi_null`; `MtxUtil.cpp::TMtxTimeLag::calc` integrates
//! translation and quaternion springs before J3D visits the joint's children.
use glam::{Mat4, Quat, Vec3, Vec4};
use libsms::prm::Prm;

#[derive(Clone, Copy, Debug)]
pub struct TimeLagParams {
    pub pos_accel: f32,
    pub pos_brake: f32,
    pub pos_limit: f32,
    pub quat_accel: f32,
    pub quat_brake: f32,
}

impl TimeLagParams {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let p = Prm::parse(bytes).map_err(|e| e.to_string())?;
        let get = |name| {
            p.f32(name)
                .filter(|v| v.is_finite() && *v >= 0.0)
                .ok_or_else(|| format!("missing or invalid {name}"))
        };
        Ok(Self {
            pos_accel: get("mPosAccel")?,
            pos_brake: get("mPosBrake")?,
            pos_limit: get("mPosLimit")?,
            quat_accel: get("mQuatAccel")?,
            quat_brake: get("mQuatBrake")?,
        })
    }
}

#[derive(Clone, Copy, Debug)]
struct Spring {
    pos: Vec3,
    velocity: Vec3,
    quat: Vec4,
    quat_velocity: Vec4,
}

impl Spring {
    fn new(target: Mat4) -> Self {
        let (_, q, pos) = target.to_scale_rotation_translation();
        Self {
            pos,
            velocity: Vec3::ZERO,
            quat: Vec4::from_array(q.to_array()),
            quat_velocity: Vec4::ZERO,
        }
    }

    fn step(&mut self, target: Mat4, p: TimeLagParams) {
        let (_, q, pos) = target.to_scale_rotation_translation();
        self.velocity = (self.velocity + (pos - self.pos) * p.pos_accel) * p.pos_brake;
        self.pos = (self.pos + self.velocity).clamp(
            pos - Vec3::splat(p.pos_limit),
            pos + Vec3::splat(p.pos_limit),
        );
        let mut q = Vec4::from_array(q.to_array());
        if self.quat.dot(q) < 0.0 {
            q = -q;
        }
        self.quat_velocity = (self.quat_velocity + (q - self.quat) * p.quat_accel) * p.quat_brake;
        self.quat += self.quat_velocity;
    }

    fn matrix(&self, target: Mat4) -> Mat4 {
        // MTXQuat normalises when building the matrix, but the spring retains its
        // unnormalised four components. Preserve the target's column lengths.
        let scale = Vec3::new(
            target.x_axis.length(),
            target.y_axis.length(),
            target.z_axis.length(),
        );
        let q = Quat::from_array(self.quat.to_array()).normalize();
        Mat4::from_scale_rotation_translation(scale, q, self.pos)
    }
}

/// Per actor, in Sunshine world units. The callback is once per 30 Hz animation
/// calculation; hosts interpolate its target at those instants, independent of FPS.
#[derive(Default, Debug)]
pub struct TimeLag {
    spring: Option<Spring>,
    previous: Mat4,
    time: f64,
    next: f64,
}

fn interpolate(a: Mat4, b: Mat4, t: f32) -> Mat4 {
    let (sa, qa, pa) = a.to_scale_rotation_translation();
    let (sb, qb, pb) = b.to_scale_rotation_translation();
    Mat4::from_scale_rotation_translation(sa.lerp(sb, t), qa.slerp(qb, t), pa.lerp(pb, t))
}

impl TimeLag {
    pub fn reset(&mut self) {
        self.spring = None;
    }

    pub fn update(&mut self, target: Mat4, time: f32, p: TimeLagParams) -> Mat4 {
        let time = time as f64;
        let dt = time - self.time;
        // Source flag 2 resets on first use / returning from culled animation.
        // Host discontinuities (clock rewind, long absence, teleport) do likewise.
        let moved = target
            .w_axis
            .truncate()
            .distance(self.previous.w_axis.truncate());
        if self.spring.is_none() || dt < 0.0 || dt > 0.25 || moved > (p.pos_limit * 4.0).max(1.0) {
            self.spring = Some(Spring::new(target));
            self.next = time + 1.0 / 30.0;
        } else if dt > 0.0 {
            while self.next <= time + 1e-6 {
                let fraction = ((self.next - self.time) / dt).clamp(0.0, 1.0) as f32;
                self.spring
                    .as_mut()
                    .unwrap()
                    .step(interpolate(self.previous, target, fraction), p);
                self.next += 1.0 / 30.0;
            }
        }
        self.previous = target;
        self.time = time;
        self.spring.as_ref().unwrap().matrix(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn params() -> TimeLagParams {
        TimeLagParams {
            pos_accel: 1.0,
            pos_brake: 0.1,
            pos_limit: 30.0,
            quat_accel: 1.0,
            quat_brake: 0.05,
        }
    }

    #[test]
    fn source_step_lags_clamps_and_keeps_quaternion_hemisphere() {
        let mut s = Spring::new(Mat4::IDENTITY);
        let target = Mat4::from_rotation_y(0.8) * Mat4::from_translation(Vec3::X * 100.0);
        s.step(target, params());
        assert!((target.w_axis.truncate() - s.pos).abs().max_element() <= 30.0001);
        let q = Quat::from_rotation_y(0.8);
        let want = Vec4::W + (Vec4::from_array(q.to_array()) - Vec4::W) * 0.05;
        assert!(s.quat.abs_diff_eq(want, 1e-6));
        assert!(s.matrix(target).is_finite());
    }

    #[test]
    fn rates_and_resets() {
        let mut results = Vec::new();
        for fps in [30, 60, 144] {
            let mut lag = TimeLag::default();
            let mut result = Mat4::IDENTITY;
            for i in 0..=fps {
                let t = i as f32 / fps as f32;
                result = lag.update(
                    Mat4::from_rotation_y(t) * Mat4::from_translation(Vec3::Y * t * 40.0),
                    t,
                    params(),
                );
            }
            results.push(result);
            let teleport = Mat4::from_translation(Vec3::splat(1000.0));
            assert!(
                lag.update(teleport, 1.01, params())
                    .abs_diff_eq(teleport, 1e-5)
            );
            assert!(
                lag.update(Mat4::IDENTITY, 0.0, params())
                    .abs_diff_eq(Mat4::IDENTITY, 1e-5)
            );
        }
        for result in &results[1..] {
            assert!(result.abs_diff_eq(results[0], 2e-4));
        }
    }
}
