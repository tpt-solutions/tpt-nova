# Changelog

All notable changes to TPT Nova are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/), and the project adheres to
semantic versioning once it leaves Alpha.

## [Unreleased]

### Fixed
- **Multiple cubes in one pass:** the live renderer wrote every cube's transform into one
  shared uniform buffer inside the draw loop. All queued writes take effect at submit, so
  every cube in a frame drew with the last transform. Each draw now has its own uniform
  buffer and bind group (`tpt-nova-render`).
- **Scene background dropped:** the renderer now honours the scene background through a
  `ClearColor` component, falling back to the engine default (`tpt-nova-render`).
- **Deterministic entity order:** `World::entities()` returned hash order, which changes per
  process. It is now sorted by index, so a tick visits entities the same way every run
  (`tpt-nova-ecs`).
- **`getrandom` on wasm32:** uses the `wasm_js` backend (`tpt-nova-app`).

### Added
- **Offscreen rendering:** `tpt_nova_render::OffscreenRenderer` renders the cube pass into an
  RGBA8 texture and reads it back, with no window. `AppfrontScene::present` uses it, so the
  appfront bridge now produces frames (`last_frame()`) instead of always reporting
  unsupported.
- **`tpt-nova-scene-graph`** (new crate): a declarative `Scene` / `Node` graph that bakes into
  ECS components for the PBR and sprite pipelines, a post-process description lowered to
  `tpt-nova-render`'s `PostProcessChain`, screen-ray picking, an `Animator` over
  `tpt-nova-anim`, and an opt-in `appfront` bridge.
- **`tpt-nova-drivers`** (new crate, Phase 8): hot-swappable behaviour drivers with an
  `arc-swap` versioned registry, a WASM sandbox (`wasm-sandbox`), trusted native drivers
  (`native-loader`), shadow comparison, an internal exact verifier, anomaly detection, and a
  verified self-repair loop. The strict registry admits a driver only with a `Verified` proof.
- **Browser build and PWA:** `tpt-nova-app` builds for `wasm32` (async renderer, rAF-driven
  loop) and ships an installable PWA (manifest, icons, cache-first service worker) through
  `scripts/build_web.sh`.
- **Post-processing and PBR seam:** `tpt-nova-render` gains a post-processing chain (bloom,
  tone mapping) and a `PbrScene` extraction step, so PBR frames can be checked headlessly.
- **Splat editing and rendering:** `tpt-nova-splat` gains in-place editing, spatial chunking
  and LOD, spherical-harmonic colour, `.compressed.ply` loading, multi-part collision
  proxies, and a depth-sorted renderer. The app spawns a demo splat cloud, and Highlight & Fix
  includes the selected splat ranges in its prompt. The agent API exposes splat commands.
- **Driver bundles:** `tpt-nova-export` stores drivers content-addressed under
  `drivers/<sha256>/` and refuses an artifact that does not match its address.
- **WAV and FLAC decoding:** `tpt-nova-audio` decodes WAV in-house and FLAC through the pinned
  tpt-cadence crates.

### Changed
- **RAG embeddings:** `tpt-nova-rag`'s `real-embeddings` feature no longer pulls in `fastembed`
  and its ONNX runtime. `RealEmbedder` calls a local embeddings server (Ollama or
  `llama-server --embedding`) over plain HTTP, so the feature adds no dependencies and the
  Apache-2.0-only and CDLA license failures from that tree are gone.
- **Windowing:** the editor and input use `tpt-window` instead of `winit`. The renderer creates
  its surface from `tpt-window` raw handles, and the app runs a caller-owned loop. `tpt-window`
  and `tpt-wasm` are git dependencies from `github.com/tpt-solutions`, allowed in `deny.toml`.
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
    `deny.toml`: `cpal` (audio output), and the wgpu/naga helpers. `smartstring` (MPL-2.0,
    via `rhai`) is also an exception. The `winit` and its windowing exceptions were
    removed once the windowing shell moved to `tpt-window`.
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
