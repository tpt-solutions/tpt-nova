//! A stable, versioned API surface for external AI agents to drive Nova.
//!
//! The engine already emits machine-readable telemetry (`nova-telemetry`) and
//! hot-applies a control file written by an external process (see `nova-app`).
//! This crate formalizes that loop into a **public, semver-stable contract**:
//!
//! * [`PROTOCOL_VERSION`] identifies the command schema so older agents fail
//!   loudly instead of silently mis-driving a newer engine,
//! * [`AgentCommand`] is the closed set of mutations an agent may request
//!   (spawn/despawn/transform — never arbitrary memory or code),
//! * [`apply_command`] / [`apply_commands`] turn parsed commands into ECS
//!   mutations,
//! * [`ControlChannel`] reads a control file on change and applies it, exactly
//!   like the shipped app but reusable by any host (editor, test harness, CI
//!   agent),
//! * [`read_telemetry_file`] lets an agent read back the same world snapshot the
//!   engine emits.
//!
//! By keeping the agent's power bounded to this command set, the self-debugging
//! loop stays safe: an agent can reposition and spawn things, but cannot
//! corrupt engine internals or execute arbitrary code.

use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use glam::EulerRot;
use nova_ecs::transform::{Mesh, MeshKind, Transform};
use nova_ecs::{Entity, Quat, Vec3, World};
use serde::{Deserialize, Serialize};

/// The command-schema version this build understands. Bump on any breaking
/// change to [`AgentCommand`] or [`ControlFile`] so mismatched agents are
/// rejected rather than mis-applied.
pub const PROTOCOL_VERSION: u32 = 1;

/// Errors raised while parsing or applying agent commands.
#[derive(Debug, thiserror::Error)]
pub enum AgentApiError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("control file protocol {found} != supported {supported}")]
    ProtocolMismatch { found: u32, supported: u32 },
    #[error("unknown entity reference: {0}")]
    UnknownEntity(String),
    #[error("malformed entity id (expected e<index>#<generation>): {0}")]
    MalformedEntityId(String),
    #[error("control file changed but failed to parse: {0}")]
    ControlParse(String),
}

/// How an agent names the entity it wants to act on.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum EntityRef {
    /// A telemetry id, e.g. `"e3#0"`.
    Id(String),
    /// A stable name registered when the entity was spawned.
    Name(String),
}

impl EntityRef {
    /// Parse a telemetry id string of the form `e<index>#<generation>`.
    pub fn from_id_string(s: &str) -> Result<Entity, AgentApiError> {
        let s = s.trim();
        let (idx_part, gen_part) = s
            .split_once('#')
            .ok_or_else(|| AgentApiError::MalformedEntityId(s.to_string()))?;
        let idx_str = idx_part
            .strip_prefix('e')
            .ok_or_else(|| AgentApiError::MalformedEntityId(s.to_string()))?;
        let index = idx_str
            .parse::<u32>()
            .map_err(|_| AgentApiError::MalformedEntityId(s.to_string()))?;
        let generation = gen_part
            .parse::<u32>()
            .map_err(|_| AgentApiError::MalformedEntityId(s.to_string()))?;
        Ok(Entity { index, generation })
    }
}

/// A single mutation an external agent may request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "op")]
pub enum AgentCommand {
    /// Spawn a new entity. `name`, if set, is registered for later reference.
    Spawn {
        name: Option<String>,
        translation: [f32; 3],
        mesh: Option<String>,
    },
    /// Remove an entity from the world.
    Despawn { target: EntityRef },
    /// Set the local transform of an entity.
    SetTransform {
        target: EntityRef,
        translation: Option<[f32; 3]>,
        rotation_euler_xyz: Option<[f32; 3]>,
        scale: Option<[f32; 3]>,
    },
    /// Set only the rotation (Euler XYZ radians) of an entity.
    SetRotation {
        target: EntityRef,
        rotation_euler_xyz: [f32; 3],
    },
}

/// The on-disk control file an agent writes for the engine to pick up.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ControlFile {
    pub protocol: u32,
    #[serde(default)]
    pub commands: Vec<AgentCommand>,
}

impl ControlFile {
    pub fn new(commands: Vec<AgentCommand>) -> Self {
        ControlFile {
            protocol: PROTOCOL_VERSION,
            commands,
        }
    }

    pub fn to_json(&self) -> Result<String, AgentApiError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn from_json(text: &str) -> Result<ControlFile, AgentApiError> {
        Ok(serde_json::from_str(text)?)
    }
}

/// A name->entity registry, stored as a world resource so spawned entities can
/// be referenced by stable name across ticks.
#[derive(Debug, Default)]
pub struct EntityRegistry {
    by_name: std::collections::HashMap<String, Entity>,
}

impl EntityRegistry {
    pub fn register(&mut self, name: &str, entity: Entity) {
        self.by_name.insert(name.to_string(), entity);
    }

    pub fn lookup(&self, name: &str) -> Option<Entity> {
        self.by_name.get(name).copied()
    }

    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }
}

/// Resolve an [`EntityRef`] to a live [`Entity`] in `world`.
fn resolve(world: &World, r: &EntityRef) -> Result<Entity, AgentApiError> {
    match r {
        EntityRef::Id(s) => {
            let want = EntityRef::from_id_string(s)?;
            // Verify the handle is still alive by matching a live entity.
            if world.entities().contains(&want) {
                Ok(want)
            } else {
                Err(AgentApiError::UnknownEntity(s.clone()))
            }
        }
        EntityRef::Name(name) => {
            let reg = world
                .resource::<EntityRegistry>()
                .ok_or_else(|| AgentApiError::UnknownEntity(name.clone()))?;
            reg.lookup(name)
                .ok_or_else(|| AgentApiError::UnknownEntity(name.clone()))
        }
    }
}

/// Apply one command to the world. Spawns register names into the
/// [`EntityRegistry`] resource (created on demand).
pub fn apply_command(world: &mut World, cmd: &AgentCommand) -> Result<(), AgentApiError> {
    match cmd {
        AgentCommand::Spawn {
            name,
            translation,
            mesh,
        } => {
            let e = world.spawn();
            world.add_component(
                e,
                Transform::from_translation(Vec3::new(
                    translation[0],
                    translation[1],
                    translation[2],
                )),
            );
            if let Some(m) = mesh {
                let kind = match m.as_str() {
                    "cube" => MeshKind::Cube,
                    _ => MeshKind::Cube, // unknown meshes fall back to a cube
                };
                world.add_component(e, Mesh { kind });
            }
            if let Some(n) = name {
                if !world.has_resource::<EntityRegistry>() {
                    world.add_resource(EntityRegistry::default());
                }
                world
                    .resource_mut::<EntityRegistry>()
                    .unwrap()
                    .register(n, e);
            }
            Ok(())
        }
        AgentCommand::Despawn { target } => {
            let e = resolve(world, target)?;
            if !world.entities().contains(&e) {
                return Err(AgentApiError::UnknownEntity(format!("{e}")));
            }
            world.despawn(e);
            Ok(())
        }
        AgentCommand::SetTransform {
            target,
            translation,
            rotation_euler_xyz,
            scale,
        } => {
            let e = resolve(world, target)?;
            let t = world
                .get_component_mut::<Transform>(e)
                .ok_or_else(|| AgentApiError::UnknownEntity(format!("{e}")))?;
            if let Some(tr) = translation {
                t.translation = Vec3::new(tr[0], tr[1], tr[2]);
            }
            if let Some(r) = rotation_euler_xyz {
                t.rotation = Quat::from_euler(EulerRot::XYZ, r[0], r[1], r[2]);
            }
            if let Some(s) = scale {
                t.scale = Vec3::new(s[0], s[1], s[2]);
            }
            Ok(())
        }
        AgentCommand::SetRotation {
            target,
            rotation_euler_xyz,
        } => {
            let e = resolve(world, target)?;
            let t = world
                .get_component_mut::<Transform>(e)
                .ok_or_else(|| AgentApiError::UnknownEntity(format!("{e}")))?;
            t.rotation = Quat::from_euler(
                EulerRot::XYZ,
                rotation_euler_xyz[0],
                rotation_euler_xyz[1],
                rotation_euler_xyz[2],
            );
            Ok(())
        }
    }
}

/// Apply a batch of commands in order, stopping at the first error.
pub fn apply_commands(world: &mut World, commands: &[AgentCommand]) -> Result<(), AgentApiError> {
    for c in commands {
        apply_command(world, c)?;
    }
    Ok(())
}

/// Read a telemetry snapshot the engine wrote to `path` (JSON, as produced by
/// `nova-telemetry::FileSink`).
pub fn read_telemetry_file<P: AsRef<Path>>(
    path: P,
) -> Result<nova_telemetry::TelemetryFrame, AgentApiError> {
    let text = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&text)?)
}

/// A file-backed control channel: the agent writes [`ControlFile`] JSON to a
/// path; the host polls it and applies any *new* version (by mtime) exactly
/// once. This is the same hot-apply loop `nova-app` runs, factored out so the
/// editor, tests, and CI agents share one implementation.
pub struct ControlChannel {
    path: PathBuf,
    last_mtime: Option<u64>,
    applied_count: u64,
}

impl ControlChannel {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        ControlChannel {
            path: path.as_ref().to_path_buf(),
            last_mtime: None,
            applied_count: 0,
        }
    }

    /// Total commands applied across the channel's lifetime.
    pub fn applied_count(&self) -> u64 {
        self.applied_count
    }

    /// Poll the control file. Returns the number of commands applied this call
    /// (0 if unchanged or absent). Rejects protocol-version mismatches.
    pub fn poll(&mut self, world: &mut World) -> Result<u32, AgentApiError> {
        let meta = match std::fs::metadata(&self.path) {
            Ok(m) => m,
            Err(_) => return Ok(0),
        };
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d: Duration| d.as_millis() as u64);
        if mtime == self.last_mtime {
            return Ok(0);
        }
        self.last_mtime = mtime;

        let text = std::fs::read_to_string(&self.path)?;
        let file = ControlFile::from_json(&text)
            .map_err(|e| AgentApiError::ControlParse(e.to_string()))?;
        if file.protocol != PROTOCOL_VERSION {
            return Err(AgentApiError::ProtocolMismatch {
                found: file.protocol,
                supported: PROTOCOL_VERSION,
            });
        }
        let n = file.commands.len() as u32;
        apply_commands(world, &file.commands)?;
        self.applied_count += n as u64;
        Ok(n)
    }
}

/// RAG-backed retrieval wired into the agent control loop.
///
/// An AI agent driving the engine (via [`apply_command`] / [`ControlChannel`])
/// frequently needs *project context* to decide what to change. [`RagAssistant`]
/// indexes the source tree/assets once, then answers natural-language queries
/// with a prompt-ready context block the agent can fold into a fix request (see
/// `nova-overlay`'s `AiFixRequest`) or print before issuing [`AgentCommand`]s.
///
/// This is the concrete "RAG-backed doc/context queries wired into AI agent
/// integration" bridge: the agent's stable surface (`nova-agent-api`) gains a
/// retrieval helper without coupling the core protocol to any embedding model.
#[cfg(feature = "rag")]
pub mod rag {
    use std::path::Path;

    use nova_rag::{index_directory, RagAgent, ScoredHit, SearchError};

    /// Errors raised by the RAG-backed assistant.
    #[derive(Debug, thiserror::Error)]
    pub enum RagAssistantError {
        #[error("rag index/search error: {0}")]
        Search(#[from] SearchError),
    }

    /// A RAG-backed assistant bound to a loaded project index.
    pub struct RagAssistant {
        agent: RagAgent,
    }

    impl RagAssistant {
        /// Build an assistant by indexing `root`, filtered to `extensions`
        /// (empty = all files; hidden / build dirs are skipped by the indexer).
        pub fn index_project(
            root: impl AsRef<Path>,
            extensions: &[&str],
        ) -> Result<Self, RagAssistantError> {
            let index = index_directory(root, extensions)?;
            Ok(RagAssistant {
                agent: RagAgent::new(index, 4),
            })
        }

        /// Build an assistant from an already-loaded [`nova_rag::Index`].
        pub fn from_index(index: nova_rag::Index) -> Self {
            RagAssistant {
                agent: RagAgent::new(index, 4),
            }
        }

        /// Retrieve the top hits for `query` (with scores) so an agent can cite
        /// the exact sources it based a change on.
        pub fn retrieve(&self, query: &str) -> Result<Vec<ScoredHit>, RagAssistantError> {
            Ok(self.agent.retrieve(query)?)
        }

        /// Retrieve a prompt-ready context block for `query` — the same string an
        /// agent would paste above its instruction when asking an LLM to drive the
        /// engine.
        pub fn context_for(&self, query: &str) -> Result<String, RagAssistantError> {
            Ok(self.agent.build_context(query)?)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use nova_rag::Index;

        #[test]
        fn assistant_retrieves_context_for_query() {
            let mut idx = Index::default_new();
            idx.add_documents([
                nova_rag::Document::new("a", "physics body collider rapier integration step"),
                nova_rag::Document::new("b", "render wgpu pipeline shader vertex fragment"),
            ]);
            let assist = RagAssistant::from_index(idx);
            let ctx = assist.context_for("physics collider rapier").unwrap();
            assert!(ctx.contains("physics body collider"));
            assert!(ctx.contains("score="));

            let hits = assist.retrieve("wgpu shader pipeline").unwrap();
            assert_eq!(hits[0].document.id, "b");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nova_ecs::transform::MeshKind;
    use nova_ecs::World;

    fn world_with_cube() -> (World, Entity) {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::default());
        world.add_component(
            e,
            Mesh {
                kind: MeshKind::Cube,
            },
        );
        (world, e)
    }

    #[test]
    fn entity_id_string_roundtrips() {
        let id = "e3#7";
        let e = EntityRef::from_id_string(id).unwrap();
        assert_eq!(e.index, 3);
        assert_eq!(e.generation, 7);
        assert!(EntityRef::from_id_string("nope").is_err());
        assert!(EntityRef::from_id_string("e3").is_err());
    }

    #[test]
    fn apply_set_transform_mutates_component() {
        let (mut world, e) = world_with_cube();
        let id = format!("{e}");
        apply_command(
            &mut world,
            &AgentCommand::SetTransform {
                target: EntityRef::Id(id),
                translation: Some([1.0, 2.0, 3.0]),
                rotation_euler_xyz: Some([0.0, std::f32::consts::FRAC_PI_2, 0.0]),
                scale: None,
            },
        )
        .unwrap();
        let t = world.get_component::<Transform>(e).unwrap();
        assert_eq!(t.translation, Vec3::new(1.0, 2.0, 3.0));
        // ~90° rotation about Y should be a unit quaternion.
        assert!((t.rotation.length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn apply_spawn_registers_name_and_mesh() {
        let mut world = World::new();
        apply_command(
            &mut world,
            &AgentCommand::Spawn {
                name: Some("player".into()),
                translation: [5.0, 0.0, 0.0],
                mesh: Some("cube".into()),
            },
        )
        .unwrap();
        assert_eq!(world.entity_count(), 1);
        let reg = world.resource::<EntityRegistry>().unwrap();
        let e = reg.lookup("player").unwrap();
        assert!(world.has_component::<Mesh>(e));
        let t = world.get_component::<Transform>(e).unwrap();
        assert_eq!(t.translation, Vec3::new(5.0, 0.0, 0.0));
    }

    #[test]
    fn apply_unknown_entity_errors() {
        let (mut world, _e) = world_with_cube();
        let err = apply_command(
            &mut world,
            &AgentCommand::SetRotation {
                target: EntityRef::Id("e999#0".into()),
                rotation_euler_xyz: [0.0, 0.0, 0.0],
            },
        );
        assert!(matches!(err, Err(AgentApiError::UnknownEntity(_))));
    }

    #[test]
    fn control_file_roundtrips_json() {
        let cf = ControlFile::new(vec![AgentCommand::SetRotation {
            target: EntityRef::Id("e0#0".into()),
            rotation_euler_xyz: [0.1, 0.2, 0.3],
        }]);
        let json = cf.to_json().unwrap();
        let back = ControlFile::from_json(&json).unwrap();
        assert_eq!(cf, back);
        assert_eq!(back.protocol, PROTOCOL_VERSION);
    }

    #[test]
    fn channel_polls_once_per_write_and_is_idempotent() {
        let dir = std::env::temp_dir();
        let path = dir.join("nova_agent_api_test.json");
        let _ = std::fs::remove_file(&path);

        let mut world = World::new();
        let mut ch = ControlChannel::new(&path);

        // No file yet -> nothing applied.
        assert_eq!(ch.poll(&mut world).unwrap(), 0);

        // Agent writes a spawn command.
        let cf = ControlFile::new(vec![AgentCommand::Spawn {
            name: Some("a".into()),
            translation: [0.0, 0.0, 0.0],
            mesh: None,
        }]);
        std::fs::write(&path, cf.to_json().unwrap()).unwrap();
        std::thread::sleep(Duration::from_millis(20));

        assert_eq!(ch.poll(&mut world).unwrap(), 1);
        assert_eq!(world.entity_count(), 1);

        // Polling again without a rewrite applies nothing (idempotent).
        assert_eq!(ch.poll(&mut world).unwrap(), 0);
        assert_eq!(ch.applied_count(), 1);

        // A new file with a despawn applies again.
        let reg = world
            .resource::<EntityRegistry>()
            .unwrap()
            .lookup("a")
            .unwrap();
        let cf2 = ControlFile::new(vec![AgentCommand::Despawn {
            target: EntityRef::Name("a".into()),
        }]);
        std::fs::write(&path, cf2.to_json().unwrap()).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        let _ = reg;
        assert_eq!(ch.poll(&mut world).unwrap(), 1);
        assert_eq!(world.entity_count(), 0);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn channel_rejects_protocol_mismatch() {
        let dir = std::env::temp_dir();
        let path = dir.join("nova_agent_api_proto_test.json");
        let _ = std::fs::remove_file(&path);
        let mut world = World::new();
        let mut ch = ControlChannel::new(&path);
        let bad = ControlFile {
            protocol: PROTOCOL_VERSION + 1,
            commands: vec![],
        };
        std::fs::write(&path, bad.to_json().unwrap()).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        let err = ch.poll(&mut world);
        assert!(matches!(err, Err(AgentApiError::ProtocolMismatch { .. })));
        let _ = std::fs::remove_file(&path);
    }
}
