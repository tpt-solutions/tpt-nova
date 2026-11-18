# tpt-nova-physics

> Rapier2D physics integration for TPT Nova's ECS (components + deterministic sync step)

Rapier2D physics exposed as plain ECS components, with a deterministic fixed-step sync that keeps Rapier state out of gameplay code.

Part of [TPT Nova](../../README.md), the AI-native, ECS-based real-time engine.

## Workspace dependencies

- `tpt-nova-ecs`
- `tpt-nova-telemetry`

## Testing

```sh
cargo test -p tpt-nova-physics
```

## License

Dual-licensed under either of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE), at your option.
