# tpt-nova-ingest

> Smart mesh ingestion for TPT Nova: .glb/.obj loading, convex decomposition, in-house convex hulls, procedural auto-rig, and 3D collider generation

Smart mesh ingestion: glTF/OBJ loading, approximate convex decomposition, procedural auto-rigging, and collider generation for an in-house 3D rigid-body solver.

Part of [TPT Nova](../../README.md), the AI-native, ECS-based real-time engine.

## Modules

| Module | Role |
|--------|------|
| `loader` | glTF and OBJ files → plain `MeshData` |
| `decompose` | `decompose_convex` (takes `DecompositionParams`): top-down approximate convex decomposition |
| `hull` | `convex_hull`, an in-house quickhull (replaces `parry3d`) |
| `rig` | `auto_rig`, procedural skeleton generation |
| `physics`, `solver3d` | `Collider3D`, `RigidBody3D`, `step_physics_3d`; a sequential-impulse solver for convex-hull compounds (replaces Rapier3D) |

## Notes

- Decomposition is not V-HACD. It splits the most concave part by the cut with the smallest total child hull volume, so part boundaries differ from V-HACD's.
- The 3D solver has no sleeping or continuous collision detection, so fast, small bodies can tunnel through thin geometry.

## Workspace dependencies

- `tpt-nova-ecs`
- `tpt-nova-anim`

## Testing

```sh
cargo test -p tpt-nova-ingest
```

## License

Dual-licensed under either of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE), at your option.
</content>
