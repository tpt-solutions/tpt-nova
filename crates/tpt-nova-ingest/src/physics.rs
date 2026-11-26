//! 3D rigid body physics for ingested meshes.
//!
//! Mirrors the 2D design in `tpt-nova-physics`: gameplay code only touches plain
//! [`RigidBody3D`] / [`Collider3D`] components and the entity [`Transform`];
//! all solver state lives in the [`PhysicsWorld3D`] resource. Each fixed tick,
//! [`step_physics_3d`] reconciles bodies, pushes kinematic velocities, advances
//! the simulation, and reads transforms back into the ECS.
//!
//! A [`Collider3D`] stores the [`ConvexPart`]s produced by the ingestion
//! pipeline. The solver (see [`crate::solver3d`]) treats them as a compound of
//! convex hulls, so a concave ingested mesh still collides correctly.
//!
//! The solver is in-house, so the workspace stays free of Apache-2.0-only
//! dependencies.

use std::collections::{HashMap, HashSet};

use glam::Vec3;
use serde::{Deserialize, Serialize};
use tpt_nova_ecs::transform::Transform;
use tpt_nova_ecs::{component::Component, Entity, World};

use crate::decompose::ConvexPart;
use crate::solver3d::{self, Body3D, CachedImpulse, LocalHull};

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

/// A collider built from convex parts of an ingested mesh (or any set of
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

/// Per-body data the solver does not need but the sync step does.
#[derive(Debug, Clone, Copy)]
struct BodyMeta {
    entity: Entity,
    kind: BodyKind3D,
}

/// The 3D simulation state, stored as a world resource.
pub struct PhysicsWorld3D {
    pub gravity: Vec3,
    bodies: Vec<Body3D>,
    meta: Vec<BodyMeta>,
    entity_to_body: HashMap<Entity, usize>,
    impulse_cache: Vec<CachedImpulse>,
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
            bodies: Vec::new(),
            meta: Vec::new(),
            entity_to_body: HashMap::new(),
            impulse_cache: Vec::new(),
        }
    }

    pub fn body_count(&self) -> usize {
        self.bodies.len()
    }

    pub fn has_body(&self, entity: Entity) -> bool {
        self.entity_to_body.contains_key(&entity)
    }

    fn remove_body(&mut self, entity: Entity) {
        if let Some(index) = self.entity_to_body.remove(&entity) {
            self.bodies.remove(index);
            // Cached impulses refer to body indices, which just shifted.
            self.impulse_cache.clear();
            self.meta.remove(index);
            self.entity_to_body.clear();
            for (i, m) in self.meta.iter().enumerate() {
                self.entity_to_body.insert(m.entity, i);
            }
        }
    }
}

/// Build the local hulls for a collider, skipping parts with no volume.
fn local_hulls(c: &Collider3D) -> Vec<LocalHull> {
    c.parts.iter().filter_map(LocalHull::from_part).collect()
}

/// Inverse principal inertia of the axis-aligned box spanning `hulls`, scaled
/// to `mass`. A box approximation keeps the solver simple and stable.
fn inv_inertia_for(hulls: &[LocalHull], mass: f32) -> Vec3 {
    if hulls.is_empty() || mass <= 0.0 {
        return Vec3::ZERO;
    }
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for h in hulls {
        for v in &h.verts {
            lo = lo.min(*v);
            hi = hi.max(*v);
        }
    }
    let s = hi - lo;
    let i = Vec3::new(
        mass / 12.0 * (s.y * s.y + s.z * s.z),
        mass / 12.0 * (s.x * s.x + s.z * s.z),
        mass / 12.0 * (s.x * s.x + s.y * s.y),
    );
    Vec3::new(inv(i.x), inv(i.y), inv(i.z))
}

fn inv(x: f32) -> f32 {
    if x > 0.0 {
        1.0 / x
    } else {
        0.0
    }
}

/// Advance the 3D physics simulation by `dt` seconds and sync with the ECS.
pub fn step_physics_3d(world: &mut World, dt: f32) {
    let mut phys = match world.remove_resource::<PhysicsWorld3D>() {
        Some(p) => p,
        None => return,
    };

    // 1/2. Reconcile entities that have bodies. Sorted so body order does not
    // depend on the ECS query order.
    let mut current: Vec<Entity> = world
        .query_1::<RigidBody3D>()
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
        let rb = *world.get_component::<RigidBody3D>(*e).unwrap();
        let t = world
            .get_component::<Transform>(*e)
            .copied()
            .unwrap_or_default();

        let mut body = Body3D {
            pos: t.translation,
            rot: t.rotation,
            vel: rb.linvel,
            angvel: rb.angvel,
            inv_mass: 0.0,
            inv_inertia_local: Vec3::ZERO,
            parts: Vec::new(),
            restitution: 0.0,
            friction: 0.5,
            sensor: false,
            is_static: rb.kind != BodyKind3D::Dynamic,
            gravity_scale: rb.gravity_scale,
            linear_damping: rb.linear_damping,
            angular_damping: rb.angular_damping,
        };
        if let Some(col) = world.get_component::<Collider3D>(*e).cloned() {
            let hulls = local_hulls(&col);
            let volume: f32 = hulls.iter().map(|h| h.volume).sum();
            let mass = (col.density * volume).max(1e-6);
            body.restitution = col.restitution;
            body.friction = col.friction;
            body.sensor = col.sensor;
            if rb.kind == BodyKind3D::Dynamic {
                body.inv_mass = inv(mass);
                if !rb.lock_rotations {
                    body.inv_inertia_local = inv_inertia_for(&hulls, mass);
                }
            }
            body.parts = hulls;
        } else if rb.kind == BodyKind3D::Dynamic {
            // A dynamic body with no collider still falls under gravity.
            body.inv_mass = 1.0;
        }
        if rb.lock_rotations {
            // Locked bodies keep their orientation: no initial spin either.
            body.angvel = Vec3::ZERO;
        }

        let index = phys.bodies.len();
        phys.bodies.push(body);
        phys.meta.push(BodyMeta {
            entity: *e,
            kind: rb.kind,
        });
        phys.entity_to_body.insert(*e, index);
    }

    // 3. Push kinematic velocities.
    for i in 0..phys.meta.len() {
        if phys.meta[i].kind != BodyKind3D::KinematicVelocity {
            continue;
        }
        if let Some(rb) = world.get_component::<RigidBody3D>(phys.meta[i].entity) {
            phys.bodies[i].vel = rb.linvel;
            phys.bodies[i].angvel = rb.angvel;
        }
    }

    // 4. Step the simulation.
    let gravity = phys.gravity;
    solver3d::step(&mut phys.bodies, gravity, dt, &mut phys.impulse_cache);

    // 5. Read back into the ECS.
    for i in 0..phys.bodies.len() {
        let m = phys.meta[i];
        let b = &phys.bodies[i];
        let (pos, rot, linvel, angvel) = (b.pos, b.rot, b.vel, b.angvel);
        if let Some(t) = world.get_component_mut::<Transform>(m.entity) {
            t.translation = pos;
            t.rotation = rot;
        }
        if let Some(rb) = world.get_component_mut::<RigidBody3D>(m.entity) {
            if rb.kind != BodyKind3D::KinematicVelocity {
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
    use tpt_nova_ecs::Vec3 as EcsVec3;

    /// A cube of the given half-extent as a single convex part (outward triangles).
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
            [0, 2, 1],
            [0, 3, 2],
            [4, 5, 6],
            [4, 6, 7],
            [0, 1, 5],
            [0, 5, 4],
            [1, 2, 6],
            [1, 6, 5],
            [2, 3, 7],
            [2, 7, 6],
            [3, 0, 4],
            [3, 4, 7],
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

    #[test]
    fn locked_rotation_body_does_not_spin() {
        let mut world = World::new();
        world.add_resource(PhysicsWorld3D::default());
        let e = world.spawn();
        world.add_component(e, Transform::from_translation(EcsVec3::new(0.0, 10.0, 0.0)));
        let mut rb = RigidBody3D::dynamic();
        rb.angvel = EcsVec3::new(0.0, 3.0, 0.0);
        rb.lock_rotations = true;
        world.add_component(e, rb);
        world.add_component(e, Collider3D::from_parts(cube_parts(0.5)));
        for _ in 0..10 {
            step_physics_3d(&mut world, 1.0 / 60.0);
        }
        let w = world.get_component::<RigidBody3D>(e).unwrap().angvel;
        assert!(w.length() < 1e-6, "locked body must not spin, w={w:?}");
    }

    #[test]
    fn cube_stack_settles_on_ground() {
        let mut world = World::new();
        world.add_resource(PhysicsWorld3D::default());
        let ground = world.spawn();
        world.add_component(ground, Transform::default());
        world.add_component(ground, RigidBody3D::fixed());
        world.add_component(ground, Collider3D::from_parts(cube_parts(5.0)));

        let lower = world.spawn();
        world.add_component(
            lower,
            Transform::from_translation(EcsVec3::new(0.0, 6.0, 0.0)),
        );
        world.add_component(lower, RigidBody3D::dynamic());
        world.add_component(lower, Collider3D::from_parts(cube_parts(0.5)));

        let upper = world.spawn();
        world.add_component(
            upper,
            Transform::from_translation(EcsVec3::new(0.0, 8.0, 0.0)),
        );
        world.add_component(upper, RigidBody3D::dynamic());
        world.add_component(upper, Collider3D::from_parts(cube_parts(0.5)));

        for _ in 0..900 {
            step_physics_3d(&mut world, 1.0 / 60.0);
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
        // Ground top y=5; lower centre ~5.5; upper ~6.5. Allow some slack.
        assert!((5.3..5.8).contains(&y_lower), "lower cube y={y_lower}");
        assert!((6.2..6.9).contains(&y_upper), "upper cube y={y_upper}");
    }
}
