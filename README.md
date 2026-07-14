# TPT Nova

An AI-native, ECS-based real-time engine that acts as the structural anchor
between generative AI outputs (video, images, meshes, splats) and interactive,
physics-driven, game-ready runtimes.

- Design & vision: [spec.txt](spec.txt)
- Build checklist / roadmap: [todo.md](todo.md)
- Alpha release gate: [docs/ALPHA_CHECKLIST.md](docs/ALPHA_CHECKLIST.md)
- Changelog: [CHANGELOG.md](CHANGELOG.md)

## Workspace crates

| Crate | Role |
|-------|------|
| `nova-ecs` | Entity/component storage, scene graph, scheduler, deterministic RNG |
| `nova-render` | wgpu renderer (cube, PBR, sprite pipelines) |
| `nova-input` | Keyboard/mouse → ECS input + action mapping |
| `nova-physics` | Rapier2D integration + sync step |
| `nova-ingest` | glTF/OBJ loading, VHACD decompose, auto-rig, Rapier3D colliders |
| `nova-scene` | Versioned scene (de)serialization (RON/JSON) + migrations |
| `nova-audio` | 2D/3D spatial audio |
| `nova-scripting` | Rust-native hot-reload gameplay |
| `nova-scripting-embedded` | Rhai sandboxed scripting |
| `nova-ui` | Immediate-mode in-game + world-space UI |
| `nova-editor` | egui/eframe scene editor (hierarchy, inspector, gizmos, Vibe GUI) |
| `nova-anim` | Skeletal animation + blending |
| `nova-telemetry` | JSON/MessagePack telemetry + AI control loop |
| `nova-neural-materials` | Live video-LLM texture feeds |
| `nova-splat` | Gaussian Splat loading + convex-hull collision proxy |
| `nova-rag` | Local vector DB + RAG retrieval over project assets/docs |
| `nova-export` | Standalone packaging + `.novapack` asset bundling |
| `nova-agent-api` | Stable, versioned external-AI-agent control API |
| `nova-overlay` | "Highlight & Fix" region selection → AI fix prompt |
| `nova-videocap` | Depth + segmentation mask → collision proxies |
| `nova-sample-game` | End-to-end sample wiring the full pipeline |

## Quick start

```sh
cargo build --workspace
cargo test --workspace
cargo run -p nova-sample-game   # runs the end-to-end pipeline headlessly
```

The self-debugging loop: the engine emits telemetry (`nova-telemetry`) and
hot-applies a control file written by an external AI agent
(`nova-agent-api`), closing the loop without a human in the middle.

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE).
