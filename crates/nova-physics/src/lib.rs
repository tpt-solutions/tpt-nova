//! Rapier2D physics integrated as ECS components + a deterministic sync step.
//!
//! Design: gameplay code only ever touches the plain [`RigidBody2D`] /
//! [`Collider2D`] components and the entity [`Transform`]. All Rapier state
//! lives in the [`PhysicsWorld`] resource. Each fixed tick, [`step_physics`]:
//!
//! 1. creates Rapier bodies/colliders for newly-seen entities,
//! 2. removes bodies for entities that lost their component or despawned,
//! 3. pushes kinematic velocities from components into the sim,
//! 4. advances the simulation by the fixed timestep, and
//! 5. reads body transforms/velocities back into the ECS.
//!
//! The timestep is fixed and the world is stepped in lockstep with the engine
//! tick, which keeps the simulation reproducible from a seed.

pub mod components;

use std::collections::{HashMap, HashSet};

use glam::{EulerRot, Quat, Vec2};
use nova_ecs::transform::Transform;
use nova_ecs::{Entity, World};
use rapier2d::prelude::*;

pub use components::{BodyKind, Collider2D, ColliderShape, RigidBody2D};

/// The Rapier simulation state, stored as a world resource.
pub struct PhysicsWorld {
    pub gravity: Vec2,
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

impl Default for PhysicsWorld {
    fn default() -> Self {
        PhysicsWorld::new(Vec2::new(0.0, -9.81))
    }
}

impl PhysicsWorld {
    pub fn new(gravity: Vec2) -> Self {
        PhysicsWorld {
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

    /// Number of rigid bodies currently in the simulation.
    pub fn body_count(&self) -> usize {
        self.bodies.len()
    }

    /// Whether an entity currently has a Rapier body.
    pub fn has_body(&self, entity: Entity) -> bool {
        self.entity_to_body.contains_key(&entity)
    }
}

fn body_type_of(kind: BodyKind) -> RigidBodyType {
    match kind {
        BodyKind::Dynamic => RigidBodyType::Dynamic,
        BodyKind::Fixed => RigidBodyType::Fixed,
        BodyKind::KinematicVelocity => RigidBodyType::KinematicVelocityBased,
    }
}

fn z_angle(rot: Quat) -> f32 {
    let (_, _, z) = rot.to_euler(EulerRot::XYZ);
    z
}

fn build_collider(c: &Collider2D) -> Collider {
    let builder = match c.shape {
        ColliderShape::Ball { radius } => ColliderBuilder::ball(radius),
        ColliderShape::Cuboid { half_x, half_y } => ColliderBuilder::cuboid(half_x, half_y),
        ColliderShape::Capsule {
            half_height,
            radius,
        } => ColliderBuilder::capsule_y(half_height, radius),
    };
    builder
        .restitution(c.restitution)
        .friction(c.friction)
        .density(c.density)
        .sensor(c.sensor)
        .build()
}

/// Advance the physics simulation by `dt` seconds and sync it with the ECS.
///
/// `dt` should be the engine's fixed timestep so the simulation stays
/// deterministic and frame-rate independent.
pub fn step_physics(world: &mut World, dt: f32) {
    // Take ownership of the physics resource so we can freely borrow `world`.
    let mut phys = match world.remove_resource::<PhysicsWorld>() {
        Some(p) => p,
        None => return,
    };

    // --- 1/2. Reconcile which entities have bodies -----------------------
    let current: Vec<Entity> = world
        .query_1::<RigidBody2D>()
        .into_iter()
        .map(|(e, _)| e)
        .collect();
    let current_set: HashSet<Entity> = current.iter().copied().collect();

    // Remove bodies for entities that no longer have a RigidBody2D.
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

    // Create bodies for newly-seen entities.
    for e in &current {
        if phys.entity_to_body.contains_key(e) {
            continue;
        }
        let rb = *world.get_component::<RigidBody2D>(*e).unwrap();
        let t = world
            .get_component::<Transform>(*e)
            .copied()
            .unwrap_or_default();

        let mut builder = RigidBodyBuilder::new(body_type_of(rb.kind))
            .translation(Vector::new(t.translation.x, t.translation.y))
            .rotation(z_angle(t.rotation))
            .linvel(Vector::new(rb.linvel.x, rb.linvel.y))
            .angvel(rb.angvel)
            .gravity_scale(rb.gravity_scale)
            .linear_damping(rb.linear_damping)
            .angular_damping(rb.angular_damping);
        if rb.lock_rotation {
            builder = builder.lock_rotations();
        }
        let handle = phys.bodies.insert(builder.build());

        if let Some(col) = world.get_component::<Collider2D>(*e).copied() {
            let collider = build_collider(&col);
            let bodies = &mut phys.bodies;
            phys.colliders.insert_with_parent(collider, handle, bodies);
        }

        phys.entity_to_body.insert(*e, handle);
    }

    // --- 3. Push kinematic velocities from components --------------------
    let pushes: Vec<(RigidBodyHandle, Vec2, f32)> = current
        .iter()
        .filter_map(|e| {
            let rb = world.get_component::<RigidBody2D>(*e)?;
            if rb.kind == BodyKind::KinematicVelocity {
                let h = *phys.entity_to_body.get(e)?;
                Some((h, rb.linvel, rb.angvel))
            } else {
                None
            }
        })
        .collect();
    for (handle, linvel, angvel) in pushes {
        if let Some(body) = phys.bodies.get_mut(handle) {
            body.set_linvel(Vector::new(linvel.x, linvel.y), true);
            body.set_angvel(angvel, true);
        }
    }

    // --- 4. Step the simulation -----------------------------------------
    phys.integration_parameters.dt = dt;
    let gravity = Vector::new(phys.gravity.x, phys.gravity.y);
    let PhysicsWorld {
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

    // --- 5. Read body state back into the ECS ---------------------------
    let readback: Vec<(Entity, RigidBodyHandle)> =
        phys.entity_to_body.iter().map(|(e, h)| (*e, *h)).collect();
    for (e, handle) in readback {
        let (pos, angle, linvel, angvel) = match phys.bodies.get(handle) {
            Some(body) => {
                let p = body.translation();
                let lv = body.linvel();
                (
                    Vec2::new(p.x, p.y),
                    body.rotation().angle(),
                    Vec2::new(lv.x, lv.y),
                    body.angvel(),
                )
            }
            None => continue,
        };
        if let Some(t) = world.get_component_mut::<Transform>(e) {
            t.translation.x = pos.x;
            t.translation.y = pos.y;
            t.rotation = Quat::from_rotation_z(angle);
        }
        if let Some(rb) = world.get_component_mut::<RigidBody2D>(e) {
            if rb.kind != BodyKind::KinematicVelocity {
                rb.linvel = linvel;
                rb.angvel = angvel;
            }
        }
    }

    world.add_resource(phys);
}

#[cfg(test)]
mod tests {
    use super::*;
    use nova_ecs::Vec3;

    #[test]
    fn dynamic_body_falls_under_gravity() {
        let mut world = World::new();
        world.add_resource(PhysicsWorld::default());

        let e = world.spawn();
        world.add_component(e, Transform::from_translation(Vec3::new(0.0, 10.0, 0.0)));
        world.add_component(e, RigidBody2D::dynamic());
        world.add_component(e, Collider2D::new(ColliderShape::ball(0.5)));

        let start_y = world.get_component::<Transform>(e).unwrap().translation.y;
        for _ in 0..60 {
            step_physics(&mut world, 1.0 / 60.0);
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
        world.add_resource(PhysicsWorld::default());

        // Ground: fixed cuboid at y=0.
        let ground = world.spawn();
        world.add_component(
            ground,
            Transform::from_translation(Vec3::new(0.0, 0.0, 0.0)),
        );
        world.add_component(ground, RigidBody2D::fixed());
        world.add_component(ground, Collider2D::new(ColliderShape::cuboid(10.0, 0.5)));

        // Ball dropped from above.
        let ball = world.spawn();
        world.add_component(ball, Transform::from_translation(Vec3::new(0.0, 5.0, 0.0)));
        world.add_component(ball, RigidBody2D::dynamic());
        world.add_component(ball, Collider2D::new(ColliderShape::ball(0.5)));

        for _ in 0..300 {
            step_physics(&mut world, 1.0 / 60.0);
        }
        let y = world
            .get_component::<Transform>(ball)
            .unwrap()
            .translation
            .y;
        // Ball radius 0.5 resting on top of ground half-height 0.5 => ~1.0.
        assert!(
            (0.5..1.6).contains(&y),
            "ball should rest near ground, y={y}"
        );
    }

    #[test]
    fn despawn_removes_body_from_sim() {
        let mut world = World::new();
        world.add_resource(PhysicsWorld::default());
        let e = world.spawn();
        world.add_component(e, Transform::default());
        world.add_component(e, RigidBody2D::dynamic());
        step_physics(&mut world, 1.0 / 60.0);
        assert_eq!(world.resource::<PhysicsWorld>().unwrap().body_count(), 1);
        world.despawn(e);
        step_physics(&mut world, 1.0 / 60.0);
        assert_eq!(world.resource::<PhysicsWorld>().unwrap().body_count(), 0);
    }
}
