//! Rapier3D physics integration for ingested meshes.
//!
//! Mirrors the 2D design in `nova-physics`: gameplay code only touches plain
//! [`RigidBody3D`] / [`Collider3D`] components and the entity [`Transform`];
//! all Rapier state lives in the [`PhysicsWorld3D`] resource. Each fixed tick,
//! [`step_physics_3d`] reconciles bodies, pushes kinematic velocities, advances
//! the simulation, and reads transforms back into the ECS.
//!
//! A [`Collider3D`] stores the VHACD [`ConvexPart`]s produced by the ingestion
//! pipeline; the step builds a Rapier compound shape (one convex hull per part)
//! so a concave ingested mesh still collides correctly.
//!
//! Note: rapier3d 0.34 is built on `glam` (via `glamx`), so all math types here
//! are `glam` types re-exported through `rapier3d::math`.

use std::collections::{HashMap, HashSet};

use glam::Vec3;
use nova_ecs::transform::Transform;
use nova_ecs::{component::Component, Entity, World};
use rapier3d::geometry::{ColliderBuilder, SharedShape};
use rapier3d::math::{Pose3, Vector};
use rapier3d::prelude::*;
use serde::{Deserialize, Serialize};

use crate::decompose::ConvexPart;

/// How a 3D body participates in the simulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BodyKind3D {
    #[default]
    Dynamic,
    Fixed,
    KinematicVelocity,
}

/// A rigid body attached to an entity.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RigidBody3D {
    pub kind: BodyKind3D,
    pub linvel: Vec3,
    pub angvel: Vec3,
    pub gravity_scale: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub lock_rotations: bool,
}

impl Default for RigidBody3D {
    fn default() -> Self {
        RigidBody3D {
            kind: BodyKind3D::Dynamic,
            linvel: Vec3::ZERO,
            angvel: Vec3::ZERO,
            gravity_scale: 1.0,
            linear_damping: 0.0,
            angular_damping: 0.0,
            lock_rotations: false,
        }
    }
}

impl RigidBody3D {
    pub fn dynamic() -> Self {
        Self::default()
    }
    pub fn fixed() -> Self {
        RigidBody3D {
            kind: BodyKind3D::Fixed,
            ..Default::default()
        }
    }
    pub fn kinematic() -> Self {
        RigidBody3D {
            kind: BodyKind3D::KinematicVelocity,
            ..Default::default()
        }
    }
}

impl Component for RigidBody3D {}

/// A collider built from VHACD convex parts of an ingested mesh (or any set of
/// convex hulls supplied by gameplay).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Collider3D {
    pub parts: Vec<ConvexPart>,
    pub restitution: f32,
    pub friction: f32,
    pub density: f32,
    pub sensor: bool,
}

impl Default for Collider3D {
    fn default() -> Self {
        Collider3D {
            parts: Vec::new(),
            restitution: 0.0,
            friction: 0.5,
            density: 1.0,
            sensor: false,
        }
    }
}

impl Collider3D {
    pub fn from_parts(parts: Vec<ConvexPart>) -> Self {
        Collider3D {
            parts,
            ..Default::default()
        }
    }
}

impl Component for Collider3D {}

/// The Rapier3D simulation state, stored as a world resource.
pub struct PhysicsWorld3D {
    pub gravity: Vec3,
    integration_parameters: IntegrationParameters,
    islands: IslandManager,
    broad_phase: DefaultBroadPhase,
    narrow_phase: NarrowPhase,
    bodies: RigidBodySet,
    colliders: ColliderSet,
    impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,
    ccd_solver: CCDSolver,
    pipeline: PhysicsPipeline,
    entity_to_body: HashMap<Entity, RigidBodyHandle>,
}

impl Default for PhysicsWorld3D {
    fn default() -> Self {
        PhysicsWorld3D::new(Vec3::new(0.0, -9.81, 0.0))
    }
}

impl PhysicsWorld3D {
    pub fn new(gravity: Vec3) -> Self {
        PhysicsWorld3D {
            gravity,
            integration_parameters: IntegrationParameters::default(),
            islands: IslandManager::new(),
            broad_phase: DefaultBroadPhase::new(),
            narrow_phase: NarrowPhase::new(),
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            ccd_solver: CCDSolver::new(),
            pipeline: PhysicsPipeline::new(),
            entity_to_body: HashMap::new(),
        }
    }

    pub fn body_count(&self) -> usize {
        self.bodies.len()
    }

    pub fn has_body(&self, entity: Entity) -> bool {
        self.entity_to_body.contains_key(&entity)
    }
}

fn body_type_of(kind: BodyKind3D) -> RigidBodyType {
    match kind {
        BodyKind3D::Dynamic => RigidBodyType::Dynamic,
        BodyKind3D::Fixed => RigidBodyType::Fixed,
        BodyKind3D::KinematicVelocity => RigidBodyType::KinematicVelocityBased,
    }
}

/// Build a Rapier compound collider from VHACD convex parts.
fn build_collider(c: &Collider3D) -> Collider {
    let mut shapes: Vec<(Pose3, SharedShape)> = Vec::new();
    for part in &c.parts {
        let pts: Vec<Vector> = part
            .vertices
            .iter()
            .map(|v| Vector::new(v[0], v[1], v[2]))
            .collect();
        if pts.len() < 4 {
            continue;
        }
        if let Some(hull) = SharedShape::convex_hull(&pts) {
            shapes.push((Pose3::identity(), hull));
        }
    }
    let compound = SharedShape::compound(shapes);
    ColliderBuilder::new(compound)
        .restitution(c.restitution)
        .friction(c.friction)
        .density(c.density)
        .sensor(c.sensor)
        .build()
}

/// Encode a `glam` quaternion as the axis*angle vector rapier's body builder
/// expects (`rotation_from_angle(angle_vector)`).
fn quat_to_axis_angle_vec(q: glam::Quat) -> Vector {
    let (axis, angle) = q.to_axis_angle();
    Vector::new(axis.x * angle, axis.y * angle, axis.z * angle)
}

/// Advance the 3D physics simulation by `dt` seconds and sync with the ECS.
pub fn step_physics_3d(world: &mut World, dt: f32) {
    let mut phys = match world.remove_resource::<PhysicsWorld3D>() {
        Some(p) => p,
        None => return,
    };

    // 1/2. Reconcile entities that have bodies.
    let current: Vec<Entity> = world
        .query_1::<RigidBody3D>()
        .into_iter()
        .map(|(e, _)| e)
        .collect();
    let current_set: HashSet<Entity> = current.iter().copied().collect();

    let stale: Vec<Entity> = phys
        .entity_to_body
        .keys()
        .copied()
        .filter(|e| !current_set.contains(e))
        .collect();
    for e in stale {
        if let Some(handle) = phys.entity_to_body.remove(&e) {
            phys.bodies.remove(
                handle,
                &mut phys.islands,
                &mut phys.colliders,
                &mut phys.impulse_joints,
                &mut phys.multibody_joints,
                true,
            );
        }
    }

    for e in &current {
        if phys.entity_to_body.contains_key(e) {
            continue;
        }
        let rb = *world.get_component::<RigidBody3D>(*e).unwrap();
        let t = world
            .get_component::<Transform>(*e)
            .copied()
            .unwrap_or_default();

        let mut builder = RigidBodyBuilder::new(body_type_of(rb.kind))
            .translation(Vector::new(
                t.translation.x,
                t.translation.y,
                t.translation.z,
            ))
            .rotation(quat_to_axis_angle_vec(t.rotation))
            .linvel(Vector::new(rb.linvel.x, rb.linvel.y, rb.linvel.z))
            .angvel(Vector::new(rb.angvel.x, rb.angvel.y, rb.angvel.z))
            .gravity_scale(rb.gravity_scale)
            .linear_damping(rb.linear_damping)
            .angular_damping(rb.angular_damping);
        if rb.lock_rotations {
            builder = builder.lock_rotations();
        }
        let handle = phys.bodies.insert(builder.build());

        if let Some(col) = world.get_component::<Collider3D>(*e).cloned() {
            if !col.parts.is_empty() {
                let collider = build_collider(&col);
                let bodies = &mut phys.bodies;
                phys.colliders.insert_with_parent(collider, handle, bodies);
            }
        }

        phys.entity_to_body.insert(*e, handle);
    }

    // 3. Push kinematic velocities.
    let pushes: Vec<(RigidBodyHandle, Vec3, Vec3)> = current
        .iter()
        .filter_map(|e| {
            let rb = world.get_component::<RigidBody3D>(*e)?;
            if rb.kind == BodyKind3D::KinematicVelocity {
                let h = *phys.entity_to_body.get(e)?;
                Some((h, rb.linvel, rb.angvel))
            } else {
                None
            }
        })
        .collect();
    for (handle, linvel, angvel) in pushes {
        if let Some(body) = phys.bodies.get_mut(handle) {
            body.set_linvel(Vector::new(linvel.x, linvel.y, linvel.z), true);
            body.set_angvel(Vector::new(angvel.x, angvel.y, angvel.z), true);
        }
    }

    // 4. Step the simulation.
    phys.integration_parameters.dt = dt;
    let gravity = Vector::new(phys.gravity.x, phys.gravity.y, phys.gravity.z);
    let PhysicsWorld3D {
        integration_parameters,
        islands,
        broad_phase,
        narrow_phase,
        bodies,
        colliders,
        impulse_joints,
        multibody_joints,
        ccd_solver,
        pipeline,
        ..
    } = &mut phys;
    pipeline.step(
        gravity,
        integration_parameters,
        islands,
        broad_phase,
        narrow_phase,
        bodies,
        colliders,
        impulse_joints,
        multibody_joints,
        ccd_solver,
        &(),
        &(),
    );

    // 5. Read back into the ECS.
    let readback: Vec<(Entity, RigidBodyHandle)> =
        phys.entity_to_body.iter().map(|(e, h)| (*e, *h)).collect();
    for (e, handle) in readback {
        let (pos, rot) = match phys.bodies.get(handle) {
            Some(body) => {
                let p = body.translation();
                let r = *body.rotation();
                // `r` is rapier's (glam 0.33) quaternion; `Transform` uses
                // nova-ecs's glam 0.29 quaternion, so rebuild it from components.
                let q = glam::Quat::from_xyzw(r.x, r.y, r.z, r.w);
                (Vec3::new(p.x, p.y, p.z), q)
            }
            None => continue,
        };
        if let Some(t) = world.get_component_mut::<Transform>(e) {
            t.translation = pos;
            t.rotation = rot;
        }
    }

    world.add_resource(phys);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decompose::ConvexPart;
    use nova_ecs::Vec3 as EcsVec3;

    /// A unit cube as a single convex part (8 verts, 12 tris).
    fn cube_parts(half: f32) -> Vec<ConvexPart> {
        let v = [
            [-half, -half, -half],
            [half, -half, -half],
            [half, half, -half],
            [-half, half, -half],
            [-half, -half, half],
            [half, -half, half],
            [half, half, half],
            [-half, half, half],
        ];
        let idx: [[u32; 3]; 12] = [
            [0, 1, 2],
            [0, 2, 3],
            [4, 6, 5],
            [4, 7, 6],
            [0, 4, 5],
            [0, 5, 1],
            [1, 5, 6],
            [1, 6, 2],
            [2, 6, 7],
            [2, 7, 3],
            [3, 7, 4],
            [3, 4, 0],
        ];
        vec![ConvexPart {
            vertices: v.to_vec(),
            indices: idx.to_vec(),
        }]
    }

    #[test]
    fn dynamic_body_falls_under_gravity() {
        let mut world = World::new();
        world.add_resource(PhysicsWorld3D::default());

        let e = world.spawn();
        world.add_component(e, Transform::from_translation(EcsVec3::new(0.0, 10.0, 0.0)));
        world.add_component(e, RigidBody3D::dynamic());
        world.add_component(e, Collider3D::from_parts(cube_parts(0.5)));

        let start_y = world.get_component::<Transform>(e).unwrap().translation.y;
        for _ in 0..60 {
            step_physics_3d(&mut world, 1.0 / 60.0);
        }
        let end_y = world.get_component::<Transform>(e).unwrap().translation.y;
        assert!(
            end_y < start_y - 1.0,
            "body should fall: {start_y} -> {end_y}"
        );
    }

    #[test]
    fn body_rests_on_fixed_ground() {
        let mut world = World::new();
        world.add_resource(PhysicsWorld3D::default());

        let ground = world.spawn();
        world.add_component(ground, Transform::default());
        world.add_component(ground, RigidBody3D::fixed());
        world.add_component(ground, Collider3D::from_parts(cube_parts(10.0)));

        let ball = world.spawn();
        world.add_component(
            ball,
            Transform::from_translation(EcsVec3::new(0.0, 5.0, 0.0)),
        );
        world.add_component(ball, RigidBody3D::dynamic());
        world.add_component(ball, Collider3D::from_parts(cube_parts(0.5)));

        for _ in 0..400 {
            step_physics_3d(&mut world, 1.0 / 60.0);
        }
        let y = world
            .get_component::<Transform>(ball)
            .unwrap()
            .translation
            .y;
        // Ground top at y = 10, ball half-height 0.5 => rests near 10.5.
        assert!(
            (10.0..11.0).contains(&y),
            "ball should rest on ground, y={y}"
        );
    }

    #[test]
    fn despawn_removes_body_from_sim() {
        let mut world = World::new();
        world.add_resource(PhysicsWorld3D::default());
        let e = world.spawn();
        world.add_component(e, Transform::default());
        world.add_component(e, RigidBody3D::dynamic());
        world.add_component(e, Collider3D::from_parts(cube_parts(0.5)));
        step_physics_3d(&mut world, 1.0 / 60.0);
        assert_eq!(world.resource::<PhysicsWorld3D>().unwrap().body_count(), 1);
        world.despawn(e);
        step_physics_3d(&mut world, 1.0 / 60.0);
        assert_eq!(world.resource::<PhysicsWorld3D>().unwrap().body_count(), 0);
    }
}
