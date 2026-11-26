# Changelog - tpt-nova-drivers

All notable changes to this crate are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/). Workspace-wide history is in
[../../CHANGELOG.md](../../CHANGELOG.md).

## [Unreleased]

### Added
- Initial crate (Phase 8): a `Driver` trait bound to one component type, a `CapabilitySet` permission model, and an `arc-swap` versioned `DriverRegistry` with migrations.
- Layer 2: `native` drivers (trusted `.so` / `.dll` through `libloading`, behind `native-loader`, with an ABI check and `export_native_driver!`; not sandboxed) and `sandbox` WASM drivers on TPT-Wasm (behind `wasm-sandbox`). The sandbox has a load-time import gate, per-import capability binding, and a per-update step budget. It has a seeded mutation fuzz test and an ungranted-import escape test.
- Layer 3: `shadow_compare` runs baseline and candidate on forks and compares an observed projection per sampled entity. `promote_if_clean` gates on the result.
- Layer 4 (internal, no external solver or AI service):
  - `verify`: exact Fourier–Motzkin elimination over rationals, with checked arithmetic that fails closed on overflow. `verify_affine` returns a `Verified` token with a private field.
  - `monitor`: invariant monitors over world state.
  - `anomaly`: per-entity EWMA drift detection with z-score flags.
  - `compile`: a `FixSpec` compiles to a driver and its affine model from one source, and `GainSearch` proposes contraction gains in a deterministic order.
  - `repair`: observe → propose → verify → shadow → preserve → deploy → learn, with `FixMemory`.
- `DriverRegistry::strict` refuses `activate()`. Drivers are admitted only through `activate_verified`, which takes a `Verified` proof, and `is_verified_active()` reports the state. The repair loop is the only caller that passes a proof. The default registry stays permissive for trusted host bootstrap.
- Examples: `selfdebug_loop` (the end-to-end repair, with latency) and `domain_drivers`.
