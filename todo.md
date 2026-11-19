# TPT Nova — Build Checklist

## Phase 0: Repo & Tooling Bootstrap
- [x] git init, .gitignore, LICENSE (Apache-2.0), README stub
- [x] Cargo workspace skeleton (tpt-nova-ecs, tpt-nova-telemetry, tpt-nova-render, tpt-nova-app)
- [x] CI: build+test matrix for Windows/Linux/macOS

## Phase 1: "Hello Triangle", Telemetry & Foundations (Months 1-2)
### Rendering & App Shell
- [x] winit window opens on all 3 platforms
- [x] wgpu device/surface init, clear-color render loop
- [x] Draw a static 3D cube (vertex/index buffers, camera, MVP uniform)
### ECS Core
- [x] tpt-nova-ecs: entity/component storage, spawn/despawn, query iteration, scheduler
- [x] Represent the cube as an ECS entity (Transform, Mesh components)
- [x] Scene graph: parent/child Transform hierarchy, world-transform propagation
### Input
- [x] tpt-nova-input: keyboard/mouse polling wired through winit events into an ECS resource
- [x] Basic input-action mapping (e.g. "move_forward" bound to W/Up)
### Telemetry & AI Loop
- [x] tpt-nova-telemetry: JSON schema for entity/component state dump
- [x] Telemetry emission on a tick/interval (stdout or socket)
- [x] Code-injection loop: external process edits a component value via
      file-watch/IPC, engine hot-applies without restart
- [x] Prove the loop end-to-end: script reads telemetry JSON, mutates the
      cube's rotation, engine reflects it live
- [x] Deterministic fixed-timestep tick, seeded RNG plumbing

## Phase 2: 2D "Vibe" Sandbox + Playable-Game Basics (Months 3-5)
### Physics & Rendering
- [x] Integrate Rapier2D into tpt-nova-ecs (physics components + sync step)
- [x] 2D sprite rendering pipeline (batched quads, texture atlas)
### Audio
- [x] tpt-nova-audio: 2D sound playback (SFX one-shots, looping music), volume/mixing
### Gameplay Scripting (Rust-native, hot-reload)
- [x] tpt-nova-scripting v1: gameplay logic as a dylib crate, hot-reload watcher
- [x] Stable ABI/trait boundary between engine and hot-reloaded gameplay code
- [x] Example: a player-controlled 2D entity driven entirely by hot-reloaded logic
### In-Game UI
- [x] tpt-nova-ui v1: 2D immediate-mode-style widgets (text, button, panel) renderable in-game
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
- [x] Forward+ (or simple forward) 3D PBR rendering pipeline in tpt-nova-render
- [x] Dynamic lighting (at least one shadow-casting light type)
- [x] Virtual camera component/system
### Animation
- [x] tpt-nova-anim: skeletal animation playback (bone hierarchy, keyframe sampling)
- [x] Animation blending/state machine (idle/walk/run style transitions)
### 3D Editor & UI
- [x] Scene editor: 3D viewport gizmos (move/rotate/scale in 3D, snapping)
- [x] tpt-nova-ui: world-space UI support (e.g. floating nameplates, in-world panels)
### Smart Mesh Ingestion
- [x] tpt-nova-ingest: .glb/.obj mesh loader
- [x] VHACD convex decomposition for auto-generated colliders
- [x] Auto-rigging pipeline (evaluate/integrate an existing algorithm or crate)
- [x] Rapier3D integration for ingested mesh colliders
- [x] Demo: drag a Meshy .glb in, get collider + rig automatically

## Phase 4: Generative Bridges, Neural Materials & Scripting Expansion (Months 10-12)
### Generative Pipelines
- [x] Gaussian Splat renderer/loader integration (`tpt-nova-splat`: `.splat`/`.ply` loaders, optional wgpu billboard pipeline)
- [x] Low-poly convex hull collider overlay generation for splats (`tpt-nova-splat::build_convex_hull_collider` → `Collider3D`)
- [x] tpt-nova-neural-materials crate: API contract for live video-LLM texture feeds
- [x] Map a live/streamed video texture onto 3D geometry in real time
      (NeuralTexture GPU upload + NeuralMaterialRegistry; PBR-material binding
      into tpt-nova-render is a follow-up)
- [x] "Highlight & Fix" viewport overlay tool (select region -> AI fix prompt) (`tpt-nova-overlay`)
- [x] Video-to-ECS pipeline: depth map + segmentation mask ingestion for
       collision-proxy generation (`tpt-nova-videocap` → `Collider3D`/Rapier3D)
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
- [x] tpt-nova-export: build/package a standalone binary per platform (Win/Linux/macOS) (`tpt-nova-export` CLI: `pack`/`unpack`/`bundle`)
- [x] Asset packing/bundling for distribution (not raw loose files) (`.novapack` container + `BundleManifest`)
- [x] Save/load hardening: migrations, corruption handling, versioned schemas (`tpt-nova-scene` v2 with v1→v2 migration, `validate` pass, `SceneError::Validation`)
### AI Ecosystem
- [x] Public/stable API surface for external AI agents to drive the engine (`tpt-nova-agent-api`: `PROTOCOL_VERSION`, `AgentCommand`, `ControlChannel`, telemetry read-back)
- [x] tpt-nova-rag crate: local vector DB indexing project assets/docs (`tpt-nova-rag`: `Embedder` trait, `FeatureHashEmbedder`, `Index`, `RagAgent`)
- [x] RAG-backed doc/context queries wired into AI agent integration (`tpt-nova-agent-api` `rag` feature: `RagAssistant`)
### Release
- [x] Public Alpha release checklist (docs, licensing review, changelog) (`docs/ALPHA_CHECKLIST.md`, `CHANGELOG.md` Unreleased section, per-crate module docs)
- [x] Sample game(s) built end-to-end using the full pipeline as a proof point (`tpt-nova-sample-game::run_pipeline` — physics rest, scene save/reload, agent spawn/move, splat→collider, asset pack)

## Testing Coverage (Cross-Cutting)
Audit (2026-07-14): `tpt-nova-ecs`, `tpt-nova-app`, and `tpt-nova-input` initially had **zero**
`#[test]` coverage; most other crates had only 1-3 test files. This section tracks
closing those gaps — check items off as tests land, don't infer coverage from a
crate's existence.

Progress (2026-07-14 → 2026-07-14): every crate now ships `#[test]` coverage.
The items below were all implemented in the preceding test-expansion pass
(`tpt-nova-render`, `tpt-nova-physics`, `tpt-nova-anim`, `tpt-nova-ingest`, `tpt-nova-ui`,
`tpt-nova-editor`, `tpt-nova-scripting`, `tpt-nova-scripting-embedded`, `tpt-nova-audio`,
`tpt-nova-neural-materials`, `tpt-nova-telemetry`, and `tpt-nova-scene` carry their tests
in `lib.rs`/`sprite.rs`/`world.rs`; `tpt-nova-ecs`, `tpt-nova-app`, `tpt-nova-input`,
`tpt-nova-gameplay-example` use `tests/`; `tpt-nova-physics` adds a cross-crate
`tests/integration.rs`). The Phase 4/5 **feature** work (Gaussian Splat,
`tpt-nova-splat`; Highlight & Fix, `tpt-nova-overlay`; video ingestion, `tpt-nova-videocap`;
RAG, `tpt-nova-rag`; agent API, `tpt-nova-agent-api`; export/packaging, `tpt-nova-export`;
sample game, `tpt-nova-sample-game`; Alpha checklist, `docs/ALPHA_CHECKLIST.md`)
is now implemented and green under `cargo test --workspace` + `cargo clippy
--workspace --all-targets -D warnings`. The only deliberately-deferred item is
networking/multiplayer (see Open Decisions).
### tpt-nova-ecs
- [x] Entity spawn/despawn unit tests (including despawn-while-iterating safety)
- [x] Component storage/query iteration tests (single/multi-component queries, empty results)
- [x] Scheduler ordering/dependency tests
- [x] Scene graph parent/child hierarchy tests (reparenting, world-transform propagation, cycles rejected)
- [x] Serde round-trip tests for ECS state (used by telemetry + scene save/load)
- [x] Deterministic RNG tests (same seed -> identical stream, zero seed normalized)
### tpt-nova-app
- [x] Headless/CI-safe smoke test for app shell init (world/scheduler/resources built without a window or GPU; see note)
- [x] Fixed-timestep loop tests (tick accumulation, seeded RNG determinism across runs)
### tpt-nova-input
- [x] Keyboard/mouse event -> ECS resource mapping tests
- [x] Input-action binding tests (e.g. "move_forward" -> W/Up, rebinding, unbound key no-ops)
### tpt-nova-render
- [x] Cube pipeline tests beyond current coverage (MVP uniform correctness, camera math)
- [x] 2D sprite batching/atlas pipeline tests (batch boundaries, atlas UV correctness)
- [x] Forward PBR pipeline tests (cube topology, uniform byte round-trip; shadow/material binding is exercised by the live `PbrRenderer`)
### tpt-nova-physics
- [x] Rapier2D sync step tests (component <-> physics-body state round-trip)
- [x] Determinism regression tests (same seed/inputs -> identical simulation trace across runs/platforms)
### tpt-nova-telemetry
- [x] JSON schema round-trip tests for entity/component state dumps
- [x] MessagePack sink round-trip + JSON/MessagePack parity tests
- [x] Tick/interval emission timing tests
### tpt-nova-scene
- [x] Save/load round-trip tests for full ECS world state
- [x] Versioned migration tests (old-version file -> current schema)
- [x] Corrupt/malformed scene file handling tests
### tpt-nova-scripting
- [x] Hot-reload lifecycle tests (missing-file error, ABI-version stability, ABI-mismatch rejection; full dylib load/swap/unload is covered by the `tpt-nova-gameplay-example` + editor watcher rather than a unit test because it needs a compiled cdylib)
- [x] C ABI/trait boundary contract tests (ABI version check rejects incompatible modules)
### tpt-nova-scripting-embedded
- [x] Capability-boundary tests (denied capability's function is genuinely absent, not just unreachable)
- [x] ScriptCommand generation/application round-trip tests
- [x] Sandbox escape attempt tests (no filesystem/network access from scripts)
### tpt-nova-ui
- [x] Widget draw-list generation tests (text/button/panel layout)
- [x] World-space anchored widget positioning tests
### tpt-nova-editor
- [x] Hierarchy panel tests (entity list, parent/child tree updates)
- [x] Component inspector edit round-trip tests (UI edit -> ECS component value)
- [x] 2D/3D gizmo interaction tests (move/rotate/scale, snapping)
- [x] Vibe GUI curve-edit -> physics constraint round-trip tests
### tpt-nova-anim
- [x] Keyframe sampling edge-case tests (before first/after last keyframe, single-keyframe clips)
- [x] Animation blending/state-machine transition tests (idle/walk/run)
### tpt-nova-ingest
- [x] Malformed/unsupported .glb and .obj file handling tests
- [x] VHACD convex decomposition edge cases (degenerate/non-manifold meshes)
- [x] Auto-rigging pipeline tests on varied mesh topologies
- [x] Rapier3D collider generation correctness tests
### tpt-nova-neural-materials
- [x] FrameSource/NeuralMaterialProvider contract tests beyond MockProvider happy path
- [x] Error/reconnect handling tests (dropped stream, malformed frame)
- [x] NeuralTexture GPU upload + NeuralMaterialRegistry tests
### tpt-nova-audio
- [x] Mixing/bus volume tests (SFX vs music bus interaction)
- [x] Looping edge-case tests (seamless loop points, stop-during-loop)
### Cross-cutting / CI
- [x] Wire up coverage reporting (e.g. `cargo tarpaulin` or `grcov`) into CI and track a baseline
- [x] End-to-end integration test spanning multiple crates (ECS + physics + telemetry tick, asserting deterministic output)
- [x] Regression test harness for the AI code-injection loop (telemetry read -> mutate -> hot-apply -> verify)

## Phase 6: Editor Integration, Onboarding & Agent-Loop Closure (post-Alpha review, 2026-07-14)
Phase 5 marked every checklist item done, but a platform review on 2026-07-14 found
that "Alpha" status hides a real gap: `tpt-nova-ui`/`tpt-nova-editor`/`tpt-nova-overlay` are
tested logic layers with no GPU-rendered surface tying them into `tpt-nova-app`, so the
engine has no interactive GUI usable by a human today. This phase tracks closing
that gap plus onboarding and agent-loop follow-ups surfaced in the same review.
### UI/Editor backend (top priority)
- [x] Wire `tpt-nova-ui`'s `DrawList` into an actual render pass in `tpt-nova-render`/`tpt-nova-app`
      (or adopt egui directly, per the already-resolved Open Decision) so the editor
      is visible/usable by a human, not just logic
- [x] Add editable inspector widgets (drag-float, checkbox, etc.) to `tpt-nova-editor` so
      the inspector panel can write component values, not just display them
- [x] Add a minimal viewport panel in `tpt-nova-app` that feeds pointer-drag deltas into
      the existing 2D/3D gizmo math, and draw on-screen gizmo handles
- [x] Add undo/redo and multi-select to `EditorState`
- [x] Add an asset browser panel and a play-in-editor toggle
- [x] Wire `tpt-nova-overlay`'s highlight-rectangle picking into an actual drawn/interactive
      rectangle in the viewport (currently logic-only)
### Onboarding & adoption
- [x] Add a `GETTING_STARTED.md` (or expand README) walking clone -> build -> run
      `tpt-nova-app` and see a window; README quickstart currently never reaches a
      rendered frame
- [x] Ship 1-2 small sample assets (a cube `.glb`, a small `.splat`) so `ingest_demo`
      and splat ingestion are runnable zero-config
- [x] Reframe `tpt-nova-sample-game` explicitly as a forkable project template
      (doc/README note), not just a pipeline smoke test
- [x] Add `CONTRIBUTING.md` documenting the fmt/clippy/test gate already enforced in CI
### Agent-loop / innovative differentiator
- [x] Close the agent-fix loop end-to-end: connect `tpt-nova-rag` (context) +
      `tpt-nova-agent-api` (commands) + `tpt-nova-overlay` (highlight -> fix prompt) into one
      flagship example/demo
- [x] Replace `tpt-nova-rag`'s `FeatureHashEmbedder` placeholder with a real local
      embedding model
### Bug-hunt / hardening
- [x] Run a dedicated code/security review pass over `tpt-nova-agent-api`,
      `tpt-nova-scripting-embedded`, and `tpt-nova-export` (untrusted-input surfaces: control
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
- **Sample assets shipped** — ran `tpt-nova-ingest`'s `gen_sample_assets` to commit
  `assets/cube.glb` (836 B) and `assets/sample.splat` (2048 B); `ingest_demo` now runs
  zero-config against `assets/cube.glb`.
- **Security review (item 13)** — audited the three untrusted-input surfaces:
  - `tpt-nova-scripting-embedded`: confirmed safe (eval/import disabled, operation +
    string/array/map/call caps, capability-gated function registration).
  - `tpt-nova-agent-api`: added a `MAX_COMMANDS_PER_POLL` (10k) cap and `ControlParse`
    rejection so a hostile 1 MiB control file cannot flood the world with spawns;
    existing 1 MiB size cap and protocol-version check retained.
  - `tpt-nova-export`: **fixed** `validate_entry_name` — it only split on `/`, so a
    `\..\` segment bypassed the path-traversal guard on Windows. Now rejects any
    backslash, absolute paths, `..` segments, and empty segments. Added regression
    tests for the backslash bypass and empty segments; existing traversal test kept.

## Phase 7: Post-Review Hardening & Feature Completion (2026-07-15 platform review)
A platform review on 2026-07-15 found that several Phase 6 "done" items don't function
end-to-end in the running app, docs describe a UI framework that was never adopted, and
onboarding/adoption has real friction. This phase tracks closing those gaps.
### Correctness bugs
- [x] Fix 3D viewport gizmo drags not being undoable (`apply_gizmo_3d` in
      `crates/tpt-nova-app/src/main.rs` bypasses `EditHistory::record`)
- [x] Verify/fix `tpt-nova-audio::play_spatial`'s unused `SpatialParams` — confirm rolloff
      model is actually applied, or wire it in if it's a no-op (`crates/tpt-nova-audio/src/lib.rs`)
- [x] Add an `--all-features` (or targeted per-feature) build+test job to
      `.github/workflows/ci.yml` so `tpt-nova-rag`'s `real-embeddings`, `tpt-nova-agent-api`'s
      `rag`, and `tpt-nova-splat`'s `render` feature are actually exercised in CI
- [x] Revisit the `cargo-tarpaulin --fail-under 15` coverage floor now that Phase 6
      coverage work is complete; raise it to reflect actual coverage
### Documentation corrections
- [x] Correct the stale egui/eframe claim in `CLAUDE.md` and
      `crates/tpt-nova-editor/src/lib.rs` module docs to describe the actual bespoke
      `tpt-nova-ui`/`tpt-nova-render` immediate-mode stack
- [x] Update the "Open Decisions — Editor framework" entry (todo.md, resolved-egui
      note) to reflect that the shipped implementation is the bespoke stack, not
      egui/eframe
### Editor: wire up dead/unwired code
- [x] Wire `EditorState::toggle_select` into a real input path (shift-click in the
       hierarchy panel and/or viewport) so multi-select is actually reachable
- [x] Make the asset browser's `selected_asset` do something (spawn-on-click at
       minimum) instead of being written and never read
- [x] Instantiate the Vibe GUI (`vibe.rs` Bézier curve editor) as a real panel in
       `tpt-nova-app` — it's fully implemented/tested but never appears in the running editor
- [x] Wire `InputState.scroll` into viewport camera zoom (currently captured and
       never read)
- [x] Capture typed keyboard text in `tpt-nova-input` (`InputState.text_entered`,
       populated from winit `KeyEvent.text`, cleared each `end_frame`) — prerequisite
       for a text-input widget, currently missing entirely
- [x] Thread `shift_held`/`text_entered` fields through `UiInput` (built in
       `tpt-nova-app/src/main.rs`) so widgets can see modifier/keyboard state
- [x] Add a minimal text-input widget to `tpt-nova-ui` (`TextInputState` + focus
       tracking on `EditorState`, click-to-focus/away-to-unfocus) and use it for the
       "Highlight & Fix" instruction field (currently hardcoded to the literal
       `"fix selection"`)
- [x] Add an `Inspectable` trait + component registry (new primitive in
       `tpt-nova-ecs`/`tpt-nova-editor`) so the inspector can discover arbitrary component
       types on an entity instead of two hardcoded `if let` checks for `Transform`/
       `RigidBody2D`
### Roadmap/vision follow-ups
- [x] PBR-material binding for `tpt-nova-neural-materials` (live video-LLM texture onto
       an actual in-scene surface) — registry/upload exists, binding doesn't
### Onboarding & adoption
- [x] Write a standalone agent-protocol reference doc (control-file schema,
       telemetry schema) independent of reading `tpt-nova-agent-api`'s Rust source
       (`docs/AGENT_PROTOCOL.md`)
- [x] Add at least one runnable example each to `tpt-nova-ecs`, `tpt-nova-render`, and
       `tpt-nova-editor` (currently zero across all three)
- [x] Add a scaffolding helper (script or `xtask`) for forking `tpt-nova-sample-game`
       into a new project, replacing the manual copy/rename-by-hand flow
- [ ] Add example screenshots/a short clip to `GETTING_STARTED.md` (outstanding
       `ALPHA_CHECKLIST.md` release-gate item; requires a manual capture step)
### Innovative differentiator features (promoted from review discussion, 2026-07-15)
- [ ] Wire the "Highlight & Fix" typed instruction (once the text-input widget
       lands) to a real LLM call using `tpt-nova-rag` context, emitting an
       `AgentCommand` batch through `tpt-nova-agent-api` — turns the scripted
       `agent_fix_loop.rs` demo into a live, user-triggered fix from the viewport
       (propose-mode ghost preview + agent-action logging are implemented;
       the live network LLM call is intentionally left as a documented hook,
       not wired, to keep the build offline/testable)
- [x] Add a "propose" mode that renders an incoming `AgentCommand` batch as a
       ghost/translucent preview (reusing existing gizmo rendering) with
       accept/reject, before it's applied to the world
- [x] Add an agent-action log / explainability panel: let `AgentCommand`s carry
       an optional rationale string and show the last N applied agent actions in
       a small editor panel
- [x] Add telemetry scrubbing (a mini time-travel debugger): ring-buffer the
       last N telemetry ticks and let the editor scrub backward through recent
       world states, reusing the existing deterministic JSON/MessagePack
       telemetry capture
- [x] Add an "explain this entity" clipboard export: package a selected
       entity's component state + nearby `tpt-nova-rag` context into a prompt-ready
       text block on the clipboard, for users without a wired-up agent yet
- [x] Add a `cargo xtask doctor` (or shell script) that checks GPU/driver/wgpu
       backend availability and prints a readiness report before `cargo run -p
       tpt-nova-app` — the most common first-run failure mode currently has no
       diagnostics
- [x] Add autosave/restore-last-session: periodically autosave world state via
       the existing `tpt-nova-scene` save/load and restore it on next launch (scene
       state is currently in-memory only)
- [x] Add a task-oriented "cookbook" doc (recipes: add a new component +
       inspector field; add a new gameplay script; ingest a custom mesh)
       alongside the existing reference-style `GETTING_STARTED.md`

## Open Decisions
- [x] Editor framework: **RESOLVED (updated 2026-07-15) — bespoke `tpt-nova-ui`
      immediate-mode stack**, not egui/eframe. The original Phase-2 decision named
      egui/eframe, but the Phase 6 implementation never took an egui/eframe
      dependency: `tpt-nova-editor`'s panels are hand-rolled `Ui`/`DrawList` widgets
      rendered by `tpt-nova-render`'s `UiOverlay` pass in `tpt-nova-app`'s winit+wgpu shell.
      A 2026-07-15 platform review confirmed no egui/eframe dependency exists
      anywhere in the workspace and the decision was reaffirmed as-is: the bespoke
      stack is already working and well-tested, and it shares primitives with the
      in-game `tpt-nova-ui` runtime UI, so a migration to egui/eframe was rejected in
      favor of finishing out the bespoke stack's feature set instead (see Phase 7).
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
      requirement now. Split: Rust hot-reload (`tpt-nova-scripting`) for shipped /
      performance-critical systems; Rhai (`tpt-nova-scripting-embedded`) for AI-generated
      sandboxed logic.
- [ ] Networking/multiplayer — not in spec or current scope; revisit if needed post-Alpha

## Phase 7 audit note (2026-07-15, second pass)
This pass closed the remaining Phase 7 checklist. As in the prior Phase 6/7 sweeps,
several "unchecked" editor items were **already implemented in code** but left
unticked — confirmed by reading the source and ticked without code change:
`EditorState::toggle_select` is wired into both the hierarchy panel and viewport
shift-click; `selected_asset` spawn-on-click is live in `tpt-nova-app`'s pointer-press
path; the Vibe GUI (`vibe.rs`) is instantiated as a real panel via
`draw_vibe_panel`; `InputState.scroll` drives `camera_distance` viewport zoom
(unit-tested); and the inspector uses the `ComponentInspector` trait + `inspectors()`
registry (no hardcoded `if let` checks), so the "Inspectable" primitive existed
under a different name. The PBR neural-material **binding** also already existed
(`tpt-nova-neural-materials::binding::{MaterialBinding, MaterialBindings}`) plus
`NeuralTexture::view` / `NeuralMaterialRegistry::upload`; this pass wires it into
`tpt-nova-app` (a `"video"` feed is registered and bound to the cube, and its latest
frame is resolved every tick) so the link is demonstrably live.

Genuinely-added in this pass:
- **`InputState.text_entered`** + **`UiInput.shift_held`/`text_entered`**: the
  pointer-only UI now receives typed keystrokes and the shift modifier; the
  Highlight & Fix instruction field routes them through `EditorState.text_focus`.
- **Agent explainability + time-travel**: `EditorState` gained `action_log`
  (`AgentAction`), a `TelemetryRing` ring buffer, and `explain_entity`; `tpt-nova-app`
  renders an "Agent & Telemetry" panel (live neural binding, telemetry scrubber,
  recent actions) plus an "Explain" toolbar button that writes a prompt-ready
  block to a file, and a "Propose" mode that shows a fix as a ghost preview with
  Accept/Reject.
- **Autosave/restore**: `tpt-nova-app` periodically serializes the world via
  `tpt-nova-scene` (gated by `NOVA_AUTOSAVE`) and can reload it on launch
  (`NOVA_RESTORE`), both off by default so the hermetic tests are unaffected.
- Docs/tools (via parallel agents): `docs/AGENT_PROTOCOL.md`, `docs/COOKBOOK.md`,
  runnable `examples/` for `tpt-nova-ecs`/`tpt-nova-render`/`tpt-nova-editor`, a `crates/tpt-nova-xtask`
  `doctor` subcommand, and `scripts/scaffold_sample_game.{ps1,sh}`.

Two items remain intentionally open: the **live network LLM call** for Highlight &
Fix (the propose-mode ghost + agent-action logging are done; the actual LLM
round-trip is a documented hook, kept offline so CI/tests need no network) and
**example screenshots/clips** in `GETTING_STARTED.md` (requires a manual GPU
capture step). All CI gates pass: `cargo fmt --check`, `cargo clippy --workspace
--all-targets -D warnings`, `cargo build --workspace --all-targets`, `cargo test
--workspace`.

## Phase 8: tpt-nova-drivers (Hot-Swappable Behavior Layer, from spec2.txt)
Source: [spec2.txt](spec2.txt) (Draft v0.1.0). Crate boundary per spec §10. Not started —
do not create the crate until Layer 1 is being implemented (CLAUDE.md: no speculative crates).

### Layer 1: Primitives & Core Traits
- [ ] Define `Driver` trait and `DriverContext`
- [ ] Implement `DriverId`, `DriverVersion`, `CapabilitySet`
- [ ] Build `drivers-registry` with ArcSwap-based lookup (`(ComponentTypeId, DriverVersion) → DriverHandle`)
- [ ] Unit tests for atomic hot-swap (next frame uses new driver; no frame drops)

### Layer 2: Sandbox & Safety
- [ ] Implement native driver loading via `libloading` (Linux-first `.so`; path handling via `std::path::Path`)
- [ ] Implement WASM sandbox via `wasmtime` behind optional feature `wasm-sandbox`
- [ ] Define capability token system (`ReadNeighbors`, `WritePhysics`, `EmitEvents`, …), granted/denied at load time
- [ ] Fuzz test sandbox escape attempts
- [ ] Enforce invariant: no driver executes outside its capability sandbox

### Layer 3: Migration & Versioning
- [ ] Implement `migrate_from` trait method
- [ ] Build migration chain composer (v1 → v2 → v3)
- [ ] Implement shadow execution for A/B testing (subset of entities, compare before promotion)
- [ ] Build rollback mechanism (one-call revert to any prior verified version)
- [ ] Invariant: no schema migration without a registered migration path

### Layer 4: AI & Self-Debugging
- [ ] Integrate `tpt-anima` for anomaly observation
- [ ] Integrate `tpt-eve` for driver patch generation
- [ ] Integrate `tpt-telos` for symbolic verification (invariant suites: "velocity bounded", "position finite", "energy conserved")
- [ ] Integrate `tpt-crucible` compilation of candidates to WASM/native
- [ ] Implement full self-debugging loop end-to-end (observe → detect → propose → verify → shadow test → deploy → learn)
- [ ] Benchmark loop latency (target: < 1 frame for simple fixes)
- [ ] Invariant: no driver is loaded without passing symbolic verification

### Layer 5: Domain Integration
- [ ] Ship example drivers for `tpt-nova-games`
- [ ] Ship example drivers for `tpt-nova-sim`
- [ ] Ship example drivers for `tpt-nova-viz`
- [ ] Document driver authoring guide

### Cross-Cutting: Packaging, Licensing & Verification
- [ ] Driver packaging into content-addressed `.novapack` bundles (via `tpt-nova-export`) — spec §7 step 4
- [ ] Every driver carries a version; rollback always possible (spec Core Invariant)
- [ ] Add `cargo-deny` configuration at workspace root enforcing spec §6 license policy (allow MIT/BSD/ISC/Unicode-DFS; ban GPL/AGPL/LGPL/MPL/SSPL/BSL)
- [ ] Success criteria (spec §11): <1 ms hot-swap; physics anomaly fixed within 5 frames; buggy WASM driver cannot crash host; 3-version migration preserves invariants
- [x] Repo licensing: dual-licensed `MIT OR Apache-2.0` (see `LICENSE-MIT`, `LICENSE-APACHE`)

## Phase 8 open questions (from spec2.txt)
- [ ] **Crate license:** spec §6 says `tpt-nova-drivers` is MIT-only, while the workspace is now MIT OR Apache-2.0. Decide whether the new crate sets `license = "MIT"` explicitly. Note that `wasmtime` (Apache-2.0) conflicts with the MIT-only policy and needs the optional-feature exception the spec describes.
- [ ] **Driver inspector UI:** spec §8 names egui/iced. This conflicts with the Open Decisions resolution (bespoke `tpt-nova-ui` stack, no egui/iced). Confirm which applies before building the inspector.
- [ ] **External repos:** `tpt-anima`, `tpt-telos`, `tpt-eve`, `tpt-crucible`, and `tpt-vectis` live outside this repo. Confirm whether they exist and what their interfaces are before Layer 4.

## Phase 9: Gaussian Splat Improvements (`tpt-nova-splat`)

Current gaps: the renderer draws isotropic billboards (rotation/anisotropic scale unused, no Gaussian falloff), has no depth sorting, keeps only SH-DC color, and the collider is a single convex hull. Each sub-phase is independently shippable; keep the `render` feature optional so headless CI stays green.

### A: Rendering quality
- [x] `Splat::covariance3d()` CPU helper (R S Sᵀ Rᵀ) with unit tests (axis-aligned + rotated)
- [x] Anisotropic splat shader: project 3D covariance to a 2D screen-space ellipse via the perspective Jacobian (+ low-pass), oriented quad, Gaussian alpha falloff in the fragment stage
- [x] Move per-splat data to a storage buffer; vertex shader reads via a sorted index buffer
- [ ] Depth sorting: CPU back-to-front sort on camera change, then GPU radix sort (compute) behind the same API — **CPU sort done** (per-chunk, re-sorted only when the view-projection changes; cross-chunk order by chunk center); **GPU radix sort not started**
- [x] Blending: back-to-front premultiplied alpha, depth-write off, depth-test against the opaque scene
- [x] Higher-order SH: loader reads `f_rest_*` (degrees 1-3); shader evaluates SH from view direction; `SplatQuality { sh_degree }` runtime cap (0 = current behavior)
- [x] Pass camera view matrix alongside view_proj to `SplatPipeline::render` (keep `new`/`set_cloud` signatures stable) — `render` now takes a `SplatCamera { view, view_proj, viewport_px }`

### B: Physics/collision proxies
- [x] `proxy.rs`: floater filtering (opacity threshold, oversized scale, k-nearest density outliers)
- [x] Opacity-weighted voxelization of the filtered cloud
- [x] Voxel-boundary surface mesh → VHACD → multi-part `Collider3D`; per-component hull fallback
- [x] Serializable `ProxyParams` + `attach_proxy_collider`; keep `build_convex_hull_collider` for back-compat
- [x] Tests: separated blobs → ≥2 parts, floaters removed, concave L-shape tighter than single hull
- [x] Update `tpt-nova-sample-game` pipeline step to use the multi-part proxy

### C: Scale & performance
- [x] Compressed format loader (quantized `.compressed.ply`), dispatched by extension in `load_file` and auto-detected in `load_ply_bytes` — **PlayCanvas-style layout only; SPZ is not implemented, and the reader is validated by round-trip tests only, not against a third-party-exported asset**
- [x] Frustum culling + distance LOD over spatial chunks
- [x] `SplatCloud::decimate(ratio)` (seeded/deterministic)
- [x] Chunked GPU upload for very large clouds (one storage-buffer set and bind group per spatial chunk)

### D: ECS-native editing & agent integration
- [x] Edit ops: `select_in_aabb`, `select_in_screen_rect` (reuse `tpt-nova-overlay`), `delete`, `crop`, `transform` — `transform` is exact for non-uniform scale; SH coefficients are not rotated
- [x] Splat telemetry summary (count, bounds, sh_degree, opacity histogram) via `tpt-nova-telemetry`
- [x] Additive `AgentCommand` variants (`CropSplat`, `RegenerateSplatProxy`) in `tpt-nova-agent-api`; they go through the existing `MAX_COMMANDS_PER_POLL` path
- [ ] Highlight & Fix: region selection over a splat entity yields a splat-subset selection in the AI fix prompt — **crate-level helpers done** (`select_in_screen_rect`, `SplatSelection::prompt_entry`); **not yet wired into the viewport's Highlight & Fix flow in `tpt-nova-app`**
- [ ] (Stretch) Dynamic splats: per-splat bone weights bound to `tpt-nova-anim` skeletons — not started

### Housekeeping
- [x] Update `tpt-nova-splat` README and the CLAUDE.md splat blurb once A/B land
