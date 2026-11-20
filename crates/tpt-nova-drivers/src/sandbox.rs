//! Layer 2 WebAssembly sandbox: `.wasm` drivers on the TPT-Wasm runtime.
//!
//! A WASM driver can reach the engine only through the host imports defined
//! here, under the `tpt_nova` module. Each import is bound to a [`Capability`]:
//!
//! | import          | capability        | effect                                   |
//! |-----------------|-------------------|------------------------------------------|
//! | `read_dt`       | `ReadTransform`   | `() -> f32`, the fixed timestep          |
//! | `emit_event`    | `EmitEvents`      | `(i32) -> ()`, recorded per update       |
//! | `write_transform`, `read_neighbors`, `write_physics` | (listed) | recognized, but **no host binding yet** |
//!
//! Loading is gated before anything runs ([`WasmDriver::load`]):
//! - every import must be a function from `tpt_nova` with a known name;
//! - its capability must be in the driver's granted set;
//! - it must have a host binding.
//!
//! Only the imports a module actually uses are bound, so it gets no more than
//! it names. Each update runs under an instruction budget
//! (`max_execution_steps`), so a non-terminating driver fails instead of
//! stalling the frame.
//!
//! The runtime grants no ambient authority: no filesystem, network, clock, or
//! random source is reachable from a driver.

use std::sync::{Arc, Mutex};

use tpt_nova_ecs::{ComponentTypeId, World};
use tpt_wasm::{Config, Engine, EngineMode, Instance, Value, ValueType};
use tpt_wasm_format::ImportDesc;
use tpt_wasm_runtime::HostFunction;
use tpt_wasm_types::{FunctionType, ResultType};

use crate::driver::{
    Capability, CapabilitySet, Driver, DriverContext, DriverError, DriverId, DriverVersion,
};

/// The import module every host function lives under.
pub const HOST_MODULE: &str = "tpt_nova";

/// Name of the export every WASM driver must provide: `() -> ()`.
pub const UPDATE_EXPORT: &str = "update";

/// Default instruction budget per update.
pub const DEFAULT_MAX_STEPS: u64 = 1_000_000;

/// Every host import name, its gating capability, and whether it has a host
/// binding. Names without a binding are refused at load time.
const HOST_IMPORTS: &[(&str, Capability, bool)] = &[
    ("read_dt", Capability::ReadTransform, true),
    ("emit_event", Capability::EmitEvents, true),
    ("write_transform", Capability::WriteTransform, false),
    ("read_neighbors", Capability::ReadNeighbors, false),
    ("write_physics", Capability::WritePhysics, false),
];

/// Why a WASM driver was refused or failed.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum WasmDriverError {
    #[error("module does not decode: {0}")]
    Decode(String),
    #[error("import {module}.{name} is not a host function")]
    UnknownImport { module: String, name: String },
    #[error("import {0} is not a function; only function imports are allowed")]
    NonFunctionImport(String),
    #[error("import {name} requires the {capability:?} capability, which was not granted")]
    CapabilityNotGranted {
        name: String,
        capability: Capability,
    },
    #[error("import {0} is recognized but has no host binding yet")]
    NoHostBinding(String),
    #[error("module must export a function named `{UPDATE_EXPORT}`")]
    MissingUpdateExport,
    #[error("engine setup failed: {0}")]
    Engine(String),
    #[error("instantiation failed: {0}")]
    Instantiate(String),
}

/// What a WASM driver is allowed to do, and how much it may run.
#[derive(Debug, Clone)]
pub struct WasmDriverSpec {
    pub id: DriverId,
    pub version: DriverVersion,
    pub component: ComponentTypeId,
    /// Capabilities granted to this driver. Imports outside this set are refused.
    pub granted: CapabilitySet,
    /// Instruction budget per update.
    pub max_steps: u64,
}

/// Per-driver host state shared with the bound host functions.
#[derive(Default)]
struct HostState {
    dt: f32,
    events: Vec<i32>,
}

#[derive(Clone, Copy)]
enum HostKind {
    ReadDt,
    EmitEvent,
}

struct HostBinding {
    kind: HostKind,
    state: Arc<Mutex<HostState>>,
}

impl HostFunction for HostBinding {
    fn function_type(&self) -> FunctionType {
        match self.kind {
            HostKind::ReadDt => FunctionType {
                params: ResultType(Vec::new()),
                results: ResultType(vec![ValueType::F32]),
            },
            HostKind::EmitEvent => FunctionType {
                params: ResultType(vec![ValueType::I32]),
                results: ResultType(Vec::new()),
            },
        }
    }

    fn call(&self, args: &[Value]) -> Result<Vec<Value>, String> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        match (self.kind, args) {
            (HostKind::ReadDt, []) => Ok(vec![Value::F32(state.dt.to_bits())]),
            (HostKind::EmitEvent, [Value::I32(code)]) => {
                state.events.push(*code);
                Ok(Vec::new())
            }
            _ => Err("host call arguments do not match the import type".to_string()),
        }
    }
}

/// A `.wasm` driver running under the sandbox.
pub struct WasmDriver {
    spec: WasmDriverSpec,
    instance: Mutex<Instance>,
    state: Arc<Mutex<HostState>>,
}

impl WasmDriver {
    /// Validate `wasm` against `spec` and instantiate it. Nothing runs here.
    pub fn load(spec: WasmDriverSpec, wasm: &[u8]) -> Result<Self, WasmDriverError> {
        let module = tpt_wasm::decode(wasm).map_err(|e| WasmDriverError::Decode(e.to_string()))?;

        let state = Arc::new(Mutex::new(HostState::default()));
        let mut bindings: Vec<(&'static str, HostKind)> = Vec::new();

        for import in &module.imports {
            let qualified = format!("{}.{}", import.module, import.name);
            if import.module != HOST_MODULE {
                return Err(WasmDriverError::UnknownImport {
                    module: import.module.clone(),
                    name: import.name.clone(),
                });
            }
            let &(name, capability, bound) = HOST_IMPORTS
                .iter()
                .find(|(n, _, _)| *n == import.name)
                .ok_or_else(|| WasmDriverError::UnknownImport {
                    module: import.module.clone(),
                    name: import.name.clone(),
                })?;
            if !matches!(import.desc, ImportDesc::Function(_)) {
                return Err(WasmDriverError::NonFunctionImport(qualified));
            }
            if !spec.granted.contains(capability) {
                return Err(WasmDriverError::CapabilityNotGranted {
                    name: qualified,
                    capability,
                });
            }
            if !bound {
                return Err(WasmDriverError::NoHostBinding(qualified));
            }
            let kind = match name {
                "read_dt" => HostKind::ReadDt,
                _ => HostKind::EmitEvent,
            };
            bindings.push((name, kind));
        }

        if !module.exports.iter().any(|e| e.name == UPDATE_EXPORT) {
            return Err(WasmDriverError::MissingUpdateExport);
        }

        let mut config = Config {
            engine_mode: EngineMode::Micro,
            ..Config::default()
        };
        config.limits.max_execution_steps = Some(spec.max_steps);
        let mut engine = Engine::new(config).map_err(|e| WasmDriverError::Engine(e.to_string()))?;
        for (name, kind) in bindings {
            engine.linker_mut().define_function(
                HOST_MODULE,
                name,
                HostBinding {
                    kind,
                    state: Arc::clone(&state),
                },
            );
        }
        let instance = engine
            .instantiate_bytes(wasm)
            .map_err(|e| WasmDriverError::Instantiate(e.to_string()))?;

        Ok(WasmDriver {
            spec,
            instance: Mutex::new(instance),
            state,
        })
    }

    /// Events the driver emitted during its most recent update.
    pub fn last_events(&self) -> Vec<i32> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .events
            .clone()
    }
}

impl Driver for WasmDriver {
    fn id(&self) -> DriverId {
        self.spec.id.clone()
    }

    fn version(&self) -> DriverVersion {
        self.spec.version
    }

    fn component(&self) -> ComponentTypeId {
        self.spec.component
    }

    fn requested_capabilities(&self) -> CapabilitySet {
        self.spec.granted.clone()
    }

    fn update(&self, ctx: &DriverContext, _world: &mut World) -> Result<(), DriverError> {
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.dt = ctx.dt;
            state.events.clear();
        }
        let mut instance = self.instance.lock().unwrap_or_else(|e| e.into_inner());
        instance
            .call(UPDATE_EXPORT, Vec::new())
            .map(|_| ())
            .map_err(|e| DriverError::Failed(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::DriverRegistry;
    use std::sync::Arc;

    #[derive(Debug, Default)]
    struct Probe;
    impl tpt_nova_ecs::Component for Probe {}

    fn leb(mut n: u32) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let byte = (n & 0x7f) as u8;
            n >>= 7;
            if n == 0 {
                out.push(byte);
                return out;
            }
            out.push(byte | 0x80);
        }
    }

    fn name(s: &str) -> Vec<u8> {
        let mut out = leb(s.len() as u32);
        out.extend(s.as_bytes());
        out
    }

    fn section(id: u8, content: Vec<u8>) -> Vec<u8> {
        let mut out = vec![id];
        out.extend(leb(content.len() as u32));
        out.extend(content);
        out
    }

    /// Hand-assembled module. Types: 0 = `() -> f32`, 1 = `(i32) -> ()`,
    /// 2 = `() -> ()`. `imports` are `(name, type index)` under `tpt_nova`, in
    /// order, so they take function indices from 0. `update` is type 2 and runs
    /// `body`, which must end with `end` (0x0b).
    fn module(imports: &[(&str, u8)], body: &[u8]) -> Vec<u8> {
        let mut wasm = b"\0asm\x01\x00\x00\x00".to_vec();
        wasm.extend(section(
            1,
            [
                vec![3],
                vec![0x60, 0, 1, 0x7d],
                vec![0x60, 1, 0x7f, 0],
                vec![0x60, 0, 0],
            ]
            .concat(),
        ));
        let mut imp = leb(imports.len() as u32);
        for (n, ty) in imports {
            imp.extend(name(HOST_MODULE));
            imp.extend(name(n));
            imp.extend([0x00, *ty]);
        }
        wasm.extend(section(2, imp));
        wasm.extend(section(3, vec![1, 2]));
        let mut exp = vec![1];
        exp.extend(name(UPDATE_EXPORT));
        exp.extend([0x00, imports.len() as u8]);
        wasm.extend(section(7, exp));
        let mut code_body = vec![0x00];
        code_body.extend(body);
        let mut code = vec![1];
        code.extend(leb(code_body.len() as u32));
        code.extend(code_body);
        wasm.extend(section(10, code));
        wasm
    }

    fn spec(granted: CapabilitySet) -> WasmDriverSpec {
        WasmDriverSpec {
            id: DriverId::new("probe-driver").unwrap(),
            version: DriverVersion::new(1, 0, 0),
            component: ComponentTypeId::of::<Probe>(),
            granted,
            max_steps: DEFAULT_MAX_STEPS,
        }
    }

    fn ctx(dt: f32) -> DriverContext {
        DriverContext::new(0, dt, CapabilitySet::empty())
    }

    fn read_and_emit() -> CapabilitySet {
        [Capability::ReadTransform, Capability::EmitEvents]
            .into_iter()
            .collect()
    }

    // With `read_dt` = 0 and `emit_event` = 1: emit_event(7); read_dt(); drop.
    const EMIT_SEVEN_THEN_READ: &[u8] = &[0x41, 0x07, 0x10, 0x01, 0x10, 0x00, 0x1a, 0x0b];

    #[test]
    fn granted_driver_runs_and_its_host_calls_reach_the_host() {
        let wasm = module(&[("read_dt", 0), ("emit_event", 1)], EMIT_SEVEN_THEN_READ);
        let driver = WasmDriver::load(spec(read_and_emit()), &wasm).expect("loads");
        let mut world = World::new();
        driver.update(&ctx(1.0 / 60.0), &mut world).expect("runs");
        assert_eq!(driver.last_events(), vec![7]);
    }

    #[test]
    fn read_dt_returns_the_context_timestep() {
        // Body: read_dt; i32.trunc_f32_s; emit_event; end.
        let body = [0x10, 0x00, 0xa8, 0x10, 0x01, 0x0b];
        let wasm = module(&[("read_dt", 0), ("emit_event", 1)], &body);
        let driver = WasmDriver::load(spec(read_and_emit()), &wasm).expect("loads");
        let mut world = World::new();
        driver.update(&ctx(42.9), &mut world).expect("runs");
        assert_eq!(driver.last_events(), vec![42]);
    }

    #[test]
    fn ungranted_capability_is_refused_before_anything_runs() {
        // The module imports emit_event, but only ReadTransform is granted.
        let wasm = module(&[("emit_event", 1)], &[0x0b]);
        let granted = CapabilitySet::empty().with(Capability::ReadTransform);
        let err = WasmDriver::load(spec(granted), &wasm)
            .err()
            .expect("refused");
        assert_eq!(
            err,
            WasmDriverError::CapabilityNotGranted {
                name: "tpt_nova.emit_event".into(),
                capability: Capability::EmitEvents,
            }
        );
    }

    #[test]
    fn unknown_host_functions_are_refused() {
        let wasm = module(&[("launch_missiles", 1)], &[0x0b]);
        assert!(matches!(
            WasmDriver::load(spec(read_and_emit()), &wasm),
            Err(WasmDriverError::UnknownImport { name, .. }) if name == "launch_missiles"
        ));
    }

    #[test]
    fn recognized_but_unbound_capabilities_are_refused_even_when_granted() {
        // write_physics is a known capability with no host binding yet.
        let wasm = module(&[("write_physics", 1)], &[0x0b]);
        let granted: CapabilitySet = [Capability::WritePhysics].into_iter().collect();
        assert_eq!(
            WasmDriver::load(spec(granted), &wasm).err(),
            Some(WasmDriverError::NoHostBinding(
                "tpt_nova.write_physics".into()
            ))
        );
    }

    #[test]
    fn imports_outside_the_host_module_are_refused() {
        let mut wasm = module(&[("emit_event", 1)], &[0x0b]);
        // Rename the module "tpt_nova" to "envvvvvv" (same length, so offsets hold).
        let pos = wasm
            .windows(8)
            .position(|w| w == b"tpt_nova")
            .expect("module name present");
        wasm[pos..pos + 8].copy_from_slice(b"envvvvvv");
        assert!(matches!(
            WasmDriver::load(spec(read_and_emit()), &wasm),
            Err(WasmDriverError::UnknownImport { module, .. }) if module == "envvvvvv"
        ));
    }

    #[test]
    fn modules_without_an_update_export_are_refused() {
        let mut wasm = module(&[], &[0x0b]);
        let pos = wasm
            .windows(6)
            .position(|w| w == b"update")
            .expect("export name present");
        wasm[pos..pos + 6].copy_from_slice(b"tick!!");
        assert_eq!(
            WasmDriver::load(spec(CapabilitySet::empty()), &wasm).err(),
            Some(WasmDriverError::MissingUpdateExport)
        );
    }

    #[test]
    fn runaway_drivers_hit_the_instruction_budget() {
        // loop; br 0; end. Never terminates on its own.
        let body = [0x03, 0x40, 0x0c, 0x00, 0x0b, 0x0b];
        let wasm = module(&[], &body);
        let mut s = spec(CapabilitySet::empty());
        s.max_steps = 10_000;
        let driver = WasmDriver::load(s, &wasm).expect("loads");
        let mut world = World::new();
        assert!(matches!(
            driver.update(&ctx(0.016), &mut world),
            Err(DriverError::Failed(_))
        ));
    }

    #[test]
    fn a_sandboxed_driver_activates_through_the_registry_under_policy() {
        let wasm = module(&[("read_dt", 0), ("emit_event", 1)], EMIT_SEVEN_THEN_READ);
        let driver = Arc::new(WasmDriver::load(spec(read_and_emit()), &wasm).expect("loads"));
        let registry = DriverRegistry::with_policy(read_and_emit());
        let component = ComponentTypeId::of::<Probe>();
        registry.register(driver).expect("registers");
        registry
            .activate(component, DriverVersion::new(1, 0, 0))
            .expect("activates under the policy");
        assert!(registry.active(component).is_some());
    }
}
