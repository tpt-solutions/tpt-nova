# Changelog - tpt-nova-render

All notable changes to this crate are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/). Workspace-wide history is in
[../../CHANGELOG.md](../../CHANGELOG.md).

## [Unreleased]

### Changed
- Renamed the package from `nova-render` to `tpt-nova-render` (workspace-wide `tpt-nova-` crate prefix).
- The wgpu surface is created from `tpt-window` raw window handles (`unsafe`, documented). The window is declared last so it drops after the surface.
- Neural-material imports are gated behind the `neural` feature.

### Added
- `OffscreenRenderer` renders the cube pass into an RGBA8 texture and reads it back, with no window.
- Post-processing chain (`post` module): `PostProcessChain`, `PostProcessPass`, `BloomSettings`, and `ToneMapping`.
- `PbrScene` extracts the camera, shadow light, and mesh model matrices from the ECS as plain data, so PBR frames can be asserted in headless tests. `PbrRenderer::render` consumes it.
- `ClearColor` component. The renderer clears with the first one in the world and falls back to the engine default.
- `wgpu` is re-exported, so downstream crates share the same device and texture types.
- Builds for `wasm32`. `PostProcessPass` uses a `MaybeSend` supertrait: `Send` on native, unconstrained on wasm32, where WebGPU buffers are not `Send`.
- Per-crate `README.md`, `CHANGELOG.md`, `keywords`, and `categories` metadata.

### Fixed
- The live renderer wrote every cube's transform into one shared uniform buffer inside the draw loop. Queued writes take effect at submit, so every cube drew with the last transform. Each draw now has its own uniform buffer and bind group, in both the base and neural loops.
- The scene background was dropped. It is now written as a `ClearColor`.
