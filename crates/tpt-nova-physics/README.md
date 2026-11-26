# tpt-nova-physics

> 2D rigid-body physics for TPT Nova's ECS (components + deterministic sync step)

In-house 2D rigid-body physics exposed as plain ECS components, with a deterministic fixed-step sync that keeps solver state out of gameplay code.

Part of [TPT Nova](../../README.md), the AI-native, ECS-based real-time engine.

## Solver

- Shapes are circles and convex polygons. Capsules are approximated as stadium polygons.
- Polygon pairs use separating-axis tests with Sutherland–Hodgman clipping, giving up to two contact points.
- Contacts are solved in a fixed order with a fixed iteration count, so identical inputs give identical outputs.
- Penetration is corrected with a velocity bias (Baumgarte stabilisation).
- The solver replaces Rapier2D, so the workspace stays free of Apache-2.0-only dependencies.

## Workspace dependencies

- `tpt-nova-ecs`
- `tpt-nova-telemetry`

## Testing

```sh
cargo test -p tpt-nova-physics
```

## License

Dual-licensed under either of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE), at your option.
</content>
