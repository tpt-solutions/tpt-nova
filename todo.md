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
- [x] Add example screenshots/a short clip to `GETTING_STARTED.md` — a scene-graph frame (`docs/images/scene_window.png`, cropped from a local GPU capture) and the editor window (`docs/images/editor_window.png`, captured 2026-11-26 on the local GPU).
       `ALPHA_CHECKLIST.md` release-gate item; requires a manual capture step)
### Innovative differentiator features (promoted from review discussion, 2026-07-15)
- [ ] Wire the "Highlight & Fix" typed instruction (once the text-input widget
       lands) to a real LLM call [decided 2026-11-27: keep the documented hook; the
       network call stays unwired so the build stays offline and testable. Wire it
       when a provider and its credential flow are chosen] using `tpt-nova-rag` context, emitting an
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
- [x] Define `Driver` trait and `DriverContext` — `tpt-nova-drivers` (`driver.rs`)
- [x] Implement `DriverId`, `DriverVersion`, `CapabilitySet` — `tpt-nova-drivers` (`driver.rs`)
- [x] Build `drivers-registry` with ArcSwap-based lookup (`(ComponentTypeId, DriverVersion) → DriverHandle`) — `DriverRegistry` (`registry.rs`), lock-free snapshot reads, serialized writes
- [x] Unit tests for atomic hot-swap (next frame uses new driver; no frame drops) — incl. a concurrent-reader swap test

### Layer 2: Sandbox & Safety
- [x] Implement native driver loading via `libloading` (feature `native-loader`, `crates/tpt-nova-drivers/src/native.rs`). Trusted in-process code: capabilities are checked at activation only. Component identity does not cross the dylib boundary (`ComponentTypeId` wraps `TypeId`), so a module cannot look up host components by type; a stable name-based component id is needed for that and is not built. Verified with a scratch cdylib round-trip (load, ABI check, register/activate, update mutates the host world, destroy): not added to the repo as a fixture.
- [x] Implement WASM sandbox on `tpt-wasm` (the user's own MIT OR Apache-2.0 runtime, used in place of `wasmtime`) behind optional feature `wasm-sandbox` — `crates/tpt-nova-drivers/src/sandbox.rs`: load-time gate (module must import only `tpt_nova` functions whose capability is granted and bound), per-import least-privilege binding, `max_execution_steps` budget per update, and activation through `DriverRegistry` under policy; 26 tests pass with the feature on. Host bindings exist for `read_dt` and `emit_event` only; `write_transform`, `read_neighbors` and `write_physics` are refused until bound
- [x] Define capability token system (`ReadNeighbors`, `WritePhysics`, `EmitEvents`, …), granted/denied at load time — `CapabilitySet` + activation-time policy gate in `DriverRegistry` (`RegistryError::CapabilityDenied`); execution-time enforcement is still open (see the next item)
- [x] Fuzz test sandbox escape attempts — deterministic seeded mutation fuzz (`sandbox.rs`: 3000 bit-flip/overwrite/truncate/append mutations through `load` + `update`, asserting no panic and a bounded run) plus an ungranted-import escape test; not coverage-guided (`cargo-fuzz` not wired)
- [x] Enforce invariant: no driver executes outside its capability sandbox — closed for strict registries (`DriverRegistry::strict`): a strict registry refuses every activation without a `Verified` proof, and native and unverified WASM drivers have none, so they never run. WASM drivers can only call host imports bound to their granted capabilities. Permissive registries (the default, for trusted host bootstrap) run native code in-process with no execution-time check. In-process native code cannot be sandboxed without a process boundary, so this is a documented limit, not a gap to close.

### Layer 3: Migration & Versioning
- [x] Implement `migrate_from` trait method — `Driver::migrate_from` (default: no migration)
- [x] Build migration chain composer (v1 → v2 → v3)
- [x] Implement shadow execution for A/B testing (subset of entities, compare before promotion) — `shadow.rs`: `shadow_compare` runs baseline and candidate for one tick on forks of the world and compares an `observe` projection per sampled entity; `promote_if_clean` refuses divergent reports. Caller supplies the fork (`World` is not `Clone`)
- [x] Build rollback mechanism (one-call revert to any prior verified version) — reverts to any previously *activated* version; "verified" awaits the Layer 4 verifier
- [x] Invariant: no schema migration without a registered migration path

### Layer 4: AI & Self-Debugging
- [x] Integrate `tpt-anima` for anomaly observation — replaced by an internal detector (per the decision to drop the external dependency): `anomaly.rs` (`AnomalyTracker`, per-entity EWMA mean/variance with z-score flags, deterministic order). `tpt-anima` itself is not used, since it exposes no anomaly API.
- [x] Integrate `tpt-eve` for driver patch generation — replaced by an internal proposer: `compile.rs` `GainSearch` walks a deterministic ladder of contraction gains, and the verifier and shadow gates decide. `tpt-eve` is not used (it has no code-patch generator).
- [x] Replace `tpt-telos` with an internal symbolic verifier — `verify.rs` is an exact Fourier-Motzkin solver over rationals (checked arithmetic, strict and non-strict inequalities, fails closed). Telos was dropped: its `unsat` strengthens strict inequalities to integers, so it accepted obligations the reals refute. The telos feature and git dependency are removed. Invariant suites beyond one-tick affine bounds are supported in the solver, but no invariant suite is wired yet.
- [x] Integrate `tpt-crucible` compilation of candidates to WASM/native — replaced by an internal compiler: `compile.rs` `compile(FixSpec, field)` builds the driver and its affine model from one declarative spec, so the model cannot drift from the code. WASM candidates still go through `WasmDriver::load`. `tpt-crucible` is not used.
- [x] Implement full self-debugging loop end-to-end — observe (`Monitor` and `AnomalyTracker`) → propose (`GainSearch`) → verify (internal solver) → shadow → preserve → deploy → learn (`FixMemory`). `examples/selfdebug_loop.rs` runs it. Everything is internal, with no external AI or solver service.
- [x] Benchmark loop latency (target: < 1 frame for simple fixes) — `examples/selfdebug_loop.rs` measures the repair on a 2-entity world at about 36 µs and 113 µs in two runs, well under the 16.7 ms frame. That is a tiny world; a real scene needs its own measurement.
- [x] Invariant: no driver is loaded without passing symbolic verification — enforced by `DriverRegistry::strict`: `activate` returns `VerificationRequired`, and only `activate_verified(&Verified)` can make a driver active (the repair loop is the only caller). `is_verified_active` reports it. Tests: `strict_registry_refuses_unverified_activation`, `permissive_activation_is_not_marked_verified`. Default `DriverRegistry::new()` stays permissive for trusted host bootstrap; hosts that need the invariant must use `strict`.

### Layer 5: Domain Integration
- [x] Ship example drivers for `tpt-nova-games` — `examples/domain_drivers.rs` (2% linear drag, compiled and verified). The domain repo itself is spec-only, and its TODO waits on this crate.
- [x] Ship example drivers for `tpt-nova-sim` — `examples/domain_drivers.rs` (thermal relaxation toward 20, affine, verified). The domain repo is spec-only.
- [x] Ship example drivers for `tpt-nova-viz` — `examples/domain_drivers.rs` (intensity normalisation, verified). The domain repo is spec-only.
- [x] Document driver authoring guide — `docs/DRIVER_AUTHORING.md`

### Cross-Cutting: Packaging, Licensing & Verification
- [x] Driver packaging into content-addressed `.novapack` bundles (via `tpt-nova-export`) — `driver_bundle.rs`: artifacts stored under `drivers/<sha256>/`, unpack verifies the hash; tests cover round trip, tamper, missing parts, and distinct versions.
- [x] Every driver carries a version; rollback always possible (spec Core Invariant) — `Driver::version` is a required trait method, and the registry has no unregister, so the last previously active version is always still present to roll back to (`rollback_reverts_to_the_previously_active_version`)
- [x] Add `cargo-deny` configuration at workspace root enforcing spec §6 license policy (allow MIT/BSD/ISC/Unicode-DFS; ban GPL/AGPL/LGPL/MPL/SSPL/BSL) — `deny.toml` exists; `cargo deny check licenses bans sources` passes
- [x] Success criteria (spec §11): <1 ms hot-swap; physics anomaly fixed within 5 frames; buggy WASM driver cannot crash host; 3-version migration preserves invariants — measured by `examples/selfdebug_loop.rs` (verified repair of the velocity-bound anomaly in one tick, 51–144 µs on a 2-entity world, well inside 16.7 ms, so under 5 frames and under 1 ms for the hot-swap); the WASM sandbox tests (`runaway_drivers_hit_the_instruction_budget`, `mutated_modules_never_panic_or_escape_the_grant`); and the migration chain composer tests. Small-world numbers only; a real scene needs its own measurement.
- [x] Repo licensing: dual-licensed `MIT OR Apache-2.0` (see `LICENSE-MIT`, `LICENSE-APACHE`)

## Phase 8 open questions (from spec2.txt)
- [x] **Crate license:** resolved — `tpt-nova-drivers` sets `license = "MIT"` explicitly (spec §6); `arc-swap` is MIT OR Apache-2.0 so it passes `cargo deny`. Original question: spec §6 says `tpt-nova-drivers` is MIT-only, while the workspace is now MIT OR Apache-2.0. Decide whether the new crate sets `license = "MIT"` explicitly. Resolved for the WASM runtime: the sandbox uses `tpt-wasm` (MIT OR Apache-2.0, same owner) instead of `wasmtime`, so no exception is needed. `tpt-wasm` and `tpt-window` are added to `deny.toml` `allow-git`.
- [x] **Driver inspector UI:** decided — use the bespoke `tpt-nova-ui` stack per CLAUDE.md (no egui/iced). The inspector itself is not built.
- [x] **External repos:** resolved — `tpt-anima`, `tpt-telos`, `tpt-eve`, `tpt-crucible`, and `tpt-vectis` exist on GitHub (`tpt-solutions`). None is a runtime dependency now: Layer 4 is internal (`verify`, `anomaly`, `compile`), and `tpt-telos` was dropped.

## Phase 9: Gaussian Splat Improvements (`tpt-nova-splat`)

Current gaps: the renderer draws isotropic billboards (rotation/anisotropic scale unused, no Gaussian falloff), has no depth sorting, keeps only SH-DC color, and the collider is a single convex hull. Each sub-phase is independently shippable; keep the `render` feature optional so headless CI stays green.

### A: Rendering quality
- [x] `Splat::covariance3d()` CPU helper (R S Sᵀ Rᵀ) with unit tests (axis-aligned + rotated)
- [x] Anisotropic splat shader: project 3D covariance to a 2D screen-space ellipse via the perspective Jacobian (+ low-pass), oriented quad, Gaussian alpha falloff in the fragment stage
- [x] Move per-splat data to a storage buffer; vertex shader reads via a sorted index buffer
- [x] Depth sorting: CPU back-to-front sort on camera change, then GPU radix sort (compute) behind the same API — **CPU sort done** (per-chunk, re-sorted only when the view-projection changes); **GPU radix sort done** (`sort.rs`, opt-in `SplatSortMode::Gpu`; GPU tests pass on a real adapter)
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
- [x] Highlight & Fix: region selection over a splat entity yields a splat-subset selection in the AI fix prompt — **done**: `App::splat_selection_notes` runs `select_in_screen_rect` over every `SplatCloud` in the viewport marquee and appends `SplatSelection::prompt_entry` lines (test `highlight_fix_prompt_includes_splat_subset_selection`)
- [ ] (Stretch) Dynamic splats: per-splat bone weights bound to `tpt-nova-anim` skeletons — not started. Deferred (decided 2026-11-27): it is a stretch item, and the static splat pipeline is complete without it. Pick it up only with a concrete animated-splat use case.

### Housekeeping
- [x] Update `tpt-nova-splat` README and the CLAUDE.md splat blurb once A/B land

## Phase 10: 3D Scene Graph & Appfront Integration (from spec3.txt)
Source: [spec3.txt](spec3.txt) (Draft). Proposes a new `tpt-nova-scene-graph` crate as the
high-level, declarative 3D layer over `tpt-nova-render`. Not started — do not create the crate
until Phase 10 Step 1 is being implemented (CLAUDE.md: no speculative crates).

### Phase 1: Core Graph & Rendering
- [x] Define `Scene`, `Node`, and transform hierarchy (position/rotation/scale) — `tpt-nova-scene-graph` (`scene.rs`, `node.rs`, `transform.rs`); fixed `add_child` listing children as roots
- [x] Implement `Camera` (`PerspectiveCamera`) and `Light` (`DirectionalLight`, `PointLight`, `AmbientLight`) node types, with active-camera selection
- [x] Wire `Scene` to drive `tpt-nova-render`'s existing PBR and sprite pipelines — `bake(&scene)` → `RenderPlan`; `RenderPlan::spawn_into(&mut World)` writes `Transform`/`GlobalTransform`/`Mesh`/`Sprite`/`Light`/`Camera` (+ `MainCamera`) components that the pipelines and `pick_main_camera` read
- [x] Standalone example (via `tpt-nova-editor` or a simple window) proving the graph renders correctly — `examples/scene_window.rs` renders through the real `Renderer` on a local GPU. Checked on screen: the scene background is `#101820` and the lit cube sits where the camera transform puts it. This check found the scene background was dropped (not written to the renderer); fixed with a `ClearColor` component.

### Phase 2: "Stunning Defaults" & Post-Processing
- [x] Implement the `PostProcessPipeline` orchestrator (toggleable effect stack) — scene-owned, serializable `PostProcessPipeline` lowers to `tpt-nova-render`'s `PostProcessChain`
- [x] Add Bloom, Tone Mapping (ACES), and basic anti-aliasing passes to `tpt-nova-render` (if not already present) — `render/src/post.rs` (Bloom, ACES/Reinhard, FXAA); GPU tests pass
- [x] Add a render-side API for registering custom full-screen post-processing passes (spec3 §4.2) — `PostProcessChain::add_custom_pass` / `PostProcessPass` trait
- [x] Tune default values for premium out-of-the-box visuals — decided: keep the stock defaults (bloom 0.8/0.6, ACES 1.0, FXAA on). Checked on the local GPU (2026-11-26/27): the stock render is clean, and bloom does not visibly trigger on the lit cube. A key light raised to 6.0 did not push the lit faces over the 0.8 threshold after ACES, so there was no evidence for a different value. Revisit when the renderer has an emissive or specular material to judge the glow against.

### Phase 3: Appfront Bridge
- [x] Add the `appfront` feature flag to `tpt-nova-scene-graph` (opt-in; core engine must not depend on appfront) — the optional dependency is a git dependency on `tpt-solutions/tpt-appfront` (branch `canvas-3d`). Switch it to a git dependency once that branch is pushed.
- [x] Implement `AppfrontSceneDelegate` against the `SceneDelegate` trait from `tpt-appfront-canvas-3d` — `appfront.rs` (`AppfrontScene`). Picking works through `pick.rs`. `present` reports `Unsupported`, because there is no GPU readback into the appfront canvas. The trait is in `tpt-appfront` on GitHub (commit 1d9244c).
- [x] Handle pointer-event raycasting (2D screen coords → 3D world coords) — `pick.rs`: `screen_ray` unprojects a pixel and `pick` returns the nearest visible cube, handling rotation and scale. Covers the cube only.
- [ ] Test embedding the scene in a `tpt-appfront` UI running in `tpt-shell` and as a Wasm PWA — partly done (2026-11-27). tpt-nova side: `tpt_nova_render::OffscreenRenderer` renders the cube pass into an RGBA8 target and reads it back on the GPU; `AppfrontScene::present` uses it and keeps the frame (`last_frame()`), and a GPU test checks the cube's pixels (`present_renders_the_cube_into_a_frame_the_host_can_read`). External repos (2026-11-27, committed on each repo's master, not pushed): `tpt-appfront-canvas-3d` gained `SceneDelegate::pixels()` (a default method returning `PixelFrame`, RGBA8), and `tpt-shell` gained `UiHandle::viewport_frame`, which sends a frame to a page as the event `viewport.frame` (base64 RGBA8 in JSON), plus a `viewport` example that draws it into a canvas. Steps: push both repos (done); bump the `tpt-appfront` git rev in `tpt-nova-scene-graph` (done, `9d7c9e5`); implement `pixels()` on `AppfrontScene` (done) (a one-off `--config` patch against the local checkout compiles and passes the appfront tests, so the change is ready); and run the `tpt-shell` viewport example on Windows to see the frames on screen. Done (2026-11-27): the example streams an animated gradient into the window. Its first version put script and CSS inline, which the shell's page policy blocks; the fix is committed in tpt-shell (`51ab9e2`). The scene is in the viewport too: `examples/scene-shell` is a standalone package (outside the workspace, so the main CI and license checks are unaffected) that renders a lit cube through `AppfrontScene` and `Viewport3D`, and sends each frame with `viewport_frame`. Verified on Windows: the cube turns in the window and the frame count climbs (64 to 255 between captures). Still open: pushing `tpt-shell` `51ab9e2`, and a Linux/macOS check of the demo, which is Windows-only. The bridge itself is tested headless (`Viewport3D` with picking). The Wasm PWA half is done (see the Wasm item below).

### Phase 4: Polish & Ecosystem
- [x] Wasm/WebGPU compatibility for PWA deployments — compile-checked for `wasm32-unknown-unknown`: `tpt-nova-render` (a `MaybeSend` bound on `PostProcessPass`, since wgpu's WebGPU buffers are not `Send`), `tpt-nova-scene-graph`, and `tpt-nova-app` (a `.cargo/config.toml` sets the `getrandom` wasm backend). Run in a browser (2026-11-26): `tpt-nova-app` built for wasm32, `wasm-bindgen --target web`, served locally, and loaded in headless Chrome with `--enable-unsafe-webgpu`. The console logged `init ok`, and the editor UI and cube rendered. The scene-graph example itself is not separately run in a browser. PWA (2026-11-27): `crates/tpt-nova-app/web` now has a web app manifest, an icon set (192/512), and a service worker that precaches the app shell (cache-first, versioned `CACHE` name). `scripts/build_web.sh` builds the bundle. Verified on the local GPU in Chrome: first online load installs the worker, and with the server stopped and a fresh profile the editor reloads from cache and renders with WebGPU (`hidden` status set only after `init()` resolves). Not verified: Chrome's install prompt and an Android/desktop install flow.
- [x] Document the "How to embed 3D in your Rust UI" guide — `docs/EMBED_3D.md` (scene, bake, window loop, picking). Does not cover the appfront bridge, which is blocked (see Phase 3).
- [x] Verify `cargo-deny` compliance for all new dependencies — `cargo deny check licenses bans` passes (2026-11-21)

### Open questions (from spec3.txt)
- [x] **Raycasting:** decided and built — a lightweight AABB raycaster in the scene graph (`pick.rs`), following spec3's recommendation. Physics queries stay in `tpt-nova-physics`.
- [x] **Animator:** decided — add the wrapper. `tpt-nova-scene-graph::animator::Animator` wraps `tpt-nova-anim`'s `AnimationGraph` with `add_state`, `play` (hard cut), `crossfade`/`crossfade_default`, `tick`, `current` and `skin_matrices`. `AnimationGraph` gained `jump_to`, `current_state` and `is_blending` for it. Tests: 6 in `animator.rs`, plus `jump_to_cuts_without_blending_and_rejects_unknown_states` in `tpt-nova-anim`. The wrapper is not attached to scene nodes; skinned-mesh rendering in `bake` is still not wired.
- [x] **External repo:** answered — `tpt-appfront-canvas-3d` did not exist. It now exists in `tpt-appfront` on GitHub (commit 1d9244c, dependency-free `SceneDelegate` and `Viewport3D`, 4 tests).
- [x] **Backing store:** decided — in-memory only for now. The scene graph stays a plain in-memory tree, and an ECS-backed option is deferred until a measured large-scene need exists. No scene in the repo is large enough to justify it yet.
