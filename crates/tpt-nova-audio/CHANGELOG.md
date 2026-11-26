# Changelog - tpt-nova-audio

All notable changes to this crate are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/). Workspace-wide history is in
[../../CHANGELOG.md](../../CHANGELOG.md).

## [Unreleased]

### Changed
- Renamed the package from `nova-audio` to `tpt-nova-audio` (workspace-wide `tpt-nova-` crate prefix).
- rodio is narrowed to the `playback` feature plus the `lewton` (Ogg Vorbis) and `minimp3` (MP3) decoders. The `symphonia` decoder stack is removed (MPL-2.0).

### Added
- In-house WAV decoder (`wav.rs`). rodio's own WAV path needs `hound`, which is Apache-2.0 only.
- FLAC decoding (`flac.rs`) through `tpt-av-cadence-flac` from the tpt-cadence suite (MIT OR Apache-2.0), pinned to a commit in `Cargo.toml`, with an integration test and a CC0 fixture.
- Per-crate `README.md`, `CHANGELOG.md`, `keywords`, and `categories` metadata.
