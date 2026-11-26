# Changelog - tpt-nova-rag

All notable changes to this crate are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/). Workspace-wide history is in
[../../CHANGELOG.md](../../CHANGELOG.md).

## [Unreleased]

### Changed
- Renamed the package from `nova-rag` to `tpt-nova-rag` (workspace-wide `tpt-nova-` crate prefix).

### Fixed
- Pinned `ort-sys` to the exact `rc.4` release to match the `ort` that `fastembed` 3.14.1 pins. The caret range also accepted `ort-sys` rc.12, which broke the build. Dropped the `tls-rustls` feature that release no longer has, and updated `RealEmbedder` to `fastembed::Error`.

### Added
- Per-crate `README.md`, `CHANGELOG.md`, `keywords`, and `categories` metadata.
