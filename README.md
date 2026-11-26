# TPT Nova

An AI-native, ECS-based real-time engine that acts as the structural anchor
between generative AI outputs (video, images, meshes, splats) and interactive,
physics-driven, game-ready runtimes.

- Design & vision: [spec.txt](spec.txt)
- Build checklist / roadmap: [todo.md](todo.md)
- Getting started (clone → build → run a window): [GETTING_STARTED.md](GETTING_STARTED.md)
- Contributing / CI gates: [CONTRIBUTING.md](CONTRIBUTING.md)
- Alpha release gate: [docs/ALPHA_CHECKLIST.md](docs/ALPHA_CHECKLIST.md)
- Changelog: [CHANGELOG.md](CHANGELOG.md) (workspace-wide; each crate also has its own `CHANGELOG.md`)

## Workspace crates

Every crate is implemented. Each one has its own `README.md` and `CHANGELOG.md`.

| Crate | Role |
|-------|------|
| `tpt-nova-ecs` | Entity/component storage, scene graph, scheduler, deterministic RNG |
| `tpt-nova-render` | wgpu renderer (cube, PBR, sprite pipelines), post-processing, offscreen rendering |
| `tpt-nova-input` | Keyboard/mouse → ECS input + action mapping (on `tpt-window` events) |
| `tpt-nova-physics` | In-house 2D rigid-body solver + deterministic sync step |
| `tpt-nova-ingest` | glTF/OBJ loading, convex decomposition, hulls, auto-rig, in-house 3D collider solver |
| `tpt-nova-scene` | Versioned scene (de)serialization (RON/JSON) + migrations |
| `tpt-nova-audio` | 2D/3D spatial audio (WAV, FLAC, Ogg, MP3) |
| `tpt-nova-scripting` | Rust-native hot-reload gameplay |
| `tpt-nova-scripting-embedded` | Rhai sandboxed scripting |
| `tpt-nova-ui` | Immediate-mode in-game + world-space UI |
| `tpt-nova-editor` | Bespoke immediate-mode scene editor (hierarchy, inspector, gizmos, Vibe GUI) on `tpt-nova-ui` |
| `tpt-nova-anim` | Skeletal animation + blending + state machine |
| `tpt-nova-telemetry` | JSON/MessagePack telemetry + AI control loop |
| `tpt-nova-neural-materials` | Live video-LLM texture feeds |
| `tpt-nova-splat` | Gaussian Splat loading, editing, LOD, multi-part collision proxies, depth-sorted renderer |
| `tpt-nova-rag` | Local vector DB + RAG retrieval over project assets/docs |
| `tpt-nova-export` | Standalone packaging, `.novapack` asset bundling, content-addressed driver bundles |
| `tpt-nova-agent-api` | Stable, versioned external-AI-agent control API |
| `tpt-nova-overlay` | "Highlight & Fix" region selection → AI fix prompt |
| `tpt-nova-videocap` | Depth + segmentation mask → collision proxies |
| `tpt-nova-sample-game` | End-to-end sample wiring the full pipeline |
| `tpt-nova-app` | Runtime shell: `tpt-window` + wgpu loop, live editor host, browser/PWA build |
| `tpt-nova-scene-graph` | Declarative 3D scene graph baked into ECS components; `Animator`; picking; opt-in appfront bridge |
| `tpt-nova-drivers` | Hot-swappable behaviour drivers with verified self-repair (Phase 8, partial) |
| `tpt-nova-xtask` | Developer task runner (GPU/driver readiness check, scaffolding helpers) |
| `tpt-nova-gameplay-example` | Example hot-reloadable gameplay module (player-controlled 2D entity) |

## Quick start

```sh
cargo build --workspace
cargo test --workspace
cargo run -p tpt-nova-sample-game   # runs the end-to-end pipeline headlessly
cargo run -p tpt-nova-app           # opens the interactive editor (needs a GPU)
```

`cargo run -p tpt-nova-app` opens a window with a live editor — a 3D viewport, a
hierarchy/inspector/asset panel stack, and a toolbar. Drag in the viewport to
move the selected entity, drag inspector fields to edit components live, press
`H` then drag a marquee to build a "Highlight & Fix" AI request, and `P` to
play/pause the simulation. See [GETTING_STARTED.md](GETTING_STARTED.md) for the
full control list and clone-to-window walkthrough.

The self-debugging loop: the engine emits telemetry (`tpt-nova-telemetry`) and
hot-applies a control file written by an external AI agent
(`tpt-nova-agent-api`), closing the loop without a human in the middle. See
`cargo run -p tpt-nova-agent-api --example agent_fix_loop` for a flagship,
end-to-end demo wiring `tpt-nova-rag` (context retrieval), `tpt-nova-agent-api`
(commands), and `tpt-nova-overlay` (highlight → fix prompt) together.

`tpt-nova-sample-game` also works as a forkable project template: it wires the
full physics/scene/agent/splat/export pipeline together and is a reasonable
starting point for a new game built on the engine, not just a smoke test.

The same app builds for the browser (WebGPU) and installs as a PWA. Run
`scripts/build_web.sh`, then serve `crates/tpt-nova-app/web`. Steps are in
[crates/tpt-nova-app/README.md](crates/tpt-nova-app/README.md).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT License ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
