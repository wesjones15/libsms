//! Garment-only pose overlays and leg clearance for retargeted characters.
//! Host adaptation: this is not a port of Nintendo's cloth dynamics.
use glam::{Mat4, Quat, Vec3};
use libsms::Skeleton;

/// Keep the stride but soften Mario's deep knee lift for a taller wearer. Bone lengths
/// remain the target's; this only reduces the gait's rotation excursion.
pub fn gait_weight(name:&str,gait:bool)->f32 {
    if gait && (name.starts_with("jnt_leg_") || name.starts_with("chn_foot_") || name.starts_with("jnt_foot_")) { 0.6 }
    else { 1. }
}

/// Graft authored local garment transforms beneath the already posed body.
pub fn graft(rig: &Skeleton, world: &mut [Mat4], donor: &[Mat4], root: usize) {
    for j in 0..rig.len() {
        if !rig.under(j,root) { continue; }
        let local = rig.parents[j].map_or(donor[j],|p| donor[p].inverse()*donor[j]);
        world[j] = rig.parents[j].map_or(local,|p| world[p]*local);
    }
}

fn frame(rig: &Skeleton, world: &[Mat4], hip: usize) -> Mat4 {
    world[hip] * rig.bind_world()[hip].inverse()
}

fn leg_spheres(rig: &Skeleton, world: &[Mat4], inverse: Mat4) -> Vec<(Vec3,f32)> {
    let mut spheres = Vec::new();
    for side in ["L","R"] {
        let names = [format!("jnt_leg_{side}1"),format!("jnt_leg_{side}2"),format!("jnt_foot_{side}")];
        let Some(ids) = names.iter().map(|s|rig.find(s)).collect::<Option<Vec<_>>>() else { continue };
        let points: Vec<_> = ids.iter().map(|&i| inverse.transform_point3(world[i].w_axis.truncate())).collect();
        let radius = points[0].distance(points[1])*0.24;
        for (segment,pair) in points.windows(2).enumerate() {
            for k in 0..=8 {
                // Leave the waist attachment pinned; legs and bones are never scaled.
                if segment==0 && k<2 { continue }
                spheres.push((pair[0].lerp(pair[1],k as f32/8.),radius));
            }
        }
    }
    spheres
}

/// Move eight authored skirt panels outward where the posed legs occupy their sector.
/// Only the provided garment joints are changed; limb lengths and the body pose are untouched.
pub fn panels(rig: &Skeleton, world: &mut [Mat4], hip: usize, chains: &[Vec<usize>]) {
    let body = frame(rig,world,hip);
    let inv = body.inverse();
    let rest = rig.bind_world();
    let spheres = leg_spheres(rig,world,inv);
    for chain in chains {
        let Some(&last) = chain.last() else { continue };
        let p = rest[last].w_axis.truncate();
        let radial = Vec3::new(p.x,0.,p.z).normalize_or_zero();
        let tangent = Vec3::new(radial.z,0.,-radial.x);
        let original: Vec<_> = chain.iter().map(|&j|world[j]).collect();
        let mut goals: Vec<_> = original.iter().map(|m| inv.transform_point3(m.w_axis.truncate())).collect();
        for (k,goal) in goals.iter_mut().enumerate().skip(1) {
            let mut extent = goal.dot(radial);
            // Account for the patch between adjacent control chains, not just its centreline.
            for &(leg,radius) in &spheres {
                let dy = (goal.y-leg.y).abs();
                if dy>=radius { continue; }
                let cross_radius = (radius*radius-dy*dy).sqrt();
                let lateral = (leg-*goal).dot(tangent).abs();
                let patch_width = goal.dot(radial).abs()*0.45;
                if lateral > cross_radius+patch_width { continue; }
                extent = extent.max(leg.dot(radial)+cross_radius+radius*0.15);
            }
            let push = (extent-goal.dot(radial)).max(0.);
            *goal += radial*push;
            // Keep the first waistband interval attached; lower segments can fan freely.
            if k==1 { *goal = inv.transform_point3(original[k].w_axis.truncate()).lerp(*goal,0.85); }
        }
        for k in 0..chain.len() {
            let (scale,rot,_) = original[k].to_scale_rotation_translation();
            let rotation = if k+1<chain.len() {
                let old = original[k+1].w_axis.truncate()-original[k].w_axis.truncate();
                let new = body.transform_vector3(goals[k+1]-goals[k]);
                if old.length_squared()>1e-8 && new.length_squared()>1e-8 {
                    Quat::from_rotation_arc(old.normalize(),new.normalize())*rot
                } else { rot }
            } else { rot };
            world[chain[k]] = Mat4::from_scale_rotation_translation(scale,rotation,body.transform_point3(goals[k]));
        }
    }
}

/// Sunshine's mesh has only two weighted dress helpers, both at the waist. Fan those
/// garment matrices to cover the current leg envelope while retaining the authored sway.
pub fn cone(rig: &Skeleton, world: &mut [Mat4], hip: usize, helpers: [usize;2],
    pivot: Vec3, height: f32, top: Vec3, bottom: Vec3) {
    let body = frame(rig,world,hip);
    let inv = body.inverse();
    let spheres = leg_spheres(rig,world,inv);
    let mut sx = 1.0f32;
    // Front and back are fitted separately: a lifted knee pushes only the front panel out.
    let (mut front, mut back) = (1.0f32,1.0f32);
    for (leg,radius) in spheres {
        let depth = ((pivot.y-leg.y)/height).clamp(0.,1.);
        let width = top.lerp(bottom,depth);
        sx = sx.max((leg.x.abs()+radius)/width.x.max(1.));
        front = front.max((leg.z+radius)/width.z.max(1.));
        back = back.max((radius-leg.z)/width.z.max(1.));
    }
    let (front,back) = (front.clamp(1.,1.5),back.clamp(1.,1.5));
    // Mean scale plus a shear that grows with depth, so the waist stays attached.
    let scale_z = (front+back)*0.5;
    let shear = (front-back)*0.5*bottom.z/height.max(1.);
    let flare = Mat4::from_scale(Vec3::new(sx.clamp(1.,1.2),1.,scale_z));
    let mut lean = Mat4::IDENTITY;
    // z += shear * (pivot.y - y); the column-major entry (col y, row z).
    lean.y_axis.z = -shear;
    let garment = body*Mat4::from_translation(pivot)*lean*flare
        *Mat4::from_translation(-pivot)*inv;
    for j in helpers { world[j] = garment*world[j]; }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn graft_changes_only_garment_subtree() {
        let bind = libsms::Srt{scale:Vec3::ONE,rot:Vec3::ZERO,trans:Vec3::Y};
        let rig = Skeleton{names:vec!["hip".into(),"leg".into(),"dress".into()],parents:vec![None,Some(0),Some(0)],
            bind:vec![bind;3],calc:libsms::MtxCalc::Basic,compensate:vec![false;3]};
        let mut world = rig.bind_world(); let leg = world[1];
        let mut donor = world.clone(); donor[2] *= Mat4::from_rotation_x(0.5);
        graft(&rig,&mut world,&donor,2);
        assert_eq!(leg,world[1]); assert!(!world[2].abs_diff_eq(rig.bind_world()[2],0.01));
    }
}
