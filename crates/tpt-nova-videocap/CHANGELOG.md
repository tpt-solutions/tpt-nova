# Changelog - tpt-nova-videocap

All notable changes to this crate are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/). Workspace-wide history is in
[../../CHANGELOG.md](../../CHANGELOG.md).

## [Unreleased]

### Changed
- Renamed the package from `nova-videocap` to `tpt-nova-videocap` (workspace-wide `tpt-nova-` crate prefix).
- Removed the `parry3d` dependency. Depth-derived proxies are built with `tpt_nova_ingest::hull::convex_hull`.

### Added
- Per-crate `README.md`, `CHANGELOG.md`, `keywords`, and `categories` metadata.
