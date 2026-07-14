//! Cross-crate integration: ECS + physics + telemetry, asserting the
//! deterministic simulation produces identical telemetry across runs.
//!
//! This is the harness behind the "Regression test harness for the AI
//! code-injection loop" and "End-to-end integration test spanning multiple
//! crates (ECS + physics + telemetry tick, asserting deterministic output)"
//! checklist items: a fixed setup must yield byte-for-byte identical telemetry
//! frames so an AI agent can correlate runs and spot regressions.

use nova_ecs::transform::Transform;
use nova_ecs::Vec3;
use nova_ecs::World;
use nova_physics::{step_physics, Collider2D, ColliderShape, PhysicsWorld, RigidBody2D};
use nova_telemetry::{dump_world, TelemetryFrame};

fn scenario(seed: u64) -> TelemetryFrame {
    let mut world = World::new();
    world.add_resource(PhysicsWorld::default());
    let e = world.spawn();
    world.add_component(e, Transform::from_translation(Vec3::new(0.0, 10.0, 0.0)));
    world.add_component(e, RigidBody2D::dynamic());
    world.add_component(e, Collider2D::new(ColliderShape::ball(0.5)));
    for _ in 0..120 {
        step_physics(&mut world, 1.0 / 60.0);
    }
    dump_world(&world, 120, seed)
}

#[test]
fn ecs_physics_telemetry_is_deterministic_across_runs() {
    let a = scenario(42);
    let b = scenario(42);
    assert_eq!(
        a, b,
        "identical setup must yield identical telemetry frames (regression guard)"
    );

    // The single dynamic body must have fallen under gravity.
    assert_eq!(a.entities.len(), 1);
    let t = a.entities[0]
        .components
        .get("Transform")
        .expect("has transform");
    let y = t["translation"][1].as_f64().unwrap();
    assert!(y < 10.0, "body should have fallen: y={y}");
}

#[test]
fn telemetry_seed_is_carried_while_physics_stays_deterministic() {
    // The physics sim here is seed-independent (no RNG in the step), but the
    // telemetry frame must still carry the requested seed so agents can
    // correlate runs by seed.
    let a = scenario(1);
    let b = scenario(2);
    assert_eq!(
        a.entities, b.entities,
        "physics is deterministic regardless of seed"
    );
    assert_eq!(a.seed, 1);
    assert_eq!(b.seed, 2);
}
