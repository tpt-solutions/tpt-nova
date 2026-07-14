# Changelog

All notable changes to TPT Nova are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/), and the project adheres to
semantic versioning once it leaves Alpha.

## [Unreleased]

### Added — Phase 4: Generative Bridges (continued)
- **`nova-splat`** (new crate): Gaussian Splat (3DGS) loading and collision-proxy
  generation.
  - `.splat` (antimatter15 32-byte) and `.ply` (ASCII + `binary_little_endian`,
    3DGS property set) loaders with SH-DC color, sigmoid opacity, `exp` scale,
    and quaternion activations.
  - `SplatCloud` ECS component and `Aabb` bounds.
  - Low-poly **convex-hull collider** generation
    (`build_convex_hull_collider`) feeding `nova-ingest`'s `Collider3D` for
    physics participation.
  - Optional `render` feature: a wgpu billboard `SplatPipeline` with an
    integration hook for `nova-render`.
- **`nova-overlay`** (new crate): "Highlight & Fix" viewport overlay tooling —
  screen-region math, entity picking by projected position, and structured
  `AiFixRequest` prompt generation for external coding agents.

### Added — Phase 5: Ecosystem, Shipping & Alpha
- **`nova-rag`** (new crate): dependency-free local vector DB (feature-hashing
  embeddings + cosine similarity), directory indexing, save/load, and a
  `RagAgent` that assembles a prompt-ready context block.
- **`nova-export`** (new crate + CLI): per-platform standalone packaging and a
  dependency-free `.novapack` asset-bundling container
  (`pack` / `unpack` / `bundle` subcommands; `PlatformTarget` for
  Windows/Linux/macOS).
- **`nova-agent-api`** (new crate): a **stable, versioned** external-AI-agent
  control API — `AgentCommand` set, `ControlChannel` hot-apply loop (protocol
  checked), and telemetry read-back. Formalizes the self-debugging loop.
- **`nova-videocap`** (new crate): video-to-ECS ingestion — depth map +
  segmentation mask → per-segment unprojected 3D points → convex-hull/box
  collision proxies ready for `nova-ingest` physics.
- **`nova-sample-game`** (new crate + demo binary): an end-to-end sample wiring
  ECS, Rapier3D physics, scene save/load, the agent API, splat ingestion, and
  asset packaging into one headless-tested pipeline.

### Changed
- **`nova-scene`**: bumped `CURRENT_SCENE_VERSION` to `2` with a real v1→v2
  migration (cameras gain a default directional `Light`), added a `validate`
  pass (duplicate ids, dangling parent/child references) enforced on load, and a
  `SceneError::Validation` variant for corruption handling.

## [0.1.0] — Pre-Alpha foundation
- ECS core, wgpu renderer, Rapier2D/3D physics, glTF/OBJ ingestion with VHACD,
  auto-rigging, skeletal animation, neural materials, audio, scripting
  (Rust hot-reload + Rhai), editor, UI, scene serialization, and telemetry with
  the AI code-injection loop. (Detailed phase history lives in `todo.md`.)
