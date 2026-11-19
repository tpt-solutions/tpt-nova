# Changelog

All notable changes to TPT Nova are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/), and the project adheres to
semantic versioning once it leaves Alpha.

## [Unreleased]

### Changed
- **Crate prefix:** every workspace crate is now `tpt-nova-*` (package names,
  directories under `crates/`, and Rust import paths `tpt_nova_*`). Example:
  `nova-ecs` is now `tpt-nova-ecs`. The `nova-control.json` and
  `nova-telemetry.json` file names and the `_nova_gameplay_*` C ABI symbols are
  unchanged.
- **License:** dual-licensed under MIT OR Apache-2.0 (`LICENSE-MIT`,
  `LICENSE-APACHE`).
- **Crate metadata:** each crate has its own `README.md`, `CHANGELOG.md`,
  `keywords`, and `categories`.
- **Dependencies (license policy):** removed the Apache-2.0-only `rapier2d`,
  `rapier3d`, and `parry3d`, and the MPL-2.0 `symphonia` decoder stack.
  - `tpt-nova-physics` now uses an in-house 2D solver (circles, convex polygons,
    capsules approximated as stadiums; sequential impulses).
  - `tpt-nova-ingest` now uses an in-house 3D solver for convex-hull compounds
    (`solver3d`), an in-house quickhull (`hull`), and an in-house approximate
    convex decomposition. Decomposition is top-down: the most concave part is
    split by the cut with the smallest total child hull volume. Part boundaries
    differ from V-HACD, and the 3D solver has no sleeping or continuous collision,
    so fast small bodies can tunnel.
  - `tpt-nova-audio` decodes WAV in-house (rodio's WAV path needs `hound`,
    Apache-only), Ogg Vorbis via `lewton`, and MP3 via `minimp3`.
    FLAC is decoded by `tpt-av-cadence-flac` from the tpt-cadence suite (MIT OR Apache-2.0),
    pinned to a commit in `Cargo.toml`. rodio's own FLAC path needs `claxon`, so it is not used.
  - `tpt-nova-ingest::decompose_convex` takes `DecompositionParams` (was
    `VHACDParameters`). `tpt-nova-audio::Sound::decode` returns `Decoded`.
  - Still Apache-2.0-only via the platform stack, listed as exceptions in
    `deny.toml`: `winit`, `cpal`, and the wgpu/naga helpers. `smartstring` (MPL-2.0,
    via `rhai`) is also an exception.
- **`deny.toml`** at the workspace root enforces the license allow-list
  (MIT, BSD, ISC, Zlib, 0BSD, Unicode, CC0, Unlicense). Apache-2.0 is not
  allowed on its own, so Apache-only crates fail unless they have an exception.
  Dual-licensed `MIT OR Apache-2.0` crates pass.

### Added — Phase 4: Generative Bridges (continued)
- **`tpt-nova-splat`** (new crate): Gaussian Splat (3DGS) loading and collision-proxy
  generation.
  - `.splat` (antimatter15 32-byte) and `.ply` (ASCII + `binary_little_endian`,
    3DGS property set) loaders with SH-DC color, sigmoid opacity, `exp` scale,
    and quaternion activations.
  - `SplatCloud` ECS component and `Aabb` bounds.
  - Low-poly **convex-hull collider** generation
    (`build_convex_hull_collider`) feeding `tpt-nova-ingest`'s `Collider3D` for
    physics participation.
  - Optional `render` feature: a wgpu billboard `SplatPipeline` with an
    integration hook for `tpt-nova-render`.
- **`tpt-nova-overlay`** (new crate): "Highlight & Fix" viewport overlay tooling —
  screen-region math, entity picking by projected position, and structured
  `AiFixRequest` prompt generation for external coding agents.

### Added — Phase 5: Ecosystem, Shipping & Alpha
- **`tpt-nova-rag`** (new crate): dependency-free local vector DB (feature-hashing
  embeddings + cosine similarity), directory indexing, save/load, and a
  `RagAgent` that assembles a prompt-ready context block.
- **`tpt-nova-export`** (new crate + CLI): per-platform standalone packaging and a
  dependency-free `.novapack` asset-bundling container
  (`pack` / `unpack` / `bundle` subcommands; `PlatformTarget` for
  Windows/Linux/macOS).
- **`tpt-nova-agent-api`** (new crate): a **stable, versioned** external-AI-agent
  control API — `AgentCommand` set, `ControlChannel` hot-apply loop (protocol
  checked), and telemetry read-back. Formalizes the self-debugging loop.
- **`tpt-nova-videocap`** (new crate): video-to-ECS ingestion — depth map +
  segmentation mask → per-segment unprojected 3D points → convex-hull/box
  collision proxies ready for `tpt-nova-ingest` physics.
- **`tpt-nova-sample-game`** (new crate + demo binary): an end-to-end sample wiring
  ECS, Rapier3D physics, scene save/load, the agent API, splat ingestion, and
  asset packaging into one headless-tested pipeline.

### Changed
- **`tpt-nova-scene`**: bumped `CURRENT_SCENE_VERSION` to `2` with a real v1→v2
  migration (cameras gain a default directional `Light`), added a `validate`
  pass (duplicate ids, dangling parent/child references) enforced on load, and a
  `SceneError::Validation` variant for corruption handling.

## [0.1.0] — Pre-Alpha foundation
- ECS core, wgpu renderer, Rapier2D/3D physics, glTF/OBJ ingestion with VHACD,
  auto-rigging, skeletal animation, neural materials, audio, scripting
  (Rust hot-reload + Rhai), editor, UI, scene serialization, and telemetry with
  the AI code-injection loop. (Detailed phase history lives in `todo.md`.)
