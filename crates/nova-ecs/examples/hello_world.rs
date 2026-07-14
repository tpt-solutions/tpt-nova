//! Minimal end-to-end example: build a world, spawn a couple of entities, run a
//! deterministic scheduler, and read the world back. No GPU or window required.

use nova_ecs::scheduler::Scheduler;
use nova_ecs::transform::{Mesh, MeshKind, Transform};
use nova_ecs::{Entity, Vec3, World};

fn main() {
    let mut world = World::new();

    // Spawn a cube at the origin.
    let cube = world.spawn();
    world.add_component(cube, Transform::from_translation(Vec3::ZERO));
    world.add_component(
        cube,
        Mesh {
            kind: MeshKind::Cube,
        },
    );

    // Spawn a second entity offset along +X.
    let other: Entity = world.spawn();
    world.add_component(other, Transform::from_translation(Vec3::new(3.0, 0.0, 0.0)));

    // A system that nudges every transform's rotation a little each step.
    let mut scheduler = Scheduler::new();
    scheduler.add_system(|w: &mut World| {
        let angle = 0.01f32;
        let q = nova_ecs::Quat::from_rotation_y(angle);
        for e in w.entities() {
            if let Some(t) = w.get_component_mut::<Transform>(e) {
                t.rotation = q * t.rotation;
            }
        }
    });

    for tick in 0..5 {
        scheduler.run(&mut world);
        println!("tick {tick}: world has {} entities", world.entity_count());
    }

    let t = world.get_component::<Transform>(cube).unwrap();
    println!(
        "cube final translation = {:?}, |rotation| = {:.3}",
        t.translation,
        t.rotation.length()
    );
}
