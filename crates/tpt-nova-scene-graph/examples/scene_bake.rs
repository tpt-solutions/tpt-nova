//! Headless example: build a small scene, bake it, and spawn the result into an
//! ECS world. Runs without a window or GPU, and shows exactly what the renderer
//! receives: a flattened camera, meshes, lights, and the post stack.

use glam::{Vec2, Vec3};
use tpt_nova_ecs::World;
use tpt_nova_scene_graph::{
    bake, Color, DirectionalLight, Node, PerspectiveCamera, PostEffectKind, PostProcessEffect,
    Scene, SpriteNode, Transform,
};

fn main() {
    let mut scene = Scene::new().with_background(Color::hex("#101820"));

    // A pivot at height 1, with a cube offset from it: the cube bakes to y = 1.
    let pivot = scene.add_node(Node::group().named("pivot").at(Vec3::new(0.0, 1.0, 0.0)));
    scene
        .add_child(
            pivot,
            Node::cube().named("crate").at(Vec3::new(2.0, 0.0, 0.0)),
        )
        .expect("pivot exists");

    let cam = scene.add_node(
        Node::camera(PerspectiveCamera::default())
            .named("main")
            .with_transform(
                Transform::from_position(Vec3::new(0.0, 3.0, 8.0)).looking_at(Vec3::ZERO),
            ),
    );
    scene
        .set_active_camera(Some(cam))
        .expect("node is a camera");

    scene.add_node(Node::light(DirectionalLight::new(
        Vec3::new(-0.3, -1.0, -0.2),
        2.0,
    )));
    scene.add_node(Node::sprite(
        SpriteNode::new(0, Vec2::splat(64.0)).with_z(1.0),
    ));

    // Turn off FXAA and tune bloom for a harsher look.
    scene.disable_post_process(PostEffectKind::Fxaa);
    scene.enable_post_process(PostProcessEffect::Bloom {
        threshold: 0.9,
        intensity: 0.4,
    });

    let plan = bake(&scene);
    println!("camera: {}", plan.camera.is_some());
    for m in &plan.meshes {
        println!("mesh node {} at {:?}", m.node, m.transform.translation);
    }
    println!(
        "sprites: {}, lights: {}",
        plan.sprites.len(),
        plan.lights.len()
    );
    println!("post effects: {}", plan.post.len());

    let mut world = World::new();
    let spawned = plan.spawn_into(&mut world);
    println!("spawned {} entities into the ECS world", spawned.len());
}
