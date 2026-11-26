# tpt-nova-splat

> Gaussian Splat (3DGS) loading, editing, collision proxies and rendering for TPT Nova

Loads Gaussian Splat captures in three layouts, edits them in place, derives multi-part collision proxies, and (behind a feature) draws them with an anisotropic, depth-sorted wgpu pipeline.

Part of [TPT Nova](../../README.md), the AI-native, ECS-based real-time engine.

## What it does

| Area | Module | Notes |
|------|--------|-------|
| Loading | `loader` | `.splat` (32-byte records), `.ply` (ASCII and binary little-endian), including higher-order SH from `f_rest_*`. |
| Compressed layout | `compress` | Quantized, 256-splat chunks (PlayCanvas-style). A `.ply` with `packed_position` is detected automatically. Round-trip tested only; not yet checked against a third-party export. |
| Color | `sh` | Real SH evaluation (degrees 0–3) matching the reference 3DGS basis. `SplatQuality { sh_degree }` caps it at runtime. |
| Editing | `edit` | `select_in_aabb`, `select_in_screen_rect` (via `tpt-nova-overlay`), `delete`, `crop`, `transform` (exact for non-uniform scale; SH is not rotated). |
| Collision | `collider`, `proxy` | `build_convex_hull_collider` (single hull) or `build_proxy_collider` (floater filter → voxels → convex decomposition, with per-component hull fallback). Parameters live in a serializable `ProxyParams`. |
| Scale | `lod` | Morton-ordered spatial chunks, frustum culling, distance LOD, and seeded deterministic `decimate`. |
| Telemetry | `telemetry`, `edit::SplatSummary` | Adds a `SplatSummary` (count, bounds, SH degree, opacity histogram) to each splat entity's telemetry dump. |
| Rendering | `render` (feature) | Oriented EWA-style ellipses, storage buffers per chunk, CPU back-to-front sort, premultiplied blending with depth writes off. |

## Rendering notes

- `SplatPipeline::render` takes a `SplatCamera { view, view_proj, viewport_px }`. Call it after the opaque passes.
- Each chunk is re-sorted only when the view-projection changes. Ordering across chunks is by chunk center, so chunks that overlap in depth can blend in the wrong order. A GPU radix sort is not implemented yet.

## Features

| Feature | Enables |
|---------|---------|
| `render` | The wgpu-backed splat render pipeline (`SplatPipeline`). Its headless smoke test runs on any machine with a wgpu adapter (including software fallback) and skips otherwise. |

## Workspace dependencies

- `tpt-nova-ecs`
- `tpt-nova-ingest` (convex decomposition, hulls, `Collider3D`)
- `tpt-nova-overlay` (screen-space selection)
- `tpt-nova-telemetry` (summary attachment)

## Testing

```sh
cargo test -p tpt-nova-splat
cargo test -p tpt-nova-splat --features render
```

## License

Dual-licensed under either of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE), at your option.
