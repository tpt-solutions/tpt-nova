//! 3D viewport gizmos: drag a selected entity in world space using a screen
//! ray. The math is camera-relative and fully unit-testable — no GPU code.
//!
//! The flow for a move gizmo is:
//! 1. Build a world-space ray from the pointer's screen position.
//! 2. Intersect that ray with a plane facing the camera, through the object.
//! 3. The world-space delta between the drag start and current points is added
//!    to the entity's translation (optionally snapped to a grid).

use glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use nova_ecs::transform::Transform;
use nova_ecs::{Entity, World};

/// Which 3D transform a gizmo applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GizmoMode3D {
    #[default]
    Move,
    Rotate,
    Scale,
}

/// A world-space ray (origin + unit direction).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    pub dir: Vec3,
}

/// Build a world-space ray from a screen pixel position. `viewport` is in
/// pixels (width, height); `screen` is also in pixels with y pointing down.
/// `inv_view_proj` is the inverse of the camera's `proj * view` matrix.
pub fn screen_to_ray(inv_view_proj: Mat4, viewport: (f32, f32), screen: Vec2) -> Ray {
    let ndc_x = (screen.x / viewport.0) * 2.0 - 1.0;
    let ndc_y = 1.0 - (screen.y / viewport.1) * 2.0; // flip y to NDC
    let near = inv_view_proj * Vec4::new(ndc_x, ndc_y, 0.0, 1.0);
    let far = inv_view_proj * Vec4::new(ndc_x, ndc_y, 1.0, 1.0);
    let near = near.truncate() / near.w;
    let far = far.truncate() / far.w;
    Ray {
        origin: near,
        dir: (far - near).normalize(),
    }
}

/// Intersect a ray with an infinite plane (`plane_point` on the plane, `normal`
/// unit). Returns `None` for a parallel or behind-origin ray.
pub fn ray_plane(ray: Ray, plane_point: Vec3, normal: Vec3) -> Option<Vec3> {
    let denom = ray.dir.dot(normal);
    if denom.abs() < 1e-6 {
        return None;
    }
    let t = (plane_point - ray.origin).dot(normal) / denom;
    if t < 0.0 {
        return None;
    }
    Some(ray.origin + ray.dir * t)
}

/// World-space point under the pointer on a camera-facing plane through
/// `anchor`. Handy both for starting a drag and tracking it.
pub fn drag_plane_point(
    inv_view_proj: Mat4,
    viewport: (f32, f32),
    camera_forward: Vec3,
    anchor: Vec3,
    screen: Vec2,
) -> Option<Vec3> {
    let ray = screen_to_ray(inv_view_proj, viewport, screen);
    ray_plane(ray, anchor, camera_forward)
}

fn snap_value(v: f32, step: f32) -> f32 {
    if step > 0.0 {
        (v / step).round() * step
    } else {
        v
    }
}

/// Apply a pointer drag (screen pixels) to the selected entity's `Transform`
/// using the active camera. `camera_forward` is the camera's view direction
/// (used as the drag-plane normal for Move and the rotation axis for Rotate).
/// Returns true if the transform changed.
#[allow(clippy::too_many_arguments)]
pub fn apply_gizmo_3d(
    world: &mut World,
    entity: Entity,
    mode: GizmoMode3D,
    inv_view_proj: Mat4,
    viewport: (f32, f32),
    camera_forward: Vec3,
    start: Vec2,
    current: Vec2,
    grid: f32,
) -> bool {
    const ROTATE_SENSITIVITY: f32 = 0.01;
    const SCALE_SENSITIVITY: f32 = 0.005;

    let t = match world.get_component_mut::<Transform>(entity) {
        Some(t) => t,
        None => return false,
    };

    match mode {
        GizmoMode3D::Move => {
            let start_pt = match drag_plane_point(
                inv_view_proj,
                viewport,
                camera_forward,
                t.translation,
                start,
            ) {
                Some(p) => p,
                None => return false,
            };
            let cur_pt = match drag_plane_point(
                inv_view_proj,
                viewport,
                camera_forward,
                t.translation,
                current,
            ) {
                Some(p) => p,
                None => return false,
            };
            let delta = cur_pt - start_pt;
            t.translation.x = snap_value(t.translation.x + delta.x, grid);
            t.translation.y = snap_value(t.translation.y + delta.y, grid);
            t.translation.z = snap_value(t.translation.z + delta.z, grid);
        }
        GizmoMode3D::Rotate => {
            let angle = (current.y - start.y) * ROTATE_SENSITIVITY;
            let q = Quat::from_axis_angle(camera_forward.normalize(), angle);
            // Premultiply so the rotation is about the camera axis in world space.
            t.rotation = q * t.rotation;
        }
        GizmoMode3D::Scale => {
            let factor = 1.0 + (current.y - start.y) * SCALE_SENSITIVITY;
            let factor = factor.max(0.01);
            t.scale = Vec3::new(
                snap_value(t.scale.x * factor, grid),
                snap_value(t.scale.y * factor, grid),
                snap_value(t.scale.z * factor, grid),
            );
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use nova_ecs::math::Mat4;

    /// A camera at (0,0,5) looking at the origin down -Z, square viewport.
    fn test_camera() -> (Mat4, Vec3, (f32, f32)) {
        let translation = Vec3::new(0.0, 0.0, 5.0);
        let rotation = Quat::IDENTITY;
        let view =
            Mat4::from_scale_rotation_translation(Vec3::ONE, rotation, translation).inverse();
        let proj = Mat4::perspective_rh(60f32.to_radians(), 1.0, 0.01, 100.0);
        let forward = rotation * Vec3::NEG_Z;
        (proj * view, forward, (800.0, 800.0))
    }

    #[test]
    fn drag_right_moves_object_along_world_x() {
        let (vp, fwd, viewport) = test_camera();
        let inv = vp.inverse();
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::from_translation(Vec3::ZERO));

        // Start at center, drag 100px to the right.
        let start = Vec2::new(400.0, 400.0);
        let current = Vec2::new(500.0, 400.0);
        let changed = apply_gizmo_3d(
            &mut world,
            e,
            GizmoMode3D::Move,
            inv,
            viewport,
            fwd,
            start,
            current,
            0.0,
        );
        assert!(changed);
        let t = world.get_component::<Transform>(e).unwrap();
        assert!(
            t.translation.x > 0.0,
            "expected +x, got {}",
            t.translation.x
        );
        assert!(
            t.translation.y.abs() < 1e-3,
            "y should be ~0, got {}",
            t.translation.y
        );
    }

    #[test]
    fn no_drag_means_no_change() {
        let (vp, fwd, viewport) = test_camera();
        let inv = vp.inverse();
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::from_translation(Vec3::new(1.0, 2.0, 3.0)));
        let s = Vec2::new(300.0, 300.0);
        apply_gizmo_3d(
            &mut world,
            e,
            GizmoMode3D::Move,
            inv,
            viewport,
            fwd,
            s,
            s,
            0.0,
        );
        let t = world.get_component::<Transform>(e).unwrap();
        assert_eq!(t.translation, Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn grid_snap_on_move() {
        let (vp, fwd, viewport) = test_camera();
        let inv = vp.inverse();
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::from_translation(Vec3::ZERO));
        let start = Vec2::new(400.0, 400.0);
        let current = Vec2::new(460.0, 420.0);
        apply_gizmo_3d(
            &mut world,
            e,
            GizmoMode3D::Move,
            inv,
            viewport,
            fwd,
            start,
            current,
            0.5,
        );
        let t = world.get_component::<Transform>(e).unwrap();
        assert!(
            (t.translation.x / 0.5).round() == (t.translation.x / 0.5),
            "x should be grid-aligned"
        );
        assert!(
            (t.translation.x * 2.0).fract().abs() < 1e-4,
            "x must be multiple of 0.5, got {}",
            t.translation.x
        );
    }

    #[test]
    fn rotate_changes_orientation() {
        let (vp, fwd, viewport) = test_camera();
        let inv = vp.inverse();
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::default());
        let s = Vec2::new(400.0, 400.0);
        let cur = Vec2::new(400.0, 300.0); // drag up 100px
        apply_gizmo_3d(
            &mut world,
            e,
            GizmoMode3D::Rotate,
            inv,
            viewport,
            fwd,
            s,
            cur,
            0.0,
        );
        let t = world.get_component::<Transform>(e).unwrap();
        assert!(
            t.rotation != Quat::IDENTITY,
            "expected a non-identity rotation"
        );
    }

    #[test]
    fn ray_plane_basic() {
        let ray = Ray {
            origin: Vec3::new(0.0, 0.0, 5.0),
            dir: Vec3::new(0.0, 0.0, -1.0),
        };
        let p = ray_plane(ray, Vec3::ZERO, Vec3::NEG_Z).unwrap();
        assert!(p.abs_diff_eq(Vec3::ZERO, 1e-5));
    }
}
