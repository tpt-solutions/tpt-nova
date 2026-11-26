# Changelog - tpt-nova-physics

All notable changes to this crate are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/). Workspace-wide history is in
[../../CHANGELOG.md](../../CHANGELOG.md).

## [Unreleased]

### Changed
- Renamed the package from `nova-physics` to `tpt-nova-physics` (workspace-wide `tpt-nova-` crate prefix).
- Removed the Apache-2.0-only `rapier2d` dependency. The rigid-body solver is in-house (`solver.rs`): circles and convex polygons, capsules approximated as stadiums, and a sequential-impulse contact solver with a fixed iteration count.

### Added
- Per-crate `README.md`, `CHANGELOG.md`, `keywords`, and `categories` metadata.
