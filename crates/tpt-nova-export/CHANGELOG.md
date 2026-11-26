# Changelog - tpt-nova-export

All notable changes to this crate are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/). Workspace-wide history is in
[../../CHANGELOG.md](../../CHANGELOG.md).

## [Unreleased]

### Changed
- Renamed the package from `nova-export` to `tpt-nova-export` (workspace-wide `tpt-nova-` crate prefix).

### Added
- Content-addressed driver bundles (`driver_bundle.rs`). A driver's manifest and artifact are stored under `drivers/<sha256>/`, where the address is the SHA-256 of the artifact. `unpack_driver` recomputes the hash and refuses an artifact that does not match its address or its manifest. Each version keeps its own address, so rollback stays safe.
- Per-crate `README.md`, `CHANGELOG.md`, `keywords`, and `categories` metadata.
