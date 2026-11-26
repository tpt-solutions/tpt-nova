//! Pointer picking (Phase 10, raycasting): a screen pixel becomes a world ray, and
//! the ray is tested against the visible cube nodes in the scene.
//!
//! The spec recommends a lightweight AABB raycaster for UI interaction, with
//! complex collision queries left to `tpt-nova-physics`. This module is the
//! first kind. Each cube is tested in its own local space, so rotation and
//! non-uniform scale are handled exactly. The hit is mapped back to world space,
//! and distances are measured there.
//!
//! Conventions: pixels have their origin at the top-left with y pointing down.
//! Depth follows wgpu's 0..1 range, which matches `PerspectiveCamera::view_proj`.

use glam::{Mat4, Vec2, Vec3};

use crate::{NodeId, Scene};

/// A world-space ray. `direction` is unit length when it comes from [`screen_ray`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    pub direction: Vec3,
}

/// An axis-aligned box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    /// The cube the renderer builds: side 1, centred on the origin.
    pub const UNIT_CUBE: Aabb = Aabb {
        min: Vec3::splat(-0.5),
        max: Vec3::splat(0.5),
    };
}

/// Ray parameter `t >= 0` of the first hit with `aabb`, or `None`. A ray that starts
/// inside the box returns `Some(0.0)`.
pub fn ray_aabb(ray: &Ray, aabb: &Aabb) -> Option<f32> {
    let mut t_min = f32::NEG_INFINITY;
    let mut t_max = f32::INFINITY;
    for axis in 0..3 {
        let o = ray.origin[axis];
        let d = ray.direction[axis];
        let (lo, hi) = (aabb.min[axis], aabb.max[axis]);
        if d.abs() < 1e-12 {
            // Parallel to this slab: miss unless the origin lies within it.
            if o < lo || o > hi {
                return None;
            }
            continue;
        }
        let inv = 1.0 / d;
        let (t0, t1) = ((lo - o) * inv, (hi - o) * inv);
        t_min = t_min.max(t0.min(t1));
        t_max = t_max.min(t0.max(t1));
        if t_min > t_max {
            return None;
        }
    }
    if t_max < 0.0 {
        return None;
    }
    Some(t_min.max(0.0))
}

/// Unproject a pixel to a world ray through `view_proj` (world to clip).
/// Returns `None` for an empty viewport or a non-invertible matrix.
pub fn screen_ray(view_proj: Mat4, viewport: Vec2, pixel: Vec2) -> Option<Ray> {
    if viewport.x <= 0.0 || viewport.y <= 0.0 {
        return None;
    }
    let det = view_proj.determinant();
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let inv = view_proj.inverse();
    let ndc_x = pixel.x / viewport.x * 2.0 - 1.0;
    let ndc_y = 1.0 - pixel.y / viewport.y * 2.0;
    let near = inv.project_point3(Vec3::new(ndc_x, ndc_y, 0.0));
    let far = inv.project_point3(Vec3::new(ndc_x, ndc_y, 1.0));
    let direction = (far - near).try_normalize()?;
    Some(Ray {
        origin: near,
        direction,
    })
}

/// The nearest cube hit under a pixel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pick {
    pub node: NodeId,
    /// Distance from the ray origin (the near plane) to the hit point, in world units.
    pub distance: f32,
    pub point: Vec3,
}

/// Pick the nearest visible mesh node under `pixel`, as seen through `camera`.
///
/// Only mesh nodes are tested. Hidden nodes (`visible_in_tree` false) are skipped.
/// Returns `None` if the camera is missing or nothing is under the pixel.
pub fn pick(scene: &Scene, camera: NodeId, viewport: Vec2, pixel: Vec2) -> Option<Pick> {
    let cam = scene.camera(camera)?;
    let cam_world = scene.world_matrix(camera)?;
    let aspect = if viewport.y > 0.0 {
        Some(viewport.x / viewport.y)
    } else {
        None
    };
    let ray = screen_ray(cam.view_proj(cam_world, aspect), viewport, pixel)?;

    let mut best: Option<Pick> = None;
    for (id, node) in scene.iter() {
        if !node.kind.is_mesh() || !scene.visible_in_tree(id) {
            continue;
        }
        let Some(world) = scene.world_matrix(id) else {
            continue;
        };
        let Some(hit) = pick_node(&ray, world) else {
            continue;
        };
        if best.is_none_or(|b| hit.1 < b.distance) {
            best = Some(Pick {
                node: id,
                distance: hit.1,
                point: hit.0,
            });
        }
    }
    best
}

/// Test one cube node: move the ray into local space, hit the unit cube, and map
/// the hit back to world space. Returns the world hit point and its distance.
fn pick_node(ray: &Ray, world: Mat4) -> Option<(Vec3, f32)> {
    let det = world.determinant();
    if !det.is_finite() || det.abs() < 1e-12 {
        return None; // a flattened node has no pickable volume
    }
    let inv = world.inverse();
    let local = Ray {
        origin: inv.transform_point3(ray.origin),
        direction: inv.transform_vector3(ray.direction),
    };
    let t = ray_aabb(&local, &Aabb::UNIT_CUBE)?;
    let hit_local = local.origin + local.direction * t;
    let point = world.transform_point3(hit_local);
    Some((point, (point - ray.origin).length()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Node, PerspectiveCamera, Transform};

    const VIEW: Vec2 = Vec2::new(800.0, 600.0);

    /// Scene with a camera at `eye` looking at the origin. Returns the scene and camera id.
    fn scene_with_camera(eye: Vec3) -> (Scene, NodeId) {
        let mut scene = Scene::new();
        let cam = scene.add_node(
            Node::camera(PerspectiveCamera::default())
                .with_transform(Transform::from_position(eye).looking_at(Vec3::ZERO)),
        );
        scene.set_active_camera(Some(cam)).unwrap();
        (scene, cam)
    }

    /// The pixel where `point` lands on screen for `camera`.
    fn pixel_of(scene: &Scene, camera: NodeId, point: Vec3) -> Vec2 {
        let cam = scene.camera(camera).unwrap();
        let vp = cam.view_proj(scene.world_matrix(camera).unwrap(), Some(VIEW.x / VIEW.y));
        let clip = vp * point.extend(1.0);
        let ndc = clip.truncate() / clip.w;
        Vec2::new((ndc.x + 1.0) * 0.5 * VIEW.x, (1.0 - ndc.y) * 0.5 * VIEW.y)
    }

    #[test]
    fn ray_through_the_centre_hits_a_cube_at_the_origin() {
        let (mut scene, cam) = scene_with_camera(Vec3::new(0.0, 0.0, 5.0));
        let cube = scene.add_node(Node::cube());
        let hit = pick(&scene, cam, VIEW, VIEW * 0.5).expect("centre hits");
        assert_eq!(hit.node, cube);
        // The camera sits at z=5 and the near face is at z=0.5, so the true distance
        // from the camera is 4.5. The ray starts on the near plane, so add it back.
        let near = scene.camera(cam).unwrap().near;
        assert!(
            (hit.distance + near - 4.5).abs() < 0.01,
            "distance {} near {near}",
            hit.distance
        );
    }

    #[test]
    fn a_ray_that_misses_returns_nothing() {
        let (mut scene, cam) = scene_with_camera(Vec3::new(0.0, 0.0, 5.0));
        scene.add_node(Node::cube());
        let corner = Vec2::new(5.0, 5.0);
        assert!(pick(&scene, cam, VIEW, corner).is_none());
    }

    #[test]
    fn the_nearest_of_two_cubes_is_picked() {
        let (mut scene, cam) = scene_with_camera(Vec3::new(0.0, 0.0, 10.0));
        let far = scene.add_node(Node::cube().at(Vec3::new(0.0, 0.0, -3.0)));
        let near = scene.add_node(Node::cube().at(Vec3::new(0.0, 0.0, 2.0)));
        let hit = pick(&scene, cam, VIEW, VIEW * 0.5).unwrap();
        assert_eq!(hit.node, near);
        assert_ne!(hit.node, far);
    }

    #[test]
    fn hidden_nodes_are_not_picked() {
        let (mut scene, cam) = scene_with_camera(Vec3::new(0.0, 0.0, 5.0));
        scene.add_node(Node::cube().hidden());
        assert!(pick(&scene, cam, VIEW, VIEW * 0.5).is_none());
    }

    #[test]
    fn scale_and_position_are_honoured_in_the_hit() {
        // Cube at x=3, scaled 2x: it spans x in [2, 4]. Aim at x = 2.5.
        let (mut scene, cam) = scene_with_camera(Vec3::new(0.0, 0.0, 10.0));
        let cube = scene.add_node(
            Node::cube()
                .at(Vec3::new(3.0, 0.0, 0.0))
                .scaled(Vec3::splat(2.0)),
        );
        let px = pixel_of(&scene, cam, Vec3::new(2.5, 0.0, 0.0));
        let hit = pick(&scene, cam, VIEW, px).expect("inside the scaled cube");
        assert_eq!(hit.node, cube);
        // The ray through world x=2.5 at z=0 enters the front face (z=1) at
        // x = 2.5 * (10 - 1) / 10 = 2.25, so that is where it hits.
        assert!((hit.point.x - 2.25).abs() < 0.01, "x {}", hit.point.x);
        // And a point at x = 1.5 is outside it.
        let outside = pixel_of(&scene, cam, Vec3::new(1.5, 0.0, 0.0));
        assert!(pick(&scene, cam, VIEW, outside).is_none());
    }

    #[test]
    fn rotation_is_honoured() {
        // A cube rotated 45 degrees about Y is wider along X than an unrotated one.
        let (mut scene, cam) = scene_with_camera(Vec3::new(0.0, 0.0, 10.0));
        let cube = scene.add_node(
            Node::cube().rotated(glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_4)),
        );
        // Corner of the rotated cube along +X is at distance sqrt(2)/2 ~ 0.707.
        let px = pixel_of(&scene, cam, Vec3::new(0.65, 0.0, 0.0));
        let hit = pick(&scene, cam, VIEW, px).expect("rotated cube reaches x=0.65");
        assert_eq!(hit.node, cube);
    }

    #[test]
    fn ray_aabb_handles_inside_parallel_and_behind() {
        let aabb = Aabb::UNIT_CUBE;
        let inside = Ray {
            origin: Vec3::ZERO,
            direction: Vec3::Z,
        };
        assert_eq!(ray_aabb(&inside, &aabb), Some(0.0));
        let behind = Ray {
            origin: Vec3::new(0.0, 0.0, 5.0),
            direction: Vec3::Z,
        };
        assert_eq!(ray_aabb(&behind, &aabb), None);
        let parallel_outside = Ray {
            origin: Vec3::new(2.0, 0.0, -5.0),
            direction: Vec3::Z,
        };
        assert_eq!(ray_aabb(&parallel_outside, &aabb), None);
    }

    #[test]
    fn degenerate_viewports_and_matrices_return_none() {
        assert!(screen_ray(Mat4::IDENTITY, Vec2::ZERO, Vec2::ZERO).is_none());
        assert!(screen_ray(Mat4::ZERO, VIEW, Vec2::ZERO).is_none());
    }
}
