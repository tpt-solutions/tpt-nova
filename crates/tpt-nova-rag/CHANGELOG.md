# Changelog - tpt-nova-rag

All notable changes to this crate are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/). Workspace-wide history is in
[../../CHANGELOG.md](../../CHANGELOG.md).

## [Unreleased]

### Changed
- Renamed the package from `nova-rag` to `tpt-nova-rag` (workspace-wide `tpt-nova-` crate prefix).
- `real-embeddings` no longer uses `fastembed`. `RealEmbedder` now calls a local embeddings server over plain HTTP (`RealEmbedder::new(endpoint, model)`, OpenAI-compatible `/v1/embeddings`), with no dependencies added. This removes `fastembed`, `ort`, `ort-sys`, `tokenizers`, `hf-hub`, `ureq`, `ring` and `webpki-roots` from the feature's tree, which had been the source of the Apache-2.0-only and CDLA license failures. `RealEmbedder::new` no longer takes no arguments.

### Added
- Per-crate `README.md`, `CHANGELOG.md`, `keywords`, and `categories` metadata.
