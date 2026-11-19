# tpt-nova-app

> TPT Nova runtime application shell (tpt-window + wgpu + ECS)

The runtime application shell: a tpt-window window, wgpu rendering, a fixed-timestep loop, the live editor UI, and telemetry/control-file hot-apply.

Part of [TPT Nova](../../README.md), the AI-native, ECS-based real-time engine.

## Workspace dependencies

- `tpt-nova-ecs`
- `tpt-nova-input`
- `tpt-nova-render`
- `tpt-nova-telemetry`
- `tpt-nova-ui`
- `tpt-nova-editor`
- `tpt-nova-overlay`
- `tpt-nova-neural-materials`
- `tpt-nova-scene`

## Testing

```sh
cargo test -p tpt-nova-app
```

## License

Dual-licensed under either of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE), at your option.
