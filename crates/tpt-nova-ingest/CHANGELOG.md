# Changelog - tpt-nova-ingest

All notable changes to this crate are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/). Workspace-wide history is in
[../../CHANGELOG.md](../../CHANGELOG.md).

## [Unreleased]

### Changed
- Renamed the package from `nova-ingest` to `tpt-nova-ingest` (workspace-wide `tpt-nova-` crate prefix).
- Removed the `rapier3d` and `parry3d` dependencies (Apache-2.0 only).
  - Convex hulls come from the in-house quickhull in `hull.rs`.
  - Collision uses the in-house 3D rigid-body solver in `solver3d.rs` for convex-hull compounds. It has no sleeping or continuous collision, so fast small bodies can tunnel.
- Convex decomposition is an in-house, top-down approximation: the most concave part is split by the cut with the smallest total child hull volume. It is not V-HACD, so part boundaries differ.

### Added
- Per-crate `README.md`, `CHANGELOG.md`, `keywords`, and `categories` metadata.
