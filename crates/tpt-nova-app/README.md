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

## Running in the browser

The web build needs WebGPU (recent Chrome or Edge with GPU enabled). The
`wasm-bindgen` CLI must be the same version as the `wasm-bindgen` crate in
`Cargo.lock` (currently 0.2.130).

```sh
cargo build -p tpt-nova-app --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir crates/tpt-nova-app/web target/wasm32-unknown-unknown/debug/tpt-nova-app.wasm
python -m http.server 8080 --directory crates/tpt-nova-app/web
```

Then open http://localhost:8080. The page is `web/index.html`. The generated
`tpt-nova-app.js` and `.wasm` files are build output and should not be committed.

## License

Dual-licensed under either of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE), at your option.
