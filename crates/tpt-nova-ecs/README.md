# tpt-nova-ecs

> Custom Entity-Component-System core for TPT Nova

A custom, data-oriented Entity-Component-System core with a scene graph, deterministic RNG, and serde support, designed to stay small enough for AI context windows.

Part of [TPT Nova](../../README.md), the AI-native, ECS-based real-time engine.

## Determinism

`World::entities()` returns entities sorted by index, so a tick visits them in the
same order on every run.

## Testing

```sh
cargo test -p tpt-nova-ecs
```

## License

Dual-licensed under either of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE), at your option.
