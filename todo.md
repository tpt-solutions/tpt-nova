# TPT Nova — Build Checklist

## Phase 0: Repo & Tooling Bootstrap
- [x] git init, .gitignore, LICENSE (Apache-2.0), README stub
- [x] Cargo workspace skeleton (nova-ecs, nova-telemetry, nova-render, nova-app)
- [x] CI: build+test matrix for Windows/Linux/macOS

## Phase 1: "Hello Triangle", Telemetry & Foundations (Months 1-2)
### Rendering & App Shell
- [x] winit window opens on all 3 platforms
- [x] wgpu device/surface init, clear-color render loop
- [x] Draw a static 3D cube (vertex/index buffers, camera, MVP uniform)
### ECS Core
- [x] nova-ecs: entity/component storage, spawn/despawn, query iteration, scheduler
- [x] Represent the cube as an ECS entity (Transform, Mesh components)
- [x] Scene graph: parent/child Transform hierarchy, world-transform propagation
### Input
- [x] nova-input: keyboard/mouse polling wired through winit events into an ECS resource
- [x] Basic input-action mapping (e.g. "move_forward" bound to W/Up)
### Telemetry & AI Loop
- [x] nova-telemetry: JSON schema for entity/component state dump
- [x] Telemetry emission on a tick/interval (stdout or socket)
- [x] Code-injection loop: external process edits a component value via
      file-watch/IPC, engine hot-applies without restart
- [x] Prove the loop end-to-end: script reads telemetry JSON, mutates the
      cube's rotation, engine reflects it live
- [x] Deterministic fixed-timestep tick, seeded RNG plumbing

## Phase 2: 2D "Vibe" Sandbox + Playable-Game Basics (Months 3-5)
### Physics & Rendering
- [x] Integrate Rapier2D into nova-ecs (physics components + sync step)
- [x] 2D sprite rendering pipeline (batched quads, texture atlas)
### Audio
- [x] nova-audio: 2D sound playback (SFX one-shots, looping music), volume/mixing
### Gameplay Scripting (Rust-native, hot-reload)
- [x] nova-scripting v1: gameplay logic as a dylib crate, hot-reload watcher
- [x] Stable ABI/trait boundary between engine and hot-reloaded gameplay code
- [x] Example: a player-controlled 2D entity driven entirely by hot-reloaded logic
### In-Game UI
- [x] nova-ui v1: 2D immediate-mode-style widgets (text, button, panel) renderable in-game
### Save/Load
- [x] Scene serialization: dump/restore full ECS world state to disk (RON/JSON)
- [x] Versioning strategy for saved scenes as components evolve
### Scene/Level Editor v1
- [x] Editor framework decision (Tauri vs ImGui) — RESOLVED: egui/eframe (see Open Decisions)
- [x] Scene hierarchy panel (list entities, parent/child tree)
- [x] Component inspector (view/edit component fields on selected entity)
- [x] 2D viewport gizmos: move/rotate/scale selected entity
### Vibe GUI
- [x] "Vibe GUI" v1: visual bezier/curve editor for one parameter (e.g. gravity)
- [x] Curve edits translate to physics constraint changes live (round-trip to Rust)

## Phase 3: 3D Cinematic Core & Smart Ingestion (Months 6-9)
### Rendering
- [x] Forward+ (or simple forward) 3D PBR rendering pipeline in nova-render
- [x] Dynamic lighting (at least one shadow-casting light type)
- [x] Virtual camera component/system
### Animation
- [x] nova-anim: skeletal animation playback (bone hierarchy, keyframe sampling)
- [x] Animation blending/state machine (idle/walk/run style transitions)
### 3D Editor & UI
- [x] Scene editor: 3D viewport gizmos (move/rotate/scale in 3D, snapping)
- [x] nova-ui: world-space UI support (e.g. floating nameplates, in-world panels)
### Smart Mesh Ingestion
- [x] nova-ingest: .glb/.obj mesh loader
- [x] VHACD convex decomposition for auto-generated colliders
- [x] Auto-rigging pipeline (evaluate/integrate an existing algorithm or crate)
- [x] Rapier3D integration for ingested mesh colliders
- [x] Demo: drag a Meshy .glb in, get collider + rig automatically

## Phase 4: Generative Bridges, Neural Materials & Scripting Expansion (Months 10-12)
### Generative Pipelines
- [ ] Gaussian Splat renderer/loader integration
- [ ] Low-poly convex hull collider overlay generation for splats
- [x] nova-neural-materials crate: API contract for live video-LLM texture feeds
- [x] Map a live/streamed video texture onto 3D geometry in real time
      (NeuralTexture GPU upload + NeuralMaterialRegistry; PBR-material binding
      into nova-render is a follow-up)
- [ ] "Highlight & Fix" viewport overlay tool (select region -> AI fix prompt)
- [ ] Video-to-ECS pipeline: depth map + segmentation mask ingestion for
      collision-proxy generation
### Scripting Expansion
- [x] Embedded scripting layer (Rhai) added alongside Rust hot-reload
- [x] Sandboxing/capability boundaries for AI-generated embedded scripts
- [x] Decide which gameplay surfaces target Rust-native vs embedded scripting
      (RESOLVED: Rust hot-reload for shipped/perf-critical systems; Rhai for
      AI-generated, sandboxed, hot-iterated logic — see Open Decisions)
### Audio Expansion
- [ ] 3D spatial audio (positional sources, listener-relative attenuation)

## Phase 5: Ecosystem, Shipping Pipeline & Alpha (Months 13+)
### Ship-a-Game Pipeline
- [ ] nova-export: build/package a standalone binary per platform (Win/Linux/macOS)
- [ ] Asset packing/bundling for distribution (not raw loose files)
- [ ] Save/load hardening: migrations, corruption handling, versioned schemas
### AI Ecosystem
- [ ] Public/stable API surface for external AI agents to drive the engine
- [ ] nova-rag crate: local vector DB indexing project assets/docs
- [ ] RAG-backed doc/context queries wired into AI agent integration
### Release
- [ ] Public Alpha release checklist (docs, licensing review, changelog)
- [ ] Sample game(s) built end-to-end using the full pipeline as a proof point

## Testing Coverage (Cross-Cutting)
Audit (2026-07-14): `nova-ecs`, `nova-app`, and `nova-input` initially had **zero**
`#[test]` coverage; most other crates had only 1-3 test files. This section tracks
closing those gaps — check items off as tests land, don't infer coverage from a
crate's existence.

Progress (2026-07-14): `nova-ecs` (20 tests), `nova-input` (13 tests), and `nova-app`
(5 tests) now have coverage. `nova-telemetry` and `nova-scene` already shipped tests
in their `lib.rs`. Remaining: `nova-render`, `nova-physics`, `nova-anim`, `nova-ingest`,
`nova-ui`, `nova-editor`, `nova-scripting`, `nova-scripting-embedded`, `nova-audio`,
and the cross-cutting/CI items.
### nova-ecs
- [x] Entity spawn/despawn unit tests (including despawn-while-iterating safety)
- [x] Component storage/query iteration tests (single/multi-component queries, empty results)
- [x] Scheduler ordering/dependency tests
- [x] Scene graph parent/child hierarchy tests (reparenting, world-transform propagation, cycles rejected)
- [x] Serde round-trip tests for ECS state (used by telemetry + scene save/load)
- [x] Deterministic RNG tests (same seed -> identical stream, zero seed normalized)
### nova-app
- [x] Headless/CI-safe smoke test for app shell init (world/scheduler/resources built without a window or GPU; see note)
- [x] Fixed-timestep loop tests (tick accumulation, seeded RNG determinism across runs)
### nova-input
- [x] Keyboard/mouse event -> ECS resource mapping tests
- [x] Input-action binding tests (e.g. "move_forward" -> W/Up, rebinding, unbound key no-ops)
### nova-render
- [ ] Cube pipeline tests beyond current coverage (MVP uniform correctness, camera math)
- [ ] 2D sprite batching/atlas pipeline tests (batch boundaries, atlas UV correctness)
- [ ] Forward PBR pipeline tests (shadow-casting light, material binding)
### nova-physics
- [ ] Rapier2D/3D sync step tests (component <-> physics-body state round-trip)
- [ ] Determinism regression tests (same seed/inputs -> identical simulation trace across runs/platforms)
### nova-telemetry
- [ ] JSON schema round-trip tests for entity/component state dumps
- [ ] MessagePack sink round-trip + JSON/MessagePack parity tests
- [ ] Tick/interval emission timing tests
### nova-scene
- [ ] Save/load round-trip tests for full ECS world state
- [ ] Versioned migration tests (old-version file -> current schema)
- [ ] Corrupt/malformed scene file handling tests
### nova-scripting
- [ ] Hot-reload lifecycle tests (load, swap, unload dylib)
- [ ] C ABI/trait boundary contract tests (stable across a rebuild)
### nova-scripting-embedded
- [ ] Capability-boundary tests (denied capability's function is genuinely absent, not just unreachable)
- [ ] ScriptCommand generation/application round-trip tests
- [ ] Sandbox escape attempt tests (no filesystem/network access from scripts)
### nova-ui
- [ ] Widget draw-list generation tests (text/button/panel layout)
- [ ] World-space anchored widget positioning tests
### nova-editor
- [ ] Hierarchy panel tests (entity list, parent/child tree updates)
- [ ] Component inspector edit round-trip tests (UI edit -> ECS component value)
- [ ] 2D/3D gizmo interaction tests (move/rotate/scale, snapping)
- [ ] Vibe GUI curve-edit -> physics constraint round-trip tests
### nova-anim
- [ ] Keyframe sampling edge-case tests (before first/after last keyframe, single-keyframe clips)
- [ ] Animation blending/state-machine transition tests (idle/walk/run)
### nova-ingest
- [ ] Malformed/unsupported .glb and .obj file handling tests
- [ ] VHACD convex decomposition edge cases (degenerate/non-manifold meshes)
- [ ] Auto-rigging pipeline tests on varied mesh topologies
- [ ] Rapier3D collider generation correctness tests
### nova-neural-materials
- [ ] FrameSource/NeuralMaterialProvider contract tests beyond MockProvider happy path
- [ ] Error/reconnect handling tests (dropped stream, malformed frame)
- [ ] NeuralTexture GPU upload + NeuralMaterialRegistry tests
### nova-audio
- [ ] Mixing/bus volume tests (SFX vs music bus interaction)
- [ ] Looping edge-case tests (seamless loop points, stop-during-loop)
### Cross-cutting / CI
- [ ] Wire up coverage reporting (e.g. `cargo tarpaulin` or `grcov`) into CI and track a baseline
- [ ] End-to-end integration test spanning multiple crates (ECS + physics + telemetry tick, asserting deterministic output)
- [ ] Regression test harness for the AI code-injection loop (telemetry read -> mutate -> hot-apply -> verify)

## Open Decisions
- [x] Editor framework: **RESOLVED — egui/eframe (Rust-native immediate-mode)** over
      Tauri/ImGui. Rationale: pure-Rust, integrates directly with the existing
      winit+wgpu stack (egui-wgpu/egui-winit), no IPC boundary or C++ toolchain,
      and its immediate-mode model matches the in-game `nova-ui` approach so tooling
      and runtime UI can share concepts.
- [x] MessagePack adoption: **RESOLVED — adopt now as an optional encoding** alongside
      JSON. Telemetry frames stay JSON by default (human/AI readable) with an opt-in
      MessagePack sink for high-frequency/large payloads via `rmp-serde`.
- [x] Embedded scripting language choice — **RESOLVED — Rhai** over WASM. Rationale:
      Rhai is a tiny, pure-Rust, safe-by-default interpreter (no filesystem/network
      surface, no `eval` unless explicitly enabled) that embeds with zero external
      toolchain and compiles in milliseconds, making it ideal for the hot-iterated,
      AI-generated logic the engine targets. Capability boundaries are enforced by
      *registering only the functions a script is granted* (a denied capability's
      function simply does not exist). WASM remains a possible future option for
      fully untrusted, portable modules, but Rhai satisfies the Phase 4 sandboxing
      requirement now. Split: Rust hot-reload (`nova-scripting`) for shipped /
      performance-critical systems; Rhai (`nova-scripting-embedded`) for AI-generated
      sandboxed logic.
- [ ] Networking/multiplayer — not in spec or current scope; revisit if needed post-Alpha
