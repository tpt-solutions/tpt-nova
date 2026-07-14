# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

TPT Nova is an AI-native, ECS-based real-time engine written in Rust, currently at Alpha (Phase 6 of `todo.md` complete). It aims to act as a "structural anchor" that wraps AI-generated visual media (video, images, meshes, Gaussian splats) in interactive, physics-driven, deterministic ECS objects. Full vision and rationale: [spec.txt](spec.txt). Build roadmap and current progress: [todo.md](todo.md) — this is the source of truth for what phase the project is in; check it before assuming a system exists.

Phases 0-5 are complete, and Phase 6 (editor integration, onboarding, and agent-loop closure — a post-Alpha hardening pass) is also complete per `todo.md`. The workspace builds, and `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`, and `cargo test --workspace` are all green (CI runs this matrix on Windows/Linux/macOS via `.github/workflows/ci.yml`, with a `cargo tarpaulin` coverage job as a soft gate). Every crate has `#[test]` coverage; see the "Testing Coverage" and "Phase 6" sections of `todo.md` for what's covered, and the 2026-07-15 audit note for the security-review pass over `nova-agent-api`, `nova-scripting-embedded`, and `nova-export`.

Implemented crates:
- Core: `nova-ecs` (ECS core + scene graph + serde + virtual-camera and light components), `nova-telemetry` (JSON + MessagePack sinks), `nova-render` (wgpu cube pipeline, 2D sprite batching/atlas pipeline, and a forward PBR pipeline with a shadow-casting directional light in `pbr.rs`), `nova-input`, `nova-app` (winit shell, now hosting the live editor UI).
- Simulation & content: `nova-physics` (Rapier2D components + deterministic sync step), `nova-scene` (RON/JSON save/load, versioned migrations, corruption handling), `nova-audio` (SFX/music/bus mixing over rodio with 3D spatial audio), `nova-anim` (skeletal skinning, keyframe sampling, pose blending, animation state machine), `nova-ingest` (.glb/.obj loading, VHACD convex decomposition, procedural auto-rig, Rapier3D collider generation), `nova-splat` (Gaussian Splat `.splat`/`.ply` loading + convex-hull collision proxy generation).
- Scripting: `nova-scripting` (hot-reloadable Rust gameplay dylibs over a C ABI, with `nova-gameplay-example` as a player controller) and `nova-scripting-embedded` (sandboxed Rhai scripting behind a capability boundary — scripts call only host-registered, capability-gated functions and enqueue typed `ScriptCommand`s the host applies to the `World`).
- UI & editor: `nova-ui` (immediate-mode widgets → draw list, plus world-space anchored widgets, now rendered via a real pass in `nova-render`/`nova-app`), `nova-editor` (hierarchy/inspector/2D+3D gizmos, undo/redo + multi-select, an asset browser, a play-in-editor toggle, and the Bézier "Vibe GUI" bound live to physics gravity — all built on the bespoke `nova-ui` stack, not egui/eframe; see below), `nova-overlay` ("Highlight & Fix" region selection in the live viewport → AI fix prompt).
- Generative/AI bridges: `nova-neural-materials` (live video-LLM texture feeds: a transport-agnostic `FrameSource`/`NeuralMaterialProvider` contract plus `NeuralTexture` GPU upload and a `NeuralMaterialRegistry` ECS resource, with a network-free `MockProvider` for tests/demos), `nova-videocap` (depth map + segmentation mask ingestion → `Collider3D`/Rapier3D collision proxies), `nova-rag` (local vector DB over project assets/docs, with a real local embedding model behind the `real-embeddings` feature alongside the default `FeatureHashEmbedder`), `nova-agent-api` (stable, versioned external-AI-agent control API: `AgentCommand`, `ControlChannel`, telemetry read-back, with a `MAX_COMMANDS_PER_POLL` cap against hostile control files).
- Shipping: `nova-export` (standalone per-platform packaging + `.novapack` asset bundling, with hardened path-traversal validation on archive entry names), `nova-sample-game` (an end-to-end sample wiring the full pipeline — physics rest, scene save/reload, agent spawn/move, splat→collider, asset pack — doubling as a forkable project template).

The flagship agent-loop demo (`crates/nova-agent-api/examples/agent_fix_loop.rs`) wires `nova-rag` (context) + `nova-agent-api` (commands) + `nova-overlay` (highlight → fix prompt) together end-to-end. Always check `todo.md` for the authoritative phase status before assuming a system's maturity — it is updated more frequently than this file.

## Commands

Standard Cargo workspace commands, run from the repo root:

- Build everything: `cargo build`
- Run tests: `cargo test`
- Run a single crate's tests: `cargo test -p nova-ecs`
- Check without building: `cargo check`
- Format: `cargo fmt`
- Lint: `cargo clippy`

## Architecture

- **Workspace layout**: a Cargo workspace (`Cargo.toml` at root) with crates under `crates/`. Shared package metadata (version, edition, license, repository) lives in `[workspace.package]`; crates inherit it via `version.workspace = true`, etc. Cross-crate dependencies should be declared once in `[workspace.dependencies]` and referenced with `.workspace = true` in each crate's `Cargo.toml`, following the pattern already used for `nova-ecs`, `nova-telemetry`, `nova-render`.
- **Core design principle — strict ECS, data over objects**: the engine is built around a flat, explicit Entity-Component-System architecture specifically so that state stays small and inspectable enough for an AI agent to reason about and modify without parsing large object hierarchies. When adding engine state, prefer plain components over nested/inherited structures.
- **Determinism & telemetry are first-class**: the engine loop is meant to be strictly deterministic (fixed-timestep, seeded RNG) and to emit structured JSON (later MessagePack) telemetry of entity/component state. This is the core AI-feedback loop the whole project exists to enable — an external agent reads telemetry, mutates code/component values, and the engine hot-applies the change. Keep this in mind when designing new components/systems: state should be serializable and observable, not hidden in opaque internal fields.
- **Crate boundaries**: see the crate list above and the table in `README.md` for what each crate under `crates/` owns. All crates listed there are implemented, not planned; don't create new crates speculatively — add them only when a corresponding `todo.md` item calls for one.
- **Rendering** targets `wgpu` (Vulkan/DX12/Metal/WebGPU) rather than a single native API, for cross-platform portability.
- **Physics** uses Rapier — Rapier2D (`nova-physics`) for 2D and Rapier3D (`nova-ingest`/`nova-splat`/`nova-videocap` collider generation) for 3D — integrated as ECS components/systems rather than a separate simulation silo.
- **Gameplay scripting** has two resolved tiers: hot-reloadable Rust dylibs (`nova-scripting`) for shipped/perf-critical systems, and embedded Rhai (`nova-scripting-embedded`) for AI-generated, sandboxed, hot-iterated logic. See "Open Decisions" in `todo.md` for the rationale.
- **Editor**: the actual front-end is a bespoke immediate-mode stack (`nova-ui` draw lists rendered by `nova-render`'s `UiOverlay` pass), hosted inside `nova-app`'s winit+wgpu shell. An earlier roadmap decision named egui/eframe, but it was never adopted — `nova-editor`/`nova-ui`/`nova-app` have no egui/eframe dependency. Don't reintroduce Tauri/ImGui/egui when touching editor-related code; extend the existing `nova-ui` widget set instead.

## Working in this repo

- `todo.md` is the authoritative checklist/roadmap; when implementing a feature, find and check off the corresponding item rather than inferring scope from `spec.txt` alone (`spec.txt` is the high-level vision doc, not a task list).
- New crates must be added both as a directory under `crates/` *and* listed in the root `Cargo.toml`'s `[workspace.members]` (and typically `[workspace.dependencies]` if other crates depend on it).
