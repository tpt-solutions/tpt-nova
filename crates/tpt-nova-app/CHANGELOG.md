# Changelog - tpt-nova-app

All notable changes to this crate are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/). Workspace-wide history is in
[../../CHANGELOG.md](../../CHANGELOG.md).

## [Unreleased]

### Changed
- Renamed the package from `nova-app` to `tpt-nova-app` (workspace-wide `tpt-nova-` crate prefix).
- The window shell moved from `winit` to `tpt-window` (git dependency). The caller owns the loop and renders on event batches. Modifier state is read from `InputState`. Shortcuts use QWERTY letter positions because `tpt-window` reports physical keys.

### Added
- Browser build: async renderer setup (`Renderer::new_async`), a `requestAnimationFrame`-driven loop (`Runtime::tick`), a browser host page, and panic reporting to the console. `std::time::Instant` is replaced by `web-time` on wasm32.
- Installable PWA in `web/`: a web app manifest, 192 and 512 px icons, and a service worker that precaches the app shell cache-first under a versioned cache name. `scripts/build_web.sh` builds the bundle.
- Demo splat cloud (4×4×4) spawned in the viewport. Highlight & Fix appends the selected splat-subset ranges to the fix prompt, not just the entity id.
- Per-crate `README.md`, `CHANGELOG.md`, `keywords`, and `categories` metadata.

### Fixed
- `getrandom` uses the `wasm_js` backend on wasm32 (set in `.cargo/config.toml`), which `ahash` → `tobj` → `tpt-nova-ingest` needs.
