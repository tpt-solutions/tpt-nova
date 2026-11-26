//! A declarative 3D scene graph for TPT Nova.
//!
//! A [`Scene`] is a plain, serializable hierarchy of [`Node`]s (meshes, sprites,
//! cameras, lights) with local transforms, an active camera, environment
//! settings, and a [`PostProcessPipeline`]. [`bake`] flattens it into a
//! [`RenderPlan`], and [`RenderPlan::spawn_into`] writes that plan as ECS
//! entities that `tpt-nova-render`'s PBR and sprite pipelines already read.
//!
//! The crate contains no wgpu code of its own. Post-processing is described here
//! and lowered to `tpt-nova-render`'s `PostProcessChain`.
//!
//! ```
//! use glam::Vec3;
//! use tpt_nova_scene_graph::{bake, DirectionalLight, Node, PerspectiveCamera, Scene};
//!
//! let mut scene = Scene::new();
//! scene.add_node(Node::camera(PerspectiveCamera::default()).at(Vec3::new(0.0, 2.0, 6.0)));
//! scene.add_node(Node::cube());
//! scene.add_node(Node::light(DirectionalLight::new(Vec3::NEG_Y, 1.5)));
//!
//! let plan = bake(&scene);
//! assert_eq!(plan.meshes.len(), 1);
//! assert!(plan.camera.is_some());
//! ```

pub mod animator;
#[cfg(feature = "appfront")]
pub mod appfront;
pub mod bake;
pub mod color;
pub mod id;
pub mod node;
pub mod pick;
pub mod post;
pub mod scene;
pub mod transform;

pub use bake::{bake, PlannedCamera, PlannedLight, PlannedMesh, PlannedSprite, RenderPlan};
pub use color::{srgb_to_linear, Color};
pub use id::NodeId;
pub use node::{
    AmbientLight, DirectionalLight, LightSource, MeshNode, Node, NodeKind, PerspectiveCamera,
    PointLight, SpriteNode,
};
pub use pick::{pick, screen_ray, Aabb, Pick, Ray};
pub use post::{PostEffectKind, PostProcessEffect, PostProcessPipeline, ToneCurve};
pub use scene::{Scene, SceneError};
pub use transform::{rotation_looking_at, Transform};
