# Changelog - tpt-nova-anim

All notable changes to this crate are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/). Workspace-wide history is in
[../../CHANGELOG.md](../../CHANGELOG.md).

## [Unreleased]

### Changed
- Renamed the package from `nova-anim` to `tpt-nova-anim` (workspace-wide `tpt-nova-` crate prefix).

### Added
- `AnimationGraph::jump_to`, `current_state`, and `is_blending`. The `Animator` wrapper in `tpt-nova-scene-graph` is built on these.
- Per-crate `README.md`, `CHANGELOG.md`, `keywords`, and `categories` metadata.
