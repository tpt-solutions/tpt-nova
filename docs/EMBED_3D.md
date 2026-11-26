# Embedding a 3D Scene in a Rust App

Audience: Rust developers who want a 3D viewport inside their own window or UI,
using `tpt-nova-scene-graph` for the scene and `tpt-nova-render` for the GPU.

This guide covers what works today: a native window, a declarative scene, GPU
rendering, and pointer picking. It does not cover the `tpt-appfront` UI bridge.
An opt-in `appfront` feature adds that bridge. See section 6.

## 1. Build a scene

A `Scene` is a tree of nodes. Each node has a transform and a kind: a group, a
mesh (currently only a cube), a sprite, a camera, or a light.

```rust
use glam::{Vec2, Vec3};
use tpt_nova_scene_graph::{
    Color, DirectionalLight, Node, PerspectiveCamera, Scene, SpriteNode, Transform,
};

let mut scene = Scene::new().with_background(Color::hex("#101820"));

let pivot = scene.add_node(Node::group().named("pivot").at(Vec3::new(0.0, 1.0, 0.0)));
scene.add_child(pivot, Node::cube().named("crate").at(Vec3::new(2.0, 0.0, 0.0)))?;

let cam = scene.add_node(
    Node::camera(PerspectiveCamera::default())
        .with_transform(Transform::from_position(Vec3::new(0.0, 3.0, 8.0)).looking_at(Vec3::ZERO)),
);
scene.set_active_camera(Some(cam))?;

scene.add_node(Node::light(DirectionalLight::new(Vec3::new(-0.3, -1.0, -0.2), 2.0)));
```

Children inherit their parent's transform, so moving `pivot` moves the crate.

## 2. Bake and spawn into the ECS world

`bake` flattens the scene into a `RenderPlan`. `spawn_into` writes the plan into a
`tpt_nova_ecs::World` as the components the renderer reads. The scene background
is written as a `ClearColor`.

```rust
use tpt_nova_ecs::World;

let mut world = World::new();
let spawned = tpt_nova_scene_graph::bake(&scene).spawn_into(&mut world);
```

`spawned` holds the entities for scene nodes. The background entity is not in it.

## 3. Render through a window

The renderer draws into any `tpt_window::Window`. You own the event loop.

```rust
use std::sync::Arc;
use std::time::Duration;
use tpt_nova_render::Renderer;
use tpt_window::{new_event_loop, EventQueue, EventSink, PollStatus, WindowBuilder, WindowEvent, DEFAULT_CAPACITY, LogicalSize};

let mut event_loop = new_event_loop()?;
let window = Arc::new(
    WindowBuilder::new()
        .with_title("my app")
        .with_inner_size(LogicalSize::new(1280.0, 720.0))
        .build(&event_loop)?,
);
let mut renderer = Renderer::new(Arc::clone(&window))?;

let mut queue = EventQueue::with_capacity(DEFAULT_CAPACITY);
'main: loop {
    let status = event_loop.step(&mut EventSink::new(&mut queue), Duration::from_millis(16));
    for ev in queue.drain() {
        if ev.event == WindowEvent::CloseRequested {
            break 'main;
        }
    }
    renderer.render(&world)?;
    if status == PollStatus::Exit {
        break;
    }
}
```

`scene_window` in `crates/tpt-nova-scene-graph/examples/` is this loop with the
scene from above. Run it with `cargo run -p tpt-nova-scene-graph --example scene_window --release`.

## 4. Pointer picking

`pick` turns a pixel into a world ray and returns the nearest visible cube.

```rust
use glam::Vec2;
use tpt_nova_scene_graph::pick;

let viewport = Vec2::new(1280.0, 720.0);
if let Some(hit) = pick(&scene, cam, viewport, Vec2::new(mouse_x, mouse_y)) {
    println!("hit node {:?} at {:?}", hit.node, hit.point);
}
```

Pixels have their origin at the top left, with y pointing down. Picking handles
rotation and non-uniform scale. Hidden nodes are skipped. It is a ray test
against the unit cube, so it does not cover other mesh shapes or physics
colliders. For those, use `tpt-nova-physics`.

## 5. Post-processing and the render target

The scene's post stack (bloom, ACES tone mapping, FXAA) is enabled by default.
Change it with `scene.enable_post_process` and `scene.disable_post_process`. The
`scene_window` example shows the stock defaults under `SCENE_DEFAULTS=1`.

![A cube on the scene background](images/scene_window.png)

## 6. Hosting in an appfront viewport (feature `appfront`)

`AppfrontScene` implements `SceneDelegate` from `tpt-appfront-canvas-3d`. The
trait is in `tpt-appfront` (crate `tpt-appfront-canvas-3d`, on GitHub). Enable the feature:

```sh
cargo test -p tpt-nova-scene-graph --features appfront
```

```rust
use tpt_appfront_canvas_3d::{Viewport3D, ViewportPoint, ViewportSize};
use tpt_nova_scene_graph::appfront::AppfrontScene;

let mut vp = Viewport3D::new(
    AppfrontScene::new(scene, cam)?,
    ViewportSize { width: 1280.0, height: 720.0 },
);
if let Some(hit) = vp.pointer_down(ViewportPoint { x: mouse_x, y: mouse_y }) {
    println!("3D object {} at {:?}", hit.id, hit.point);
}
```

Picking works. Presenting pixels does not: the GPU renderer has no readback into
the appfront canvas, so `frame` returns `PresentStatus::Unsupported`. The
dependency is a git dependency on `tpt-solutions/tpt-appfront`.

## 7. Web (wasm32)

`tpt-nova-scene-graph`, `tpt-nova-render`, and `tpt-nova-app` compile for
`wasm32-unknown-unknown`. The app needs the `getrandom` backend set in
`.cargo/config.toml`. These builds are compile-checked only. Nothing here has been
run in a browser yet.

## What is not available

- **GPU output inside appfront.** Presenting into the appfront canvas needs a
  readback path that does not exist yet. Picking works.
- **Animation wrapper.** Animate transforms yourself for now. spec3 proposes a
  higher-level `Animator`, which is not built.
- **Non-cube meshes in the scene graph.** `MeshKind` has only `Cube`. Load other
  geometry through `tpt-nova-ingest` and spawn it as ECS components.
