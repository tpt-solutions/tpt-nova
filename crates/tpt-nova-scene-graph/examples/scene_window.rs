//! On-screen check: bake a scene graph and render it through the real GPU
//! `Renderer` in a window. Use `scene_bake` for the headless equivalent.
//!
//! Run: `cargo run -p tpt-nova-scene-graph --example scene_window --release`
//! Close the window or press Escape to exit. `SCENE_DEFAULTS=1` keeps the stock post
//! defaults instead of the harsher bloom used for the captured screenshot. Set `TPT_EXAMPLE_SECONDS=10` to
//! exit on its own (used for unattended screenshots).

use std::sync::Arc;
use std::time::{Duration, Instant};

use glam::{Vec2, Vec3};
use tpt_nova_ecs::World;
use tpt_nova_render::Renderer;
use tpt_nova_scene_graph::{
    bake, Color, DirectionalLight, Node, PerspectiveCamera, PostEffectKind, PostProcessEffect,
    Scene, SpriteNode, Transform,
};
use tpt_window::{
    new_event_loop, ElementState, EventQueue, EventSink, KeyCode, LogicalSize, PollStatus,
    WindowBuilder, WindowEvent, DEFAULT_CAPACITY,
};

fn build_scene() -> Scene {
    let mut scene = Scene::new().with_background(Color::hex("#101820"));

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

    // `SCENE_DEFAULTS=1` keeps the stock post stack (bloom, ACES, FXAA) so its
    // look can be judged on the GPU. Otherwise a harsher bloom is used.
    if std::env::var("SCENE_DEFAULTS").is_err() {
        scene.disable_post_process(PostEffectKind::Fxaa);
        scene.enable_post_process(PostProcessEffect::Bloom {
            threshold: 0.9,
            intensity: 0.4,
        });
    }
    scene
}

fn run() -> Result<(), String> {
    let limit = std::env::var("TPT_EXAMPLE_SECONDS")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .map(Duration::from_secs_f64);
    let started = Instant::now();

    let mut event_loop = new_event_loop().map_err(|e| format!("event loop: {e:?}"))?;
    let window = Arc::new(
        WindowBuilder::new()
            .with_title("TPT Nova — scene graph")
            .with_inner_size(LogicalSize::new(1280.0, 720.0))
            .build(&event_loop)
            .map_err(|e| format!("window: {e:?}"))?,
    );
    let mut renderer =
        Renderer::new(Arc::clone(&window)).map_err(|e| format!("renderer: {e:?}"))?;

    let mut world = World::new();
    let spawned = bake(&build_scene()).spawn_into(&mut world);
    println!("spawned {} entities", spawned.len());

    let mut queue = EventQueue::with_capacity(DEFAULT_CAPACITY);
    let mut frames = 0u64;
    'main: loop {
        let status = event_loop.step(&mut EventSink::new(&mut queue), Duration::from_millis(16));
        for ev in queue.drain() {
            match ev.event {
                WindowEvent::CloseRequested => break 'main,
                WindowEvent::KeyboardInput {
                    state: ElementState::Pressed,
                    key_code: KeyCode::Escape,
                    ..
                } => break 'main,
                _ => {}
            }
        }
        renderer
            .render(&world)
            .map_err(|e| format!("render: {e:?}"))?;
        frames += 1;
        if status == PollStatus::Exit || limit.is_some_and(|l| started.elapsed() >= l) {
            break;
        }
    }
    println!("rendered {frames} frames");
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("scene_window failed: {e}");
        std::process::exit(1);
    }
}
