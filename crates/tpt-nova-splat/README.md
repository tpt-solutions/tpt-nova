# tpt-nova-splat

> Gaussian Splat (3DGS) loading and collision proxy generation for TPT Nova

Gaussian Splat (3DGS) loading for `.splat` and `.ply` files, with convex-hull collision proxy generation so captured scenes take part in physics.

Part of [TPT Nova](../../README.md), the AI-native, ECS-based real-time engine.

## Features

| Feature | Enables |
|---------|---------|
| `render` | Enables the wgpu-backed splat render pipeline (`SplatPipeline`). |

## Workspace dependencies

- `tpt-nova-ecs`
- `tpt-nova-ingest`

## Testing

```sh
cargo test -p tpt-nova-splat
```

## License

Dual-licensed under either of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE), at your option.
