# Driver Authoring Guide

Audience: engine contributors and gameplay authors writing a `tpt-nova-drivers`
behavior, either as Rust or as a WebAssembly module.

A **driver** is a hot-swappable behavior bound to one component type. It is
versioned, it can be activated or rolled back at runtime, and it can be
shadow-tested against the active version before promotion. The engine never
hands a driver ambient access to the world: a driver gets only what its granted
capabilities allow.

Status against the roadmap (`todo.md`, Phase 8): the Rust `Driver` trait,
registry, migrations, rollback, and shadow execution are implemented. The WASM
sandbox (`wasm-sandbox`) and native `.so`/`.dll` loading (`native-loader`) are
feature-gated. The Layer 4 repair loop (`repair`) verifies, shadow-tests, and
deploys a fix. Its patch proposer is a hook, not a model. Section 8 explains the
native loader's limits. Section 10 describes the repair loop.

## 1. Core concepts

| Concept | Where | What it is |
|---|---|---|
| `Driver` | `driver.rs` | The trait you implement: `id`, `version`, `component`, `requested_capabilities`, `update`, optional `migrate_from`. |
| `DriverId` | `driver.rs` | A stable name, such as `"gravity-tweak"`. Versions of one behavior share an id. Empty ids are rejected. |
| `DriverVersion` | `driver.rs` | `major.minor.patch`. The newest version is the maximum. |
| `CapabilitySet` | `driver.rs` | The permissions a driver requests or is granted. |
| `DriverRegistry` | `registry.rs` | Stores drivers by `(component, version)`, keeps one active version per component, and holds rollback history. Reads are lock-free. |
| `shadow_compare` / `promote_if_clean` | `shadow.rs` | Runs a candidate beside the baseline on a fork of the world and promotes only when the sample agrees. |
| `migrate` | `migrate.rs` | Upgrades serialized component data one version at a time. |

Rule of thumb: keep a driver stateless. Anything it must remember belongs in a
component, so that swapping the driver does not lose state.

## 2. Writing a Rust driver

```rust
use std::sync::Arc;
use tpt_nova_drivers::{
    Capability, CapabilitySet, Driver, DriverContext, DriverError, DriverId,
    DriverRegistry, DriverVersion,
};
use tpt_nova_ecs::{Component, ComponentTypeId, World};

#[derive(Debug, Default)]
struct Speed { value: f32 }
impl Component for Speed {}

struct SpeedDamping { version: DriverVersion, factor: f32 }

impl Driver for SpeedDamping {
    fn id(&self) -> DriverId { DriverId::new("speed-damping").unwrap() }
    fn version(&self) -> DriverVersion { self.version }
    fn component(&self) -> ComponentTypeId { ComponentTypeId::of::<Speed>() }

    fn requested_capabilities(&self) -> CapabilitySet {
        CapabilitySet::empty().with(Capability::ReadTransform)
    }

    fn update(&self, ctx: &DriverContext, world: &mut World) -> Result<(), DriverError> {
        ctx.require(Capability::ReadTransform)?;
        let entities = world.entities();
        for e in entities {
            if let Some(s) = world.get_component_mut::<Speed>(e) {
                s.value *= self.factor.powf(ctx.dt);
            }
        }
        Ok(())
    }
}
```

Guidelines:

- **Request only what you use.** `requested_capabilities` is checked at
  activation. A driver that requests a capability the registry policy does not
  grant cannot be activated.
- **Call `ctx.require(cap)?` before a capability-gated operation** if you want
  a clear error instead of silent behavior.
- **Be deterministic.** Use `ctx.tick` and `ctx.dt`, not wall-clock time or an
  unseeded RNG. The engine's fixed-timestep, seeded-RNG guarantee depends on it.
- **Return `DriverError::Failed` for bad state** rather than panicking. An error
  from one driver is reported; it does not take down the process.

## 3. Registering, activating, and rolling back

```rust
let registry = DriverRegistry::with_policy(
    CapabilitySet::empty().with(Capability::ReadTransform),
);
let c = ComponentTypeId::of::<Speed>();
registry.register(Arc::new(SpeedDamping { version: DriverVersion::new(1, 0, 0), factor: 0.99 }))?;
registry.activate(c, DriverVersion::new(1, 0, 0))?;

// Each frame: take one snapshot and run the active drivers from it.
let snap = registry.snapshot();
for driver in snap.active_drivers() {
    driver.update(&ctx, &mut world)?;
}

// Revert to the version that was active before the current one.
registry.rollback(c)?;
```

Activation takes effect on the next `snapshot()`. A frame already in progress
keeps the driver it started with, so a swap never splits a frame.

## 4. Shadow-testing a candidate before promotion

Shadow execution is the gate between "compiles" and "active in the live world".
It runs the baseline and the candidate for one tick, each on a fork of the
world, and compares the state you choose to observe for a sample of entities.
The live world is not touched.

```rust
use tpt_nova_drivers::{promote_if_clean, shadow_compare};

let report = shadow_compare(
    baseline.as_ref(),
    candidate.as_ref(),
    &world,
    &ctx,
    &sample,                 // any subset of live entities
    |w| build_fork(w),       // World is not Clone: fork via scene round-trip
    |w, e| serde_json::json!(w.get_component::<Speed>(e).map(|s| s.value)),
)?;

if report.is_clean() {
    promote_if_clean(&registry, c, &report)?;
} else {
    eprintln!("candidate diverged on {} entities", report.divergent.len());
}
```

Things to know:

- `World` is not `Clone`. The fork closure is your responsibility. A scene
  round-trip (`tpt_nova_scene::dump_world` then `load_world`) is the usual choice.
- Comparison covers only what `observe` returns. Include every field the
  candidate is supposed to change, or divergence in that field goes unnoticed.
- `promote_if_clean` refuses a divergent report, so a bad candidate cannot reach
  the active slot through that path.
- The report is one tick. It catches immediate differences, not drift over
  many frames.

## 5. Migrating component data

When a version changes the schema of its component's serialized data, implement
`migrate_from` on the new version. The migration composer refuses any path with
a missing hop, so schema changes always have a registered migration.

```rust
fn migrate_from(
    &self,
    from: DriverVersion,
    data: &serde_json::Value,
) -> Option<Result<serde_json::Value, DriverError>> {
    if from == DriverVersion::new(1, 0, 0) {
        let mut out = data.clone();
        out["damping"] = out.get("factor").cloned().unwrap_or(serde_json::json!(0.99));
        Some(Ok(out))
    } else {
        None // no registered path from this version
    }
}
```

Use `tpt_nova_drivers::migrate(...)` to walk a chain such as `1.0.0 -> 2.0.0 -> 3.0.0`.

## 6. Writing a WASM driver

WASM drivers are for untrusted or AI-generated behavior. They run under the
`wasm-sandbox` feature on `tpt-wasm`.

```toml
[dependencies]
tpt-nova-drivers = { path = "...", features = ["wasm-sandbox"] }
```

A WASM module:

- must export `update` as `() -> ()`;
- may import only from the `tpt_nova` module, and only these functions:

| Import | Signature | Capability | Status |
|---|---|---|---|
| `read_dt` | `() -> f32` | `ReadTransform` | bound |
| `emit_event` | `(i32) -> ()` | `EmitEvents` | bound; events recorded per update |
| `write_transform` | listed, not bound | `WriteTransform` | refused at load time |
| `read_neighbors` | listed, not bound | `ReadNeighbors` | refused at load time |
| `write_physics` | listed, not bound | `WritePhysics` | refused at load time |

Loading (`WasmDriver::load`) is gated before anything runs. The load fails if:

- an import is outside `tpt_nova` or has an unknown name;
- an import's capability is not in the spec's granted set;
- an import has no host binding yet.

Each update runs under an instruction budget (`WasmDriverSpec::max_steps`,
default 1,000,000). A non-terminating module fails the update instead of
stalling the frame. The module has no filesystem, network, clock, or random
access.

Load one like this:

```rust
use tpt_nova_drivers::sandbox::{WasmDriver, WasmDriverSpec};

let spec = WasmDriverSpec {
    id: DriverId::new("wasm-probe")?,
    version: DriverVersion::new(1, 0, 0),
    component: ComponentTypeId::of::<Speed>(),
    granted: CapabilitySet::empty().with(Capability::ReadTransform),
    max_steps: 1_000_000,
};
let driver = WasmDriver::load(spec, &wasm_bytes)?;
registry.register(Arc::new(driver))?;
```

A WASM driver goes through the same registry, activation policy, shadow test,
and rollback path as a Rust driver.

## 7. Checklist before you ship a driver

- [ ] `id` is stable across versions; `version` is bumped for every behavior change.
- [ ] `requested_capabilities` lists exactly what `update` uses.
- [ ] State lives in components, not in the driver struct.
- [ ] Output is deterministic for a given `tick`, `dt`, and seed.
- [ ] Any schema change has a `migrate_from` path from every version still in use.
- [ ] A shadow test with an `observe` closure covers every field the driver changes.
- [ ] `cargo test -p tpt-nova-drivers` passes, and so does `--features wasm-sandbox`
      if you use WASM.

## 8. Native drivers (trusted, feature `native-loader`)

A native driver is a `cdylib` that exports its `Driver` through three C-ABI
symbols. `export_native_driver!` generates them:

```rust
// in a cdylib crate that depends on tpt-nova-drivers with features = ["native-loader"]
tpt_nova_drivers::export_native_driver!(MyGravityDriver);
```

The host loads it with `NativeDriver::load(path)` and registers the result like
any other driver. The loader copies the file to a temp path first, so the
original can be rebuilt while loaded.

Limits you must plan around:

- **Trusted code.** A native driver runs in-process with full access. Only the
  activation-time capability policy applies. Use the WASM sandbox for anything
  untrusted.
- **Same toolchain.** The `dyn Driver` vtable is Rust ABI. Build the module with
  the same `rustc` and `tpt-nova-drivers` version as the host.
- **Component identity does not cross the dylib boundary.** `ComponentTypeId`
  wraps `TypeId`, which differs between a host and a module. A module cannot
  look up a host component by type. The host must key the registry on
  `driver.component()`, and the module's own components are invisible to the
  host. A stable, name-based component id would fix this, and it is not built.
- **Entity creation is visible to the host.** A module can spawn entities in the
  host world, and the host sees them through the normal indices.

## 9. Not yet available

- Symbolic verification of a driver's *code* (Layer 4 verifies a declared
  one-tick model, see section 10).
- AI-generated patches. The repair loop's `Proposer` is a hook. Nothing here
  calls a model.
- Shipping drivers in `.novapack` bundles (`tpt-nova-export`).

## 10. Self-debugging repair (Layer 4)

When a monitored invariant is violated, the repair loop can propose, verify,
and deploy a fix. Run the example to see it:

```sh
cargo run -p tpt-nova-drivers --example selfdebug_loop
```

The gates are:

1. **Verify.** The candidate declares an affine one-tick model,
   `next = (num*cur + offset)/den`. It must map the invariant's bounds into
   themselves. The check is an internal exact Fourier-Motzkin solver over
   rationals, so it reasons over the reals, matching `f32` driver state. It
   fails closed on overflow.
2. **Shadow.** The candidate runs on a fork of the checkpoint world. The
   invariant must hold for the sampled entities afterwards. This is an empirical
   check that the code matches the declared model.
3. **Preserve.** A projection of the state the fix must not touch must match
   the active driver's output.
4. **Deploy and learn.** Only then is the driver registered and activated, and
   the fix is remembered for that invariant.

Because `Verified` has a private field, the repair path cannot activate a
candidate without passing step 1. Direct `registry.activate` calls bypass the
loop, so they are your responsibility.

Measured on this machine, a repair on the example's two-entity world takes tens
of microseconds, well under the 16.7 ms frame budget at 60 Hz. The `world`
size used for the measurement is tiny, so scale the number for a real scene.
