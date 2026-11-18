# tpt-nova-scripting-embedded

> Sandboxed, embedded (Rhai) scripting for TPT Nova, with capability boundaries for AI-generated scripts

Sandboxed, embedded Rhai scripting for AI-generated and user-authored logic, with an explicit capability boundary around what scripts can call.

Part of [TPT Nova](../../README.md), the AI-native, ECS-based real-time engine.

## Workspace dependencies

- `tpt-nova-ecs`

## Testing

```sh
cargo test -p tpt-nova-scripting-embedded
```

## License

Dual-licensed under either of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE), at your option.
