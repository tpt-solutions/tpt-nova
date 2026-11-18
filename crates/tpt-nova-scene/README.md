# tpt-nova-scene

> Scene (de)serialization for TPT Nova: dump/restore the full ECS world to RON/JSON with versioned migrations

Versioned scene save/load: dump the ECS world to RON or JSON, remap entity references on restore, and run schema migrations on older files.

Part of [TPT Nova](../../README.md), the AI-native, ECS-based real-time engine.

## Workspace dependencies

- `tpt-nova-ecs`
- `tpt-nova-physics`

## Testing

```sh
cargo test -p tpt-nova-scene
```

## License

Dual-licensed under either of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE), at your option.
