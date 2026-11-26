# tpt-nova-drivers

> Hot-swappable behaviour drivers for TPT Nova, with a verified self-repair loop (Phase 8, partial)

Hot-swappable behavior drivers for TPT Nova: a driver is a behavior bound to one
component type, and the registry swaps its versions at runtime.

Part of [TPT Nova](../../README.md), the AI-native, ECS-based real-time engine.

## Core

- `Driver`: a behavior bound to one component type, with an id, a semantic
  version, requested capabilities, and an `update` entry point.
- `DriverRegistry`: versioned storage keyed by `(component, version)`, with one
  active version per component. Reads are lock-free through `arc-swap`; writes
  are serialized and publish a new state atomically.
- `DriverRegistry::strict`: refuses `activate()`. A driver is admitted only
  through `activate_verified`, which takes a sealed `Verified` proof. The
  default registry stays permissive for trusted host bootstrap.
- `CapabilitySet` / `DriverContext::require`: the permission model.

## Layers

- **Layer 2, sandboxing:**
  - `sandbox::WasmDriver` (behind the `wasm-sandbox` feature): WASM drivers gated
    by granted capabilities and an instruction budget.
  - `native::NativeDriver` (behind `native-loader`): trusted `.so`/`.dll` drivers
    through `libloading`, with an ABI check and `export_native_driver!`. Native
    code is not sandboxed. See the authoring guide for the limits.
- **Layer 3, shadow comparison:** `shadow::shadow_compare` / `promote_if_clean`
  run a candidate beside the baseline on a fork of the world and promote it only
  when the sampled entities agree.
- **Layer 4, verified repair (internal, no external solver or AI service):**
  - `verify`: an exact Fourier–Motzkin solver over rationals.
  - `monitor`: invariants over world state.
  - `anomaly`: per-entity EWMA drift detection.
  - `compile`: a declarative `FixSpec` yields both the driver and its model, plus
    the `GainSearch` patch proposer.
  - `repair`: observe → propose → verify → shadow → preserve → deploy → learn.

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

## Examples

```sh
cargo run -p tpt-nova-drivers --example selfdebug_loop
cargo run -p tpt-nova-drivers --example domain_drivers
```

`selfdebug_loop` runs the end-to-end repair and reports its latency.
`domain_drivers` shows drag, thermal relaxation, and intensity normalisation,
compiled and verified. The domain repositories are spec-only, so these show the
intended shape against this crate's API.

## Features

| Feature | Enables |
|---------|---------|
| `native-loader` | Trusted native `.so`/`.dll` drivers via `libloading`. |
| `wasm-sandbox` | WASM drivers on TPT-Wasm (git dependencies, off by default). |

## Testing

```sh
cargo test -p tpt-nova-drivers
cargo test -p tpt-nova-drivers --features native-loader
```

## License

MIT only. Spec §6 requires this crate to be MIT-only.
