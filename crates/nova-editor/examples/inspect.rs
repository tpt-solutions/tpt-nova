//! Headless example of `nova-editor`'s generic inspector: snapshot an entity's
//! components as editable fields, mutate one through its dotted path, and confirm
//! the change lands in the world. No GPU or window required.

use nova_ecs::transform::Transform;
use nova_ecs::{Vec3, World};
use nova_editor::{inspect_entity, set_field};

fn main() {
    let mut world = World::new();
    let e = world.spawn();
    world.add_component(e, Transform::from_translation(Vec3::new(1.0, 2.0, 3.0)));

    // Snapshot the entity's inspectable fields.
    let inspection = inspect_entity(&world, e);
    println!("inspecting {} component(s):", inspection.len());
    for comp in &inspection {
        for field in &comp.fields {
            println!("  {} = {}", field.path, field.value);
        }
    }

    // Edit a field by dotted path (the same string the inspector UI binds to).
    assert!(set_field(&mut world, e, "Transform.translation.x", 42.0));

    let t = world.get_component::<Transform>(e).unwrap();
    println!("after edit, translation.x = {}", t.translation.x);
    assert!((t.translation.x - 42.0).abs() < 1e-5);
}
