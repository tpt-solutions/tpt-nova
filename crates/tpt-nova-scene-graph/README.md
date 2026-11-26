# tpt-nova-scene-graph

> Declarative 3D scene graph for TPT Nova: bakes into the ECS components the renderer already reads

A declarative 3D scene graph for TPT Nova. A `Scene` is a plain, serializable
hierarchy of nodes (meshes, sprites, cameras, lights) with local transforms, an
active camera, environment settings, and a post-processing stack.

Part of [TPT Nova](../../README.md), the AI-native, ECS-based real-time engine.

The crate holds no wgpu code of its own. It produces data the existing renderer consumes:

- `bake(&scene)` flattens the hierarchy into a `RenderPlan` of world-space
  meshes, sprites, lights, the active camera, and a `PostProcessEffect` list.
- `RenderPlan::spawn_into(&mut World)` writes the plan as ECS entities using the
  components `tpt-nova-render`'s PBR and sprite pipelines read. The active camera
  is tagged `MainCamera`, so `tpt_nova_ecs::pick_main_camera` resolves it.
- `PostProcessPipeline::to_render_effects()` lowers the post stack into
  `tpt-nova-render`'s `PostProcessChain`. The "stunning defaults" are bloom,
  ACES tone mapping, and FXAA.
- `pick` / `screen_ray` map a pixel to the nearest visible node. Each node is
  tested in local space, so rotation and scale are exact.
- `Animator` wraps `tpt-nova-anim`'s state machine with `add_state`, `play`
  (hard cut), `crossfade`, `tick`, `current`, and `skin_matrices`.

## Features

| Feature | Enables |
|---------|---------|
| `appfront` | `appfront::AppfrontScene`, an opt-in implementation of `SceneDelegate` from `tpt-appfront-canvas-3d` (a git dependency). Presenting renders offscreen through `tpt-nova-render`'s `OffscreenRenderer`; `last_frame()` and `SceneDelegate::pixels` return the frame. |

## Examples

```sh
cargo run -p tpt-nova-scene-graph --example scene_bake
cargo run -p tpt-nova-scene-graph --example scene_window --release   # needs a GPU
```

- `scene_bake` builds a small scene, bakes it, and spawns it into an ECS world without opening a window.
- `scene_window` renders a baked scene through the real renderer in a `tpt-window` window.

## Notes

- Directional lights store their travel direction; baking aims the ECS light's
  rotation so the forward axis matches it.
- Ambient light nodes are not drawn separately. The scene-level ambient term is
  the single one used.
- The plan is already in world space, so `spawn_into` writes no `Parent` links.

## Workspace dependencies

- `tpt-nova-ecs`
- `tpt-nova-render`
- `tpt-nova-anim`

## Testing

```sh
cargo test -p tpt-nova-scene-graph
cargo test -p tpt-nova-scene-graph --features appfront
```

## License

Dual-licensed under either of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE), at your option.
