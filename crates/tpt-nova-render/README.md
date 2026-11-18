# tpt-nova-render

> wgpu-based rendering for TPT Nova

wgpu rendering for TPT Nova: the cube pipeline, a forward PBR pipeline with a shadow-casting directional light, 2D sprite batching, and the UI overlay pass.

Part of [TPT Nova](../../README.md), the AI-native, ECS-based real-time engine.

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
