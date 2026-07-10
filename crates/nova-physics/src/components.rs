//! ECS-facing physics components.
//!
//! These are plain, serializable data components. They describe *intent*
//! (a body is dynamic, a collider is a 0.5-radius ball, etc). The actual
//! Rapier simulation state lives in the [`PhysicsWorld`](crate::PhysicsWorld)
//! resource and is created/synced by [`step_physics`](crate::step_physics).

use glam::Vec2;
use nova_ecs::component::Component;
use serde::{Deserialize, Serialize};

/// How a body participates in the simulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BodyKind {
    /// Fully simulated: moved by forces, gravity, and collisions.
    #[default]
    Dynamic,
    /// Never moves; infinite mass. Good for ground/walls.
    Fixed,
    /// Moved explicitly by gameplay code via its velocity; not pushed by others.
    KinematicVelocity,
}

/// A rigid body attached to an entity. Its transform is driven by the physics
/// step (for dynamic/kinematic bodies) and read back into the entity's
/// [`Transform`](nova_ecs::transform::Transform).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RigidBody2D {
    pub kind: BodyKind,
    /// Linear velocity (world units / second).
    pub linvel: Vec2,
    /// Angular velocity about the Z axis (radians / second).
    pub angvel: f32,
    /// Multiplier on the world gravity applied to this body.
    pub gravity_scale: f32,
    /// Linear damping (velocity decay per second).
    pub linear_damping: f32,
    /// Angular damping.
    pub angular_damping: f32,
    /// If true the body cannot rotate (common for characters).
    pub lock_rotation: bool,
}

impl Default for RigidBody2D {
    fn default() -> Self {
        RigidBody2D {
            kind: BodyKind::Dynamic,
            linvel: Vec2::ZERO,
            angvel: 0.0,
            gravity_scale: 1.0,
            linear_damping: 0.0,
            angular_damping: 0.0,
            lock_rotation: false,
        }
    }
}

impl RigidBody2D {
    pub fn dynamic() -> Self {
        Self::default()
    }

    pub fn fixed() -> Self {
        RigidBody2D {
            kind: BodyKind::Fixed,
            ..Default::default()
        }
    }

    pub fn kinematic() -> Self {
        RigidBody2D {
            kind: BodyKind::KinematicVelocity,
            ..Default::default()
        }
    }

    pub fn with_linvel(mut self, v: Vec2) -> Self {
        self.linvel = v;
        self
    }
}

/// The collision shape for an entity. Dimensions are half-extents / radii in
/// world units, matching Rapier's conventions.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ColliderShape {
    /// A circle of the given radius.
    Ball { radius: f32 },
    /// An axis-aligned box with the given half-extents.
    Cuboid { half_x: f32, half_y: f32 },
    /// A horizontal capsule of the given half-height and radius.
    Capsule { half_height: f32, radius: f32 },
}

impl ColliderShape {
    pub fn ball(radius: f32) -> Self {
        ColliderShape::Ball { radius }
    }

    pub fn cuboid(half_x: f32, half_y: f32) -> Self {
        ColliderShape::Cuboid { half_x, half_y }
    }
}

/// A collider attached to an entity. If the entity also has a [`RigidBody2D`]
/// the collider is parented to that body; otherwise it becomes a static
/// (fixed) collider on its own.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Collider2D {
    pub shape: ColliderShape,
    /// Bounciness in `[0, 1]`.
    pub restitution: f32,
    /// Surface friction.
    pub friction: f32,
    /// Mass density (used to compute mass for dynamic bodies).
    pub density: f32,
    /// If true, the collider reports intersections but does not push bodies.
    pub sensor: bool,
}

impl Default for Collider2D {
    fn default() -> Self {
        Collider2D {
            shape: ColliderShape::Ball { radius: 0.5 },
            restitution: 0.0,
            friction: 0.5,
            density: 1.0,
            sensor: false,
        }
    }
}

impl Collider2D {
    pub fn new(shape: ColliderShape) -> Self {
        Collider2D {
            shape,
            ..Default::default()
        }
    }

    pub fn with_restitution(mut self, r: f32) -> Self {
        self.restitution = r;
        self
    }
}

impl Component for RigidBody2D {}
impl Component for Collider2D {}
