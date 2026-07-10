# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

TPT Nova is an early-stage (pre-production), AI-native, ECS-based real-time engine written in Rust. It aims to act as a "structural anchor" that wraps AI-generated visual media (video, images, meshes, Gaussian splats) in interactive, physics-driven, deterministic ECS objects. Full vision and rationale: [spec.txt](spec.txt). Build roadmap and current progress: [todo.md](todo.md) — this is the source of truth for what phase the project is in; check it before assuming a system exists.

Phases 0 and 1 are complete, and Phase 2 (2D "vibe" sandbox + playable-game basics) is implemented. The workspace builds, and `cargo fmt --check`, `cargo clippy -D warnings`, and `cargo test` are all green (CI runs this matrix on Windows/Linux/macOS via `.github/workflows/ci.yml`). Implemented crates: `nova-ecs` (ECS core + scene graph + serde), `nova-telemetry` (JSON + MessagePack sinks), `nova-render` (wgpu cube pipeline + 2D sprite batching/atlas pipeline), `nova-input`, `nova-app` (winit shell), `nova-physics` (Rapier2D components + deterministic sync step), `nova-scene` (RON/JSON save/load with versioned migrations), `nova-audio` (SFX/music/bus mixing over rodio), `nova-scripting` (hot-reloadable gameplay dylibs over a C ABI) with `nova-gameplay-example` (a player controller), `nova-ui` (immediate-mode widgets → draw list), and `nova-editor` (hierarchy/inspector/gizmos + the Bézier "Vibe GUI" bound live to physics gravity). Always check `todo.md` for the authoritative phase status before assuming a system's maturity.

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
- **Planned crate boundaries** (per `todo.md`; not all exist yet):
  - `nova-ecs` — entity/component storage, queries, scheduler, scene graph (Transform hierarchy).
  - `nova-telemetry` — JSON/MessagePack schema and emission for engine/entity state.
  - `nova-render` — wgpu-based rendering (starts as a 3D cube, grows into a forward+ PBR pipeline).
  - `nova-app` — the application/window shell (winit + wgpu device/surface setup, main loop).
  - Later phases introduce further crates as needed: `nova-input`, `nova-audio`, `nova-scripting` (hot-reloadable gameplay dylibs), `nova-ui`, `nova-anim`, `nova-ingest` (mesh/.glb ingestion, VHACD colliders, auto-rigging), `nova-neural-materials`, `nova-export`, `nova-rag`. Don't create these speculatively — add them when the corresponding `todo.md` phase is actually being worked on.
- **Rendering** targets `wgpu` (Vulkan/DX12/Metal/WebGPU) rather than a single native API, for cross-platform portability.
- **Physics** is planned via Rapier (Rapier2D in Phase 2, Rapier3D in Phase 3), integrated as ECS components/systems rather than a separate simulation silo.
- **Gameplay scripting** is planned as hot-reloadable Rust dylibs first (Phase 2), with an embedded scripting layer (Rhai or WASM — undecided, see Open Decisions in `todo.md`) added later for sandboxed AI-generated scripts.
- **Editor**: framework choice (Tauri/web vs. custom ImGui) is an open decision (see `todo.md` "Open Decisions") to be resolved before Phase 2 GUI work — don't assume one when touching editor-related code.

## Working in this repo

- `todo.md` is the authoritative checklist/roadmap; when implementing a feature, find and check off the corresponding item rather than inferring scope from `spec.txt` alone (`spec.txt` is the high-level vision doc, not a task list).
- New crates must be added both as a directory under `crates/` *and* listed in the root `Cargo.toml`'s `[workspace.members]` (and typically `[workspace.dependencies]` if other crates depend on it).
