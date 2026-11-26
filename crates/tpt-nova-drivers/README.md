# tpt-nova-drivers

Hot-swappable behavior drivers for TPT Nova (Phase 8, Layers 1-4, partial).

- `Driver`: a behavior bound to one component type, with an id, a semantic
  version, requested capabilities, and an `update` entry point.
- `DriverRegistry`: versioned storage keyed by `(component, version)`, with one
  active version per component. Reads are lock-free through `arc-swap`; writes
  are serialized and publish a new state atomically.
- `CapabilitySet` / `DriverContext::require`: the permission model. Layer 1
  defines and checks the set; sandbox enforcement is Layer 2.

- `shadow::shadow_compare` / `promote_if_clean`: run a candidate beside the
  baseline on a fork of the world and promote it only when the sampled entities
  agree (Layer 3).
- `sandbox::WasmDriver` (behind the `wasm-sandbox` feature): WASM drivers gated
  by granted capabilities and an instruction budget (Layer 2).

- `native::NativeDriver` (behind `native-loader`): trusted `.so`/`.dll` drivers
  through `libloading`, with an ABI check and `export_native_driver!` (Layer 2).
  Native code is not sandboxed. See the authoring guide for the limits.
- `monitor` (invariants over world state), `anomaly` (per-entity EWMA drift
  detection), `verify` (an internal exact Fourier-Motzkin solver over rationals),
  `compile` (a declarative `FixSpec` that yields both the driver and its model,
  plus the `GainSearch` patch proposer), and `repair` (verify → shadow →
  preserve → deploy → learn). Layer 4 is internal: no external solver or AI
  service is used.

Authoring guide: [docs/DRIVER_AUTHORING.md](../../docs/DRIVER_AUTHORING.md).

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
