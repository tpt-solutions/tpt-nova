//! Bake a [`Scene`] into the flat data the renderer already consumes.
//!
//! [`bake`] flattens the hierarchy into world-space [`RenderPlan`] entries: one
//! per visible mesh, sprite, light, and the active camera. [`RenderPlan::spawn_into`]
//! then writes those as ECS entities using the components `tpt-nova-render`'s PBR
//! and sprite pipelines (and `tpt_nova_ecs::pick_main_camera`) read, so the
//! scene graph drives the existing renderer without a second code path.

use glam::{Mat4, Quat, Vec3};
use tpt_nova_ecs::{Camera, Entity, GlobalTransform, Light, MainCamera, Mesh, Transform, World};
use tpt_nova_render::post::Effect;
use tpt_nova_render::Sprite;

use crate::color::Color;
use crate::id::NodeId;
use crate::node::{AmbientLight, LightSource, NodeKind};
use crate::scene::Scene;
use crate::transform::rotation_looking_at;

/// A mesh drawn in world space.
#[derive(Debug, Clone)]
pub struct PlannedMesh {
    pub node: NodeId,
    pub transform: Transform,
    pub mesh: Mesh,
}

/// A sprite drawn in world space.
#[derive(Debug, Clone)]
pub struct PlannedSprite {
    pub node: NodeId,
    pub transform: Transform,
    pub sprite: Sprite,
}

/// A light in world space. Directional lights carry their rotation so the ECS
/// forward axis equals the authored direction.
#[derive(Debug, Clone)]
pub struct PlannedLight {
    pub node: NodeId,
    pub transform: Transform,
    pub light: Light,
}

/// The active camera in world space.
#[derive(Debug, Clone)]
pub struct PlannedCamera {
    pub node: NodeId,
    pub transform: Transform,
    pub camera: Camera,
}

/// Everything the renderer needs for one frame, already flattened.
#[derive(Debug, Clone, Default)]
pub struct RenderPlan {
    pub camera: Option<PlannedCamera>,
    pub meshes: Vec<PlannedMesh>,
    pub sprites: Vec<PlannedSprite>,
    pub lights: Vec<PlannedLight>,
    pub ambient: AmbientLight,
    pub background: Color,
    /// Post-process stack, in run order, ready for `PostProcessChain::set_effects`.
    pub post: Vec<Effect>,
}

/// Bake `scene` into a [`RenderPlan`].
///
/// Hidden nodes (or nodes with a hidden ancestor) are skipped. Ambient light
/// nodes are not drawn separately: the scene-level [`Scene::ambient`] is the
/// single ambient term, which matches the PBR shader's uniform ambient.
pub fn bake(scene: &Scene) -> RenderPlan {
    let mut plan = RenderPlan {
        ambient: scene.ambient(),
        background: scene.background(),
        post: scene.post_process().to_render_effects(),
        ..Default::default()
    };

    if let Some(cam_id) = scene.active_camera() {
        if let (Some(node), Some(world)) = (scene.get(cam_id), world_transform(scene, cam_id)) {
            if let NodeKind::Camera(cam) = &node.kind {
                plan.camera = Some(PlannedCamera {
                    node: cam_id,
                    transform: world,
                    camera: cam.to_ecs(),
                });
            }
        }
    }

    for id in scene.depth_first() {
        if !scene.visible_in_tree(id) {
            continue;
        }
        let Some(node) = scene.get(id) else { continue };
        let Some(world) = world_transform(scene, id) else {
            continue;
        };
        match &node.kind {
            NodeKind::Mesh(mesh) => plan.meshes.push(PlannedMesh {
                node: id,
                transform: world,
                mesh: mesh.to_ecs(),
            }),
            NodeKind::Sprite(sprite) => plan.sprites.push(PlannedSprite {
                node: id,
                transform: world,
                sprite: sprite.to_ecs(),
            }),
            NodeKind::Light(LightSource::Directional(dir)) => {
                // The ECS light reads its direction from rotation, so aim the
                // local -Z axis along the authored travel direction.
                let transform = Transform {
                    rotation: rotation_looking_at(dir.direction, Vec3::Y),
                    ..world
                };
                plan.lights.push(PlannedLight {
                    node: id,
                    transform,
                    light: dir.to_ecs(),
                });
            }
            NodeKind::Light(LightSource::Point(point)) => plan.lights.push(PlannedLight {
                node: id,
                transform: world,
                light: point.to_ecs(),
            }),
            NodeKind::Light(LightSource::Ambient(_)) | NodeKind::Group | NodeKind::Camera(_) => {}
        }
    }

    plan
}

impl RenderPlan {
    /// Spawn the plan as ECS entities. The active camera entity is tagged with
    /// [`MainCamera`] so `tpt_nova_ecs::pick_main_camera` resolves it.
    ///
    /// Returns the spawned entities in the order: camera, meshes, sprites, lights.
    /// Only the flattened world transform is written; no `Parent` links are made,
    /// because the plan is already in world space.
    pub fn spawn_into(&self, world: &mut World) -> Vec<Entity> {
        let mut spawned = Vec::new();

        if let Some(cam) = &self.camera {
            let e = spawn_at(world, &cam.transform);
            world.add_component(e, cam.camera);
            world.add_component(e, MainCamera::default());
            spawned.push(e);
        }
        for m in &self.meshes {
            let e = spawn_at(world, &m.transform);
            world.add_component(e, m.mesh);
            spawned.push(e);
        }
        for s in &self.sprites {
            let e = spawn_at(world, &s.transform);
            world.add_component(e, s.sprite);
            spawned.push(e);
        }
        for l in &self.lights {
            let e = spawn_at(world, &l.transform);
            world.add_component(e, l.light);
            spawned.push(e);
        }
        // The scene background travels as a `ClearColor` entity, which the
        // renderer reads as the clear color.
        // Not a scene node, so it is not part of the returned handles.
        let bg = world.spawn();
        world.add_component(bg, tpt_nova_render::ClearColor(self.background.to_array()));
        spawned
    }
}

/// Spawn an entity carrying the local `Transform` and the matching
/// `GlobalTransform`. Plans are flattened, so both hold the same placement;
/// `GlobalTransform` is what `pick_main_camera` and the render path read.
fn spawn_at(world: &mut World, transform: &Transform) -> Entity {
    let e = world.spawn();
    world.add_component(e, *transform);
    world.add_component(e, GlobalTransform(transform.matrix()));
    e
}

/// The node's world-space transform, decomposed from its accumulated matrix.
fn world_transform(scene: &Scene, id: NodeId) -> Option<Transform> {
    let m: Mat4 = scene.world_matrix(id)?;
    let (scale, rotation, translation): (Vec3, Quat, Vec3) = m.to_scale_rotation_translation();
    Some(Transform {
        translation,
        rotation,
        scale,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::{DirectionalLight, Node, PerspectiveCamera, PointLight, SpriteNode};
    use glam::Vec2;

    #[test]
    fn bake_flattens_world_transforms() {
        let mut scene = Scene::new();
        let root = scene.add_node(Node::group().at(Vec3::new(0.0, 1.0, 0.0)));
        scene
            .add_child(root, Node::cube().at(Vec3::new(1.0, 0.0, 0.0)))
            .unwrap();
        let plan = bake(&scene);
        assert_eq!(plan.meshes.len(), 1);
        let t = plan.meshes[0].transform;
        assert!((t.translation - Vec3::new(1.0, 1.0, 0.0)).length() < 1e-5);
    }

    #[test]
    fn hidden_subtrees_are_not_baked() {
        let mut scene = Scene::new();
        let root = scene.add_node(Node::group().hidden());
        scene.add_child(root, Node::cube()).unwrap();
        scene.add_node(Node::sprite(SpriteNode::new(3, Vec2::splat(8.0))));
        let plan = bake(&scene);
        assert!(plan.meshes.is_empty());
        assert_eq!(plan.sprites.len(), 1);
        assert_eq!(plan.sprites[0].sprite.region, 3);
    }

    #[test]
    fn directional_light_direction_survives_the_bake() {
        let mut scene = Scene::new();
        let dir = Vec3::new(0.0, -1.0, 0.0);
        scene.add_node(Node::light(DirectionalLight::new(dir, 2.0)));
        let plan = bake(&scene);
        assert_eq!(plan.lights.len(), 1);
        let fwd = plan.lights[0].transform.rotation * Vec3::NEG_Z;
        assert!((fwd - dir).length() < 1e-4, "forward = {fwd:?}");
        assert_eq!(plan.lights[0].light.intensity, 2.0);
    }

    #[test]
    fn point_lights_keep_their_position() {
        let mut scene = Scene::new();
        scene.add_node(Node::light(PointLight::new(4.0, 8.0)).at(Vec3::new(0.0, 2.0, 0.0)));
        let plan = bake(&scene);
        assert_eq!(plan.lights.len(), 1);
        assert_eq!(
            plan.lights[0].transform.translation,
            Vec3::new(0.0, 2.0, 0.0)
        );
    }

    #[test]
    fn active_camera_is_baked_and_tagged_on_spawn() {
        let mut scene = Scene::new();
        let cam =
            scene.add_node(Node::camera(PerspectiveCamera::default()).at(Vec3::new(0.0, 0.0, 5.0)));
        let plan = bake(&scene);
        let baked = plan.camera.as_ref().expect("camera baked");
        assert_eq!(baked.node, cam);

        let mut world = World::new();
        let spawned = plan.spawn_into(&mut world);
        assert_eq!(spawned.len(), 1);
        let shot = tpt_nova_ecs::pick_main_camera(&world, 16.0 / 9.0);
        assert!(shot.is_some(), "spawned camera should be picked as main");
    }

    #[test]
    fn post_stack_and_background_are_carried_into_the_plan() {
        let scene = Scene::new();
        let plan = bake(&scene);
        assert_eq!(plan.post.len(), 3);
        assert_eq!(plan.background, scene.background());
    }

    #[test]
    fn spawn_carries_the_scene_background_as_clear_color() {
        let mut scene = Scene::new().with_background(Color::hex("#101820"));
        scene.add_node(Node::cube());
        let plan = bake(&scene);
        let mut world = World::new();
        let spawned = plan.spawn_into(&mut world);
        assert_eq!(spawned.len(), 1, "background is not a scene node");
        let clear = world.query_1::<tpt_nova_render::ClearColor>();
        assert_eq!(clear.len(), 1);
        assert_eq!(clear[0].1 .0, scene.background().to_array());
    }

    #[test]
    fn spawn_writes_renderable_components() {
        let mut scene = Scene::new();
        scene.add_node(Node::cube());
        scene.add_node(Node::sprite(SpriteNode::default()));
        let plan = bake(&scene);
        let mut world = World::new();
        let spawned = plan.spawn_into(&mut world);
        assert_eq!(spawned.len(), 2);
        assert!(world.get_component::<Mesh>(spawned[0]).is_some());
        assert!(world.get_component::<Sprite>(spawned[1]).is_some());
    }
}
