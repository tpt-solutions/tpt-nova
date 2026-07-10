//! 2D viewport gizmos: translate/rotate/scale the selected entity from pointer
//! drags, with optional grid/angle snapping.

use glam::{EulerRot, Quat, Vec2};
use nova_ecs::transform::Transform;
use nova_ecs::{Entity, World};

/// Which transformation the gizmo currently applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GizmoMode {
    #[default]
    Move,
    Rotate,
    Scale,
}

/// Snapping settings for gizmo edits.
#[derive(Debug, Clone, Copy)]
pub struct GizmoSnap {
    /// Grid size for translate/scale (0 = no snap).
    pub grid: f32,
    /// Angle step in radians for rotate (0 = no snap).
    pub angle: f32,
}

impl Default for GizmoSnap {
    fn default() -> Self {
        GizmoSnap {
            grid: 0.0,
            angle: 0.0,
        }
    }
}

fn snap_to(value: f32, step: f32) -> f32 {
    if step > 0.0 {
        (value / step).round() * step
    } else {
        value
    }
}

/// Apply a pointer drag (in world units) to the selected entity's `Transform`
/// according to `mode`. Returns true if a transform was modified.
///
/// - **Move**: adds the drag to translation X/Y.
/// - **Rotate**: rotates about Z by `drag.x * ROTATE_SENSITIVITY` radians.
/// - **Scale**: adds `drag * SCALE_SENSITIVITY` to scale X/Y (uniform-ish).
pub fn apply_gizmo(
    world: &mut World,
    entity: Entity,
    mode: GizmoMode,
    drag: Vec2,
    snap: GizmoSnap,
) -> bool {
    const ROTATE_SENSITIVITY: f32 = 0.01;
    const SCALE_SENSITIVITY: f32 = 0.01;

    let t = match world.get_component_mut::<Transform>(entity) {
        Some(t) => t,
        None => return false,
    };

    match mode {
        GizmoMode::Move => {
            let nx = snap_to(t.translation.x + drag.x, snap.grid);
            let ny = snap_to(t.translation.y + drag.y, snap.grid);
            t.translation.x = nx;
            t.translation.y = ny;
        }
        GizmoMode::Rotate => {
            let (x, y, z) = t.rotation.to_euler(EulerRot::XYZ);
            let nz = snap_to(z + drag.x * ROTATE_SENSITIVITY, snap.angle);
            t.rotation = Quat::from_euler(EulerRot::XYZ, x, y, nz);
        }
        GizmoMode::Scale => {
            t.scale.x = snap_to(t.scale.x + drag.x * SCALE_SENSITIVITY, snap.grid);
            t.scale.y = snap_to(t.scale.y + drag.y * SCALE_SENSITIVITY, snap.grid);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use nova_ecs::Vec3;

    fn world_with_entity() -> (World, Entity) {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::default());
        (world, e)
    }

    #[test]
    fn move_adds_translation() {
        let (mut world, e) = world_with_entity();
        apply_gizmo(
            &mut world,
            e,
            GizmoMode::Move,
            Vec2::new(3.0, -2.0),
            GizmoSnap::default(),
        );
        let t = world.get_component::<Transform>(e).unwrap();
        assert_eq!(t.translation.x, 3.0);
        assert_eq!(t.translation.y, -2.0);
    }

    #[test]
    fn move_snaps_to_grid() {
        let (mut world, e) = world_with_entity();
        let snap = GizmoSnap {
            grid: 1.0,
            angle: 0.0,
        };
        apply_gizmo(&mut world, e, GizmoMode::Move, Vec2::new(2.4, 0.6), snap);
        let t = world.get_component::<Transform>(e).unwrap();
        assert_eq!(t.translation.x, 2.0);
        assert_eq!(t.translation.y, 1.0);
    }

    #[test]
    fn rotate_changes_z_angle() {
        let (mut world, e) = world_with_entity();
        apply_gizmo(
            &mut world,
            e,
            GizmoMode::Rotate,
            Vec2::new(100.0, 0.0),
            GizmoSnap::default(),
        );
        let t = world.get_component::<Transform>(e).unwrap();
        let (_, _, z) = t.rotation.to_euler(EulerRot::XYZ);
        assert!((z - 1.0).abs() < 1e-4, "expected ~1.0 rad, got {z}");
    }

    #[test]
    fn scale_adjusts_scale() {
        let (mut world, e) = world_with_entity();
        world.get_component_mut::<Transform>(e).unwrap().scale = Vec3::new(1.0, 1.0, 1.0);
        apply_gizmo(
            &mut world,
            e,
            GizmoMode::Scale,
            Vec2::new(50.0, 100.0),
            GizmoSnap::default(),
        );
        let t = world.get_component::<Transform>(e).unwrap();
        assert!((t.scale.x - 1.5).abs() < 1e-4);
        assert!((t.scale.y - 2.0).abs() < 1e-4);
    }
}
