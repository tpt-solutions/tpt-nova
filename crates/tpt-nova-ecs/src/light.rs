//! Dynamic lights: directional and point sources, with an optional shadow flag.
//!
//! A [`Light`] is pure, serializable data. Its position/direction is taken from
//! the entity's [`GlobalTransform`](crate::transform::GlobalTransform) by the
//! renderer, so the ECS stays free of GPU types.

use crate::component::Component;
use crate::math::{Quat, Vec3};
use serde::{Deserialize, Serialize};

/// The kind of light source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LightKind {
    /// Parallel rays travelling along the light's forward axis (e.g. the sun).
    /// Only direction (from the entity rotation) matters; position is ignored.
    #[default]
    Directional,
    /// Radiates from a point, attenuating to zero at [`Light::range`].
    Point,
}

/// A renderable light. Paired with a [`GlobalTransform`] (for position/aim) and
/// optionally a [`crate::transform::Camera`] if it also casts shadows.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Light {
    pub kind: LightKind,
    /// Linear RGB color, components roughly in `[0, 1]`; `intensity` scales it.
    pub color: Vec3,
    /// Brightness multiplier.
    pub intensity: f32,
    /// Point lights only: distance (world units) at which the light fades to 0.
    pub range: f32,
    /// Whether the light renders a shadow map.
    pub cast_shadows: bool,
    /// Directional lights only: half-size of the orthographic shadow frustum
    /// in world units (bigger = softer/cheaper but coarser shadows).
    pub shadow_extent: f32,
}

impl Default for Light {
    fn default() -> Self {
        Light {
            kind: LightKind::Directional,
            color: Vec3::new(1.0, 1.0, 1.0),
            intensity: 1.0,
            range: 10.0,
            cast_shadows: false,
            shadow_extent: 20.0,
        }
    }
}

impl Light {
    pub fn directional() -> Self {
        Light {
            kind: LightKind::Directional,
            ..Default::default()
        }
    }

    pub fn point() -> Self {
        Light {
            kind: LightKind::Point,
            ..Default::default()
        }
    }

    pub fn with_shadows(mut self) -> Self {
        self.cast_shadows = true;
        self
    }

    /// World-space direction the light *travels* (points away from the source).
    /// For a directional light this is the `-Z` axis of its transform.
    pub fn direction(&self, rotation: Quat) -> Vec3 {
        rotation * Vec3::NEG_Z
    }
}

impl Component for Light {}
