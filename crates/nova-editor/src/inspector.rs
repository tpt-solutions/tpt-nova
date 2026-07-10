//! Component inspector: read component fields for display and write edits back.
//!
//! Fields are addressed by a dotted path like `Transform.translation.x`, which
//! is what a UI row binds to. This keeps the editor generic: the panel iterates
//! [`inspect_entity`] to draw rows and calls [`set_field`] when a value changes.

use glam::{EulerRot, Quat};
use nova_ecs::transform::Transform;
use nova_ecs::{Entity, World};
use nova_physics::RigidBody2D;

/// A single editable scalar field.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    /// Dotted path, e.g. `Transform.translation.x`.
    pub path: String,
    pub value: f32,
}

/// A component's inspectable fields.
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentInspection {
    pub component: String,
    pub fields: Vec<Field>,
}

fn f(path: &str, value: f32) -> Field {
    Field {
        path: path.to_string(),
        value,
    }
}

/// Snapshot every supported component on `entity` as editable fields.
pub fn inspect_entity(world: &World, entity: Entity) -> Vec<ComponentInspection> {
    let mut out = Vec::new();

    if let Some(t) = world.get_component::<Transform>(entity) {
        let (_, _, rot_z) = t.rotation.to_euler(EulerRot::XYZ);
        out.push(ComponentInspection {
            component: "Transform".to_string(),
            fields: vec![
                f("Transform.translation.x", t.translation.x),
                f("Transform.translation.y", t.translation.y),
                f("Transform.translation.z", t.translation.z),
                f("Transform.rotation_z", rot_z),
                f("Transform.scale.x", t.scale.x),
                f("Transform.scale.y", t.scale.y),
                f("Transform.scale.z", t.scale.z),
            ],
        });
    }

    if let Some(rb) = world.get_component::<RigidBody2D>(entity) {
        out.push(ComponentInspection {
            component: "RigidBody2D".to_string(),
            fields: vec![
                f("RigidBody2D.linvel.x", rb.linvel.x),
                f("RigidBody2D.linvel.y", rb.linvel.y),
                f("RigidBody2D.angvel", rb.angvel),
                f("RigidBody2D.gravity_scale", rb.gravity_scale),
                f("RigidBody2D.linear_damping", rb.linear_damping),
                f("RigidBody2D.angular_damping", rb.angular_damping),
            ],
        });
    }

    out
}

/// Apply an edit to a field by path. Returns true if the field was recognized
/// and written.
pub fn set_field(world: &mut World, entity: Entity, path: &str, value: f32) -> bool {
    if let Some(rest) = path.strip_prefix("Transform.") {
        if let Some(t) = world.get_component_mut::<Transform>(entity) {
            return set_transform_field(t, rest, value);
        }
        return false;
    }
    if let Some(rest) = path.strip_prefix("RigidBody2D.") {
        if let Some(rb) = world.get_component_mut::<RigidBody2D>(entity) {
            return set_rigidbody_field(rb, rest, value);
        }
        return false;
    }
    false
}

fn set_transform_field(t: &mut Transform, field: &str, value: f32) -> bool {
    match field {
        "translation.x" => t.translation.x = value,
        "translation.y" => t.translation.y = value,
        "translation.z" => t.translation.z = value,
        "scale.x" => t.scale.x = value,
        "scale.y" => t.scale.y = value,
        "scale.z" => t.scale.z = value,
        "rotation_z" => {
            // Preserve X/Y euler, replace Z (2D-friendly rotation editing).
            let (x, y, _) = t.rotation.to_euler(EulerRot::XYZ);
            t.rotation = Quat::from_euler(EulerRot::XYZ, x, y, value);
        }
        _ => return false,
    }
    true
}

fn set_rigidbody_field(rb: &mut RigidBody2D, field: &str, value: f32) -> bool {
    match field {
        "linvel.x" => rb.linvel.x = value,
        "linvel.y" => rb.linvel.y = value,
        "angvel" => rb.angvel = value,
        "gravity_scale" => rb.gravity_scale = value,
        "linear_damping" => rb.linear_damping = value,
        "angular_damping" => rb.angular_damping = value,
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use nova_ecs::Vec3;

    #[test]
    fn inspects_transform_fields() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::from_translation(Vec3::new(1.0, 2.0, 3.0)));
        let inspection = inspect_entity(&world, e);
        assert_eq!(inspection.len(), 1);
        let tx = inspection[0]
            .fields
            .iter()
            .find(|f| f.path == "Transform.translation.x")
            .unwrap();
        assert_eq!(tx.value, 1.0);
    }

    #[test]
    fn edits_round_trip_into_world() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::default());
        assert!(set_field(&mut world, e, "Transform.translation.y", 42.0));
        assert_eq!(
            world.get_component::<Transform>(e).unwrap().translation.y,
            42.0
        );
    }

    #[test]
    fn rotation_z_edit_is_readable_back() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::default());
        set_field(&mut world, e, "Transform.rotation_z", 1.0);
        let fields = inspect_entity(&world, e);
        let rz = fields[0]
            .fields
            .iter()
            .find(|f| f.path == "Transform.rotation_z")
            .unwrap();
        assert!((rz.value - 1.0).abs() < 1e-5);
    }

    #[test]
    fn unknown_field_is_rejected() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::default());
        assert!(!set_field(&mut world, e, "Transform.nope", 1.0));
        assert!(!set_field(&mut world, e, "Missing.field", 1.0));
    }
}
