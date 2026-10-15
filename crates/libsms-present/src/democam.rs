//! The Shine Get demo camera: the path the game flies the camera along when Mario gets a Shine
//! Sprite (`TCameraBck`, Camera/CameraBck.cpp; `fireGetStar`, MarDirectorEvent.cpp).
//!
//! The path is a joint animation (`/common/camera/camera_demo_shine_get_outside.bck`, or
//! `..._inside.bck` for a Shine under a roof) on a rig of two joints, `cam_position` and
//! `cam_interest` (`camera_model.bmd`). Each frame the camera's eye is where the first is, it
//! looks at the second, its up is the first's Y axis, and its vertical field of view in degrees is
//! the first's Y scale (`TCameraBck::updateDemo`). Everything is relative to the origin the demo
//! was started with, Mario's feet, and turned about it by the Shine's yaw
//! (`CPolarSubCamera::updateDemoCamera_`).
//!
//! The clips are authored at 60 frames a second, like the rest of the game's (`SMSGetAnmFrameRate`
//! is 2 at the 30 the game runs at), so the 420 frames of the path last seven seconds.

use std::path::Path;

use glam::{Mat4, Vec3};
use libsms::{Clip, Skeleton};

/// Clip frames a second.
pub const FPS: f32 = 60.0;

/// Where the camera is at one moment of the demo, in sim units, with the demo's origin at zero
/// and its yaw not yet applied.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shot {
    pub eye: Vec3,
    pub at: Vec3,
    /// Unit up vector.
    pub up: Vec3,
    /// Vertical field of view, degrees.
    pub fov: f32,
}

pub struct DemoCamera {
    rig: Skeleton,
    position: usize,
    interest: usize,
    /// `outside`, `inside`.
    clips: [Clip; 2],
}

impl DemoCamera {
    /// Loads the rig and both Shine Get paths from the extracted `assets/` folder; None without
    /// them (an older extraction).
    pub fn load(dir: &Path) -> Option<DemoCamera> {
        let rig = Skeleton::from_bmd(&std::fs::read(dir.join("camera/camera_model.bmd")).ok()?)?;
        let clip = |name: &str| {
            let data = std::fs::read(dir.join(format!("camera/camera_demo_shine_get_{name}.bck"))).ok()?;
            Clip::parse(&data)
        };
        Some(DemoCamera {
            position: rig.find("cam_position")?,
            interest: rig.find("cam_interest")?,
            clips: [clip("outside")?, clip("inside")?],
            rig,
        })
    }

    /// The path's length in clip frames.
    pub fn frames(&self, inside: bool) -> f32 {
        self.clips[inside as usize].frames
    }

    /// The camera at clip frame `frame` (clamped to the path).
    pub fn shot(&self, inside: bool, frame: f32) -> Shot {
        let clip = &self.clips[inside as usize];
        let frame = frame.clamp(0.0, clip.frames);
        let pose: Vec<Mat4> = self.rig.pose(clip, frame);
        let fov = clip.sample(self.position, frame).map_or(50.0, |s| s.scale.y);
        Shot {
            eye: pose[self.position].w_axis.truncate(),
            at: pose[self.interest].w_axis.truncate(),
            up: pose[self.position].y_axis.truncate().normalize_or(Vec3::Y),
            fov: if fov.is_finite() && fov > 1.0 { fov.min(150.0) } else { 50.0 },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> Option<DemoCamera> {
        DemoCamera::load(&libsms::assets::find(None)?)
    }

    #[test]
    fn the_outside_path_starts_close_under_the_shine_and_pulls_away() {
        let Some(c) = camera() else {
            eprintln!("skipped: needs assets/ with the Shine Get camera");
            return;
        };
        assert!(c.frames(false) > 400.0);
        let (a, b) = (c.shot(false, 0.0), c.shot(false, 300.0));
        // A few metres from Mario at first, looking up to where he holds the Shine.
        assert!(a.eye.length() < 400.0, "{:?}", a.eye);
        assert!(a.at.y > 200.0, "looks up at the Shine: {:?}", a.at);
        assert!(b.eye.length() > a.eye.length(), "pulls away");
        assert!((a.up.length() - 1.0).abs() < 1e-3);
        assert!(a.fov > 20.0 && a.fov < 90.0, "fov {}", a.fov);
    }

    #[test]
    fn the_inside_path_keeps_closer_to_a_ceiling() {
        let Some(c) = camera() else { return };
        let (out, inside) = (c.shot(false, 400.0), c.shot(true, 400.0));
        assert!(inside.eye.length() < out.eye.length(), "{:?} {:?}", inside.eye, out.eye);
    }
}
