//! Virtual camera: choose which [`Camera`](crate::transform::Camera) the player
//! sees through, and blend between cameras for cinematic transitions.
//!
//! Tagging a camera entity with [`MainCamera`] marks it as a candidate for the
//! active shot; when several are tagged, the highest [`MainCamera::priority`]
//! wins. A [`CameraRig`] smoothly blends between a "from" and "to" shot so
//! cutscenes can dolly between viewpoints without a hard cut. The resolved shot
//! is published as an [`ActiveCamera`] resource the renderer reads each frame.

use crate::component::Component;
use crate::math::{Mat4, Quat, Vec3};
use crate::transform::{Camera, GlobalTransform};
use crate::World;

/// Marks a camera entity as a candidate for the active shot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MainCamera {
    /// Higher priority cameras win the shot. A cutscene camera at priority 10
    /// beats the gameplay camera at 0; tie-break is entity index (lowest wins).
    pub priority: i32,
}

impl Component for MainCamera {}

/// A fully-resolved camera shot: the pose plus projection the renderer uses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VirtualCamera {
    pub translation: Vec3,
    pub rotation: Quat,
    pub fov_y: f32,
    pub aspect: f32,
    pub near: f32,
    pub far: f32,
}

impl Default for VirtualCamera {
    fn default() -> Self {
        let d = Camera::default();
        VirtualCamera {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            fov_y: d.fov_y,
            aspect: d.aspect,
            near: d.near,
            far: d.far,
        }
    }
}

impl VirtualCamera {
    pub fn from_world(translation: Vec3, rotation: Quat, cam: Camera) -> Self {
        VirtualCamera {
            translation,
            rotation,
            fov_y: cam.fov_y,
            aspect: cam.aspect,
            near: cam.near,
            far: cam.far,
        }
    }

    /// The camera's world-space view matrix (right-handed, looks down -Z).
    pub fn view(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(Vec3::ONE, self.rotation, self.translation).inverse()
    }

    pub fn proj(&self) -> Mat4 {
        Mat4::perspective_rh(self.fov_y, self.aspect, self.near, self.far)
    }

    pub fn view_proj(&self) -> Mat4 {
        self.proj() * self.view()
    }

    pub fn with_aspect(mut self, aspect: f32) -> Self {
        self.aspect = aspect;
        self
    }
}

/// The resolved camera shot the renderer uses this frame.
#[derive(Debug, Clone, Copy, Default)]
pub struct ActiveCamera(pub VirtualCamera);

/// Drives a smooth transition between two camera shots (the "virtual camera").
/// `weight` runs `0 -> 1` from `from` to `to`, eased with a smoothstep.
#[derive(Debug, Clone, Copy)]
pub struct CameraRig {
    pub from: VirtualCamera,
    pub to: VirtualCamera,
    weight: f32,
}

impl CameraRig {
    pub fn new(to: VirtualCamera) -> Self {
        CameraRig {
            from: to,
            to,
            weight: 1.0,
        }
    }

    /// Begin a transition to `next`, starting from `current`.
    pub fn begin_transition(&mut self, current: VirtualCamera, next: VirtualCamera) {
        self.from = current;
        self.to = next;
        self.weight = 0.0;
    }

    /// Advance the blend by `dt` seconds over a `duration`-second transition.
    pub fn advance(&mut self, dt: f32, duration: f32) {
        if duration <= 0.0 {
            self.weight = 1.0;
            return;
        }
        self.weight = (self.weight + dt / duration).clamp(0.0, 1.0);
    }

    /// The eased, current shot.
    pub fn resolve(&self) -> VirtualCamera {
        blend_cameras(&self.from, &self.to, smoothstep(self.weight))
    }

    pub fn is_complete(&self) -> bool {
        self.weight >= 1.0
    }
}

/// Linearly (and spherically, for rotation) interpolate two camera shots.
pub fn blend_cameras(a: &VirtualCamera, b: &VirtualCamera, t: f32) -> VirtualCamera {
    let t = t.clamp(0.0, 1.0);
    VirtualCamera {
        translation: a.translation.lerp(b.translation, t),
        rotation: a.rotation.slerp(b.rotation, t),
        fov_y: a.fov_y + (b.fov_y - a.fov_y) * t,
        aspect: a.aspect + (b.aspect - a.aspect) * t,
        near: a.near + (b.near - a.near) * t,
        far: a.far + (b.far - a.far) * t,
    }
}

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Find the camera shot for the highest-priority [`MainCamera`] entity, or
/// `None` if no camera is tagged. `aspect` overrides the camera's stored
/// aspect ratio to match the render target.
pub fn pick_main_camera(world: &World, aspect: f32) -> Option<VirtualCamera> {
    let mut best: Option<(i32, u32, VirtualCamera)> = None;
    for (e, cam, gt, main) in world.query_3::<Camera, GlobalTransform, MainCamera>() {
        let (_, rotation, _) = gt.0.to_scale_rotation_translation();
        let shot = VirtualCamera::from_world(gt.translation(), rotation, *cam).with_aspect(aspect);
        let key = (main.priority, e.index);
        match best {
            Some((bp, bi, _)) if (bp, bi) >= (key.0, key.1) => {}
            _ => best = Some((key.0, key.1, shot)),
        }
    }
    best.map(|(_, _, shot)| shot)
}

fn cameras_close(a: &VirtualCamera, b: &VirtualCamera) -> bool {
    const E: f32 = 1e-3;
    a.translation.distance_squared(b.translation) < E * E
        && a.rotation.dot(b.rotation).abs() > 0.9999
        && (a.fov_y - b.fov_y).abs() < E
        && (a.aspect - b.aspect).abs() < E
        && (a.near - b.near).abs() < E
        && (a.far - b.far).abs() < E
}

/// Recompute the active camera from the world and the rig, publishing the
/// result into the [`ActiveCamera`] resource and returning it.
///
/// When the highest-priority camera changes and no transition is in flight, a
/// new eased transition is started. Call this once per frame before rendering.
pub fn update_active_camera(
    world: &mut World,
    rig: &mut CameraRig,
    dt: f32,
    aspect: f32,
    transition_duration: f32,
) -> VirtualCamera {
    let target = pick_main_camera(world, aspect).unwrap_or_default();
    if rig.is_complete() && !cameras_close(&rig.to, &target) {
        rig.begin_transition(rig.resolve(), target);
    }
    rig.advance(dt, transition_duration);
    let active = rig.resolve();
    world.add_resource(ActiveCamera(active));
    active
}
