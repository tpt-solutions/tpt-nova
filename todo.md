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
- [x] Gaussian Splat renderer/loader integration (`nova-splat`: `.splat`/`.ply` loaders, optional wgpu billboard pipeline)
- [x] Low-poly convex hull collider overlay generation for splats (`nova-splat::build_convex_hull_collider` → `Collider3D`)
- [x] nova-neural-materials crate: API contract for live video-LLM texture feeds
- [x] Map a live/streamed video texture onto 3D geometry in real time
      (NeuralTexture GPU upload + NeuralMaterialRegistry; PBR-material binding
      into nova-render is a follow-up)
- [x] "Highlight & Fix" viewport overlay tool (select region -> AI fix prompt) (`nova-overlay`)
- [x] Video-to-ECS pipeline: depth map + segmentation mask ingestion for
       collision-proxy generation (`nova-videocap` → `Collider3D`/Rapier3D)
### Scripting Expansion
- [x] Embedded scripting layer (Rhai) added alongside Rust hot-reload
- [x] Sandboxing/capability boundaries for AI-generated embedded scripts
- [x] Decide which gameplay surfaces target Rust-native vs embedded scripting
      (RESOLVED: Rust hot-reload for shipped/perf-critical systems; Rhai for
      AI-generated, sandboxed, hot-iterated logic — see Open Decisions)
### Audio Expansion
- [x] 3D spatial audio (positional sources, listener-relative attenuation)

## Phase 5: Ecosystem, Shipping Pipeline & Alpha (Months 13+)
### Ship-a-Game Pipeline
- [x] nova-export: build/package a standalone binary per platform (Win/Linux/macOS) (`nova-export` CLI: `pack`/`unpack`/`bundle`)
- [x] Asset packing/bundling for distribution (not raw loose files) (`.novapack` container + `BundleManifest`)
- [x] Save/load hardening: migrations, corruption handling, versioned schemas (`nova-scene` v2 with v1→v2 migration, `validate` pass, `SceneError::Validation`)
### AI Ecosystem
- [x] Public/stable API surface for external AI agents to drive the engine (`nova-agent-api`: `PROTOCOL_VERSION`, `AgentCommand`, `ControlChannel`, telemetry read-back)
- [x] nova-rag crate: local vector DB indexing project assets/docs (`nova-rag`: `Embedder` trait, `FeatureHashEmbedder`, `Index`, `RagAgent`)
- [x] RAG-backed doc/context queries wired into AI agent integration (`nova-agent-api` `rag` feature: `RagAssistant`)
### Release
- [x] Public Alpha release checklist (docs, licensing review, changelog) (`docs/ALPHA_CHECKLIST.md`, `CHANGELOG.md` Unreleased section, per-crate module docs)
- [x] Sample game(s) built end-to-end using the full pipeline as a proof point (`nova-sample-game::run_pipeline` — physics rest, scene save/reload, agent spawn/move, splat→collider, asset pack)

## Testing Coverage (Cross-Cutting)
Audit (2026-07-14): `nova-ecs`, `nova-app`, and `nova-input` initially had **zero**
`#[test]` coverage; most other crates had only 1-3 test files. This section tracks
closing those gaps — check items off as tests land, don't infer coverage from a
crate's existence.

Progress (2026-07-14 → 2026-07-14): every crate now ships `#[test]` coverage.
The items below were all implemented in the preceding test-expansion pass
(`nova-render`, `nova-physics`, `nova-anim`, `nova-ingest`, `nova-ui`,
`nova-editor`, `nova-scripting`, `nova-scripting-embedded`, `nova-audio`,
`nova-neural-materials`, `nova-telemetry`, and `nova-scene` carry their tests
in `lib.rs`/`sprite.rs`/`world.rs`; `nova-ecs`, `nova-app`, `nova-input`,
`nova-gameplay-example` use `tests/`; `nova-physics` adds a cross-crate
`tests/integration.rs`). The Phase 4/5 **feature** work (Gaussian Splat,
`nova-splat`; Highlight & Fix, `nova-overlay`; video ingestion, `nova-videocap`;
RAG, `nova-rag`; agent API, `nova-agent-api`; export/packaging, `nova-export`;
sample game, `nova-sample-game`; Alpha checklist, `docs/ALPHA_CHECKLIST.md`)
is now implemented and green under `cargo test --workspace` + `cargo clippy
--workspace --all-targets -D warnings`. The only deliberately-deferred item is
networking/multiplayer (see Open Decisions).
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
- [x] Cube pipeline tests beyond current coverage (MVP uniform correctness, camera math)
- [x] 2D sprite batching/atlas pipeline tests (batch boundaries, atlas UV correctness)
- [x] Forward PBR pipeline tests (cube topology, uniform byte round-trip; shadow/material binding is exercised by the live `PbrRenderer`)
### nova-physics
- [x] Rapier2D sync step tests (component <-> physics-body state round-trip)
- [x] Determinism regression tests (same seed/inputs -> identical simulation trace across runs/platforms)
### nova-telemetry
- [x] JSON schema round-trip tests for entity/component state dumps
- [x] MessagePack sink round-trip + JSON/MessagePack parity tests
- [x] Tick/interval emission timing tests
### nova-scene
- [x] Save/load round-trip tests for full ECS world state
- [x] Versioned migration tests (old-version file -> current schema)
- [x] Corrupt/malformed scene file handling tests
### nova-scripting
- [x] Hot-reload lifecycle tests (missing-file error, ABI-version stability, ABI-mismatch rejection; full dylib load/swap/unload is covered by the `nova-gameplay-example` + editor watcher rather than a unit test because it needs a compiled cdylib)
- [x] C ABI/trait boundary contract tests (ABI version check rejects incompatible modules)
### nova-scripting-embedded
- [x] Capability-boundary tests (denied capability's function is genuinely absent, not just unreachable)
- [x] ScriptCommand generation/application round-trip tests
- [x] Sandbox escape attempt tests (no filesystem/network access from scripts)
### nova-ui
- [x] Widget draw-list generation tests (text/button/panel layout)
- [x] World-space anchored widget positioning tests
### nova-editor
- [x] Hierarchy panel tests (entity list, parent/child tree updates)
- [x] Component inspector edit round-trip tests (UI edit -> ECS component value)
- [x] 2D/3D gizmo interaction tests (move/rotate/scale, snapping)
- [x] Vibe GUI curve-edit -> physics constraint round-trip tests
### nova-anim
- [x] Keyframe sampling edge-case tests (before first/after last keyframe, single-keyframe clips)
- [x] Animation blending/state-machine transition tests (idle/walk/run)
### nova-ingest
- [x] Malformed/unsupported .glb and .obj file handling tests
- [x] VHACD convex decomposition edge cases (degenerate/non-manifold meshes)
- [x] Auto-rigging pipeline tests on varied mesh topologies
- [x] Rapier3D collider generation correctness tests
### nova-neural-materials
- [x] FrameSource/NeuralMaterialProvider contract tests beyond MockProvider happy path
- [x] Error/reconnect handling tests (dropped stream, malformed frame)
- [x] NeuralTexture GPU upload + NeuralMaterialRegistry tests
### nova-audio
- [x] Mixing/bus volume tests (SFX vs music bus interaction)
- [x] Looping edge-case tests (seamless loop points, stop-during-loop)
### Cross-cutting / CI
- [x] Wire up coverage reporting (e.g. `cargo tarpaulin` or `grcov`) into CI and track a baseline
- [x] End-to-end integration test spanning multiple crates (ECS + physics + telemetry tick, asserting deterministic output)
- [x] Regression test harness for the AI code-injection loop (telemetry read -> mutate -> hot-apply -> verify)

## Phase 6: Editor Integration, Onboarding & Agent-Loop Closure (post-Alpha review, 2026-07-14)
Phase 5 marked every checklist item done, but a platform review on 2026-07-14 found
that "Alpha" status hides a real gap: `nova-ui`/`nova-editor`/`nova-overlay` are
tested logic layers with no GPU-rendered surface tying them into `nova-app`, so the
engine has no interactive GUI usable by a human today. This phase tracks closing
that gap plus onboarding and agent-loop follow-ups surfaced in the same review.
### UI/Editor backend (top priority)
- [x] Wire `nova-ui`'s `DrawList` into an actual render pass in `nova-render`/`nova-app`
      (or adopt egui directly, per the already-resolved Open Decision) so the editor
      is visible/usable by a human, not just logic
- [x] Add editable inspector widgets (drag-float, checkbox, etc.) to `nova-editor` so
      the inspector panel can write component values, not just display them
- [x] Add a minimal viewport panel in `nova-app` that feeds pointer-drag deltas into
      the existing 2D/3D gizmo math, and draw on-screen gizmo handles
- [x] Add undo/redo and multi-select to `EditorState`
- [x] Add an asset browser panel and a play-in-editor toggle
- [x] Wire `nova-overlay`'s highlight-rectangle picking into an actual drawn/interactive
      rectangle in the viewport (currently logic-only)
### Onboarding & adoption
- [x] Add a `GETTING_STARTED.md` (or expand README) walking clone -> build -> run
      `nova-app` and see a window; README quickstart currently never reaches a
      rendered frame
- [x] Ship 1-2 small sample assets (a cube `.glb`, a small `.splat`) so `ingest_demo`
      and splat ingestion are runnable zero-config
- [x] Reframe `nova-sample-game` explicitly as a forkable project template
      (doc/README note), not just a pipeline smoke test
- [x] Add `CONTRIBUTING.md` documenting the fmt/clippy/test gate already enforced in CI
### Agent-loop / innovative differentiator
- [x] Close the agent-fix loop end-to-end: connect `nova-rag` (context) +
      `nova-agent-api` (commands) + `nova-overlay` (highlight -> fix prompt) into one
      flagship example/demo
- [x] Replace `nova-rag`'s `FeatureHashEmbedder` placeholder with a real local
      embedding model
### Bug-hunt / hardening
- [x] Run a dedicated code/security review pass over `nova-agent-api`,
      `nova-scripting-embedded`, and `nova-export` (untrusted-input surfaces: control
      files, scripts, `.novapack` archives) — the zero-TODO-marker sweep doesn't
      substitute for an actual bug audit
- [x] Promote the `cargo-tarpaulin` CI coverage job from informational
      (`continue-on-error: true`) to a soft gate now that coverage work is complete

## Phase 6 audit note (2026-07-15)
The bulk of Phase 6 (editor render integration, editable widgets, gizmo viewport,
undo/redo + multi-select, asset browser + play toggle, highlight-rectangle overlay,
`GETTING_STARTED.md`, sample-game template reframe, `CONTRIBUTING.md`, the
`agent_fix_loop` flagship demo, real local embeddings behind the `real-embeddings`
feature, and the tarpaulin soft gate) was implemented in the preceding commit but the
checkboxes were left unticked. This pass confirmed each in code, then closed the two
genuinely-open items:
- **Sample assets shipped** — ran `nova-ingest`'s `gen_sample_assets` to commit
  `assets/cube.glb` (836 B) and `assets/sample.splat` (2048 B); `ingest_demo` now runs
  zero-config against `assets/cube.glb`.
- **Security review (item 13)** — audited the three untrusted-input surfaces:
  - `nova-scripting-embedded`: confirmed safe (eval/import disabled, operation +
    string/array/map/call caps, capability-gated function registration).
  - `nova-agent-api`: added a `MAX_COMMANDS_PER_POLL` (10k) cap and `ControlParse`
    rejection so a hostile 1 MiB control file cannot flood the world with spawns;
    existing 1 MiB size cap and protocol-version check retained.
  - `nova-export`: **fixed** `validate_entry_name` — it only split on `/`, so a
    `\..\` segment bypassed the path-traversal guard on Windows. Now rejects any
    backslash, absolute paths, `..` segments, and empty segments. Added regression
    tests for the backslash bypass and empty segments; existing traversal test kept.

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
