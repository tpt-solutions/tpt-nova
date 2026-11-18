# tpt-nova-videocap

> Video-to-ECS ingestion for TPT Nova: depth map + segmentation mask -> collision proxies

Video-to-ECS ingestion: depth maps and segmentation masks become low-poly collision proxies for Rapier3D.

Part of [TPT Nova](../../README.md), the AI-native, ECS-based real-time engine.

## Workspace dependencies

- `tpt-nova-ecs`
- `tpt-nova-ingest`

## Testing

```sh
cargo test -p tpt-nova-videocap
```

## License

Dual-licensed under either of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE), at your option.
