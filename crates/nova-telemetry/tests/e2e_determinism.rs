//! Cross-crate end-to-end test: ECS + physics + telemetry on a fixed timestep.
//!
//! The engine's core AI-feedback loop depends on (a) a deterministic fixed-step
//! simulation and (b) structured telemetry that faithfully reflects world state.
//! This test drives a falling rigid body through `nova-physics`, dumps the world
//! with `nova-telemetry`, and asserts the same seed + inputs produce byte-identical
//! telemetry on two independent runs — the determinism contract the whole
//! hot-apply loop is built on.

use nova_ecs::transform::Transform;
use nova_ecs::{Vec3, World};
use nova_physics::{step_physics, Collider2D, ColliderShape, RigidBody2D};
use nova_telemetry::{dump_world, TelemetryFrame};

fn simulate(seed: u64) -> TelemetryFrame {
    let mut world = World::new();
    world.add_resource(nova_physics::PhysicsWorld::default());

    let e = world.spawn();
    world.add_component(e, Transform::from_translation(Vec3::new(0.0, 10.0, 0.0)));
    world.add_component(e, RigidBody2D::dynamic());
    world.add_component(e, Collider2D::new(ColliderShape::ball(0.5)));

    let dt = 1.0 / 60.0;
    for tick in 0..120u64 {
        step_physics(&mut world, dt);
        if tick % 30 == 0 {
            // Telemetry is emitted on a tick interval in the real engine; here we
            // just confirm a dump at any point reflects the current deterministic state.
            let _ = dump_world(&world, tick, seed);
        }
    }
    dump_world(&world, 120, seed)
}

#[test]
fn physics_into_telemetry_is_deterministic_across_runs() {
    let a = simulate(12345);
    let b = simulate(12345);

    // Same seed + inputs => identical telemetry payload (JSON byte-for-byte).
    let ja = serde_json::to_string(&a).expect("serialize");
    let jb = serde_json::to_string(&b).expect("serialize");
    assert_eq!(
        ja, jb,
        "two deterministic runs must produce identical telemetry"
    );

    // The body must have actually fallen under gravity.
    let t = &a.entities[0].components;
    let translation = t.get("Transform").expect("transform present");
    let y = translation["translation"]["y"].as_f64().expect("y value");
    assert!(y < 9.0, "body should have fallen, y={y}");
}

#[test]
fn different_seed_changes_telemetry() {
    // The seed is part of the payload, so differing seeds must differ even if
    // the simulation itself is identical. This pins the seed into telemetry.
    let a = simulate(1);
    let b = simulate(2);
    assert_eq!(a.seed, 1);
    assert_eq!(b.seed, 2);
    assert_ne!(
        serde_json::to_string(&a).unwrap(),
        serde_json::to_string(&b).unwrap()
    );
}
