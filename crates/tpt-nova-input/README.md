# tpt-nova-input

> Keyboard/mouse input and action mapping for TPT Nova, wired into the ECS

Keyboard and mouse input as an ECS resource, with semantic action mapping so bindings are data rather than code.

Part of [TPT Nova](../../README.md), the AI-native, ECS-based real-time engine.

## Notes

- Input consumes `tpt-window` events. Keys are physical (`KeyCode`), so bindings
  refer to key positions, not layout-dependent labels.
- Text comes from IME commit events, because `tpt-window` keyboard events carry physical keys only.

## Testing

```sh
cargo test -p tpt-nova-input
```

## License

Dual-licensed under either of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE), at your option.
