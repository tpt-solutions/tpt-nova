# tpt-nova-app

> TPT Nova runtime application shell (tpt-window + wgpu + ECS)

The runtime application shell: a tpt-window window, wgpu rendering, a fixed-timestep loop, the live editor UI, and telemetry/control-file hot-apply.

Part of [TPT Nova](../../README.md), the AI-native, ECS-based real-time engine.

## Windowing and input

The window comes from `tpt-window` (a git dependency), not `winit`. The caller owns the event loop and renders on event batches. Because `tpt-window` reports physical keys, editor shortcuts use QWERTY letter positions. Text input comes from IME commits.

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
scripts/build_web.sh   # release build + wasm-bindgen into crates/tpt-nova-app/web
python -m http.server 8080 --directory crates/tpt-nova-app/web
```

Then open http://localhost:8080. The page is `web/index.html`. The generated
`tpt-nova-app.js` and `.wasm` files are build output and should not be committed.

### Installable PWA

The `web/` folder is also an installable PWA:

- `manifest.webmanifest` and the icons in `web/icons/` make the page installable
  as a standalone app.
- `sw.js` precaches the app shell (page, manifest, icons, the generated JS and
  WASM) and serves it cache-first, so the editor opens offline after the first
  visit.
- The cache name is versioned (`CACHE` in `sw.js`). Bump it whenever a shell file
  changes, or clients keep the old shell. A browser may serve the previous shell
  for one more visit after an update; that is the normal service-worker lag.

Service workers need a secure context: `localhost` for development, HTTPS for
anything deployed.

## License

Dual-licensed under either of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE), at your option.
