# Changelog - tpt-nova-ecs

All notable changes to this crate are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/). Workspace-wide history is in
[../../CHANGELOG.md](../../CHANGELOG.md).

## [Unreleased]

### Changed
- Renamed the package from `nova-ecs` to `tpt-nova-ecs` (workspace-wide `tpt-nova-` crate prefix).

### Fixed
- `World::entities()` collected entities into a `HashMap` and returned them in hash order, which is randomised per process. Entities are now sorted by index, so a tick visits them in the same order on every run.

### Added
- Per-crate `README.md`, `CHANGELOG.md`, `keywords`, and `categories` metadata.
