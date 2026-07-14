# Getting Started with TPT Nova

This guide takes you from a fresh clone to a window with a live, interactive
editor. It assumes a working Rust toolchain (stable, latest).

## 1. Prerequisites

- **Rust** (stable, latest): install via [rustup](https://rustup.rs).
- **Platform system libraries** (needed to build `winit`/`wgpu`):
  - **Linux:** `libx11-dev libxrandr-dev libxi-dev libxcursor-dev
    libxkbcommon-dev libwayland-dev libudev-dev libasound2-dev pkg-config`
    (or the equivalent for your distro).
  - **Windows / macOS:** nothing extra — the toolchain provides what's needed.
- **A GPU** with up-to-date drivers (the renderer uses `wgpu`/Vulkan/Metal/DX12).

## 2. Clone & build

```sh
git clone https://github.com/tpt-nova/tpt-nova.git
cd tpt-nova
cargo build --workspace
```

The first build pulls and compiles a few heavier dependencies (Rapier physics,
`wgpu`). Subsequent builds are much faster.

## 3. Run the editor (see a window)

```sh
cargo run -p nova-app
```

You should see a window titled **"TPT Nova — Editor"** containing:

- a 3D **viewport** (a cube, lit, on the center of the screen),
- a **Hierarchy** panel (left) listing scene entities,
- an **Inspector** (right) where you can drag component values
  (`Transform.translation.x`, `scale`, …) and have them apply live,
- an **Assets** browser (bottom-left) with a Play/Pause toggle,
- a **toolbar** (top) with editor/tool/gizmo/play/undo/redo controls.

### Editor controls

| Key / mouse                       | Action                                         |
|-----------------------------------|------------------------------------------------|
| Click an entity in the viewport  | Select it (gizmo handle appears)              |
| Shift-click entity (viewport or Hierarchy) | Toggle it in the multi-selection    |
| Drag in the viewport (Gizmo tool)| Move / rotate / scale the selection (undoable)|
| Mouse wheel                       | Dolly the viewport camera (zoom)              |
| Click an asset, then click viewport | Spawn that asset as a new entity            |
| `G`                               | Cycle gizmo mode: Move → Rotate → Scale        |
| `H`                               | Toggle the "Highlight & Fix" marquee tool     |
| Click the instruction field (Highlight tool), then type | Set the fix request text (replaces the old hardcoded `"fix selection"`) |
| Drag a marquee (Highlight tool)   | Select a region → builds an AI fix request     |
| `P`                               | Toggle play / pause (simulation)              |
| `E`                               | Toggle the editor UI off/on                   |
| `Ctrl+Z` / `Ctrl+Y`               | Undo / redo inspector **and** gizmo edits      |
| `Esc`                             | Clear selection / unfocus the text field       |
| WASD / arrows                     | (Cube) spin via the movement system            |

There is also a **Vibe GUI** (Bézier curve editor) panel along the bottom of
the viewport: drag its control points to shape a curve that maps to a gravity
magnitude, and watch the live `gravity y` read-out update.

> **Screenshots / clip:** A short capture of the running editor (the cube
> viewport with the Hierarchy / Inspector / Assets / Vibe panels and the
> Highlight & Fix marquee) is an outstanding `ALPHA_CHECKLIST.md` release-gate
> item. It requires a **manual capture** step (run `cargo run -p nova-app` on a
> machine with a GPU, then grab a frame) — drop the image here and reference it
> once captured.
>
> ![TPT Nova editor](docs/editor_screenshot.png)

The editor is fully wired to the renderer: every panel is a `nova-ui`
`DrawList` composited over the 3D scene by `nova-render`'s `UiOverlay` pass, so
what you see is the real, interactive engine — not a mock-up.

## 4. Run the headless pipeline demo

No GPU required for the end-to-end sample that exercises the *whole* pipeline
(ECS + Rapier3D physics + scene save/load + agent control API + splat ingestion
+ asset packaging):

```sh
cargo run -p nova-sample-game
```

## 5. Ingest a mesh (zero-config)

```sh
cargo run -p nova-ingest --bin ingest_demo        # uses assets/cube.glb
cargo run -p nova-ingest --bin ingest_demo my_model.glb
```

This loads the mesh, auto-generates a convex-decomposition collider and an
auto-rig, drops it onto a ground plane, and simulates the fall.

## 6. Tests

```sh
cargo test --workspace
```

The workspace enforces `cargo fmt --all -- --check` and
`cargo clippy --workspace --all-targets -- -D warnings` in CI; run them locally
before opening a PR (see `CONTRIBUTING.md`).

## Next steps

- Read the [README](README.md) for the crate map and the self-debugging loop.
- Read [todo.md](todo.md) for the full roadmap and what "Alpha" means here.
- The engine is AI-native: an external agent can read telemetry and hot-apply
  changes via a control file — see `nova-agent-api` and the
  [Agent Protocol reference](docs/AGENT_PROTOCOL.md).
- Want to build your own game on the engine? Fork the sample project instead of
  copying it by hand: `pwsh scripts/fork_sample_game.ps1 -Name my-game`.
