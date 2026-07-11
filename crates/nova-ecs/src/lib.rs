//! Custom Entity-Component-System core for TPT Nova.
//!
//! `nova-ecs` provides a small, explicit, data-oriented ECS designed to be
//! trivially serializable for AI context windows. It is the structural anchor
//! that turns "dumb" generative assets into interactive, queryable state.

pub mod camera;
pub mod component;
pub mod entity;
pub mod light;
pub mod math;
pub mod rng;
pub mod scene_graph;
pub mod scheduler;
pub mod storage;
pub mod transform;
pub mod world;

pub use camera::{
    blend_cameras, pick_main_camera, update_active_camera, ActiveCamera, CameraRig, MainCamera,
    VirtualCamera,
};
pub use component::{Component, ComponentTypeId};
pub use entity::Entity;
pub use light::{Light, LightKind};
pub use math::{Mat4, Quat, Vec3};
pub use rng::{DeterministicRng, RngResource};
pub use scene_graph::{propagate_transforms, Children, Parent};
pub use scheduler::{Schedule, Scheduler, System};
pub use storage::MapStorage;
pub use transform::{Camera, GlobalTransform, Mesh, MeshKind, Transform};
pub use world::World;
