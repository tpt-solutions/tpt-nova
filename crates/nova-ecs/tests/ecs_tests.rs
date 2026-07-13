//! Integration tests for the `nova-ecs` core (entity/component/query/scheduler/
//! scene-graph). These exercise the public API the way gameplay and tooling code
//! does, and close the zero-test-coverage gap flagged in the build checklist.

use nova_ecs::scene_graph::{propagate_transforms, Children, Parent};
use nova_ecs::scheduler::{Schedule, Scheduler};
use nova_ecs::transform::{GlobalTransform, Mesh, MeshKind, Transform};
use nova_ecs::{Component, Entity, Vec3, World};

/// A tiny resource/component type used to exercise generic storage.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Counter(u32);
impl Component for Counter {}

fn spawn_with_transform(world: &mut World, t: Transform) -> Entity {
    let e = world.spawn();
    world.add_component(e, t);
    world.add_component(e, GlobalTransform::identity());
    e
}

// ---- Entity spawn / despawn ------------------------------------------------

#[test]
fn spawn_increments_entity_count() {
    let mut world = World::new();
    assert_eq!(world.entity_count(), 0);
    let a = world.spawn();
    let b = world.spawn();
    assert_ne!(a, b);
    assert_eq!(world.entity_count(), 2);
}

#[test]
fn despawn_removes_entity_and_components() {
    let mut world = World::new();
    let e = world.spawn();
    world.add_component(e, Counter(5));
    assert!(world.has_component::<Counter>(e));

    world.despawn(e);
    assert_eq!(world.entity_count(), 0);
    assert!(!world.has_component::<Counter>(e));
    assert!(world.get_component::<Counter>(e).is_none());
}

#[test]
fn despawn_invalid_handle_is_safe() {
    let mut world = World::new();
    // Should be a no-op, not a panic.
    world.despawn(Entity::INVALID);
    assert_eq!(world.entity_count(), 0);
}

#[test]
fn despawn_frees_index_and_bumps_generation() {
    let mut world = World::new();
    let e = world.spawn();
    let idx = e.index;
    let gen = e.generation;
    world.despawn(e);

    let e2 = world.spawn();
    assert_eq!(e2.index, idx);
    assert_eq!(e2.generation, gen + 1);

    // The stale handle must no longer resolve any component.
    assert!(!world.has_component::<Counter>(e));
}

#[test]
fn despawn_while_iterating_is_safe() {
    let mut world = World::new();
    for _ in 0..8 {
        let e = world.spawn();
        world.add_component(e, Counter(1));
    }
    assert_eq!(world.entity_count(), 8);

    // `World::entities` returns an owned snapshot, so despatching during the
    // loop never invalidates the iterator.
    for e in world.entities() {
        world.despawn(e);
    }
    assert_eq!(world.entity_count(), 0);
}

// ---- Component storage / query ---------------------------------------------

#[test]
fn add_get_remove_component_roundtrip() {
    let mut world = World::new();
    let e = world.spawn();
    assert!(world.get_component::<Counter>(e).is_none());

    world.add_component(e, Counter(42));
    assert_eq!(world.get_component::<Counter>(e).unwrap().0, 42);

    let prev = world.remove_component::<Counter>(e);
    assert_eq!(prev.unwrap().0, 42);
    assert!(!world.has_component::<Counter>(e));
}

#[test]
fn query_1_returns_only_matching_entities() {
    let mut world = World::new();
    let with = world.spawn();
    world.add_component(with, Counter(1));
    let without = world.spawn();

    let found = world.query_1::<Counter>();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].0, with);
    assert!(world.query_1::<Mesh>().is_empty());
    assert!(!world.has_component::<Counter>(without));
}

#[test]
fn query_2_requires_both_components() {
    let mut world = World::new();
    let both = world.spawn();
    world.add_component(both, Counter(1));
    world.add_component(both, Transform::default());

    let only_counter = world.spawn();
    world.add_component(only_counter, Counter(2));

    let found = world.query_2::<Counter, Transform>();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].0, both);
    assert_eq!(found[0].1 .0, 1);
}

#[test]
fn query_3_requires_all_three_components() {
    let mut world = World::new();
    let all = world.spawn();
    world.add_component(all, Counter(1));
    world.add_component(all, Transform::default());
    world.add_component(
        all,
        Mesh {
            kind: MeshKind::Cube,
        },
    );

    let found = world.query_3::<Counter, Transform, Mesh>();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].0, all);
    assert_eq!(found[0].3.kind, MeshKind::Cube);
}

#[test]
fn query_empty_when_storage_absent() {
    let world = World::new();
    assert!(world.query_1::<Counter>().is_empty());
    assert!(world.query_2::<Counter, Transform>().is_empty());
    assert!(world.query_3::<Counter, Transform, Mesh>().is_empty());
}

// ---- Scheduler ------------------------------------------------------------

#[test]
fn scheduler_runs_systems_in_insertion_order() {
    let mut world = World::new();
    world.add_resource(Vec::<u32>::new());

    let mut scheduler = Scheduler::new();
    scheduler.add_system(|w| w.resource_mut::<Vec<u32>>().unwrap().push(1));
    scheduler.add_system(|w| w.resource_mut::<Vec<u32>>().unwrap().push(2));
    scheduler.add_system(|w| w.resource_mut::<Vec<u32>>().unwrap().push(3));

    scheduler.run(&mut world);

    assert_eq!(*world.resource::<Vec<u32>>().unwrap(), vec![1, 2, 3]);
}

#[test]
fn scheduler_system_can_mutate_world() {
    let mut world = World::new();
    let e = world.spawn();

    let mut scheduler = Scheduler::new();
    scheduler.add_system(move |w| {
        w.add_component(e, Counter(7));
    });
    scheduler.run(&mut world);

    assert_eq!(world.get_component::<Counter>(e).unwrap().0, 7);
}

#[test]
fn schedule_runs_stages_in_order() {
    let mut world = World::new();
    world.add_resource(Vec::<&'static str>::new());

    let mut schedule = Schedule::new();
    schedule.add_stage("early");
    schedule.add_stage("late");
    schedule.add_system_to("early", |w| {
        w.resource_mut::<Vec<&str>>().unwrap().push("early")
    });
    schedule.add_system_to("late", |w| {
        w.resource_mut::<Vec<&str>>().unwrap().push("late")
    });

    schedule.run(&mut world);
    assert_eq!(
        *world.resource::<Vec<&str>>().unwrap(),
        vec!["early", "late"]
    );
}

// ---- Scene graph ----------------------------------------------------------

#[test]
fn world_transform_propagates_to_child() {
    let mut world = World::new();
    let parent = spawn_with_transform(
        &mut world,
        Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)),
    );
    let child = spawn_with_transform(
        &mut world,
        Transform::from_translation(Vec3::new(0.0, 2.0, 0.0)),
    );
    world.add_component(child, Parent(parent));
    world.add_component(parent, Children(vec![child]));

    propagate_transforms(&mut world);

    let child_world = world
        .get_component::<GlobalTransform>(child)
        .unwrap()
        .translation();
    assert!((child_world - Vec3::new(1.0, 2.0, 0.0)).length() < 1e-5);

    let parent_world = world
        .get_component::<GlobalTransform>(parent)
        .unwrap()
        .translation();
    assert!((parent_world - Vec3::new(1.0, 0.0, 0.0)).length() < 1e-5);
}

#[test]
fn reparenting_updates_world_transform() {
    let mut world = World::new();
    let root_a = spawn_with_transform(
        &mut world,
        Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)),
    );
    let root_b = spawn_with_transform(
        &mut world,
        Transform::from_translation(Vec3::new(0.0, 0.0, 5.0)),
    );
    let child = spawn_with_transform(
        &mut world,
        Transform::from_translation(Vec3::new(0.0, 2.0, 0.0)),
    );
    world.add_component(child, Parent(root_a));
    world.add_component(root_a, Children(vec![child]));

    propagate_transforms(&mut world);
    let first = world
        .get_component::<GlobalTransform>(child)
        .unwrap()
        .translation();
    assert!((first - Vec3::new(1.0, 2.0, 0.0)).length() < 1e-5);

    // Reparent child under root_b.
    world.remove_component::<Parent>(child);
    world.remove_component::<Children>(root_a);
    world.add_component(child, Parent(root_b));
    world.add_component(root_b, Children(vec![child]));

    propagate_transforms(&mut world);
    let second = world
        .get_component::<GlobalTransform>(child)
        .unwrap()
        .translation();
    assert!((second - Vec3::new(0.0, 2.0, 5.0)).length() < 1e-5);
}

#[test]
fn scene_graph_cycle_does_not_infinite_loop() {
    let mut world = World::new();
    let a = spawn_with_transform(
        &mut world,
        Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)),
    );
    let b = spawn_with_transform(
        &mut world,
        Transform::from_translation(Vec3::new(0.0, 1.0, 0.0)),
    );
    // Mutual parent/child links form a cycle.
    world.add_component(a, Parent(b));
    world.add_component(a, Children(vec![b]));
    world.add_component(b, Parent(a));
    world.add_component(b, Children(vec![a]));

    // Must terminate; GlobalTransforms are still created (as identity) for every
    // Transform-bearing entity, but the cycle is never resolved into a chain.
    propagate_transforms(&mut world);

    assert_eq!(world.entity_count(), 2);
    assert_eq!(
        world.get_component::<GlobalTransform>(a).unwrap().0,
        nova_ecs::math::Mat4::IDENTITY
    );
    assert_eq!(
        world.get_component::<GlobalTransform>(b).unwrap().0,
        nova_ecs::math::Mat4::IDENTITY
    );
}

// ---- Serde round-trip for components ---------------------------------------

#[test]
fn component_serde_roundtrips() {
    use nova_ecs::transform::Camera;
    use serde_json::{from_str, to_string};

    let t = Transform::from_translation(Vec3::new(1.0, 2.0, 3.0));
    let t2: Transform = from_str(&to_string(&t).unwrap()).unwrap();
    assert_eq!(t, t2);

    let m = Mesh {
        kind: MeshKind::Cube,
    };
    let m2: Mesh = from_str(&to_string(&m).unwrap()).unwrap();
    assert_eq!(m, m2);

    let cam = Camera::default();
    let cam2: Camera = from_str(&to_string(&cam).unwrap()).unwrap();
    assert_eq!(cam, cam2);

    let mut world = World::new();
    let e1 = world.spawn();
    let e2 = world.spawn();

    let p = Parent(e1);
    let p2: Parent = from_str(&to_string(&p).unwrap()).unwrap();
    assert_eq!(p, p2);

    let c = Children(vec![e1, e2]);
    let c2: Children = from_str(&to_string(&c).unwrap()).unwrap();
    assert_eq!(c, c2);
}

// ---- Deterministic RNG ----------------------------------------------------

#[test]
fn rng_is_deterministic_for_same_seed() {
    use nova_ecs::rng::DeterministicRng;

    let mut a = DeterministicRng::new(0x1234_ABCD);
    let mut b = DeterministicRng::new(0x1234_ABCD);
    let seq_a: Vec<u64> = (0..16).map(|_| a.next_u64()).collect();
    let seq_b: Vec<u64> = (0..16).map(|_| b.next_u64()).collect();
    assert_eq!(seq_a, seq_b);
}

#[test]
fn rng_differs_for_different_seeds() {
    use nova_ecs::rng::DeterministicRng;

    let mut a = DeterministicRng::new(1);
    let mut b = DeterministicRng::new(2);
    assert_ne!(a.next_u64(), b.next_u64());
}

#[test]
fn rng_seed_zero_is_normalized() {
    use nova_ecs::rng::DeterministicRng;
    // A zero seed must not lock the generator into its trivial state.
    let mut a = DeterministicRng::new(0);
    assert_ne!(a.next_u64(), 0);

    // Two zero-seeded generators still produce identical streams.
    let mut a2 = DeterministicRng::new(0);
    let mut b2 = DeterministicRng::new(0);
    assert_eq!(a2.next_u64(), b2.next_u64());
}
