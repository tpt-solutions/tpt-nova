# Changelog - tpt-nova-input

All notable changes to this crate are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/). Workspace-wide history is in
[../../CHANGELOG.md](../../CHANGELOG.md).

## [Unreleased]

### Changed
- Renamed the package from `nova-input` to `tpt-nova-input` (workspace-wide `tpt-nova-` crate prefix).
- Consumes `tpt-window` events instead of `winit` events. Text input now comes from IME commits, because `tpt-window` keyboard events carry physical keys only.

### Added
- Per-crate `README.md`, `CHANGELOG.md`, `keywords`, and `categories` metadata.
