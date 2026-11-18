# tpt-nova-scripting

> Hot-reloadable Rust gameplay modules for TPT Nova: stable C-ABI boundary + dylib reload watcher

Hot-reloadable Rust gameplay modules: a stable C ABI boundary with a dylib reload watcher, so gameplay code can change without restarting the engine.

Part of [TPT Nova](../../README.md), the AI-native, ECS-based real-time engine.

## Workspace dependencies

- `tpt-nova-ecs`
- `tpt-nova-gameplay-example`
- `tpt-nova-input`

## Testing

```sh
cargo test -p tpt-nova-scripting
```

## License

Dual-licensed under either of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE), at your option.
