//! Collision proxy generation for splat clouds.
//!
//! A captured Gaussian Splat scene is hundreds of thousands of anisotropic
//! Gaussians — far too many to simulate individually. The pragmatic proxy is a
//! single **low-poly convex hull** wrapping every splat center: one [`ConvexPart`]
//! fed into `nova-ingest`'s [`Collider3D`], which the existing Rapier3D step
//! (`nova_ingest::step_physics_3d`) already consumes as a compound hull. This
//! gives captured environments solid, stable collision at a fraction of the
//! cost, at the expense of concavities (acceptable for a first-pass proxy).

use nova_ecs::transform::Transform;
use nova_ecs::World;
use nova_ingest::decompose::ConvexPart;
use nova_ingest::Collider3D;
use parry3d::math::Vector;
use parry3d::transformation::try_convex_hull;

use crate::{SplatCloud, SplatError};

/// Build a single convex-hull [`Collider3D`] enclosing every splat center.
///
/// Returns an error if the cloud has fewer than four non-coplanar points (a
/// valid hull needs at least a tetrahedron of distinct points).
pub fn build_convex_hull_collider(cloud: &SplatCloud) -> Result<Collider3D, SplatError> {
    if cloud.is_empty() {
        return Err(SplatError::EmptyCloud);
    }
    let points: Vec<Vector> = cloud
        .splats
        .iter()
        .map(|s| Vector::new(s.position[0], s.position[1], s.position[2]))
        .collect();

    // Returns the triangulated hull (`Vec<[u32; 3]>`), exactly what `ConvexPart`
    // expects: the hull vertices and their triangle indices. `try_convex_hull`
    // avoids panicking on degenerate input.
    let (hull_pts, indices) = match try_convex_hull(&points) {
        Ok(h) => h,
        Err(_) => {
            return Err(SplatError::MalformedSplat(
                "splat cloud is degenerate (cannot form a 3D hull)".into(),
            ))
        }
    };
    if hull_pts.len() < 4 || indices.is_empty() {
        return Err(SplatError::MalformedSplat(
            "splat cloud is degenerate (cannot form a 3D hull)".into(),
        ));
    }

    let vertices: Vec<[f32; 3]> = hull_pts.iter().map(|p| [p.x, p.y, p.z]).collect();
    let part = ConvexPart { vertices, indices };
    Ok(Collider3D::from_parts(vec![part]))
}

/// Attach the convex-hull collider to an existing entity that owns `cloud`.
///
/// The entity is also given a [`Transform`] (identity unless already present)
/// so the physics step can place the rigid body. Returns the same entity for
/// chaining.
pub fn attach_hull_collider(
    world: &mut World,
    entity: nova_ecs::Entity,
    cloud: &SplatCloud,
) -> Result<nova_ecs::Entity, SplatError> {
    let collider = build_convex_hull_collider(cloud)?;
    if world.get_component::<Transform>(entity).is_none() {
        world.add_component(entity, Transform::default());
    }
    world.add_component(entity, collider);
    Ok(entity)
}

/// Count the triangles in the generated hull (useful for asserting the proxy
/// really is "low-poly").
pub fn hull_triangle_count(cloud: &SplatCloud) -> Result<usize, SplatError> {
    Ok(build_convex_hull_collider(cloud)?.parts[0].indices.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nova_ecs::World;

    fn tetra_cloud() -> SplatCloud {
        SplatCloud::new(vec![
            splat([0.0, 0.0, 0.0]),
            splat([1.0, 0.0, 0.0]),
            splat([0.0, 1.0, 0.0]),
            splat([0.0, 0.0, 1.0]),
        ])
    }

    fn splat(p: [f32; 3]) -> crate::Splat {
        crate::Splat {
            position: p,
            scale: [0.01; 3],
            rotation: [1.0, 0.0, 0.0, 0.0],
            color: [1.0; 4],
            opacity: 1.0,
        }
    }

    #[test]
    fn hull_collider_wraps_all_centers() {
        let collider = build_convex_hull_collider(&tetra_cloud()).unwrap();
        assert_eq!(collider.parts.len(), 1);
        // A tetrahedron's convex hull is the 4 corners and 4 triangles.
        assert_eq!(collider.parts[0].vertices.len(), 4);
        assert_eq!(collider.parts[0].indices.len(), 4);
    }

    #[test]
    fn empty_cloud_is_rejected() {
        assert!(matches!(
            build_convex_hull_collider(&SplatCloud::default()),
            Err(SplatError::EmptyCloud)
        ));
    }

    #[test]
    fn coplanar_cloud_is_rejected() {
        // All points coincide: no 3D hull can be formed.
        let flat = SplatCloud::new(vec![
            splat([1.0, 0.0, 0.0]),
            splat([1.0, 0.0, 0.0]),
            splat([1.0, 0.0, 0.0]),
            splat([1.0, 0.0, 0.0]),
        ]);
        assert!(build_convex_hull_collider(&flat).is_err());
    }

    #[test]
    fn attach_wires_collider_into_world() {
        let mut world = World::new();
        let e = world.spawn();
        let cloud = tetra_cloud();
        let e2 = attach_hull_collider(&mut world, e, &cloud).unwrap();
        assert_eq!(e, e2);
        let col = world.get_component::<Collider3D>(e).unwrap();
        assert_eq!(col.parts.len(), 1);
        assert!(world.has_component::<Transform>(e));
    }

    #[test]
    fn hull_is_low_poly() {
        // A dense grid of points still collapses to a single bounding hull with
        // a bounded, small triangle count.
        let mut splats = Vec::new();
        for x in 0..5 {
            for y in 0..5 {
                for z in 0..5 {
                    splats.push(splat([x as f32, y as f32, z as f32]));
                }
            }
        }
        let count = hull_triangle_count(&SplatCloud::new(splats)).unwrap();
        assert!(
            count > 0 && count < 1000,
            "hull should stay low-poly, got {count}"
        );
    }
}
