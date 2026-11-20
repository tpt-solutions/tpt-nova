# tpt-nova-scene-graph

A declarative 3D scene graph for TPT Nova. A `Scene` is a plain, serializable
hierarchy of nodes (meshes, sprites, cameras, lights) with local transforms, an
active camera, environment settings, and a post-processing stack.

The crate holds no wgpu code. It produces data the existing renderer consumes:

- `bake(&scene)` flattens the hierarchy into a `RenderPlan` of world-space
  meshes, sprites, lights, the active camera, and a `PostProcessEffect` list.
- `RenderPlan::spawn_into(&mut World)` writes the plan as ECS entities using the
  components `tpt-nova-render`'s PBR and sprite pipelines read. The active camera
  is tagged `MainCamera`, so `tpt_nova_ecs::pick_main_camera` resolves it.
- `PostProcessPipeline::to_render_effects()` lowers the post stack into
  `tpt-nova-render`'s `PostProcessChain`. The "stunning defaults" are bloom,
  ACES tone mapping, and FXAA.

## Example

```sh
cargo run -p tpt-nova-scene-graph --example scene_bake
```

It builds a small scene, bakes it, and spawns it into an ECS world without
opening a window.

## Notes

- Directional lights store their travel direction; baking aims the ECS light's
  rotation so the forward axis matches it.
- Ambient light nodes are not drawn separately. The scene-level ambient term is
  the single one used.
- The plan is already in world space, so `spawn_into` writes no `Parent` links.
