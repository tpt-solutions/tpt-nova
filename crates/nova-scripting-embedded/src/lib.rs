//! Sandboxed, embedded scripting for TPT Nova.
//!
//! Alongside the Rust hot-reload modules ([`nova_scripting`](https://docs.rs)),
//! TPT Nova supports *embedded* scripts written in [Rhai](https://rhai.rs).
//! Embedded scripts are the surface we hand to AI agents and user "vibes":
//! they run in Rhai's safe interpreter (no filesystem, no `eval` by default)
//! and are further constrained by an explicit **capability boundary**.
//!
//! ## Capability boundary
//!
//! A script can only call the functions the host chose to register for it.
//! Those functions do not touch the [`World`] directly; instead they enqueue
//! typed [`ScriptCommand`]s. The host drains the queue after each run and
//! applies the commands with full `&mut World` access — so a script's power is
//! exactly the set of capabilities it was granted, nothing more. Deny a
//! capability and its function simply does not exist, so even attempting to
//! call it is a (caught) error rather than an escape.
//!
//! This is the recommended split (see the resolved Open Decision in
//! `todo.md`): **Rust hot-reload** for performance-critical, shipped gameplay
//! systems; **embedded Rhai** for AI-generated, sandboxed, hot-iterated logic.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use nova_ecs::{Entity, Quat, Transform, Vec3, World};
use rhai::Engine;
use serde::{Deserialize, Serialize};

/// A single permission a script may be granted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Capability {
    /// Read ECS state (telemetry, queries) via host-provided helpers.
    ReadWorld,
    /// Mutate existing entities (transforms, components).
    WriteWorld,
    /// Create new entities.
    Spawn,
    /// Emit log lines that the host captures.
    Log,
    /// Trigger network / external events.
    Net,
    /// Touch the local filesystem.
    Fs,
}

/// The set of capabilities granted to a script.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    read_world: bool,
    write_world: bool,
    spawn: bool,
    log: bool,
    net: bool,
    fs: bool,
}

impl Capabilities {
    /// No capabilities: the script can do nothing but error on unknown calls.
    pub fn none() -> Self {
        Capabilities::default()
    }

    /// Every capability: full (host-vetted) control surface.
    pub fn all() -> Self {
        Capabilities {
            read_world: true,
            write_world: true,
            spawn: true,
            log: true,
            net: true,
            fs: true,
        }
    }

    pub fn grant(&mut self, c: Capability) -> &mut Self {
        match c {
            Capability::ReadWorld => self.read_world = true,
            Capability::WriteWorld => self.write_world = true,
            Capability::Spawn => self.spawn = true,
            Capability::Log => self.log = true,
            Capability::Net => self.net = true,
            Capability::Fs => self.fs = true,
        }
        self
    }

    pub fn revoke(&mut self, c: Capability) -> &mut Self {
        match c {
            Capability::ReadWorld => self.read_world = false,
            Capability::WriteWorld => self.write_world = false,
            Capability::Spawn => self.spawn = false,
            Capability::Log => self.log = false,
            Capability::Net => self.net = false,
            Capability::Fs => self.fs = false,
        }
        self
    }

    pub fn can(self, c: Capability) -> bool {
        match c {
            Capability::ReadWorld => self.read_world,
            Capability::WriteWorld => self.write_world,
            Capability::Spawn => self.spawn,
            Capability::Log => self.log,
            Capability::Net => self.net,
            Capability::Fs => self.fs,
        }
    }
}

impl FromIterator<Capability> for Capabilities {
    fn from_iter<I: IntoIterator<Item = Capability>>(iter: I) -> Self {
        let mut caps = Capabilities::none();
        for c in iter {
            caps.grant(c);
        }
        caps
    }
}

/// A command a script produced, applied to the world by the host.
#[derive(Debug, Clone, PartialEq)]
pub enum ScriptCommand {
    /// Create a new entity (a stable local `id` is handed back to the script).
    Spawn { id: u32 },
    /// Set an entity's local transform.
    SetTransform {
        id: u32,
        pos: (f32, f32, f32),
        rot_euler: (f32, f32, f32),
        scale: (f32, f32, f32),
    },
    /// Emit a log line (captured by the host).
    Log(String),
    /// Trigger a named game event (e.g. to be routed to telemetry/network).
    EmitEvent(String),
}

/// Errors from compiling or running an embedded script.
#[derive(Debug, Clone)]
pub enum ScriptError {
    /// The script failed to compile or referenced an unregistered function.
    Compile(String),
    /// The script compiled but failed at runtime.
    Runtime(String),
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScriptError::Compile(s) => write!(f, "script compile error: {s}"),
            ScriptError::Runtime(s) => write!(f, "script runtime error: {s}"),
        }
    }
}

impl std::error::Error for ScriptError {}

impl From<Box<rhai::EvalAltResult>> for ScriptError {
    fn from(e: Box<rhai::EvalAltResult>) -> Self {
        // Parse errors surface through the same path in Rhai; distinguish by
        // inspecting the message is fragile, so we treat both as runtime
        // failures here. Hosts only need to know "it didn't run".
        ScriptError::Runtime(e.to_string())
    }
}

/// A sandboxed Rhai runtime with a capability-bounded API.
///
/// Construct with the capabilities you want to grant; only those functions are
/// registered. Keep it as a local in your app, or (because it is `Send + Sync`)
/// as a `World` resource — but apply commands with [`EmbeddedRuntime::apply`]
/// using a `World` *other* than the one holding the runtime to avoid a borrow
/// conflict.
pub struct EmbeddedRuntime {
    engine: Engine,
    caps: Capabilities,
    queue: Arc<Mutex<Vec<ScriptCommand>>>,
    ids: Arc<Mutex<HashMap<u32, Option<Entity>>>>,
    logs: Arc<Mutex<Vec<String>>>,
}

impl EmbeddedRuntime {
    /// Build a runtime that only exposes functions for the granted capabilities.
    pub fn new(caps: Capabilities) -> Self {
        let mut engine = Engine::new();
        // No `eval`/`call_fn` surface by default in Rhai; we additionally keep
        // the engine free of filesystem/network modules so scripts cannot reach
        // the OS except through the host-registered commands below.

        let queue = Arc::new(Mutex::new(Vec::new()));
        let ids = Arc::new(Mutex::new(HashMap::new()));
        let next = Arc::new(AtomicU32::new(0));
        let logs = Arc::new(Mutex::new(Vec::new()));

        if caps.can(Capability::Log) {
            let q = Arc::clone(&queue);
            engine.register_fn("log", move |msg: &str| {
                if let Ok(mut v) = q.lock() {
                    v.push(ScriptCommand::Log(msg.to_string()));
                }
            });
        }

        if caps.can(Capability::Spawn) {
            let q = Arc::clone(&queue);
            let next = Arc::clone(&next);
            let ids = Arc::clone(&ids);
            engine.register_fn("spawn_entity", move || -> i64 {
                let id = next.fetch_add(1, Ordering::Relaxed);
                if let Ok(mut m) = ids.lock() {
                    m.insert(id, None);
                }
                if let Ok(mut v) = q.lock() {
                    v.push(ScriptCommand::Spawn { id });
                }
                id as i64
            });
        }

        if caps.can(Capability::WriteWorld) {
            let q = Arc::clone(&queue);
            engine.register_fn(
                "set_transform",
                move |id: i64,
                      px: f64,
                      py: f64,
                      pz: f64,
                      rx: f64,
                      ry: f64,
                      rz: f64,
                      sx: f64,
                      sy: f64,
                      sz: f64| {
                    if let Ok(mut v) = q.lock() {
                        v.push(ScriptCommand::SetTransform {
                            id: id as u32,
                            pos: (px as f32, py as f32, pz as f32),
                            rot_euler: (rx as f32, ry as f32, rz as f32),
                            scale: (sx as f32, sy as f32, sz as f32),
                        });
                    }
                },
            );

            let q = Arc::clone(&queue);
            engine.register_fn("emit_event", move |name: &str| {
                if let Ok(mut v) = q.lock() {
                    v.push(ScriptCommand::EmitEvent(name.to_string()));
                }
            });
        }

        EmbeddedRuntime {
            engine,
            caps,
            queue,
            ids,
            logs,
        }
    }

    /// The capabilities this runtime was built with.
    pub fn capabilities(&self) -> Capabilities {
        self.caps
    }

    /// Compile and run `code`. Commands are enqueued for [`apply`](Self::apply).
    pub fn run(&mut self, code: &str) -> Result<(), ScriptError> {
        self.engine.run(code)?;
        Ok(())
    }

    /// Number of commands currently waiting to be applied.
    pub fn pending(&self) -> usize {
        self.queue.lock().map(|v| v.len()).unwrap_or(0)
    }

    /// Drain enqueued commands and apply them to `world`.
    pub fn apply(&mut self, world: &mut World) {
        let cmds = {
            match self.queue.lock() {
                Ok(mut q) => std::mem::take(&mut *q),
                Err(_) => return,
            }
        };
        let mut ids = match self.ids.lock() {
            Ok(ids) => ids,
            Err(_) => return,
        };
        for cmd in cmds {
            match cmd {
                ScriptCommand::Spawn { id } => {
                    let e = world.spawn();
                    world.add_component(e, Transform::default());
                    ids.insert(id, Some(e));
                }
                ScriptCommand::SetTransform {
                    id,
                    pos,
                    rot_euler,
                    scale,
                } => {
                    if let Some(Some(e)) = ids.get(&id) {
                        let t = Transform {
                            translation: Vec3::new(pos.0, pos.1, pos.2),
                            rotation: Quat::from_euler(
                                glam::EulerRot::XYZ,
                                rot_euler.0,
                                rot_euler.1,
                                rot_euler.2,
                            ),
                            scale: Vec3::new(scale.0, scale.1, scale.2),
                        };
                        world.add_component(*e, t);
                    }
                }
                ScriptCommand::Log(msg) => {
                    if let Ok(mut l) = self.logs.lock() {
                        l.push(msg);
                    }
                }
                ScriptCommand::EmitEvent(name) => {
                    if let Ok(mut l) = self.logs.lock() {
                        l.push(format!("[event] {name}"));
                    }
                }
            }
        }
    }

    /// Convenience: run a script and immediately apply its commands.
    pub fn run_and_apply(&mut self, code: &str, world: &mut World) -> Result<(), ScriptError> {
        self.run(code)?;
        self.apply(world);
        Ok(())
    }

    /// Take the log lines captured since the last call.
    pub fn take_logs(&mut self) -> Vec<String> {
        match self.logs.lock() {
            Ok(mut l) => std::mem::take(&mut *l),
            Err(_) => Vec::new(),
        }
    }
}

impl Default for EmbeddedRuntime {
    fn default() -> Self {
        EmbeddedRuntime::new(Capabilities::all())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nova_ecs::Transform;

    #[test]
    fn capabilities_serialize_round_trip() {
        let caps = Capabilities::none()
            .grant(Capability::Spawn)
            .grant(Capability::Log)
            .clone();
        let json = serde_json::to_string(&caps).unwrap();
        let back: Capabilities = serde_json::from_str(&json).unwrap();
        assert!(back.can(Capability::Spawn));
        assert!(back.can(Capability::Log));
        assert!(!back.can(Capability::WriteWorld));
    }

    #[test]
    fn full_capabilities_spawn_and_transform_entities() {
        let mut rt = EmbeddedRuntime::new(Capabilities::all());
        let mut world = World::new();
        let code = r#"
            let a = spawn_entity();
            set_transform(a, 1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0);
            let b = spawn_entity();
            set_transform(b, 4.0, 5.0, 6.0, 0.0, 0.0, 0.0, 2.0, 2.0, 2.0);
            log("spawned two entities");
            emit_event("ready");
        "#;
        rt.run_and_apply(code, &mut world).expect("script runs");
        assert_eq!(world.entity_count(), 2);
        assert_eq!(
            rt.take_logs(),
            vec!["spawned two entities", "[event] ready"]
        );
    }

    #[test]
    fn denied_capability_blocks_the_function() {
        // No capabilities at all: spawn_entity must not even exist.
        let mut rt = EmbeddedRuntime::new(Capabilities::none());
        let mut world = World::new();
        let err = rt.run_and_apply("spawn_entity();", &mut world).unwrap_err();
        assert!(rt.pending() == 0, "no commands should be queued");
        assert!(world.entity_count() == 0);
        assert!(format!("{err}").contains("spawn_entity"));
    }

    #[test]
    fn log_only_cannot_spawn() {
        let caps = Capabilities::none().grant(Capability::Log).clone();
        let mut rt = EmbeddedRuntime::new(caps);
        let mut world = World::new();
        rt.run_and_apply(r#"log("hello from sandbox"); spawn_entity();"#, &mut world)
            .expect_err("spawn_entity is not registered under log-only caps");
        rt.apply(&mut world);
        // Only the log command is valid; spawn errored before queueing anything.
        assert_eq!(rt.take_logs(), vec!["hello from sandbox"]);
        assert_eq!(world.entity_count(), 0);
    }

    #[test]
    fn transform_is_applied_to_world() {
        let mut rt = EmbeddedRuntime::new(Capabilities::all());
        let mut world = World::new();
        rt.run_and_apply(
            "let e = spawn_entity(); set_transform(e, 7.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0);",
            &mut world,
        )
        .unwrap();
        let entities = world.entities();
        let t = world.get_component::<Transform>(entities[0]).unwrap();
        assert_eq!(t.translation, Vec3::new(7.0, 0.0, 0.0));
    }
}
