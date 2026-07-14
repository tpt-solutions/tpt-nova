//! Headless `nova-ecs` demo: build a tiny scene graph, propagate world-space
//! transforms, and print the result. No GPU or window required.
//!
//! Run with: `cargo run -p nova-ecs --example hello_world`

use nova_ecs::scene_graph::{propagate_transforms, Children, Parent};
use nova_ecs::scheduler::{Schedule, Scheduler};
use nova_ecs::transform::{GlobalTransform, Transform};
use nova_ecs::world::World;
use nova_ecs::{Entity, Vec3};

fn main() {
    let mut world = World::new();

    // Root entity at the origin.
    let root = world.spawn();
    world.add_component(root, Transform::from_translation(Vec3::new(0.0, 0.0, 0.0)));

    // A child offset from the root.
    let child = world.spawn();
    world.add_component(child, Transform::from_translation(Vec3::new(2.0, 0.0, 0.0)));
    world.add_component(child, Parent(root));
    world.add_component(root, Children(vec![child]));

    // A grandchild offset from the child.
    let grandchild = world.spawn();
    world.add_component(
        grandchild,
        Transform::from_translation(Vec3::new(0.0, 3.0, 0.0)),
    );
    world.add_component(grandchild, Parent(child));
    world.add_component(child, Children(vec![grandchild]));

    // Drive the scene graph from a scheduler so the example shows real usage.
    let mut scheduler = Scheduler::new();
    scheduler.add_system(|w: &mut World| propagate_transforms(w));
    scheduler.run(&mut world);

    // Same thing staged explicitly through a `Schedule`.
    let mut schedule = Schedule::new();
    schedule.add_stage("update");
    schedule.add_system_to("update", |w: &mut World| propagate_transforms(w));
    schedule.run(&mut world);

    print_scene(&world, root, child, grandchild);

    assert_eq!(world.entity_count(), 3);
    println!(
        "hello_world: {}/3 entities have world transforms",
        world.entity_count()
    );
}

fn print_scene(world: &World, root: Entity, child: Entity, grandchild: Entity) {
    let print = |label: &str, e: Entity| {
        let gt = world
            .get_component::<GlobalTransform>(e)
            .expect("transform must be propagated");
        let t = gt.translation();
        println!(
            "  {label:<11} {e} -> world position ({:.2}, {:.2}, {:.2})",
            t.x, t.y, t.z
        );
    };

    println!("Scene graph (after propagate_transforms):");
    print("root", root);
    print("child", child);
    print("grandchild", grandchild);

    // The grandchild should be root + (2,0,0) + (0,3,0) = (2,3,0).
    let gt = world.get_component::<GlobalTransform>(grandchild).unwrap();
    let t = gt.translation();
    assert!((t - Vec3::new(2.0, 3.0, 0.0)).length() < 1e-4);
}
