# tpt-nova-drivers

Hot-swappable behavior drivers for TPT Nova (Phase 8, Layer 1).

- `Driver`: a behavior bound to one component type, with an id, a semantic
  version, requested capabilities, and an `update` entry point.
- `DriverRegistry`: versioned storage keyed by `(component, version)`, with one
  active version per component. Reads are lock-free through `arc-swap`; writes
  are serialized and publish a new state atomically.
- `CapabilitySet` / `DriverContext::require`: the permission model. Layer 1
  defines and checks the set; sandbox enforcement is Layer 2.

Not yet implemented: the native `.so` loader, the WASM sandbox, migrations,
rollback, shadow execution, and the AI self-debugging loop (Phase 8 Layers 2-4).

```rust
let registry = DriverRegistry::new();
registry.register(Arc::new(MyGravityV1))?;
registry.register(Arc::new(MyGravityV2))?;
registry.activate(ComponentTypeId::of::<Gravity>(), DriverVersion::new(1, 0, 0))?;

// Each frame: take one snapshot and run the active drivers from it.
let snap = registry.snapshot();
for driver in snap.active_drivers() {
    driver.update(&ctx, &mut world)?;
}
```

License: MIT (spec §6 requires MIT-only for this crate).
