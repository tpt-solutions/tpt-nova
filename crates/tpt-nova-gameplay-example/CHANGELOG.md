# Changelog - tpt-nova-gameplay-example

All notable changes to this crate are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/). Workspace-wide history is in
[../../CHANGELOG.md](../../CHANGELOG.md).

## [Unreleased]

### Changed
- Renamed the package from `nova-gameplay-example` to `tpt-nova-gameplay-example` (workspace-wide `tpt-nova-` crate prefix).
- Tests press the `move_right` binding with the `tpt-window` key code `KeyCode::D`, since `tpt-nova-input` now reads `tpt-window` events.

### Added
- Per-crate `README.md`, `CHANGELOG.md`, `keywords`, and `categories` metadata.
