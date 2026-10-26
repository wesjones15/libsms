//! Rotation retargeting between disc skeletons with different joint counts and hierarchies.
//! Host adaptation, following the bind-world delta approach used by libssbm-assets::sms.
use glam::{Mat4, Quat};
use libsms::{Skeleton, Srt};

pub struct Retarget {
    source_bind: Vec<Quat>,
    target_bind: Vec<Quat>,
    /// Target joint -> source joint; unmapped joints inherit their parent's motion.
    mapping: Vec<Option<usize>>,
}

fn rotations(world: &[Mat4]) -> Vec<Quat> {
    world.iter().map(|m| m.to_scale_rotation_translation().1.normalize()).collect()
}

impl Retarget {
    /// Equal names map automatically; aliases are `(target name, source name)`.
    pub fn new(source: &Skeleton, target: &Skeleton, aliases: &[(&str, &str)]) -> Self {
        let mapping = target.names.iter().map(|name| {
            let source_name = aliases.iter().find(|(t, _)| *t == name)
                .map_or(name.as_str(), |(_, s)| *s);
            source.find(source_name)
        }).collect();
        Self { source_bind: rotations(&source.bind_world()),
            target_bind: rotations(&target.bind_world()), mapping }
    }

    /// Transfer a source pose's world rotation changes, retaining target bone lengths.
    /// Root placement/ground anchoring belongs to the host, as with the Luigi skin.
    pub fn pose(&self, target: &Skeleton, source_world: &[Mat4]) -> Vec<Mat4> {
        self.pose_weighted(target,source_world, |_| 1.)
    }

    /// Reduce a joint's rotation delta without changing its translation or scale.
    /// Useful for adapting a high-knee gait to a long-legged character in a dress.
    pub fn pose_weighted(&self,target:&Skeleton,source_world:&[Mat4],weight:impl Fn(&str)->f32)->Vec<Mat4> {
        let source_rot = rotations(source_world);
        let mut world_rot: Vec<Quat> = Vec::with_capacity(target.len());
        let mut locals: Vec<Srt> = Vec::with_capacity(target.len());
        for i in 0..target.len() {
            let parent = target.parents[i].filter(|&p| p < i);
            let parent_rot = parent.map_or(Quat::IDENTITY, |p| world_rot[p]);
            let desired = self.mapping[i].and_then(|s| source_rot.get(s).map(|r| {
                let delta = *r * self.source_bind[s].inverse();
                Quat::IDENTITY.slerp(delta,weight(&target.names[i]).clamp(0.,1.)) * self.target_bind[i]
            }));
            let mut local = target.bind[i];
            if let Some(desired) = desired {
                let q = (parent_rot.inverse() * desired).normalize();
                let (z, y, x) = q.to_euler(glam::EulerRot::ZYX);
                local.rot = glam::Vec3::new(x, y, z);
            }
            world_rot.push((parent_rot * local.quat()).normalize());
            locals.push(local);
        }
        target.world(&locals)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use libsms::MtxCalc;

    fn rig(names: &[&str], parents: Vec<Option<usize>>, offsets: &[Vec3]) -> Skeleton {
        Skeleton { names: names.iter().map(|n| n.to_string()).collect(), parents,
            bind: offsets.iter().map(|&t| Srt { scale: Vec3::ONE, rot: Vec3::ZERO, trans: t }).collect(),
            calc: MtxCalc::Softimage, compensate: vec![false; names.len()] }
    }

    #[test]
    fn different_hierarchy_keeps_target_lengths_and_moves_unmapped_children() {
        let source = rig(&["root", "helper", "hand"], vec![None, Some(0), Some(1)],
            &[Vec3::ZERO, Vec3::X, Vec3::X]);
        let target = rig(&["root", "wrist", "jewel"], vec![None, Some(0), Some(1)],
            &[Vec3::ZERO, Vec3::X * 4.0, Vec3::X]);
        let r = Retarget::new(&source, &target, &[("wrist", "hand")]);
        let rest = r.pose(&target, &source.bind_world());
        for (a,b) in rest.iter().zip(target.bind_world()) { assert!(a.abs_diff_eq(b, 1e-5)); }
        let mut locals = source.bind.clone();
        locals[1].rot.z = std::f32::consts::FRAC_PI_2;
        let posed = r.pose(&target, &source.world(&locals));
        assert!(posed[1].w_axis.truncate().abs_diff_eq(Vec3::X * 4.0, 1e-5));
        assert!(posed[2].w_axis.truncate().abs_diff_eq(Vec3::new(4.0,1.0,0.0), 1e-5));
        assert!(posed.iter().all(|m| m.is_finite()));
    }
}
