# Changelog - tpt-nova-agent-api

All notable changes to this crate are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/). Workspace-wide history is in
[../../CHANGELOG.md](../../CHANGELOG.md).

## [Unreleased]

### Changed
- Renamed the package from `nova-agent-api` to `tpt-nova-agent-api` (workspace-wide `tpt-nova-` crate prefix).

### Added
- Splat commands are exposed over the control channel. The crate depends on `tpt-nova-splat`, and an integration test covers splat commands end to end.
- Per-crate `README.md`, `CHANGELOG.md`, `keywords`, and `categories` metadata.
