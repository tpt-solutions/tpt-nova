# tpt-nova-render

> wgpu-based rendering for TPT Nova

wgpu rendering for TPT Nova: the cube pipeline, a forward PBR pipeline with a shadow-casting directional light, 2D sprite batching, the UI overlay pass, a post-processing chain, and offscreen rendering.

Part of [TPT Nova](../../README.md), the AI-native, ECS-based real-time engine.

## Notes

- `OffscreenRenderer` renders the cube pass into an RGBA8 texture and reads it back, with no window.
- `PostProcessChain` (bloom, tone mapping) is the target of `tpt-nova-scene-graph`'s post-process lowering.
- `PbrScene` extracts camera, shadow light, and mesh model matrices from the ECS as plain data, so PBR frames can be checked in headless tests.
- The renderer clears with the first `ClearColor` component in the world, falling back to the engine default.
- The crate builds for `wasm32`. It re-exports `wgpu`, so downstream crates share the same device and texture types.

## Features

| Feature | Enables |
|---------|---------|
| `neural` | Wires a live `tpt-nova-neural-materials` feed onto in-scene surfaces: entities with a `MaterialBinding` get their latest decoded frame uploaded as an albedo texture. |

## Workspace dependencies

- `tpt-nova-ecs`
- `tpt-nova-ui`

## Testing

```sh
cargo test -p tpt-nova-render
```

## License

Dual-licensed under either of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE), at your option.
