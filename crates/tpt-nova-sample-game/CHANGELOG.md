# Changelog - tpt-nova-sample-game

All notable changes to this crate are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/). Workspace-wide history is in
[../../CHANGELOG.md](../../CHANGELOG.md).

## [Unreleased]

### Changed
- Renamed the package from `nova-sample-game` to `tpt-nova-sample-game` (workspace-wide `tpt-nova-` crate prefix).
- The splat step derives a multi-part proxy collider (`build_proxy_collider`) instead of a single convex hull. Test splats are opaque, so the cloud spans 3D.
- The physics step runs on the in-house 3D solver in `tpt-nova-ingest`. Rapier is no longer used.

### Added
- Per-crate `README.md`, `CHANGELOG.md`, `keywords`, and `categories` metadata.
