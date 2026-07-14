# AGENTS.md

Rust Cargo workspace (ECS-based real-time engine). Crates live under `crates/`;
see `README.md` for the per-crate map. `CLAUDE.md` and `CONTRIBUTING.md` hold
fuller detail — this file is only the non-obvious operational facts.

## CI gates (must be green before pushing)

Run in this order from repo root:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace --all-targets
cargo test --workspace
```

- `clippy ... -D warnings` turns every lint into a hard error: unused
  imports, dead code, and clippy suggestions block merge. Treat warnings as errors locally.
- CI runs the exact matrix on Windows / Linux / macOS.
- `cargo test -p nova-ecs` runs a single crate's tests.

## Roadmap truth

`todo.md` is the authoritative phase/status checklist — check it before
assuming a system exists. `spec.txt` is vision prose, not a task list.
When implementing a feature, find and tick the matching `todo.md` item.

## Workspace structure rules

- New crates must be added to `[workspace.members]` in root `Cargo.toml` *and*
  listed in `[workspace.dependencies]` if other crates depend on them.
- Cross-crate deps are declared once in `[workspace.dependencies]` and
  referenced with `version.workspace = true` etc. Don't hardcode per-crate.
- Don't create new crates speculatively — only when a `todo.md` item calls for one.

## Architecture quirks (resolved decisions — don't reopen)

- Strict ECS: prefer plain serializable/observable components over nested objects.
  The engine loop is deterministic (fixed-timestep, seeded RNG) and emits
  telemetry for an external AI agent to read and hot-apply. Keep state inspectable.
- Rendering = `wgpu` (Vulkan/DX12/Metal/WebGPU). Physics = Rapier
  (Rapier2D in `nova-physics`; Rapier3D for colliders in `nova-ingest`/`nova-splat`/`nova-videocap`).
- Editor = egui/eframe inside `nova-app`'s winit+wgpu shell. Do not reintroduce Tauri/ImGui.
- Two scripting tiers: hot-reloadable Rust dylibs (`nova-scripting`) for shipped code;
  sandboxed Rhai (`nova-scripting-embedded`) for AI logic. Rhai `eval`/`import` are disabled and
  capabilities are enforced by only registering granted functions.

## Untrusted-input surfaces — add hostile-input regression tests when touched

- `nova-agent-api`: parses an external control file. Caps: 1 MiB size,
  `MAX_COMMANDS_PER_POLL` = 10_000 commands, and a `PROTOCOL_VERSION` check.
- `nova-scripting-embedded`: Rhai sandbox — no filesystem/network, capability-gated functions.
- `nova-export`: unpacks `.novapack` archives; bounds-checks length prefixes and rejects
  path traversal (backslashes, `..`, absolute, empty segments).

## Run targets

- `cargo run -p nova-app` — interactive editor; **needs a GPU + drivers**.
- `cargo run -p nova-sample-game` — headless end-to-end pipeline demo (no GPU);
  also the forkable project template.
- `cargo run -p nova-ingest --example gen_sample_assets` — regenerates the only
  committed binary assets under `assets/` (`cube.glb`, `sample.splat`).
- Linux builds need system libs (libx11/libxrandr/libxi/libxcursor/libxkbcommon/libwayland/libudev/libasound2 + pkg-config).
- `nova-rag` defaults to `FeatureHashEmbedder`; the real local model is behind
  the `real-embeddings` feature (off by default).

## Commit hygiene

- Keep commits focused, short imperative subjects, explain *why*.
- Never commit secrets, model weights, or large binaries — only tiny `assets/` samples.
- Update `todo.md` checkboxes when closing roadmap items.
