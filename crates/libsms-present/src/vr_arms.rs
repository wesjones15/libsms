//! VR arms for J3D bodies (libcommon's `vr_arms`): Mario, his same-rig skins and Yoshi here,
//! and the J3D bodies libpikmin borrows (Pikmin 2's Louie, the Pianta). Luigi's HSD body goes
//! through `libssbm_assets::vr_arms` ([`crate::luigi_skin::Luigi::vr_mesh`]).
//!
//! The mesh is laid out as the views draw a J3D body: every batch's vertices in batch order, its
//! index list offset by its first vertex. A batch no material shows (`Cull::All`) keeps its place
//! but no weights.

use std::path::Path;

use libcommon::vr_arms::{Arms, Mesh, Side};
use libsms_assets::bmd::Cull;
use libsms_assets::{Batch, Bmd, Pose};

use glam::Mat4;

/// `bmd`'s body in its bind pose, as the views lay it out.
pub fn mesh(bmd: &Bmd, batches: &[Batch]) -> Mesh {
    let bind = bmd.bind_world();
    let pose = Pose::new(bmd, Mat4::IDENTITY, &bind);
    let mut m = Mesh {
        parents: bmd.joints.iter().map(|j| j.parent).collect(),
        bind: bind.iter().map(|b| b.to_cols_array()).collect(),
        ..Mesh::default()
    };
    for b in batches {
        let first = m.positions.len() as u32;
        let shown = bmd.materials.get(b.material).is_some_and(|mat| mat.cull != Cull::All);
        for v in &b.vertices {
            m.positions.push(pose.skin(v).0.into());
            m.weights.push(if shown { weights(bmd, v.skin) } else { Vec::new() });
        }
        m.triangles.extend(b.indices.chunks_exact(3).map(|t| [first + t[0], first + t[1], first + t[2]]));
    }
    m
}

fn weights(bmd: &Bmd, skin: libsms_assets::bmd::Skin) -> Vec<(u16, f32)> {
    use libsms_assets::bmd::Skin;
    match skin {
        Skin::Joint(j) => vec![(j, 1.0)],
        Skin::Envelope(e) => bmd.envelopes.get(e as usize).cloned().unwrap_or_default(),
    }
}

/// How a J3D rig names its arms.
#[derive(Clone, Copy, Debug)]
pub enum Rig {
    /// A hand joint under a forearm joint, by name (right, left).
    Hands(&'static str, &'static str),
    /// The whole arm one joint off the body (right, left): [`Side::limb`].
    Limbs(&'static str, &'static str),
}

/// Mario's 29 joints (and Piantissimo's, on the same rig), and Yoshi's 38: `jnt_hand_*` under
/// `jnt_arm_*2`.
pub const MARIO: Rig = Rig::Hands("jnt_hand_R", "jnt_hand_L");
/// The Pianta (`mom_model.bmd`, `mow_model.bmd`): `hand*_jnt` under `arm*B_jnt`.
pub const PIANTA: Rig = Rig::Hands("handR_jnt", "handL_jnt");
/// Pikmin 2's Louie (`orima3.bmd`): `rhandjnt` and `lhandjnt` straight off the spine.
pub const LOUIE: Rig = Rig::Limbs("rhandjnt", "lhandjnt");

/// The sides of `rig` on `bmd` (right, left).
pub fn sides(bmd: &Bmd, mesh: &Mesh, rig: Rig) -> [Option<Side>; 2] {
    match rig {
        Rig::Hands(r, l) => [r, l].map(|n| bmd.joint(n).and_then(|w| Side::from_wrist(mesh, w))),
        Rig::Limbs(r, l) => [r, l].map(|n| bmd.joint(n).map(Side::limb)),
    }
}

/// A J3D body's cut from the cache (cut and cached first if it isn't there): the body, laid out
/// as the views draw it, and its arms.
pub fn load_or_cut(assets: &Path, name: &str, bmd: &Bmd, batches: &[Batch], rig: Rig) -> (Mesh, Arms, bool) {
    let m = mesh(bmd, batches);
    let (arms, made) = Arms::load_or_cut(assets, name, &m, sides(bmd, &m, rig));
    (m, arms, made)
}
