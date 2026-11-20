//! Local transform for a scene node: translation, rotation, and scale.

use glam::{Mat3, Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

/// A node's local placement.
///
/// This mirrors [`tpt_nova_ecs::Transform`] but uses scene-graph naming
/// (`position` rather than `translation`) and adds the ergonomic helpers a
/// declarative 3D API needs (`looking_at`, `rotate_y`, Euler input).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Transform::IDENTITY
    }
}

impl Transform {
    pub const IDENTITY: Transform = Transform {
        position: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    pub fn from_position(position: Vec3) -> Self {
        Transform {
            position,
            ..Transform::IDENTITY
        }
    }

    /// Build from human-friendly Euler angles in degrees (applied Z, then Y,
    /// then X).
    pub fn from_euler_degrees(x: f32, y: f32, z: f32) -> Self {
        Transform {
            rotation: Quat::from_euler(
                glam::EulerRot::XYZ,
                x.to_radians(),
                y.to_radians(),
                z.to_radians(),
            ),
            ..Transform::IDENTITY
        }
    }

    pub fn with_position(mut self, position: Vec3) -> Self {
        self.position = position;
        self
    }

    pub fn with_rotation(mut self, rotation: Quat) -> Self {
        self.rotation = rotation;
        self
    }

    pub fn with_scale(mut self, scale: Vec3) -> Self {
        self.scale = scale;
        self
    }

    /// Rotate a further `radians` around the local Y axis.
    pub fn rotate_y(&mut self, radians: f32) {
        self.rotation *= Quat::from_rotation_y(radians);
    }

    /// Aim the node's local `-Z` (the camera/light forward axis) at `target`.
    pub fn looking_at(mut self, target: Vec3) -> Self {
        let forward = (target - self.position).normalize_or_zero();
        if forward.length_squared() > 0.0 {
            self.rotation = rotation_looking_at(forward, Vec3::Y);
        }
        self
    }

    /// The local-to-parent matrix.
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.position)
    }

    /// Local `-Z` axis (the direction a camera or light faces).
    pub fn forward(&self) -> Vec3 {
        self.rotation * Vec3::NEG_Z
    }

    pub fn to_ecs(&self) -> tpt_nova_ecs::Transform {
        tpt_nova_ecs::Transform {
            translation: self.position,
            rotation: self.rotation,
            scale: self.scale,
        }
    }

    pub fn from_ecs(t: &tpt_nova_ecs::Transform) -> Self {
        Transform {
            position: t.translation,
            rotation: t.rotation,
            scale: t.scale,
        }
    }
}

/// The rotation that aims local `-Z` along `forward` with `up` as the hint.
///
/// Used by both [`Transform::looking_at`] and the light bake, so a directional
/// light's authored `direction` survives the trip through the ECS rotation.
pub fn rotation_looking_at(forward: Vec3, up: Vec3) -> Quat {
    let forward = forward.normalize_or_zero();
    if forward.length_squared() == 0.0 {
        return Quat::IDENTITY;
    }
    let right = forward.cross(up);
    let right = if right.length_squared() < 1e-8 {
        forward.cross(Vec3::Z)
    } else {
        right
    }
    .normalize();
    let true_up = right.cross(forward).normalize();
    Quat::from_mat3(&Mat3::from_cols(right, true_up, -forward))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_transform_is_identity() {
        assert_eq!(Transform::default().matrix(), Mat4::IDENTITY);
        assert_eq!(Transform::default().forward(), Vec3::NEG_Z);
    }

    #[test]
    fn looking_at_faces_the_target() {
        let t = Transform::from_position(Vec3::new(0.0, 0.0, 5.0)).looking_at(Vec3::ZERO);
        let f = t.forward();
        assert!((f - Vec3::NEG_Z).length() < 1e-5, "forward = {f:?}");
    }

    #[test]
    fn rotate_y_accumulates() {
        let mut t = Transform::IDENTITY;
        t.rotate_y(std::f32::consts::FRAC_PI_2);
        let f = t.forward();
        assert!((f - Vec3::NEG_X).length() < 1e-5, "forward = {f:?}");
    }

    #[test]
    fn ecs_round_trip() {
        let t = Transform::from_position(Vec3::new(1.0, 2.0, 3.0))
            .with_scale(Vec3::splat(2.0))
            .with_rotation(Quat::from_rotation_x(0.4));
        assert_eq!(Transform::from_ecs(&t.to_ecs()), t);
    }
}
