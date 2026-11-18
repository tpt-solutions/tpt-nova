# tpt-nova-telemetry

> Structured JSON/MessagePack telemetry for TPT Nova's AI-observable engine state

Structured JSON and MessagePack telemetry of ECS state, emitted so external AI agents can observe the engine and close the self-debugging loop.

Part of [TPT Nova](../../README.md), the AI-native, ECS-based real-time engine.

## Workspace dependencies

- `tpt-nova-ecs`
- `tpt-nova-physics`

## Testing

```sh
cargo test -p tpt-nova-telemetry
```

## License

Dual-licensed under either of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE), at your option.
