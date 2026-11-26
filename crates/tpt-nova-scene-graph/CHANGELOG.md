# Changelog - tpt-nova-scene-graph

All notable changes to this crate are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/). Workspace-wide history is in
[../../CHANGELOG.md](../../CHANGELOG.md).

## [Unreleased]

### Added
- Initial crate: a declarative `Scene` / `Node` graph of meshes, sprites, cameras, and lights with local transforms. `bake` flattens it into a `RenderPlan`, and `RenderPlan::spawn_into` writes ECS components that the PBR and sprite pipelines read.
- Post-process pipeline description (`PostProcessPipeline`), lowered to `tpt-nova-render`'s `PostProcessChain`.
- Picking (`pick`): `screen_ray` unprojects a pixel through view-projection, and `pick` returns the nearest visible node. Each node is tested in local space, so rotation and scale are exact.
- `Animator` wrapper over `tpt-nova-anim`: `add_state`, `play` (hard cut), `crossfade`, `tick`, `current`, `is_blending`, and `skin_matrices`.
- Examples: `scene_bake` (no window) and `scene_window` (renders a baked scene through the real renderer; checked on a local GPU).
- Optional `appfront` feature: `AppfrontScene` implements `SceneDelegate` from `tpt-appfront-canvas-3d`. Presenting renders offscreen through `OffscreenRenderer`, `last_frame()` returns the frame, and `SceneDelegate::pixels` reads it back.
- Per-crate `README.md`, `CHANGELOG.md`, `keywords`, and `categories` metadata.
