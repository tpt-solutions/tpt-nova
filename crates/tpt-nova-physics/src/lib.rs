//! 2D rigid body physics integrated as ECS components + a deterministic sync step.
//!
//! Design: gameplay code only ever touches the plain [`RigidBody2D`] /
//! [`Collider2D`] components and the entity [`Transform`]. All solver state
//! lives in the [`PhysicsWorld`] resource. Each fixed tick, [`step_physics`]:
//!
//! 1. creates solver bodies for newly-seen entities,
//! 2. removes bodies for entities that lost their component or despawned,
//! 3. pushes kinematic velocities from components into the sim,
//! 4. advances the simulation by the fixed timestep, and
//! 5. reads body transforms/velocities back into the ECS.
//!
//! The solver is in-house (see [`solver`]), replacing Rapier2D so the workspace
//! stays free of Apache-2.0-only dependencies. Bodies are stepped in entity
//! order and the timestep is fixed, so the simulation is reproducible from a seed.

pub mod components;
pub mod solver;

use std::collections::{HashMap, HashSet};

use glam::{EulerRot, Quat, Vec2};
use tpt_nova_ecs::transform::Transform;
use tpt_nova_ecs::{Entity, World};

pub use components::{BodyKind, Collider2D, ColliderShape, RigidBody2D};
use solver::{capsule_polygon, LocalShape, SolverBody};

/// Per-body data the solver does not need but the sync step does.
#[derive(Debug, Clone, Copy)]
struct BodyMeta {
    entity: Entity,
    kind: BodyKind,
    gravity_scale: f32,
    linear_damping: f32,
    angular_damping: f32,
}

/// The 2D simulation state, stored as a world resource.
pub struct PhysicsWorld {
    pub gravity: Vec2,
    bodies: Vec<SolverBody>,
    meta: Vec<BodyMeta>,
    entity_to_body: HashMap<Entity, usize>,
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
            bodies: Vec::new(),
            meta: Vec::new(),
            entity_to_body: HashMap::new(),
        }
    }

    /// Number of rigid bodies currently in the simulation.
    pub fn body_count(&self) -> usize {
        self.bodies.len()
    }

    /// Whether an entity currently has a body.
    pub fn has_body(&self, entity: Entity) -> bool {
        self.entity_to_body.contains_key(&entity)
    }

    fn remove_body(&mut self, entity: Entity) {
        if let Some(index) = self.entity_to_body.remove(&entity) {
            self.bodies.remove(index);
            self.meta.remove(index);
            self.reindex();
        }
    }

    fn reindex(&mut self) {
        self.entity_to_body.clear();
        for (i, m) in self.meta.iter().enumerate() {
            self.entity_to_body.insert(m.entity, i);
        }
    }
}

fn z_angle(rot: Quat) -> f32 {
    let (_, _, z) = rot.to_euler(EulerRot::XYZ);
    z
}

fn local_shape(shape: ColliderShape) -> LocalShape {
    match shape {
        ColliderShape::Ball { radius } => LocalShape::Circle { radius },
        ColliderShape::Cuboid { half_x, half_y } => LocalShape::Polygon(vec![
            Vec2::new(-half_x, -half_y),
            Vec2::new(half_x, -half_y),
            Vec2::new(half_x, half_y),
            Vec2::new(-half_x, half_y),
        ]),
        ColliderShape::Capsule {
            half_height,
            radius,
        } => LocalShape::Polygon(capsule_polygon(half_height, radius)),
    }
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
    // Sorted so body order (and therefore contact resolution order) does not
    // depend on the ECS query order.
    let mut current: Vec<Entity> = world
        .query_1::<RigidBody2D>()
        .into_iter()
        .map(|(e, _)| e)
        .collect();
    current.sort_unstable();
    let current_set: HashSet<Entity> = current.iter().copied().collect();

    let stale: Vec<Entity> = phys
        .entity_to_body
        .keys()
        .copied()
        .filter(|e| !current_set.contains(e))
        .collect();
    for e in stale {
        phys.remove_body(e);
    }

    for e in &current {
        if phys.entity_to_body.contains_key(e) {
            continue;
        }
        let rb = *world.get_component::<RigidBody2D>(*e).unwrap();
        let t = world
            .get_component::<Transform>(*e)
            .copied()
            .unwrap_or_default();
        let collider = world.get_component::<Collider2D>(*e).copied();

        let mut body = SolverBody {
            pos: Vec2::new(t.translation.x, t.translation.y),
            angle: z_angle(t.rotation),
            vel: rb.linvel,
            angvel: rb.angvel,
            inv_mass: 0.0,
            inv_inertia: 0.0,
            shape: None,
            restitution: 0.0,
            friction: 0.5,
            sensor: false,
            is_static: rb.kind != BodyKind::Dynamic,
        };
        if let Some(col) = collider {
            let shape = local_shape(col.shape);
            let props = shape.mass_props(col.density);
            body.shape = Some(shape);
            body.restitution = col.restitution;
            body.friction = col.friction;
            body.sensor = col.sensor;
            if rb.kind == BodyKind::Dynamic {
                body.inv_mass = inv(props.mass);
                if !rb.lock_rotation {
                    body.inv_inertia = inv(props.inertia);
                }
            }
        } else if rb.kind == BodyKind::Dynamic {
            // A dynamic body with no collider still falls under gravity.
            body.inv_mass = 1.0;
        }
        if rb.lock_rotation {
            body.angvel = 0.0;
        }

        let index = phys.bodies.len();
        phys.bodies.push(body);
        phys.meta.push(BodyMeta {
            entity: *e,
            kind: rb.kind,
            gravity_scale: rb.gravity_scale,
            linear_damping: rb.linear_damping,
            angular_damping: rb.angular_damping,
        });
        phys.entity_to_body.insert(*e, index);
    }

    // --- 3. Push kinematic velocities from components ---------------------
    for (i, m) in phys.meta.iter().enumerate() {
        if m.kind != BodyKind::KinematicVelocity {
            continue;
        }
        if let Some(rb) = world.get_component::<RigidBody2D>(m.entity) {
            phys.bodies[i].vel = rb.linvel;
            phys.bodies[i].angvel = rb.angvel;
        }
    }

    // --- 4. Step the simulation -----------------------------------------
    let gravity = phys.gravity;
    let scales: Vec<f32> = phys.meta.iter().map(|m| m.gravity_scale).collect();
    let damping: Vec<(f32, f32)> = phys
        .meta
        .iter()
        .map(|m| (m.linear_damping, m.angular_damping))
        .collect();
    solver::step(&mut phys.bodies, gravity, &scales, &damping, dt);

    // --- 5. Read body state back into the ECS ---------------------------
    for i in 0..phys.bodies.len() {
        let m = phys.meta[i];
        let b = &phys.bodies[i];
        let (pos, angle, linvel, angvel) = (b.pos, b.angle, b.vel, b.angvel);
        if let Some(t) = world.get_component_mut::<Transform>(m.entity) {
            t.translation.x = pos.x;
            t.translation.y = pos.y;
            t.rotation = Quat::from_rotation_z(angle);
        }
        if let Some(rb) = world.get_component_mut::<RigidBody2D>(m.entity) {
            if rb.kind != BodyKind::KinematicVelocity {
                rb.linvel = linvel;
                rb.angvel = angvel;
            }
        }
    }

    world.add_resource(phys);
}

fn inv(x: f32) -> f32 {
    if x > 0.0 {
        1.0 / x
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tpt_nova_ecs::Vec3;

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

    #[test]
    fn kinematic_component_velocity_drives_transform() {
        // The component's linvel must round-trip into the physics body and back
        // out as a moved Transform (component -> body -> ECS).
        let mut world = World::new();
        world.add_resource(PhysicsWorld::default());
        let e = world.spawn();
        world.add_component(e, Transform::from_translation(Vec3::ZERO));
        world.add_component(e, RigidBody2D::kinematic().with_linvel(Vec2::new(2.0, 0.0)));

        let x0 = world.get_component::<Transform>(e).unwrap().translation.x;
        step_physics(&mut world, 0.5);
        let x1 = world.get_component::<Transform>(e).unwrap().translation.x;
        // Kinematic velocity-based body integrates position by linvel * dt.
        assert!(
            (x1 - x0 - 1.0).abs() < 1e-3,
            "expected ~1.0 of travel, got {}",
            x1 - x0
        );
    }

    #[test]
    fn dynamic_body_velocity_syncs_back_to_component() {
        // After a step, the body's velocity is read back into the RigidBody2D
        // component (body -> component round-trip), so gravity changes it.
        let mut world = World::new();
        world.add_resource(PhysicsWorld::default());
        let e = world.spawn();
        world.add_component(e, Transform::from_translation(Vec3::new(0.0, 10.0, 0.0)));
        let mut rb = RigidBody2D::dynamic();
        rb.linvel = Vec2::new(0.0, 5.0);
        world.add_component(e, rb);
        world.add_component(e, Collider2D::new(ColliderShape::ball(0.5)));

        step_physics(&mut world, 1.0 / 60.0);
        let v = world.get_component::<RigidBody2D>(e).unwrap().linvel;
        // Gravity (-9.81) over 1/60s should have lowered the upward velocity.
        assert!(v.y < 5.0, "gravity should have reduced upward velocity");
        assert!(v.y > 4.5, "but only slightly, got {}", v.y);
    }

    #[test]
    fn simulation_is_deterministic_for_identical_setup() {
        fn scenario() -> Vec3 {
            let mut world = World::new();
            world.add_resource(PhysicsWorld::default());
            let e = world.spawn();
            world.add_component(e, Transform::from_translation(Vec3::new(0.0, 10.0, 0.0)));
            world.add_component(e, RigidBody2D::dynamic());
            world.add_component(e, Collider2D::new(ColliderShape::ball(0.5)));
            for _ in 0..120 {
                step_physics(&mut world, 1.0 / 60.0);
            }
            world.get_component::<Transform>(e).unwrap().translation
        }
        let a = scenario();
        let b = scenario();
        assert_eq!(
            a, b,
            "two identical simulations must produce identical state (determinism regression)"
        );
    }

    #[test]
    fn box_stacks_on_box_and_comes_to_rest() {
        let mut world = World::new();
        world.add_resource(PhysicsWorld::default());
        let ground = world.spawn();
        world.add_component(ground, Transform::default());
        world.add_component(ground, RigidBody2D::fixed());
        world.add_component(ground, Collider2D::new(ColliderShape::cuboid(10.0, 0.5)));

        let lower = world.spawn();
        world.add_component(lower, Transform::from_translation(Vec3::new(0.0, 2.0, 0.0)));
        world.add_component(lower, RigidBody2D::dynamic());
        world.add_component(lower, Collider2D::new(ColliderShape::cuboid(0.5, 0.5)));

        let upper = world.spawn();
        world.add_component(upper, Transform::from_translation(Vec3::new(0.0, 4.0, 0.0)));
        world.add_component(upper, RigidBody2D::dynamic());
        world.add_component(upper, Collider2D::new(ColliderShape::cuboid(0.5, 0.5)));

        for _ in 0..600 {
            step_physics(&mut world, 1.0 / 60.0);
        }
        let y_lower = world
            .get_component::<Transform>(lower)
            .unwrap()
            .translation
            .y;
        let y_upper = world
            .get_component::<Transform>(upper)
            .unwrap()
            .translation
            .y;
        // Ground top y=0.5; lower box centre ~1.0; upper ~2.0. Allow some slack.
        assert!((0.8..1.2).contains(&y_lower), "lower box y={y_lower}");
        assert!((1.8..2.3).contains(&y_upper), "upper box y={y_upper}");
    }
}
