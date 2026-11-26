# tpt-nova-export

> Standalone packaging + asset bundling for TPT Nova games

Standalone packaging: a dependency-free `.novapack` asset container and per-platform game bundling, with hardened path-traversal checks on archive entries.

Part of [TPT Nova](../../README.md), the AI-native, ECS-based real-time engine.

## Driver bundles

`driver_bundle` stores a driver's manifest and artifact content-addressed under `drivers/<sha256>/`, where the address is the SHA-256 of the artifact. `unpack_driver` recomputes the hash and refuses an artifact that does not match its address or its manifest. Each version keeps its own address, so rolling back to an earlier driver is safe.

## Testing

```sh
cargo test -p tpt-nova-export
```

## License

Dual-licensed under either of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE), at your option.
