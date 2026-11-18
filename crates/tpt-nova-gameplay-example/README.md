# tpt-nova-gameplay-example

> Example hot-reloadable gameplay module: a player-controlled 2D entity

An example hot-reloadable gameplay module: a player-controlled 2D entity, built as a cdylib for hot reload and as an rlib for in-process tests.

Part of [TPT Nova](../../README.md), the AI-native, ECS-based real-time engine.

## Workspace dependencies

- `tpt-nova-ecs`
- `tpt-nova-physics`
- `tpt-nova-scripting`
- `tpt-nova-input`

## Testing

```sh
cargo test -p tpt-nova-gameplay-example
```

## License

Dual-licensed under either of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE), at your option.
