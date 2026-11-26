# Changelog - tpt-nova-splat

All notable changes to this crate are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/). Workspace-wide history is in
[../../CHANGELOG.md](../../CHANGELOG.md).

## [Unreleased]

### Changed
- Renamed the package from `nova-splat` to `tpt-nova-splat` (workspace-wide `tpt-nova-` crate prefix).
- Removed the `parry3d` dependency. Convex hulls come from `tpt-nova-ingest`'s in-house `hull`.

### Added
- In-place editing (`edit`): AABB and screen-rect selection, `delete`, `crop`, and `transform` (exact for non-uniform scale; SH is not rotated).
- Spatial chunking and LOD (`lod`): Morton-ordered chunks, frustum culling, distance LOD, and seeded deterministic `decimate`.
- Spherical-harmonic colour (`sh`), degrees 0–3, capped at runtime by `SplatQuality`.
- Compressed `.compressed.ply` loading (`compress`): quantized 256-splat chunks.
- Multi-part collision proxies (`proxy`): floater filter, voxels, decomposition, and a per-component hull fallback. Parameters live in a serializable `ProxyParams`.
- `SplatSummary` telemetry (`telemetry`) on each splat entity.
- Depth-sorted, anisotropic wgpu renderer behind the `render` feature (`sort`, `render`).
- Per-crate `README.md`, `CHANGELOG.md`, `keywords`, and `categories` metadata.
