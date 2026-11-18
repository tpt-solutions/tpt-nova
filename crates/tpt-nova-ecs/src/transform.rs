//! Core transform, mesh, and camera components.
//!
//! `Transform` is the local-space placement of an entity. `GlobalTransform` is
//! the cached world-space matrix produced by the scene-graph propagation system
//! (see [`crate::scene_graph`]).

use crate::component::Component;
use crate::math::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

/// Local-space transform: translation, rotation, and scale.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Transform {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }
}

impl Transform {
    pub fn new(translation: Vec3, rotation: Quat, scale: Vec3) -> Self {
        Transform {
            translation,
            rotation,
            scale,
        }
    }

    pub fn from_translation(translation: Vec3) -> Self {
        Transform {
            translation,
            ..Default::default()
        }
    }

    pub fn from_rotation(rotation: Quat) -> Self {
        Transform {
            rotation,
            ..Default::default()
        }
    }

    /// Local model matrix.
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }
}

/// Cached world-space transform, recomputed each frame by the scene graph.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GlobalTransform(pub Mat4);

impl Default for GlobalTransform {
    fn default() -> Self {
        GlobalTransform(Mat4::IDENTITY)
    }
}

impl GlobalTransform {
    pub fn identity() -> Self {
        GlobalTransform(Mat4::IDENTITY)
    }

    pub fn matrix(&self) -> Mat4 {
        self.0
    }

    pub fn translation(&self) -> Vec3 {
        self.0.w_axis.truncate()
    }
}

/// A renderable mesh reference. The actual geometry lives in the renderer and
/// is keyed by [`MeshKind`]; the ECS stays free of GPU types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mesh {
    pub kind: MeshKind,
}

impl Default for Mesh {
    fn default() -> Self {
        Mesh {
            kind: MeshKind::Cube,
        }
    }
}

/// Enumerates the built-in geometry the renderer knows how to build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MeshKind {
    Cube,
}

/// A camera component. The view matrix is derived from the entity's
/// [`GlobalTransform`]; this struct holds only projection parameters.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    pub fov_y: f32,
    pub near: f32,
    pub far: f32,
    pub aspect: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            fov_y: 60.0_f32.to_radians(),
            near: 0.01,
            far: 1000.0,
            aspect: 16.0 / 9.0,
        }
    }
}

impl Camera {
    pub fn perspective(&self) -> Mat4 {
        Mat4::perspective_rh(self.fov_y, self.aspect, self.near, self.far)
    }
}

impl Component for Transform {}
impl Component for GlobalTransform {}
impl Component for Mesh {}
impl Component for MeshKind {}
impl Component for Camera {}
